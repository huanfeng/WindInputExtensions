// SPDX-License-Identifier: MPL-2.0
//! 音色。键盘、打字机是录音做的音效包（见 [`crate::pack`]），`custom` 是用户自己的音效包；
//! 钢琴、木琴要按键出不同音高，在这里启动时现场合成：
//!
//! - `piano`：钢琴。数字键按简谱：1~7 是中音 do re mi fa sol la si（C 大调），8、9 是高音
//!   do re，0 是休止不出声，可以照简谱弹。字母、符号按键盘排布映射到五声音阶（宫商角徵羽），
//!   怎么乱打都不刺耳：下排最低、中排、上排依次升高；空格是低音五度、回车是和弦、退格是一声闷音。
//! - `marimba`：马林巴（木琴），映射同钢琴、高一个八度，余音短，适合打字快的人。

use serde::Deserialize;

use crate::wav::RATE;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Keyboard,
    Typewriter,
    Piano,
    Marimba,
    Custom,
}

/// 一段合成好的音效。`name` 是播放时的查找键。
pub struct Clip {
    pub name: String,
    pub pcm: Vec<i16>,
}

impl Theme {
    /// 与清单里的音色选项一一对应（有测试守着）。
    #[cfg(test)]
    pub const ALL: [Theme; 5] = [
        Theme::Keyboard,
        Theme::Typewriter,
        Theme::Piano,
        Theme::Marimba,
        Theme::Custom,
    ];
    pub const SYNTH: [Theme; 2] = [Theme::Piano, Theme::Marimba];

    /// 音效包音色对应 `packs/` 下的目录名；合成音色为 `None`。
    pub fn pack(self) -> Option<&'static str> {
        match self {
            Theme::Keyboard => Some("keyboard"),
            Theme::Typewriter => Some("typewriter"),
            Theme::Custom => Some("custom"),
            Theme::Piano | Theme::Marimba => None,
        }
    }

    /// 合成音色这次按键用哪段音效；`None` 表示不出声（修饰键、休止，或这是音效包音色）。
    pub fn pick(self, key: Option<&str>, class: &str) -> Option<String> {
        if class == "modifier" || self.pack().is_some() {
            return None;
        }
        Some(match class {
            "space" | "enter" | "backspace" => class.to_string(),
            "navigation" | "edit" => "soft".to_string(),
            _ => match key.map(note_of) {
                Some(Note::Rest) => return None,
                Some(Note::Midi(m)) => format!("n{m}"),
                None => format!("n{DEFAULT_MIDI}"),
            },
        })
    }

    /// 合成音色的全部音效；音效包音色为空。
    pub fn render(self) -> Vec<Clip> {
        let clip = |name: &str, pcm: Vec<f32>| Clip {
            name: name.to_string(),
            pcm: finish(pcm),
        };
        match self {
            Theme::Piano | Theme::Marimba => {
                let note = |m: i32, gain: f32| -> Vec<f32> {
                    if self == Theme::Piano {
                        piano(m, gain)
                    } else {
                        marimba(m + 12, gain)
                    }
                };
                let mut v: Vec<Clip> = all_notes()
                    .into_iter()
                    .map(|m| clip(&format!("n{m}"), note(m, 1.0)))
                    .collect();
                v.push(clip(
                    "space",
                    mix(&[(note(48, 0.7), 0), (note(55, 0.5), 0)]),
                ));
                let roll = (RATE / 100) as usize;
                v.push(clip(
                    "enter",
                    mix(&[
                        (note(60, 0.6), 0),
                        (note(64, 0.5), roll),
                        (note(67, 0.5), 2 * roll),
                    ]),
                ));
                v.push(clip("backspace", damped(note(43, 0.8), 0.12)));
                v.push(clip("soft", damped(note(76, 0.35), 0.08)));
                v
            }
            _ => Vec::new(),
        }
    }
}

/// 键盘排布里找不到的键（F 键、小键盘符号……）用中央 C。
const DEFAULT_MIDI: i32 = 60;

/// 键盘四排，各排从左到右；起点是五声音阶上的 (八度, 音级)。
const ROWS: &[(&[&str], i32, usize)] = &[
    (
        &[
            "z", "x", "c", "v", "b", "n", "m", "comma", "period", "slash",
        ],
        3,
        0,
    ),
    (
        &[
            "a",
            "s",
            "d",
            "f",
            "g",
            "h",
            "j",
            "k",
            "l",
            "semicolon",
            "quote",
        ],
        3,
        3,
    ),
    (
        &[
            "q",
            "w",
            "e",
            "r",
            "t",
            "y",
            "u",
            "i",
            "o",
            "p",
            "lbracket",
            "rbracket",
            "backslash",
        ],
        4,
        0,
    ),
    (&["backtick", "minus", "equal"], 5, 0),
];
const PENTATONIC: [i32; 5] = [0, 2, 4, 7, 9];

