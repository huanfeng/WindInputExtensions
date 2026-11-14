// SPDX-License-Identifier: MPL-2.0
//! `keysound.toml`（扩展目录内），改了即时生效（下一次按键时发现文件变了就重读）。
//!
//! 音色与音量也是扩展选项（见 extension.toml 的 `[[option]]`），在设置端改；设置端下发的值
//! 盖过本文件里的同名项。本文件主要管设置端表达不了的：自备音效文件的映射。
//!
//! ```toml
//! theme = "piano"              # 音色：keyboard（默认，键盘）/ typewriter（打字机）/ piano（钢琴）
//!                              # / marimba（木琴）/ custom（自定义：packs/custom/ 里的 WAV，见 pack.rs）
//! volume = 0.6                 # 0.0 ~ 1.0
//! max_latency_ms = 80          # 事件晚到超过这么久就不播了（播出来也对不上手）
//! debug = false                # 排查音质用：逐条记收到的按键事件，并把送进声卡的混音录成
//!                              # debug_out.pcm（44.1kHz 单声道 s16le，只 Windows）
//!
//! # 以下可选：用自己的 WAV（16 位 PCM）盖掉内置音色。查找顺序：具体键 → 分类 → default → 音色。
//! # default = "sounds/my.wav"
//! [class]                      # 分类名见 wind_keys::names::vk_key_class
//! # enter = "sounds/enter.wav"
//! # navigation = ""            # 空串 = 这类键不出声
//! [key]                        # 物理键名见 wind_keys::names::vk_key_name
//! # semicolon = "sounds/semicolon.wav"
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;

use crate::pack::{PACKS_DIR, Pack};
use crate::synth::Theme;

pub const CONFIG_FILE: &str = "keysound.toml";
/// 用户文件在音效表里的名字前缀，与内置音色的名字分开。
const FILE_PREFIX: &str = "file:";

#[derive(Debug, Clone, Deserialize)]
pub struct SoundMap {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_latency")]
    pub max_latency_ms: u64,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub class: BTreeMap<String, String>,
    #[serde(default)]
    pub key: BTreeMap<String, String>,
    /// 音效包音色读进来的包（[`Self::clips`] 时装上）。
    #[serde(skip)]
    pack: Pack,
}

fn default_volume() -> f32 {
    0.6
}

fn default_latency() -> u64 {
    80
}

impl Default for SoundMap {
    fn default() -> SoundMap {
        SoundMap {
            theme: Theme::default(),
            volume: default_volume(),
            max_latency_ms: default_latency(),
            debug: false,
            default: None,
            class: BTreeMap::new(),
            key: BTreeMap::new(),
            pack: Pack::default(),
        }
    }
}

