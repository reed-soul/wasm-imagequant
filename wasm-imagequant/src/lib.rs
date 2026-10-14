mod color;
mod dither;
mod png;
mod quant;

use color::RGBA;
use dither::DitherEngine;
use quant::WuQuantizer;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct QuantResult {
    png_bytes: Vec<u8>,
}

#[wasm_bindgen]
impl QuantResult {
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.png_bytes.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn size(&self) -> usize {
        self.png_bytes.len()
    }
}

#[wasm_bindgen]
pub fn compress_png(
    rgba_data: &[u8],
    width: u32,
    height: u32,
    max_colors: u32,
    dither: f32,
) -> Result<QuantResult, JsValue> {
    let pixel_count = (width * height) as usize;
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

    let mut quantizer = WuQuantizer::new();
    let palette = quantizer.generate_palette(&pixels, max_colors.clamp(2, 256) as usize);

    let indices = DitherEngine::dither(
        &pixels,
        width as usize,
        height as usize,
        &palette,
        dither.clamp(0.0, 1.0),
    );

    let png_bytes = png::encode_indexed_png(&indices, width, height, &palette.entries)
        .map_err(|e| JsValue::from_str(&e))?;

    Ok(QuantResult { png_bytes })
}