/// 简谱 1~9 → C4 起的 C 大调音阶（8、9 是高音 do re）。
const SOLFEGE: [i32; 9] = [60, 62, 64, 65, 67, 69, 71, 72, 74];

enum Note {
    Midi(i32),
    /// 简谱里的 0：休止，不出声。
    Rest,
}

fn note_of(key: &str) -> Note {
    let key = key
        .strip_prefix("numpad")
        .filter(|k| k.len() == 1)
        .unwrap_or(key);
    match key.parse::<usize>() {
        Ok(0) => Note::Rest,
        Ok(d) if d <= 9 => Note::Midi(SOLFEGE[d - 1]),
        _ => Note::Midi(key_midi(key).unwrap_or(DEFAULT_MIDI)),
    }
}

/// 字母、符号：键盘排布上的五声音阶。
fn key_midi(key: &str) -> Option<i32> {
    ROWS.iter().find_map(|(row, octave, start)| {
        row.iter().position(|k| *k == key).map(|i| {
            let step = start + i;
            12 * (octave + 1) + PENTATONIC[step % 5] + 12 * (step / 5) as i32
        })
    })
}

fn all_notes() -> Vec<i32> {
    let mut v: Vec<i32> = ROWS
        .iter()
        .flat_map(|(row, ..)| row.iter().filter_map(|k| key_midi(k)))
        .collect();
    v.push(DEFAULT_MIDI);
    v.extend(SOLFEGE);
    v.sort();
    v.dedup();
    v
}

fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

fn samples(secs: f32) -> usize {
    (secs * RATE as f32) as usize
}

/// 相位用 f64 算：音符后段相位到上万弧度，f32 在这个量级上的误差约 0.004 弧度，
/// 表现为相位抖动——高次泛音上听得出是一层「滋」声的底噪。
fn sine(f: f32, i: usize) -> f32 {
    (std::f64::consts::TAU * f as f64 * i as f64 / RATE as f64).sin() as f32
}

/// 钢琴：带非谐性的泛音列，高次泛音衰减得快；两根弦微微失谐出合唱感。按键音要短，
/// 余音收在 0.6 秒内；泛音只留到 6kHz，再高就发刺。
fn piano(midi: i32, gain: f32) -> Vec<f32> {
    let f0 = hz(midi);
    let tau = (0.4 * (262.0 / f0).powf(0.5)).clamp(0.15, 0.5);
    let n = samples((tau * 2.5).min(0.6));
    let partials: Vec<(f32, f32, f32)> = (1..=6)
        .map(|k| {
            let k = k as f32;
            let f = k * f0 * (1.0 + 0.0004 * k * k).sqrt();
            (f, 1.0 / k.powf(1.8), tau / (1.0 + 0.6 * (k - 1.0)))
        })
        .filter(|(f, ..)| *f < 6_000.0)
        .collect();
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let mut s = 0.0;
            for &(f, amp, tk) in &partials {
                let env = 0.6 * (-t / (tk * 0.22)).exp() + 0.4 * (-t / tk).exp();
                s += amp * env * (sine(f * 0.9995, i) + sine(f * 1.0005, i)) * 0.5;
            }
            s * attack(t, 0.004) * gain
        })
        .collect()
}

/// 马林巴：基音 + 4 倍、10 倍泛音（音板调过的木琴就是这个比例），泛音衰减快，余音短。
fn marimba(midi: i32, gain: f32) -> Vec<f32> {
    let f0 = hz(midi);
    let tau = (0.22 * (523.0 / f0).powf(0.4)).clamp(0.08, 0.4);
    let n = samples(tau * 5.0);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let s = sine(f0, i) * (-t / tau).exp()
                + 0.3 * sine(f0 * 4.0, i) * (-t / (tau * 0.25)).exp()
                + 0.08 * sine(f0 * 9.8, i) * (-t / (tau * 0.1)).exp();
            s * attack(t, 0.001) * gain
        })
        .collect()
}

fn attack(t: f32, secs: f32) -> f32 {
    if t >= secs {
        1.0
    } else {
        0.5 - 0.5 * (std::f32::consts::PI * t / secs).cos()
    }
}

