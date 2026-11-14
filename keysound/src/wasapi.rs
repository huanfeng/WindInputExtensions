// SPDX-License-Identifier: MPL-2.0
//! Windows 主播放后端：WASAPI 共享模式、事件驱动。
//!
//! waveOut 是老接口，在系统里要再转一层才到音频引擎；按键音要低延迟、缓冲给得小，每次输出流
//! 冷启动开头都带一下杂音。播放器、游戏用的都是 WASAPI，标准做法是**先把缓冲填满再启动**，
//! 开头就是干净的。这里照此做：44.1kHz 单声道 16 位交给系统自动转换（AUTOCONVERTPCM），
//! 缓冲约 30ms。保活、断流计数、调试录音与 waveOut 后端同一套（见 `player::win`）。
//!
//! 默认输出设备变了（连上蓝牙、拔耳机）要跟过去：旧设备未必失效（虚拟声卡一直都在），
//! 不查就会一直往旧设备送。从空闲醒来时、出声期间每隔 [`RETRY`] 比一次默认设备，变了就重开；
//! 流出错也丢掉重开。

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
    IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM,
    WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject,
};
use windows::core::w;

use crate::mixer::Mixer;
use crate::player::win::{Cmd, KEEPALIVE, Underruns, record};
use crate::wav::RATE;

/// 缓冲时长（100ns 单位）：30ms。
const BUFFER_HNS: i64 = 300_000;
/// 打开失败后多久再试；出声期间也按这个间隔查默认设备变没变。
const RETRY: Duration = Duration::from_secs(2);

struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    frames: u32,
    running: bool,
    /// 打开时的设备 id，与当前默认设备比对用。
    device: String,
}

/// 起音频线程；本机打不开 WASAPI 时返回 `None`，调用方退回 waveOut。
pub fn spawn() -> Option<Sender<Cmd>> {
    let (tx, rx) = channel::<Cmd>();
    let (ready_tx, ready_rx) = channel::<bool>();
    std::thread::Builder::new()
        .name("keysound-wasapi".into())
        .spawn(move || run(rx, ready_tx))
        .ok()?;
    ready_rx.recv().ok().filter(|ok| *ok).map(|_| tx)
}

fn log(msg: &str) {
    eprintln!("{} [keysound] {msg}", wind_hook::stamp::utc_stamp());
}

// SAFETY（本文件全部 unsafe）：COM 在本线程以 MTA 初始化，接口对象只在本线程用；
// GetBuffer 给的指针在 ReleaseBuffer 之前有效，且恰好容纳所请求的帧数。
fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let p = device.GetId()?;
        let id = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(id)
    }
}

/// 当前默认输出设备的 id；拿不到（比如一个输出设备都没有）为 `None`。
fn default_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .and_then(|d| device_id(&d))
        .ok()
}

fn friendly_name(device: &IMMDevice) -> String {
    unsafe { device.OpenPropertyStore(STGM_READ) }
        .and_then(|s| unsafe { s.GetValue(&PKEY_Device_FriendlyName) })
        .map(|v| v.to_string())
        .unwrap_or_else(|e| format!("（读不到设备名: {e}）"))
}

fn open(enumerator: &IMMDeviceEnumerator, event: HANDLE) -> windows::core::Result<Stream> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 1,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * 2,
            nBlockAlign: 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            BUFFER_HNS,
            0,
            &fmt,
            None,
        )?;
        client.SetEventHandle(event)?;
        let frames = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        log(&format!("输出设备: {}", friendly_name(&device)));
        Ok(Stream {
            client,
            render,
            frames,
            running: false,
            device: device_id(&device)?,
        })
    }
}

impl Stream {
    /// 把缓冲里空着的部分填上；还没启动就在填满后启动。返回缓冲是否已被放空（断流）。
    fn pump(&mut self, mixer: &mut Mixer) -> windows::core::Result<bool> {
        unsafe {
            let padding = self.client.GetCurrentPadding()?;
            let drained = self.running && padding == 0;
            let avail = self.frames - padding;
            if avail > 0 {
                let silent = mixer.is_idle();
                let ptr = self.render.GetBuffer(avail)?;
                let out = std::slice::from_raw_parts_mut(ptr.cast::<i16>(), avail as usize);
                mixer.fill(out);
                if !silent {
                    record(out);
                }
                self.render.ReleaseBuffer(avail, 0)?;
            }
            if !self.running {
                self.client.Start()?;
                self.running = true;
            }
            Ok(drained)
        }
    }

    fn stop(&mut self) {
        if self.running {
            unsafe {
                let _ = self.client.Stop();
                let _ = self.client.Reset();
            }
            self.running = false;
        }
    }
}

fn run(rx: Receiver<Cmd>, ready: Sender<bool>) {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            let _ = ready.send(false);
            return;
        }
        // 交给 MMCSS 按音频线程调度（播放器的通行做法），比单纯提优先级更稳。
        let mut task = 0u32;
        let _ = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task);
        let Ok(event) = CreateEventW(None, false, false, None) else {
            let _ = ready.send(false);
            return;
        };
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            let _ = ready.send(false);
            return;
        };
        let mut stream = match open(&enumerator, event) {
            Ok(s) => Some(s),
            Err(e) => {
                log(&format!("WASAPI 打不开（{e}），改用 waveOut"));
                let _ = ready.send(false);
                return;
            }
        };
        let _ = ready.send(true);

        let mut mixer = Mixer::default();
        let mut underruns = Underruns::default();
        let mut last_sound: Option<Instant> = None;
        let mut retry_at: Option<Instant> = None;
        let mut checked_at = Instant::now();
        loop {
            let alive = !mixer.is_idle() || last_sound.is_some_and(|t| t.elapsed() < KEEPALIVE);
            let woke = !alive;
            if !alive {
                if let Some(s) = &mut stream {
                    s.stop();
                }
                underruns.streaming = false;
                let Ok((pcm, gain)) = rx.recv() else { return };
                mixer.start(pcm, gain);
            }
            if woke || checked_at.elapsed() >= RETRY {
                checked_at = Instant::now();
                if let Some(s) = &mut stream
                    && default_id(&enumerator).is_some_and(|id| id != s.device)
                {
                    log("默认输出设备变了，改用新设备");
                    s.stop();
                    stream = None;
                    retry_at = None;
                }
            }
            while let Ok((pcm, gain)) = rx.try_recv() {
                mixer.start(pcm, gain);
            }
            if !mixer.is_idle() {
                last_sound = Some(Instant::now());
            }
            if stream.is_none() && retry_at.is_none_or(|t| t.elapsed() >= RETRY) {
                match open(&enumerator, event) {
                    Ok(s) => stream = Some(s),
                    Err(e) => {
                        retry_at = Some(Instant::now());
                        log(&format!("重新打开音频设备失败: {e}"));
                    }
                }
            }
            match stream.as_mut().map(|s| s.pump(&mut mixer)) {
                Some(Ok(drained)) => {
                    underruns.check(drained);
                    underruns.streaming = true;
                    WaitForSingleObject(event, 50);
                }
                Some(Err(e)) => {
                    log(&format!("音频设备失效（{e}），稍后重开"));
                    stream = None;
                    retry_at = None;
                }
                None => {
                    // 没有设备：照样消耗掉声音，免得设备回来时一下涌出一串旧音。
                    let mut sink = [0i16; 512];
                    mixer.fill(&mut sink);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
}
