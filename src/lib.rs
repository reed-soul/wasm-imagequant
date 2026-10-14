//! wasm-imagequant: 高性能自研感知色彩量化与 PNG 压缩引擎
//!
//! 全面基于 Oklab 感知色彩空间，采用改良方差中位切分 + Lloyd 沃罗诺伊质心松弛 +
//! VP-Tree 视点度量树加速 + 自适应蛇形误差扩散抖动 + 自适应位深 PNG 编码。

mod color;
mod dither;
mod hist;
mod png;
mod quant;
mod vptree;

pub use color::{ColorOklab, RGBA};
pub use dither::DitherEngine;
pub use hist::{HistItem, Histogram};
pub use png::encode_indexed_png;
pub use quant::{Palette, PerceptualQuantizer};
pub use vptree::VpTree;

use wasm_bindgen::prelude::*;

/// 量化压缩结果封装
#[wasm_bindgen]
pub struct QuantResult {
    png_bytes: Vec<u8>,
    palette_len: usize,
}

#[wasm_bindgen]
impl QuantResult {
    /// 压缩后的 PNG 完整二进制流
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.png_bytes.clone()
    }

    /// 压缩后的 PNG 字节大小
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> usize {
        self.png_bytes.len()
    }

    /// 实际分配的调色板颜色数
    #[wasm_bindgen(getter)]
    pub fn palette_size(&self) -> usize {
        self.palette_len
    }
}

/// 纯 Rust 原生压缩接口（供单元测试、基准测试与服务端复用）
pub fn quantize_and_compress(
    rgba_data: &[u8],
    width: u32,
    height: u32,
    max_colors: u32,
    dither: f32,
) -> Result<Vec<u8>, String> {
    let w = width as usize;
    let h = height as usize;
    let pixel_count = w * h;

    if rgba_data.len() < pixel_count * 4 {
        return Err("Invalid RGBA buffer length".to_string());
    }

    let pixels: Vec<RGBA> = (0..pixel_count)
        .map(|i| {
            let offset = i * 4;
            RGBA::new(
                rgba_data[offset],
                rgba_data[offset + 1],
                rgba_data[offset + 2],
                rgba_data[offset + 3],
            )
        })
        .collect();

    // 1. 构建高精度感知直方图
    let mut hist = Histogram::build(&pixels, w, h);

    // 2. 双阶段色彩量化 (方差切分 + 沃罗诺伊松弛)
    let palette = PerceptualQuantizer::quantize(&mut hist, max_colors.clamp(2, 256) as usize);

    // 3. 自适应蛇形误差扩散抖动
    let indices = DitherEngine::dither(&pixels, w, h, &palette, dither.clamp(0.0, 1.0));

    // 4. 自适应位深 Indexed-PNG 编码
    png::encode_indexed_png(&indices, width, height, &palette.entries)
}

/// WebAssembly 暴露的核心压缩入口函数
#[wasm_bindgen]
pub fn compress_png(
    rgba_data: &[u8],
    width: u32,
    height: u32,
    max_colors: u32,
    dither: f32,
) -> Result<QuantResult, JsValue> {
    let w = width as usize;
    let h = height as usize;
    let pixel_count = w * h;

    if rgba_data.len() < pixel_count * 4 {
        return Err(JsValue::from_str("Invalid RGBA buffer length"));
    }

    let pixels: Vec<RGBA> = (0..pixel_count)
        .map(|i| {
            let offset = i * 4;
            RGBA::new(
                rgba_data[offset],
                rgba_data[offset + 1],
                rgba_data[offset + 2],
                rgba_data[offset + 3],
            )
        })
        .collect();

    let mut hist = Histogram::build(&pixels, w, h);
    let palette = PerceptualQuantizer::quantize(&mut hist, max_colors.clamp(2, 256) as usize);
    let pal_len = palette.entries.len();

    let indices = DitherEngine::dither(&pixels, w, h, &palette, dither.clamp(0.0, 1.0));

    let png_bytes = png::encode_indexed_png(&indices, width, height, &palette.entries)
        .map_err(|e| JsValue::from_str(&e))?;

    Ok(QuantResult {
        png_bytes,
        palette_len: pal_len,
    })
}
