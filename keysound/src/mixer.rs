//! 多声部混音（纯计算，Windows 后端在音频线程里调用）。
//!
//! PlaySound 同一时刻只放一个声音，新的一按就把上一个截断：截断处爆音，钢琴的余音也留不住。
//! 这里把同时在响的音效叠加起来。同一段音效再次触发时（长按连发、同一个键连打），旧的那个
//! 10ms 内淡出——真钢琴同一根弦重击也是先止住再响，否则连发三十下会越叠越响。

use std::sync::Arc;

use crate::wav::RATE;

/// 同时在响的上限。超了就让最早的淡出。
pub const MAX_VOICES: usize = 12;
const RELEASE: usize = (RATE / 100) as usize;

struct Voice {
    pcm: Arc<[i16]>,
    pos: usize,
    gain: f32,
    /// 正在淡出：还剩多少采样。
    release: Option<usize>,
}

pub struct Mixer {
    voices: Vec<Voice>,
    /// 限幅器当前的增益（≤ 1）。
    limit: f32,
}

impl Default for Mixer {
    fn default() -> Mixer {
        Mixer {
            voices: Vec::new(),
            limit: 1.0,
        }
    }
}

/// 限幅门槛：叠加超过它就整体压低增益，而不是逐个采样削——逐采样压就是「滋啦」的破音。
const LIMIT: f32 = 0.7;
/// 压下去之后回升的速度：约 80ms 回到满增益。
const LIMIT_RELEASE: f32 = 1.0 / (RATE as f32 * 0.08);

impl Mixer {
    pub fn start(&mut self, pcm: Arc<[i16]>, gain: f32) {
        for v in &mut self.voices {
            if Arc::ptr_eq(&v.pcm, &pcm) && v.release.is_none() {
                v.release = Some(RELEASE);
            }
        }
        let live = self.voices.iter().filter(|v| v.release.is_none()).count();
        if live >= MAX_VOICES
            && let Some(v) = self.voices.iter_mut().find(|v| v.release.is_none())
        {
            v.release = Some(RELEASE);
        }
        if self.voices.len() >= MAX_VOICES * 2 {
            self.voices.remove(0);
        }
        self.voices.push(Voice {
            pcm,
            pos: 0,
            gain,
            release: None,
        });
    }

    #[cfg_attr(not(windows), allow(dead_code))] // 只有 Windows 的音频线程要知道何时停
    pub fn is_idle(&self) -> bool {
        self.voices.is_empty()
    }

    /// 混出下一段。没有声部时写零。
    pub fn fill(&mut self, out: &mut [i16]) {
        let mut acc = vec![0f32; out.len()];
        for v in &mut self.voices {
            for a in acc.iter_mut() {
                let Some(&s) = v.pcm.get(v.pos) else { break };
                let fade = match &mut v.release {
                    Some(0) => break,
                    Some(left) => {
                        *left -= 1;
                        *left as f32 / RELEASE as f32
                    }
                    None => 1.0,
                };
                *a += s as f32 * v.gain * fade;
                v.pos += 1;
            }
        }
        self.voices
            .retain(|v| v.pos < v.pcm.len() && v.release != Some(0));
        for (o, a) in out.iter_mut().zip(acc) {
            let x = a / i16::MAX as f32;
            let need = if x.abs() > LIMIT {
                LIMIT / x.abs()
            } else {
                1.0
            };
            self.limit = need.min(self.limit + LIMIT_RELEASE).min(1.0);
            *o = (soft_clip(x * self.limit) * i16::MAX as f32) as i16;
        }
    }
}

/// 叠加超过满幅时圆滑压下去，而不是削顶（削顶是刺耳的破音）。
fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let m = x.abs();
    if m <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((m - KNEE) / (1.0 - KNEE)).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(v: i16, n: usize) -> Arc<[i16]> {
        vec![v; n].into()
    }

    #[test]
    fn overlapping_clips_are_summed() {
        let mut m = Mixer::default();
        m.start(clip(1000, 4), 1.0);
        m.start(clip(2000, 2), 1.0);
        let mut out = [0i16; 4];
        m.fill(&mut out);
        assert_eq!(out, [3000, 3000, 1000, 1000]);
        assert!(m.is_idle());
    }

    #[test]
    fn retrigger_fades_the_old_voice_instead_of_stacking() {
        let mut m = Mixer::default();
        let c = clip(10_000, RELEASE * 4);
        m.start(c.clone(), 1.0);
        m.start(c, 1.0);
        let mut out = vec![0i16; RELEASE * 2];
        m.fill(&mut out);
        assert!(
            out[0] > 19_000 - 100 && out[0] < 20_000,
            "刚开始两个叠着（正淡出）"
        );
        assert_eq!(out[RELEASE + 1], 10_000, "旧的淡完只剩新的");
    }

    #[test]
    fn voice_count_is_bounded() {
        let mut m = Mixer::default();
        for i in 0..100 {
            m.start(clip(100 + i, 10_000), 1.0);
        }
        assert!(m.voices.len() <= MAX_VOICES * 2);
        assert!(m.voices.iter().filter(|v| v.release.is_none()).count() <= MAX_VOICES);
    }

    #[test]
    fn loud_sums_are_limited_then_recover() {
        let mut m = Mixer::default();
        for i in 0..2 {
            m.start(clip(20_000 + i, 2), 1.0);
        }
        m.start(clip(10_000, RATE as usize), 1.0);
        let mut out = vec![0i16; RATE as usize];
        m.fill(&mut out);
        let at_limit = (LIMIT * i16::MAX as f32) as i16;
        assert!(
            (at_limit - 10..=at_limit + 10).contains(&out[0]),
            "超限时整体压到门槛：{}",
            out[0]
        );
        assert!(out[2] < 10_000, "刚压过，增益还没回满：{}", out[2]);
        assert_eq!(out[RATE as usize - 1], 10_000, "约 80ms 后回到满增益");
    }

    #[test]
    fn gain_scales() {
        let mut m = Mixer::default();
        m.start(clip(10_000, 1), 0.5);
        let mut out = [0i16; 1];
        m.fill(&mut out);
        assert_eq!(out[0], 5_000);
    }
}