impl SoundMap {
    /// 读扩展目录里的配置；没有就用默认，写错了报错（扩展 stderr 进宿主的扩展日志）。
    pub fn load(dir: &Path) -> Result<SoundMap, String> {
        match std::fs::read_to_string(dir.join(CONFIG_FILE)) {
            Ok(text) => toml::from_str::<SoundMap>(&text)
                .map(|mut c| {
                    c.volume = c.volume.clamp(0.0, 1.0);
                    c
                })
                .map_err(|e| format!("{CONFIG_FILE} 格式错误: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SoundMap::default()),
            Err(e) => Err(format!("读 {CONFIG_FILE} 失败: {e}")),
        }
    }

    /// 盖上设置端下发的扩展选项（宿主已按清单校验过；不认识的值忽略）。
    pub fn apply_options(&mut self, values: &serde_json::Map<String, serde_json::Value>) {
        if let Some(t) = values
            .get("theme")
            .and_then(|v| Theme::deserialize(v.clone()).ok())
        {
            self.theme = t;
        }
        if let Some(v) = values.get("volume").and_then(serde_json::Value::as_f64) {
            self.volume = (v as f32).clamp(0.0, 1.0);
        }
    }

    /// 这次按键该播哪段音效（[`Self::clips`] 里的名字）；`None` 表示不出声。
    pub fn pick(&self, key: Option<&str>, class: &str) -> Option<String> {
        let custom = key
            .and_then(|k| self.key.get(k))
            .or_else(|| self.class.get(class))
            .or(self.default.as_ref());
        match custom {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(format!("{FILE_PREFIX}{s}")),
            None if self.theme.pack().is_some() => self.pack.pick(key, class),
            None => self.theme.pick(key, class),
        }
    }

    /// 本配置用得到的全部音效：合成的或音效包里的 + 用户文件读进来的。读不了的文件记日志、跳过。
    pub fn clips(&mut self, dir: &Path) -> HashMap<String, Vec<i16>> {
        let mut clips = self.theme.render();
        if let Some(name) = self.theme.pack() {
            let (pack, pcm) = Pack::load(&dir.join(PACKS_DIR).join(name));
            if pack.is_empty() {
                eprintln!(
                    "{} [keysound] 音效包 {PACKS_DIR}/{name} 里没有可用的 WAV，不会出声",
                    wind_hook::stamp::utc_stamp()
                );
            }
            self.pack = pack;
            clips.extend(pcm);
        }
        let mut out: HashMap<String, Vec<i16>> =
            clips.into_iter().map(|c| (c.name, c.pcm)).collect();
        let files = self
            .default
            .iter()
            .chain(self.class.values())
            .chain(self.key.values());
        for rel in files.filter(|s| !s.is_empty()) {
            let name = format!("{FILE_PREFIX}{rel}");
            if out.contains_key(&name) {
                continue;
            }
            match std::fs::read(dir.join(rel))
                .map_err(|e| e.to_string())
                .and_then(|b| crate::wav::decode(&b))
            {
                Ok(pcm) => {
                    out.insert(name, pcm);
                }
                Err(e) => eprintln!(
                    "{} [keysound] 读 {rel} 失败: {e}",
                    wind_hook::stamp::utc_stamp()
                ),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_key_beats_class_beats_default_beats_theme() {
        let mut c = SoundMap {
            theme: Theme::Piano,
            ..SoundMap::default()
        };
        assert_eq!(
            c.pick(Some("a"), "letter"),
            Theme::Piano.pick(Some("a"), "letter")
        );
        c.default = Some("d.wav".into());
        c.class.insert("letter".into(), "l.wav".into());
        c.key.insert("a".into(), "a.wav".into());
        assert_eq!(c.pick(Some("a"), "letter").as_deref(), Some("file:a.wav"));
        assert_eq!(c.pick(Some("b"), "letter").as_deref(), Some("file:l.wav"));
        assert_eq!(c.pick(None, "space").as_deref(), Some("file:d.wav"));
    }

    #[test]
    fn host_options_override_file() {
        let mut c = SoundMap::default();
        let v = serde_json::json!({"theme": "marimba", "volume": 0.25});
        c.apply_options(v.as_object().unwrap());
        assert_eq!((c.theme, c.volume), (Theme::Marimba, 0.25));
        let v = serde_json::json!({"theme": "organ"});
        c.apply_options(v.as_object().unwrap());
        assert_eq!(c.theme, Theme::Marimba, "不认识的值不动");
    }

    #[test]
    fn pack_theme_plays_from_its_directory() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join(PACKS_DIR).join("typewriter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("default-1.wav"), crate::wav::encode(&[7])).unwrap();
        std::fs::write(dir.join("enter.wav"), crate::wav::encode(&[8])).unwrap();
        let mut c = SoundMap {
            theme: Theme::Typewriter,
            ..SoundMap::default()
        };
        let clips = c.clips(t.path());
        assert_eq!(clips[&c.pick(Some("a"), "letter").unwrap()], vec![7]);
        assert_eq!(clips[&c.pick(Some("enter"), "enter").unwrap()], vec![8]);

        c.theme = Theme::Custom;
        let clips = c.clips(t.path());
        assert!(clips.is_empty(), "custom 包不存在");
        assert_eq!(c.pick(Some("a"), "letter"), None, "不会拿上一个包的音效");
    }

    #[test]
    fn manifest_choices_match_themes() {
        let manifest: toml::Value = toml::from_str(include_str!("../extension.toml")).unwrap();
        let values: Vec<String> = manifest["option"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["key"].as_str() == Some("theme"))
            .unwrap()["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["value"].as_str().unwrap().to_string())
            .collect();
        let themes: Vec<String> = Theme::ALL
            .iter()
            .map(|t| format!("{t:?}").to_lowercase())
            .collect();
        assert_eq!(values, themes, "清单里的音色选项要与内置音色一一对应");
    }

    #[test]
    fn empty_entry_means_silent() {
        let mut c = SoundMap::default();
        c.class.insert("navigation".into(), String::new());
        assert_eq!(c.pick(Some("left"), "navigation"), None);
    }

    #[test]
    fn missing_file_is_default_and_bad_file_errors() {
        let t = tempfile::tempdir().unwrap();
        let c = SoundMap::load(t.path()).unwrap();
        assert_eq!((c.theme, c.max_latency_ms), (Theme::Keyboard, 80));
        std::fs::write(t.path().join(CONFIG_FILE), "theme = \"organ\"").unwrap();
        assert!(
            SoundMap::load(t.path()).is_err(),
            "不认识的音色要报错，不能静默回落"
        );
    }

    #[test]
    fn parses_user_file_and_loads_custom_clips() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("s.wav"), crate::wav::encode(&[1, 2, 3])).unwrap();
        std::fs::write(
            t.path().join(CONFIG_FILE),
            "theme = \"piano\"\nvolume = 3.0\n[key]\nsemicolon = \"s.wav\"\nquote = \"missing.wav\"\n",
        )
        .unwrap();
        let mut c = SoundMap::load(t.path()).unwrap();
        assert_eq!((c.theme, c.volume), (Theme::Piano, 1.0), "音量夹到 0~1");
        let clips = c.clips(t.path());
        assert_eq!(clips["file:s.wav"], vec![1, 2, 3]);
        assert!(!clips.contains_key("file:missing.wav"), "读不了的跳过");
        let n = c.pick(Some("a"), "letter").unwrap();
        assert!(clips.contains_key(&n), "音色音效也在表里：{n}");
    }
}
