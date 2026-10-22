//! 播放：音效在内存里（44.1kHz 单声道 16 位），按键时只做一次非阻塞调用。
//!
//! - Windows：自己的音频线程，[`crate::mixer`] 叠加多个声部，钢琴余音不会被下一键截断。
//!   输出走 WASAPI（[`crate::wasapi`]），打不开时退回 waveOut。
//! - Linux：依次尝试 `pw-play` / `paplay` / `aplay`；macOS：`afplay`。外部命令要文件，
//!   音效先落到扩展目录的 `sounds/.cache/`。外部命令有启动开销，同时在放的超过
//!   [`MAX_INFLIGHT`] 个就跳过，免得快速连打时堆积。

use std::collections::HashMap;
use std::sync::Arc;

pub struct Player {
    clips: HashMap<String, Arc<[i16]>>,
    #[cfg(windows)]
    out: Option<std::sync::mpsc::Sender<(Arc<[i16]>, f32)>>,
    #[cfg(windows)]
    backend: &'static str,
    #[cfg(not(windows))]
    cache: std::path::PathBuf,
    #[cfg(not(windows))]
    files: HashMap<String, std::path::PathBuf>,
    #[cfg(not(windows))]
    backend: Option<&'static str>,
    #[cfg(not(windows))]
    inflight: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(not(windows))]
const MAX_INFLIGHT: usize = 6;

impl Player {
    #[cfg(windows)]
    pub fn new(_dir: &std::path::Path) -> Player {
        let (out, backend) = match crate::wasapi::spawn() {
            Some(tx) => (Some(tx), "WASAPI 混音"),
            None => match win::spawn() {
                Some(tx) => (Some(tx), "waveOut 混音"),
                None => (None, "无（打开音频设备失败）"),
            },
        };
        Player {
            clips: HashMap::new(),
            out,
            backend,
        }
    }

    #[cfg(not(windows))]
    pub fn new(dir: &std::path::Path) -> Player {
        Player {
            clips: HashMap::new(),
            cache: dir.join("sounds").join(".cache"),
            files: HashMap::new(),
            backend: find_backend(),
            inflight: Default::default(),
        }
    }

    /// 换一套音效（启动与配置变更时）。
    pub fn set_clips(&mut self, clips: HashMap<String, Vec<i16>>) {
        self.clips = clips.into_iter().map(|(k, v)| (k, v.into())).collect();
        #[cfg(not(windows))]
        self.write_cache();
    }

    pub fn play(&self, name: &str, volume: f32) {
        let Some(pcm) = self.clips.get(name) else {
            return;
        };
        #[cfg(windows)]
        if let Some(out) = &self.out {
            let _ = out.send((pcm.clone(), volume));
        }
        #[cfg(not(windows))]
        self.spawn(name, pcm, volume);
    }

    /// 把送进声卡的混音录到 `path`（追加）；`None` 停录。只 Windows 有自己的混音。
    pub fn set_record(&self, path: Option<&std::path::Path>) {
        #[cfg(windows)]
        {
            let file = path.and_then(|p| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .ok()
            });
            *win::RECORD.lock().unwrap_or_else(|e| e.into_inner()) = file;
        }
        #[cfg(not(windows))]
        let _ = path;
    }

    pub fn backend_name(&self) -> &'static str {
        #[cfg(windows)]
        return self.backend;
        #[cfg(not(windows))]
        return self.backend.unwrap_or("无");
    }
}

/// waveOut 后端（WASAPI 打不开时的退路），以及两个后端共用的保活、断流计数、调试录音。
#[cfg(windows)]
pub(crate) mod win {
    use std::sync::Arc;
    use std::sync::mpsc::{Receiver, Sender, channel};

    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Media::Audio::{
        CALLBACK_EVENT, HWAVEOUT, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE,
        waveOutOpen, waveOutPrepareHeader, waveOutWrite,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        WaitForSingleObject,
    };
    use windows::core::PSTR;

