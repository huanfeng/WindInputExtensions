//! 按键音扩展（扩展程序示例）。由扩展宿主在扩展目录下拉起，stdin 收事件、stdout 发握手，
//! stderr 进宿主写的扩展日志。stdin EOF（宿主退出或停用本扩展）即退出。
//!
//! 扩展目录布局：
//!
//! ```text
//! extensions/keysound/
//!   extension.toml      清单（见本 crate 的 extension.toml）
//!   keysound[.exe]
//!   packs/           音效包：keyboard、typewriter 随扩展分发，custom 放自己的（见 pack.rs）
//!   keysound.toml    可选：音色、音量、自备音效，见 config.rs；改了即时生效
//! ```
#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod dump;
mod mixer;
mod pack;
mod player;
mod synth;
#[cfg(windows)]
mod wasapi;
mod wav;

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use wind_hook::frame::{Frame, read_frame, write_frame};
use wind_hook::msg::{KeyEvent, Msg};

/// 判断事件晚到了多久。事件时间戳是核心的单调时钟，与本进程的时钟没有共同零点；
/// 记下「本地时间 − 事件时间」的最小值当作基准（那一条近似零延迟），之后每条与它比。
#[derive(Default)]
struct Lag {
    base: Option<i64>,
}

impl Lag {
    fn of(&mut self, ts: u64, local_ms: u64) -> u64 {
        let d = local_ms as i64 - ts as i64;
        let base = *self.base.get_or_insert(d);
        let base = base.min(d);
        self.base = Some(base);
        (d - base) as u64
    }
}

macro_rules! log {
    ($($a:tt)*) => { eprintln!("{} [keysound] {}", wind_hook::stamp::utc_stamp(), format_args!($($a)*)) };
}

/// 配置文件改了就重读。只在按键时顺手看一眼，且至多每秒一次。
struct Watch {
    mtime: Option<SystemTime>,
    checked: Instant,
}

impl Watch {
    const EVERY: Duration = Duration::from_secs(1);

    fn new(dir: &Path) -> Watch {
        Watch {
            mtime: config_mtime(dir),
            checked: Instant::now(),
        }
    }

    fn changed(&mut self, dir: &Path) -> bool {
        if self.checked.elapsed() < Self::EVERY {
            return false;
        }
        self.checked = Instant::now();
        let now = config_mtime(dir);
        std::mem::replace(&mut self.mtime, now) != now
    }
}

fn config_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(config::CONFIG_FILE))
        .and_then(|m| m.modified())
        .ok()
}

type Options = serde_json::Map<String, serde_json::Value>;

/// 读配置文件、盖上设置端下发的选项、换上对应的音效。
fn load(dir: &Path, options: &Options, player: &mut player::Player) -> config::SoundMap {
    let mut cfg = config::SoundMap::load(dir).unwrap_or_else(|e| {
        log!("{e}，改用默认配置");
        config::SoundMap::default()
    });
    cfg.apply_options(options);
    player.set_record(cfg.debug.then(|| dir.join("debug_out.pcm")).as_deref());
    player.set_clips(cfg.clips(dir));
    log!("音色 {:?}，音量 {:.2}", cfg.theme, cfg.volume);
    cfg
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, out] = args.as_slice()
        && flag == "--dump"
    {
        if let Err(e) = dump::run(Path::new(out)) {
            eprintln!("导出失败: {e}");
            std::process::exit(1);
        }
        return;
    }
    let dir = std::env::current_dir().unwrap_or_default();
    let mut player = player::Player::new(&dir);
    log!("启动，播放后端: {}", player.backend_name());
    let mut watch = Watch::new(&dir);
    let mut options = Options::new();
    let mut cfg = load(&dir, &options, &mut player);

    let mut stdout = std::io::stdout();
    let hello = Msg::Hello {
        protocol: vec![wind_hook::PROTOCOL_VERSION],
        want: vec!["key.detail".into()],
    };
    if write_frame(&mut stdout, &Frame::new(hello)).is_err() {
        return;
    }

    let start = Instant::now();
    let mut lag = Lag::default();
    let mut stdin = std::io::stdin().lock();
    while let Ok(Some(f)) = read_frame(&mut stdin) {
        match f.msg {
            Msg::Welcome { granted, .. } => {
                if !granted.iter().any(|g| g.starts_with("key.")) {
                    log!("没有获授按键事件，不会出声（请在设置里授予按键权限）");
                }
            }
            Msg::Options { values } => {
                options = values;
                cfg = load(&dir, &options, &mut player);
            }
            Msg::Key(KeyEvent {
                ts,
                key,
                class,
                handled,
                observed,
                ..
            }) => {
                if watch.changed(&dir) {
                    cfg = load(&dir, &options, &mut player);
                }
                let now = start.elapsed().as_millis() as u64;
                let late = lag.of(ts, now);
                let pick = (late <= cfg.max_latency_ms)
                    .then(|| cfg.pick(key.as_deref(), &class))
                    .flatten();
                if cfg.debug {
                    log!(
                        "事件 本地{now}ms ts={ts} 晚{late}ms seq={} key={key:?} class={class} handled={handled} observed={observed} → {pick:?}",
                        f.seq
                    );
                }
                if let Some(name) = pick {
                    player.play(&name, cfg.volume);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lag;

    #[test]
    fn lag_is_measured_against_fastest_delivery() {
        let mut l = Lag::default();
        assert_eq!(l.of(1000, 50), 0);
        assert_eq!(l.of(1010, 90), 30, "比基准晚 30ms");
        assert_eq!(l.of(1100, 120), 0, "更快的一条刷新基准");
        assert_eq!(l.of(1200, 260), 40);
    }
}
