//! Minimal BLP2 decoder: the top mipmap only, as RGBA.
//!
//! Layout: "BLP2", type u32, compression u8 (1 palette, 2 DXT, 3 BGRA),
//! alpha depth u8, alpha type u8 (0 DXT1, 1 DXT3, 7 DXT5), has mips u8,
//! width u32, height u32, 16 mip offsets u32, 16 mip sizes u32, then a
//! 256-entry BGRA palette (1172 bytes of header in all).

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "BLP header cut short".into())
}

pub fn decode(b: &[u8]) -> Result<Rgba, String> {
    if b.len() < 1172 || &b[0..4] != b"BLP2" {
        return Err("not a BLP2 file".into());
    }
    let compression = b[8];
    let alpha_depth = b[9];
    let alpha_type = b[10];
    let width = u32_at(b, 12)?;
    let height = u32_at(b, 16)?;
    let offset = u32_at(b, 20)? as usize;
    let size = u32_at(b, 84)? as usize;
    let data = b.get(offset..offset + size).ok_or("BLP mipmap 0 outside the file")?;
    let (w, h) = (width as usize, height as usize);
    let mut pixels = vec![0u8; w * h * 4];

    match compression {
        2 => {
            let format = match (alpha_depth, alpha_type) {
                (0 | 1, _) => texpresso::Format::Bc1,
                (_, 1) => texpresso::Format::Bc2,
                (_, 7) => texpresso::Format::Bc3,
                (_, 0) => texpresso::Format::Bc2,
                _ => return Err(format!("unsupported DXT alpha type {alpha_type}")),
            };
            if data.len() < format.compressed_size(w, h) {
                return Err("DXT data shorter than expected".into());
            }
            format.decompress(data, w, h, &mut pixels);
            if alpha_depth == 0 {
                pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            }
        }
        1 => {
            let palette = &b[148..1172];
            let n = w * h;
            let idx = data.get(..n).ok_or("palette data cut short")?;
            for (i, &k) in idx.iter().enumerate() {
                let c = &palette[k as usize * 4..k as usize * 4 + 4];
                pixels[i * 4..i * 4 + 4].copy_from_slice(&[c[2], c[1], c[0], 255]);
            }
            let alpha = &data[n..];
            for i in 0..n {
                let a = match alpha_depth {
                    0 => 255,
                    1 => alpha.get(i / 8).map_or(255, |v| if v >> (i % 8) & 1 == 1 { 255 } else { 0 }),
                    4 => alpha.get(i / 2).map_or(255, |v| ((v >> ((i % 2) * 4)) & 0xF) * 17),
                    8 => alpha.get(i).copied().unwrap_or(255),
                    d => return Err(format!("unsupported alpha depth {d}")),
                };
                pixels[i * 4 + 3] = a;
            }
        }
        3 => {
            let src = data.get(..w * h * 4).ok_or("BGRA data cut short")?;
            for (d, s) in pixels.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
        }
        c => return Err(format!("unsupported BLP compression {c}")),
    }
    Ok(Rgba { width, height, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(compression: u8, alpha_depth: u8, w: u32, h: u32, data_len: u32) -> Vec<u8> {
        let mut b = vec![0u8; 1172];
        b[0..4].copy_from_slice(b"BLP2");
        b[4] = 1;
        b[8] = compression;
        b[9] = alpha_depth;
        b[12..16].copy_from_slice(&w.to_le_bytes());
        b[16..20].copy_from_slice(&h.to_le_bytes());
        b[20..24].copy_from_slice(&1172u32.to_le_bytes());
        b[84..88].copy_from_slice(&data_len.to_le_bytes());
        b
    }

    #[test]
    fn palette_image() {
        let mut b = header(1, 0, 2, 1, 2);
        // Palette entry 1 = BGRA (10, 20, 30).
        b[148 + 4..148 + 8].copy_from_slice(&[10, 20, 30, 0]);
        b.extend([1u8, 0]);
        let img = decode(&b).unwrap();
        assert_eq!(img.pixels, vec![30, 20, 10, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn bgra_image() {
        let mut b = header(3, 8, 1, 1, 4);
        b.extend([1u8, 2, 3, 4]);
        assert_eq!(decode(&b).unwrap().pixels, vec![3, 2, 1, 4]);
    }

    #[test]
    fn dxt1_solid_block() {
        // One 4x4 DXT1 block, both endpoints pure red (0xF800), all indices 0.
        let mut b = header(2, 0, 4, 4, 8);
        b.extend([0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        let img = decode(&b).unwrap();
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(img.pixels.len(), 64);
    }

    #[test]
    fn rejects_other_files() {
        assert!(decode(b"PNG").is_err());
    }
}
