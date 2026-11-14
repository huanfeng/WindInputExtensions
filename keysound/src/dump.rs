// SPDX-License-Identifier: MPL-2.0
//! `keysound --dump <目录>`：把各合成音色的全部音效、以及一段模拟打字（经同一个混音器离线混出）
//! 写成 WAV。用系统播放器听这些文件，能把「合成出来的音质」和「实时播放链路」分开排查。

use std::path::Path;
use std::sync::Arc;

use crate::mixer::Mixer;
use crate::synth::Theme;
use crate::wav::{RATE, encode};

const VOLUME: f32 = 0.6;
const BLOCK: usize = 512;

pub fn run(out: &Path) -> std::io::Result<()> {
    for theme in Theme::SYNTH {
        let name = format!("{theme:?}").to_lowercase();
        let dir = out.join(&name);
        std::fs::create_dir_all(&dir)?;
        let clips: Vec<(String, Arc<[i16]>)> = theme
            .render()
            .into_iter()
            .map(|c| (c.name, c.pcm.into()))
            .collect();
        for (n, pcm) in &clips {
            std::fs::write(dir.join(format!("{n}.wav")), encode(pcm))?;
        }
        let text = "hello world 1155665 4433221";
        for (suffix, gap_ms) in [("typing", 150), ("fast", 60)] {
            let pcm = typing(theme, &clips, text, gap_ms);
            std::fs::write(out.join(format!("{name}_{suffix}.wav")), encode(&pcm))?;
        }
    }
    Ok(())
}

/// 按 `gap_ms` 的间隔依次「按下」`text` 里的键，经混音器混成一段。
fn typing(theme: Theme, clips: &[(String, Arc<[i16]>)], text: &str, gap_ms: usize) -> Vec<i16> {
    let gap = RATE as usize * gap_ms / 1000;
    let presses: Vec<(usize, Arc<[i16]>)> = text
        .chars()
        .enumerate()
        .filter_map(|(i, c)| {
            let (key, class) = match c {
                ' ' => ("space".to_string(), "space"),
                '0'..='9' => (c.to_string(), "digit"),
                _ => (c.to_string(), "letter"),
            };
            let pick = theme.pick(Some(&key), class)?;
            let pcm = clips.iter().find(|(n, _)| *n == pick)?.1.clone();
            Some((i * gap, pcm))
        })
        .collect();
    let total = text.chars().count() * gap + RATE as usize;
    let mut mixer = Mixer::default();
    let mut out = vec![0i16; total.div_ceil(BLOCK) * BLOCK];
    let mut next = 0;
    for (b, chunk) in out.chunks_mut(BLOCK).enumerate() {
        while next < presses.len() && presses[next].0 <= b * BLOCK {
            mixer.start(presses[next].1.clone(), VOLUME);
            next += 1;
        }
        mixer.fill(chunk);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_clips_and_typing_samples() {
        let t = tempfile::tempdir().unwrap();
        super::run(t.path()).unwrap();
        for f in [
            "piano_typing.wav",
            "marimba_fast.wav",
            "piano/n60.wav",
            "marimba/enter.wav",
        ] {
            assert!(t.path().join(f).is_file(), "{f}");
        }
    }
}
