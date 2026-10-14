use crate::color::RGBA;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let crc = crc32(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

pub fn encode_indexed_png(
    indices: &[u8],
    width: u32,
    height: u32,
    palette: &[RGBA],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(32 * 1024);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(3);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    write_chunk(&mut out, b"IHDR", &ihdr);

    let mut plte = Vec::with_capacity(palette.len() * 3);
    for c in palette {
        plte.push(c.r);
        plte.push(c.g);
        plte.push(c.b);
    }
    write_chunk(&mut out, b"PLTE", &plte);

    let last_trans_idx = palette.iter().rposition(|c| c.a < 255);
    if let Some(last_idx) = last_trans_idx {
        let mut trns = Vec::with_capacity(last_idx + 1);
        for c in &palette[..=last_idx] {
            trns.push(c.a);
        }
        write_chunk(&mut out, b"tRNS", &trns);
    }

    let w = width as usize;
    let h = height as usize;
    let mut scanlines = Vec::with_capacity(h * (w + 1));
    for y in 0..h {
        scanlines.push(0x00);
        let start = y * w;
        scanlines.extend_from_slice(&indices[start..start + w]);
    }

    let compressed_idat = miniz_oxide::deflate::compress_to_vec_zlib(&scanlines, 6);
    write_chunk(&mut out, b"IDAT", &compressed_idat);
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}