/// 把一段音截短：`secs` 之后很快衰减到零（闷音）。
fn damped(mut v: Vec<f32>, secs: f32) -> Vec<f32> {
    let start = samples(secs);
    let fade = samples(0.05);
    v.truncate(start + fade);
    for (j, s) in v.iter_mut().enumerate().skip(start) {
        *s *= (-((j - start) as f32) / (fade as f32 / 5.0)).exp();
    }
    v
}

/// 叠加几段（各自带起始偏移）。
fn mix(parts: &[(Vec<f32>, usize)]) -> Vec<f32> {
    let len = parts
        .iter()
        .map(|(v, off)| v.len() + off)
        .max()
        .unwrap_or(0);
    let mut out = vec![0.0; len];
    for (v, off) in parts {
        for (j, s) in v.iter().enumerate() {
            out[off + j] += s;
        }
    }
    out
}

/// 收尾：峰值归一到 -8dB（给叠加留余量）、末尾淡出（截断处不爆音），转 16 位。
fn finish(mut v: Vec<f32>) -> Vec<i16> {
    let peak = v.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        let k = PEAK / peak;
        v.iter_mut().for_each(|s| *s *= k);
    }
    let fade = (v.len() / 5).min(samples(0.08));
    let n = v.len();
    for j in 0..fade {
        v[n - fade + j] *= 1.0 - (j + 1) as f32 / fade as f32;
    }
    v.iter().map(|s| (s * i16::MAX as f32) as i16).collect()
}

const PEAK: f32 = 0.4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pick_has_a_rendered_clip() {
        let keys = [
            "a",
            "z",
            "p",
            "1",
            "9",
            "semicolon",
            "numpad5",
            "f5",
            "space",
            "enter",
            "backspace",
            "left",
        ];
        let classes = [
            "letter",
            "letter",
            "letter",
            "digit",
            "digit",
            "punct",
            "numpad",
            "function",
            "space",
            "enter",
            "backspace",
            "navigation",
        ];
        for theme in Theme::SYNTH {
            let names: Vec<String> = theme.render().into_iter().map(|c| c.name).collect();
            for (k, c) in keys.iter().zip(classes) {
                let n = theme.pick(Some(k), c).unwrap();
                assert!(names.contains(&n), "{theme:?} 选了 {n}（{k}）却没合成");
            }
            assert_eq!(theme.pick(Some("lshift"), "modifier"), None, "修饰键不出声");
            assert!(
                theme.pick(None, "letter").is_some(),
                "密码框里没有键名也要出声"
            );
        }
    }

    #[test]
    fn piano_rows_rise_and_stay_pentatonic() {
        let z = key_midi("z").unwrap();
        let a = key_midi("a").unwrap();
        let q = key_midi("q").unwrap();
        assert!(z < a && a < q, "下排 < 中排 < 上排");
        for row in ROWS {
            for k in row.0 {
                let m = key_midi(k).unwrap();
                assert!(PENTATONIC.contains(&(m % 12)), "{k}={m} 不在五声音阶上");
            }
        }
    }

    #[test]
    fn sine_stays_precise_late_in_a_note() {
        let i = samples(0.6);
        let exact = (std::f64::consts::TAU * 5_987.0 * i as f64 / RATE as f64).sin();
        assert!(
            (sine(5_987.0, i) as f64 - exact).abs() < 1e-6,
            "相位误差会变成底噪"
        );
    }

    #[test]
    fn digits_follow_jianpu() {
        let midi = |k| match note_of(k) {
            Note::Midi(m) => Some(m),
            Note::Rest => None,
        };
        assert_eq!(midi("1"), Some(60), "1 = 中央 C");
        assert_eq!(midi("5"), Some(67), "5 = sol");
        assert_eq!(midi("7"), Some(71));
        assert_eq!(midi("8"), Some(72), "8 = 高音 do");
        assert_eq!(midi("numpad3"), midi("3"));
        assert_eq!(midi("0"), None, "0 = 休止");
        assert_eq!(Theme::Piano.pick(Some("0"), "digit"), None);
        assert_eq!(
            Theme::Keyboard.pick(Some("a"), "letter"),
            None,
            "音效包音色不在这里选"
        );
    }

    #[test]
    fn clips_are_normalized_and_end_silent() {
        for theme in Theme::SYNTH {
            for c in theme.render() {
                let peak = c.pcm.iter().map(|s| s.unsigned_abs()).max().unwrap();
                let want = (PEAK * i16::MAX as f32) as u16;
                assert!(
                    (want - 300..=want).contains(&peak),
                    "{}: 峰值 {peak}",
                    c.name
                );
                assert!(
                    c.pcm.last().unwrap().unsigned_abs() < 50,
                    "{}: 结尾没淡出",
                    c.name
                );
            }
        }
    }
}
