// SPDX-License-Identifier: MPL-2.0
//! WAV 读写：内部一律 44.1kHz 单声道 16 位。用户自备的文件读进来时转成这个格式
//! （立体声取平均、采样率线性重采样），只认 PCM 16 位。

pub const RATE: u32 = 44_100;

/// PCM → 带文件头的 WAV（非 Windows 后端要落成文件交给外部播放器；`--dump` 导出）。
pub fn encode(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// WAV → 44.1kHz 单声道 PCM。
pub fn decode(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("不是 WAV 文件".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = u32_at(i + 4) as usize;
        let body = i + 8;
        let end = body.saturating_add(len).min(bytes.len());
        match &bytes[i..i + 4] {
            b"fmt " if end - body >= 16 => {
                fmt = Some((
                    u16_at(body),
                    u16_at(body + 2),
                    u32_at(body + 4),
                    u16_at(body + 14),
                ))
            }
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        i = body + len + (len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("缺 fmt 段")?;
    let data = data.ok_or("缺 data 段")?;
    if tag != 1 || bits != 16 || channels == 0 || rate == 0 {
        return Err(format!("只支持 16 位 PCM（这个是格式 {tag}、{bits} 位）"));
    }
    let ch = channels as usize;
    let mono: Vec<f32> = data
        .chunks_exact(2 * ch)
        .map(|f| {
            let sum: i32 = f
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| i16::from_le_bytes(*s) as i32)
                .sum();
            sum as f32 / ch as f32
        })
        .collect();
    Ok(resample(&mono, rate))
}

fn resample(src: &[f32], rate: u32) -> Vec<i16> {
    if rate == RATE {
        return src.iter().map(|&s| s as i16).collect();
    }
    let step = rate as f64 / RATE as f64;
    let n = (src.len() as f64 / step) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * step;
            let j = x as usize;
            let frac = (x - j as f64) as f32;
            let a = src[j];
            let b = *src.get(j + 1).unwrap_or(&a);
            (a + (b - a) * frac) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let pcm = vec![0i16, 1000, -1000, i16::MAX, i16::MIN];
        assert_eq!(decode(&encode(&pcm)).unwrap(), pcm);
    }

    #[test]
    fn stereo_22k_is_downmixed_and_resampled() {
        let mut w = encode(&[]);
        // 改成 22050Hz 立体声：两帧 (100,300) (200,400)
        w[22..24].copy_from_slice(&2u16.to_le_bytes());
        w[24..28].copy_from_slice(&22_050u32.to_le_bytes());
        for s in [100i16, 300, 200, 400] {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w[40..44].copy_from_slice(&8u32.to_le_bytes());
        let pcm = decode(&w).unwrap();
        assert_eq!(pcm.len(), 4, "采样率翻倍");
        assert_eq!(pcm[0], 200, "左右取平均");
        assert_eq!(pcm[2], 300);
    }

    #[test]
    fn rejects_non_pcm16() {
        let mut w = encode(&[0, 0]);
        w[34..36].copy_from_slice(&8u16.to_le_bytes());
        assert!(decode(&w).is_err());
        assert!(decode(b"not a wav").is_err());
    }
}
