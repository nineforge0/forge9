//! Bink 2 video headers (`KB2` + revision letter).
//!
//! After the 4-byte magic come u32 fields: file size - 8, frame count, largest
//! frame size, frame count again, width, height, frame-rate numerator and
//! denominator.

use anyhow::{bail, Result};
use super::u32_at;

#[derive(Debug, Clone, Copy)]
pub struct Bink {
    pub revision: char,
    pub file_size: u64,
    pub frames: u32,
    pub largest_frame: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

impl Bink {
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 { 0.0 } else { f64::from(self.fps_num) / f64::from(self.fps_den) }
    }

    pub fn duration_secs(&self) -> f64 {
        let fps = self.fps();
        if fps == 0.0 { 0.0 } else { f64::from(self.frames) / fps }
    }
}

pub fn parse(data: &[u8]) -> Result<Bink> {
    if !data.starts_with(b"KB2") || data.len() < 4 + 8 * 4 { bail!("not a Bink 2 header"); }
    let f = |i: usize| u32_at(data, 4 + 4 * i).unwrap();
    let b = Bink {
        revision: data[3] as char,
        file_size: u64::from(f(0)) + 8,
        frames: f(1),
        largest_frame: f(2),
        width: f(4),
        height: f(5),
        fps_num: f(6),
        fps_den: f(7),
    };
    if f(3) != b.frames { bail!("frame count fields disagree ({} vs {})", b.frames, f(3)); }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header() {
        let mut d = b"KB2j".to_vec();
        for v in [10_000u32 - 8, 300, 4096, 300, 1920, 1080, 30000, 1001] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        let b = parse(&d).unwrap();
        assert_eq!((b.revision, b.file_size, b.width, b.height), ('j', 10_000, 1920, 1080));
        assert!((b.fps() - 29.97).abs() < 0.01);
        assert!((b.duration_secs() - 10.01).abs() < 0.01);
    }
}