    use crate::mixer::Mixer;
    use crate::wav::RATE;

    /// 每块 11.6ms，排 4 块：缓冲延迟约 46ms。块再小，系统稍一卡就断流，断处是爆音。
    const FRAMES: usize = 512;
    const BUFS: usize = 4;
    /// 最后一个音放完后继续送静音、让输出流不断的时长。流一断，下次出声要冷启动，
    /// Windows 音频引擎在冷启动的头一小段会出杂音（有线、蓝牙都一样）——表现为「停一会儿
    /// 再打，头几个键带滋声，连续打就没事」。不一直保活：有在播的音频流会挡系统睡眠。
    pub(crate) const KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(120);

    pub(crate) type Cmd = (Arc<[i16]>, f32);

    /// 调试录音（见 [`super::Player::set_record`]）。音频线程只 try_lock，拿不到就跳过这一块。
    pub static RECORD: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

    pub(crate) fn record(pcm: &[i16]) {
        if let Ok(mut rec) = RECORD.try_lock()
            && let Some(f) = rec.as_mut()
        {
            use std::io::Write;
            let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
            let _ = f.write_all(&bytes);
        }
    }

    pub fn spawn() -> Option<Sender<Cmd>> {
        let (tx, rx) = channel::<Cmd>();
        let (ready_tx, ready_rx) = channel::<bool>();
        std::thread::Builder::new()
            .name("keysound-audio".into())
            .spawn(move || run(rx, ready_tx))
            .ok()?;
        ready_rx.recv().ok().filter(|ok| *ok).map(|_| tx)
    }

    /// 断流计数：一直在出声、回来时却发现排队的块全播完了，设备就空转过一段——那里是爆音。
    /// 写进扩展日志，至多 10 秒一行。
    #[derive(Default)]
    pub(crate) struct Underruns {
        pub(crate) streaming: bool,
        count: u64,
        logged: Option<std::time::Instant>,
    }

    impl Underruns {
        pub(crate) fn check(&mut self, all_drained: bool) {
            if !(self.streaming && all_drained) {
                return;
            }
            self.count += 1;
            if self.logged.is_none_or(|t| t.elapsed().as_secs() >= 10) {
                self.logged = Some(std::time::Instant::now());
                eprintln!(
                    "{} [keysound] 音频断流累计 {} 次",
                    wind_hook::stamp::utc_stamp(),
                    self.count
                );
            }
        }
    }

    struct Buf {
        hdr: Box<WAVEHDR>,
        pcm: Box<[i16; FRAMES]>,
    }

