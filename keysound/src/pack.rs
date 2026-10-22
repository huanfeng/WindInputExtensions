//! 音效包：扩展目录 `packs/<名字>/` 下的一组 WAV（16 位 PCM），按文件名分组。
//!
//! 文件名去掉扩展名、再去掉末尾的 `-数字` 就是组名：`default-1.wav`、`default-2.wav` 同属
//! `default` 组，每次从组里随机挑一个，连按同一个键也不会每次一模一样。按键时依次找：
//!
//! 1. 物理键名组（`a`、`semicolon`……，见 `wind_keys::names::vk_key_name`）
//! 2. 分类组（`space`、`enter`、`backspace`、`navigation`……，见 `vk_key_class`）
//! 3. `default` 组——修饰键除外，修饰键只在有对应组时出声
//!
//! 自带的 `keyboard`、`typewriter` 两个包随扩展分发；`custom` 留给用户放自己的音效。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;

use crate::synth::Clip;

pub const PACKS_DIR: &str = "packs";
/// 包内音效在音效表里的名字前缀，与合成音色、用户映射的文件分开。
const PREFIX: &str = "pack.";

#[derive(Debug, Clone, Default)]
pub struct Pack {
    /// 组名 → 组内音效名（按文件名排序）。
    groups: BTreeMap<String, Vec<String>>,
    rng: Cell<u32>,
}

impl Pack {
    /// 读一个包目录；读不了、解不开的文件记日志、跳过。目录不存在得到空包。
    pub fn load(dir: &Path) -> (Pack, Vec<Clip>) {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
            .collect();
        files.sort();
        let mut pack = Pack {
            groups: BTreeMap::new(),
            rng: Cell::new(0x9E37_79B9),
        };
        let mut clips = Vec::new();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| crate::wav::decode(&b))
            {
                Ok(pcm) => {
                    let name = format!("{PREFIX}{stem}");
                    pack.groups
                        .entry(group_of(stem).to_string())
                        .or_default()
                        .push(name.clone());
                    clips.push(Clip { name, pcm });
                }
                Err(e) => eprintln!(
                    "{} [keysound] 读 {} 失败: {e}",
                    wind_hook::stamp::utc_stamp(),
                    path.display()
                ),
            }
        }
        (pack, clips)
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// 这次按键播哪段（[`Pack::load`] 返回的音效名）；`None` 表示不出声。
    pub fn pick(&self, key: Option<&str>, class: &str) -> Option<String> {
        let fallback = (class != "modifier").then_some("default");
        let group = key
            .and_then(|k| self.groups.get(k))
            .or_else(|| self.groups.get(class))
            .or_else(|| fallback.and_then(|g| self.groups.get(g)))?;
        Some(group[self.next() as usize % group.len()].clone())
    }

    /// xorshift，够打散就行。
    fn next(&self) -> u32 {
        let mut x = self.rng.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng.set(x);
        x
    }
}

/// `default-2` → `default`；`f-1` → `f`；没有 `-数字` 后缀的原样返回。
fn group_of(stem: &str) -> &str {
    match stem.rsplit_once('-') {
        Some((g, n)) if !g.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            g
        }
        _ => stem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(files: &[&str]) -> (tempfile::TempDir, Pack) {
        let t = tempfile::tempdir().unwrap();
        for f in files {
            std::fs::write(t.path().join(f), crate::wav::encode(&[1, 2, 3])).unwrap();
        }
        let (p, clips) = Pack::load(t.path());
        assert_eq!(clips.len(), files.len());
        (t, p)
    }

    #[test]
    fn numbered_files_form_one_group() {
        assert_eq!(group_of("default-12"), "default");
        assert_eq!(group_of("space"), "space");
        assert_eq!(group_of("my-key"), "my-key", "后缀不是数字就是名字的一部分");
        assert_eq!(group_of("-1"), "-1");
    }

    #[test]
    fn key_beats_class_beats_default() {
        let (_t, p) = pack(&["default-1.wav", "default-2.wav", "space.wav", "a.wav"]);
        assert_eq!(p.pick(Some("a"), "letter").as_deref(), Some("pack.a"));
        assert_eq!(
            p.pick(Some("space"), "space").as_deref(),
            Some("pack.space")
        );
        let b = p.pick(Some("b"), "letter").unwrap();
        assert!(b.starts_with("pack.default-"), "{b}");
        assert!(p.pick(None, "letter").is_some(), "密码框里没有键名也要出声");
    }

    #[test]
    fn variants_are_spread() {
        let (_t, p) = pack(&["default-1.wav", "default-2.wav", "default-3.wav"]);
        let mut seen: Vec<String> = (0..50)
            .filter_map(|_| p.pick(Some("a"), "letter"))
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3, "同一个键连按也会轮到每个变体");
    }

    #[test]
    fn modifiers_sound_only_with_their_own_group() {
        let (_t, p) = pack(&["default.wav"]);
        assert_eq!(p.pick(Some("lshift"), "modifier"), None);
        let (_t, p) = pack(&["default.wav", "modifier.wav"]);
        assert_eq!(
            p.pick(Some("lshift"), "modifier").as_deref(),
            Some("pack.modifier")
        );
    }

    #[test]
    fn bundled_packs_cover_the_main_groups() {
        for name in ["keyboard", "typewriter"] {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(PACKS_DIR)
                .join(name);
            let (p, clips) = Pack::load(&dir);
            for g in ["default", "space", "enter", "backspace"] {
                assert!(p.groups.contains_key(g), "{name} 缺 {g}");
            }
            for c in &clips {
                assert!(c.pcm.len() > 1000, "{name}/{} 太短", c.name);
                assert!(
                    c.pcm.last().unwrap().unsigned_abs() < 100,
                    "{name}/{}: 结尾没淡出",
                    c.name
                );
            }
        }
    }

    #[test]
    fn missing_dir_and_junk_files() {
        let (p, clips) = Pack::load(Path::new("/nonexistent/keysound/pack"));
        assert!(p.is_empty() && clips.is_empty());
        assert_eq!(p.pick(Some("a"), "letter"), None);

        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("README.md"), "x").unwrap();
        std::fs::write(t.path().join("bad.wav"), "not a wav").unwrap();
        std::fs::write(t.path().join("ok.WAV"), crate::wav::encode(&[1])).unwrap();
        let (p, clips) = Pack::load(t.path());
        assert_eq!(clips.len(), 1, "非 WAV、坏文件跳过");
        assert_eq!(p.pick(Some("ok"), "letter").as_deref(), Some("pack.ok"));
    }
}
