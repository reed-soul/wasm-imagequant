//! 高压缩率 Indexed-PNG 编码器 (Indexed PNG Encoder)
//!
//! 特性：
//! 1. 自适应位深选择 (Adaptive Bit Depth)：根据调色板色彩数量自动使用 1-bit、2-bit、4-bit 或 8-bit 打包
//! 2. 紧凑型 tRNS 裁剪：结合调色板透明色前置排序，最小化透明信息体积
//! 3. 硬件友好纯 Rust 无外部 C 依赖实现，直接利用 miniz_oxide 执行最优 Deflate 压缩

use crate::color::RGBA;

/// 计算 PNG 标准 CRC32 校验码
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// 写入 PNG Chunk 块 (Length + Tag + Data + CRC)
fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let crc = crc32(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// 编码生成 8-bit/4-bit/2-bit/1-bit Indexed-PNG 二进制字节流
pub fn encode_indexed_png(
    indices: &[u8],
    width: u32,
    height: u32,
    palette: &[RGBA],
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || palette.is_empty() {
        return Err("Invalid image dimensions or empty palette".to_string());
    }

    let pal_len = palette.len();
    // 自动选择最小必要位深 (Bit Depth)
    let bit_depth: u8 = if pal_len <= 2 {
        1
    } else if pal_len <= 4 {
        2
    } else if pal_len <= 16 {
        4
    } else {
        8
    };

    let mut out = Vec::with_capacity(32 * 1024);

    // 1. PNG 文件魔数头部
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // 2. IHDR 头信息块
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(bit_depth); // 1, 2, 4, 8
    ihdr.push(3);         // Color Type: 3 = Indexed Color
    ihdr.push(0);         // Compression: 0 = Deflate
    ihdr.push(0);         // Filter Method: 0 = Standard
    ihdr.push(0);         // Interlace: 0 = None
    write_chunk(&mut out, b"IHDR", &ihdr);

    // 3. PLTE 调色板块
    let mut plte = Vec::with_capacity(pal_len * 3);
    for c in palette {
        plte.push(c.r);
        plte.push(c.g);
        plte.push(c.b);
    }
    write_chunk(&mut out, b"PLTE", &plte);

    // 4. tRNS 透明通道块 (仅记录到最后一个非 255 的索引)
    let last_trans_idx = palette.iter().rposition(|c| c.a < 255);
    if let Some(last_idx) = last_trans_idx {
        let mut trns = Vec::with_capacity(last_idx + 1);
        for c in &palette[..=last_idx] {
            trns.push(c.a);
        }
        write_chunk(&mut out, b"tRNS", &trns);
    }

    // 5. IDAT 图像数据块 (逐行打包并添加 Filter 0 字节)
    let w = width as usize;
    let h = height as usize;

    let scanline_bytes = match bit_depth {
        1 => (w + 7) / 8,
        2 => (w + 3) / 4,
        4 => (w + 1) / 2,
        _ => w,
    };

    let mut raw_data = Vec::with_capacity(h * (scanline_bytes + 1));

    for y in 0..h {
        // 每行前缀 Filter Type 0 (None)
        raw_data.push(0x00);
        let row_start = y * w;
        let row_indices = &indices[row_start..row_start + w];

        match bit_depth {
            1 => {
                let mut byte = 0u8;
                let mut bits_filled = 0;
                for &idx in row_indices {
                    byte = (byte << 1) | (idx & 1);
                    bits_filled += 1;
                    if bits_filled == 8 {
                        raw_data.push(byte);
                        byte = 0;
                        bits_filled = 0;
                    }
                }
                if bits_filled > 0 {
                    byte <<= 8 - bits_filled;
                    raw_data.push(byte);
                }
            }
            2 => {
                let mut byte = 0u8;
                let mut bits_filled = 0;
                for &idx in row_indices {
                    byte = (byte << 2) | (idx & 3);
                    bits_filled += 2;
                    if bits_filled == 8 {
                        raw_data.push(byte);
                        byte = 0;
                        bits_filled = 0;
                    }
                }
                if bits_filled > 0 {
                    byte <<= 8 - bits_filled;
                    raw_data.push(byte);
                }
            }
            4 => {
                let mut byte = 0u8;
                let mut bits_filled = 0;
                for &idx in row_indices {
                    byte = (byte << 4) | (idx & 15);
                    bits_filled += 4;
                    if bits_filled == 8 {
                        raw_data.push(byte);
                        byte = 0;
                        bits_filled = 0;
                    }
                }
                if bits_filled > 0 {
                    byte <<= 8 - bits_filled;
                    raw_data.push(byte);
                }
            }
            _ => {
                raw_data.extend_from_slice(row_indices);
            }
        }
    }

    // 使用行业标准 Deflate 级别 6 (在体积与编码性能之间取得最佳平衡)
    let compressed_idat = miniz_oxide::deflate::compress_to_vec_zlib(&raw_data, 6);
    write_chunk(&mut out, b"IDAT", &compressed_idat);

    // 6. IEND 结束块
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}