    fn run(rx: Receiver<Cmd>, ready: Sender<bool>) {
        // SAFETY: 以下全是 winmm / kernel32 调用；WAVEHDR 与采样缓冲都装箱，地址在线程存活期间不变，
        // 设备从不关闭（扩展退出即进程退出），故驱动持有的指针始终有效。
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            let Ok(event) = CreateEventW(None, false, false, None) else {
                let _ = ready.send(false);
                return;
            };
            let fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let mut dev = HWAVEOUT::default();
            if waveOutOpen(
                Some(&mut dev),
                WAVE_MAPPER,
                &fmt,
                event.0 as usize,
                0,
                CALLBACK_EVENT,
            ) != 0
            {
                eprintln!(
                    "{} [keysound] 打开音频设备失败",
                    wind_hook::stamp::utc_stamp()
                );
                let _ = CloseHandle(event);
                let _ = ready.send(false);
                return;
            }
            let hdr_size = std::mem::size_of::<WAVEHDR>() as u32;
            let mut bufs: Vec<Buf> = (0..BUFS)
                .map(|_| {
                    let mut b = Buf {
                        hdr: Box::new(WAVEHDR::default()),
                        pcm: Box::new([0; FRAMES]),
                    };
                    b.hdr.lpData = PSTR(b.pcm.as_mut_ptr().cast());
                    b.hdr.dwBufferLength = (FRAMES * 2) as u32;
                    waveOutPrepareHeader(dev, &mut *b.hdr, hdr_size);
                    b.hdr.dwFlags |= WHDR_DONE; // 还没交给设备，视同已播完
                    b
                })
                .collect();
            let _ = ready.send(true);

            let mut mixer = Mixer::default();
            let mut underruns = Underruns::default();
            let mut last_sound: Option<std::time::Instant> = None;
            loop {
                let alive = !mixer.is_idle() || last_sound.is_some_and(|t| t.elapsed() < KEEPALIVE);
                if !alive {
                    // 保活期过了：整条线程睡在通道上，不占 CPU，输出流随之停下。
                    underruns.streaming = false;
                    let Ok((pcm, gain)) = rx.recv() else { return };
                    mixer.start(pcm, gain);
                }
                while let Ok((pcm, gain)) = rx.try_recv() {
                    mixer.start(pcm, gain);
                }
                if !mixer.is_idle() {
                    last_sound = Some(std::time::Instant::now());
                }
                underruns.check(bufs.iter().all(|b| b.hdr.dwFlags & WHDR_DONE != 0));
                for b in bufs.iter_mut().filter(|b| b.hdr.dwFlags & WHDR_DONE != 0) {
                    let silent = mixer.is_idle();
                    mixer.fill(&mut b.pcm[..]); // 没有声部时写静音
                    if !silent {
                        record(&b.pcm[..]);
                    }
                    b.hdr.dwFlags &= !WHDR_DONE;
                    waveOutWrite(dev, &mut *b.hdr, hdr_size);
                }
                underruns.streaming = true;
                // 设备每播完一块置一次事件；超时只是兜底，不依赖它。
                WaitForSingleObject(event, 20);
            }
        }
    }
}

#[cfg(not(windows))]
impl Player {
    fn write_cache(&mut self) {
        self.files.clear();
        if let Err(e) = std::fs::create_dir_all(&self.cache) {
            eprintln!(
                "{} [keysound] 建 {} 失败: {e}",
                wind_hook::stamp::utc_stamp(),
                self.cache.display()
            );
            return;
        }
        for (name, pcm) in &self.clips {
            let path = self.cache.join(format!("{name}.wav"));
            match std::fs::write(&path, crate::wav::encode(pcm)) {
                Ok(()) => {
                    self.files.insert(name.clone(), path);
                }
                Err(e) => eprintln!(
                    "{} [keysound] 写 {} 失败: {e}",
                    wind_hook::stamp::utc_stamp(),
                    path.display()
                ),
            }
        }
    }

    fn spawn(&self, name: &str, _pcm: &Arc<[i16]>, volume: f32) {
        use std::sync::atomic::Ordering;
        let (Some(cmd), Some(file)) = (self.backend, self.files.get(name)) else {
            return;
        };
        if self.inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
            return;
        }
        let mut c = std::process::Command::new(cmd);
        match cmd {
            "aplay" => {
                c.arg("-q");
            }
            "pw-play" => {
                c.arg(format!("--volume={volume:.2}"));
            }
            "paplay" => {
                c.arg(format!("--volume={}", (volume * 65536.0) as u32));
            }
            "afplay" => {
                c.args(["-v", &format!("{volume:.2}")]);
            }
            _ => {}
        }
        let spawned = c
            .arg(file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            let inflight = self.inflight.clone();
            inflight.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let _ = child.wait();
                inflight.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }
}

#[cfg(not(windows))]
fn find_backend() -> Option<&'static str> {
    let candidates: &[&'static str] = if cfg!(target_os = "macos") {
        &["afplay"]
    } else {
        &["pw-play", "paplay", "aplay"]
    };
    let path = std::env::var_os("PATH")?;
    candidates
        .iter()
        .copied()
        .find(|c| std::env::split_paths(&path).any(|d| d.join(c).is_file()))
}
