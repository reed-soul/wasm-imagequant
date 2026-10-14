use crate::color::RGBA;
use crate::quant::Palette;

pub struct DitherEngine;

impl DitherEngine {
    pub fn dither(
        pixels: &[RGBA],
        width: usize,
        height: usize,
        palette: &Palette,
        dither_strength: f32,
    ) -> Vec<u8> {
        let mut indices = vec![0u8; width * height];
        let mut err_r = vec![0.0f32; width * height];
        let mut err_g = vec![0.0f32; width * height];
        let mut err_b = vec![0.0f32; width * height];
        let mut err_a = vec![0.0f32; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                let orig = pixels[idx];

                let cur_r = (orig.r as f32 + err_r[idx]).clamp(0.0, 255.0);
                let cur_g = (orig.g as f32 + err_g[idx]).clamp(0.0, 255.0);
                let cur_b = (orig.b as f32 + err_b[idx]).clamp(0.0, 255.0);
                let cur_a = (orig.a as f32 + err_a[idx]).clamp(0.0, 255.0);

                let adjusted_pixel = RGBA::new(cur_r as u8, cur_g as u8, cur_b as u8, cur_a as u8);
                let pal_idx = palette.find_nearest(&adjusted_pixel.to_oklab());
                indices[idx] = pal_idx;

                if dither_strength <= 0.001 {
                    continue;
                }

                let mapped_color = palette.entries[pal_idx as usize];
                let diff_r = (cur_r - mapped_color.r as f32) * dither_strength;
                let diff_g = (cur_g - mapped_color.g as f32) * dither_strength;
                let diff_b = (cur_b - mapped_color.b as f32) * dither_strength;
                let diff_a = (cur_a - mapped_color.a as f32) * dither_strength;

                if x + 1 < width {
                    let next = idx + 1;
                    err_r[next] += diff_r * (7.0 / 16.0);
                    err_g[next] += diff_g * (7.0 / 16.0);
                    err_b[next] += diff_b * (7.0 / 16.0);
                    err_a[next] += diff_a * (7.0 / 16.0);
                }
                if y + 1 < height {
                    if x > 0 {
                        let next = (y + 1) * width + (x - 1);
                        err_r[next] += diff_r * (3.0 / 16.0);
                        err_g[next] += diff_g * (3.0 / 16.0);
                        err_b[next] += diff_b * (3.0 / 16.0);
                        err_a[next] += diff_a * (3.0 / 16.0);
                    }
                    {
                        let next = (y + 1) * width + x;
                        err_r[next] += diff_r * (5.0 / 16.0);
                        err_g[next] += diff_g * (5.0 / 16.0);
                        err_b[next] += diff_b * (5.0 / 16.0);
                        err_a[next] += diff_a * (5.0 / 16.0);
                    }
                    if x + 1 < width {
                        let next = (y + 1) * width + (x + 1);
                        err_r[next] += diff_r * (1.0 / 16.0);
                        err_g[next] += diff_g * (1.0 / 16.0);
                        err_b[next] += diff_b * (1.0 / 16.0);
                        err_a[next] += diff_a * (1.0 / 16.0);
                    }
                }
            }
        }
        indices
    }
}
