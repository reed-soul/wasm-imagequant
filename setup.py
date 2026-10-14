import os

project_name = "wasm-imagequant"

files = {
    "Cargo.toml": """[package]
name = "wasm-imagequant"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
miniz_oxide = "0.7"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
""",

    "src/color.rs": """#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ColorOklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
    pub alpha: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct RGBA {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RGBA {
    #[inline(always)]
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[inline(always)]
    fn srgb_to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    pub fn to_oklab(&self) -> ColorOklab {
        let alpha = self.a as f32 / 255.0;
        if alpha <= 0.001 {
            return ColorOklab { l: 0.0, a: 0.0, b: 0.0, alpha: 0.0 };
        }

        let r_lin = Self::srgb_to_linear(self.r as f32 / 255.0);
        let g_lin = Self::srgb_to_linear(self.g as f32 / 255.0);
        let b_lin = Self::srgb_to_linear(self.b as f32 / 255.0);

        let l = 0.4122214708 * r_lin + 0.5363325363 * g_lin + 0.0514459929 * b_lin;
        let m = 0.2119034982 * r_lin + 0.6806995451 * g_lin + 0.1073969566 * b_lin;
        let s = 0.0883024619 * r_lin + 0.2817188376 * g_lin + 0.6299787005 * b_lin;

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        ColorOklab {
            l: 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
            a: 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
            b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
            alpha,
        }
    }
}

impl ColorOklab {
    #[inline(always)]
    pub fn perceptual_distance_sq(&self, other: &Self) -> f32 {
        let avg_alpha = (self.alpha + other.alpha) * 0.5;
        if avg_alpha < 0.01 {
            return 0.0;
        }

        let d_alpha = self.alpha - other.alpha;
        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;

        (dl * dl + da * da + db * db) * avg_alpha + (d_alpha * d_alpha * 2.0)
    }
}
""",

    "src/quant.rs": """use crate::color::{ColorOklab, RGBA};

#[derive(Clone, Debug)]
pub struct Palette {
    pub entries: Vec<RGBA>,
    pub oklab_entries: Vec<ColorOklab>,
}

impl Palette {
    pub fn new(colors: Vec<RGBA>) -> Self {
        let oklab_entries = colors.iter().map(|c| c.to_oklab()).collect();
        Self { entries: colors, oklab_entries }
    }

    #[inline(always)]
    pub fn find_nearest(&self, target_oklab: &ColorOklab) -> u8 {
        let mut min_dist = f32::MAX;
        let mut best_index = 0;

        for (i, pal_color) in self.oklab_entries.iter().enumerate() {
            let dist = target_oklab.perceptual_distance_sq(pal_color);
            if dist < min_dist {
                min_dist = dist;
                best_index = i as u8;
            }
        }
        best_index
    }
}

const HIST_SIZE: usize = 33;
const HIST_LEN: usize = HIST_SIZE * HIST_SIZE * HIST_SIZE;

pub struct WuQuantizer {
    weights: Vec<f32>,
    mr: Vec<f32>,
    mg: Vec<f32>,
    mb: Vec<f32>,
    ma: Vec<f32>,
}

#[derive(Clone, Copy)]
struct Box3D {
    r0: usize, r1: usize,
    g0: usize, g1: usize,
    b0: usize, b1: usize,
}

impl WuQuantizer {
    pub fn new() -> Self {
        Self {
            weights: vec![0.0; HIST_LEN],
            mr: vec![0.0; HIST_LEN],
            mg: vec![0.0; HIST_LEN],
            mb: vec![0.0; HIST_LEN],
            ma: vec![0.0; HIST_LEN],
        }
    }

    #[inline(always)]
    fn get_index(r: usize, g: usize, b: usize) -> usize {
        (r * HIST_SIZE * HIST_SIZE) + (g * HIST_SIZE) + b
    }

    fn build_histogram(&mut self, pixels: &[RGBA]) {
        self.weights.fill(0.0);
        self.mr.fill(0.0);
        self.mg.fill(0.0);
        self.mb.fill(0.0);
        self.ma.fill(0.0);

        for p in pixels {
            let in_r = (p.r >> 3) as usize + 1;
            let in_g = (p.g >> 3) as usize + 1;
            let in_b = (p.b >> 3) as usize + 1;
            let a_w = p.a as f32 / 255.0;

            let idx = Self::get_index(in_r, in_g, in_b);
            self.weights[idx] += a_w;
            self.mr[idx] += (p.r as f32) * a_w;
            self.mg[idx] += (p.g as f32) * a_w;
            self.mb[idx] += (p.b as f32) * a_w;
            self.ma[idx] += p.a as f32;
        }

        for r in 1..HIST_SIZE {
            for g in 1..HIST_SIZE {
                let mut lw = 0.0;
                let mut lr = 0.0;
                let mut lg = 0.0;
                let mut lb = 0.0;
                let mut la = 0.0;

                for b in 1..HIST_SIZE {
                    let idx = Self::get_index(r, g, b);
                    lw += self.weights[idx];
                    lr += self.mr[idx];
                    lg += self.mg[idx];
                    lb += self.mb[idx];
                    la += self.ma[idx];

                    let p_idx = Self::get_index(r, g - 1, b);
                    self.weights[idx] = self.weights[p_idx] + lw;
                    self.mr[idx] = self.mr[p_idx] + lr;
                    self.mg[idx] = self.mg[p_idx] + lg;
                    self.mb[idx] = self.mb[p_idx] + lb;
                    self.ma[idx] = self.ma[p_idx] + la;
                }
            }
        }
    }

    fn cube_sum(&self, table: &[f32], c: &Box3D) -> f32 {
        (table[Self::get_index(c.r1, c.g1, c.b1)]
            - table[Self::get_index(c.r1, c.g1, c.b0)]
            - table[Self::get_index(c.r1, c.g0, c.b1)]
            + table[Self::get_index(c.r1, c.g0, c.b0)])
            - (table[Self::get_index(c.r0, c.g1, c.b1)]
                - table[Self::get_index(c.r0, c.g1, c.b0)]
                - table[Self::get_index(c.r0, c.g0, c.b1)]
                + table[Self::get_index(c.r0, c.g0, c.b0)])
    }

    pub fn generate_palette(&mut self, pixels: &[RGBA], max_colors: usize) -> Palette {
        self.build_histogram(pixels);

        let mut cubes = Vec::with_capacity(max_colors);
        cubes.push(Box3D {
            r0: 0, r1: HIST_SIZE - 1,
            g0: 0, g1: HIST_SIZE - 1,
            b0: 0, b1: HIST_SIZE - 1,
        });

        while cubes.len() < max_colors {
            let mut best_idx = None;
            let mut max_weight = 0.0;

            for (i, c) in cubes.iter().enumerate() {
                let w = self.cube_sum(&self.weights, c);
                if w > max_weight && (c.r1 - c.r0 > 1 || c.g1 - c.g0 > 1 || c.b1 - c.b0 > 1) {
                    max_weight = w;
                    best_idx = Some(i);
                }
            }

            match best_idx {
                Some(idx) => {
                    let b = cubes.swap_remove(idx);
                    let dr = b.r1 - b.r0;
                    let dg = b.g1 - b.g0;
                    let db = b.b1 - b.b0;
                    let mut b2 = b;

                    if dr >= dg && dr >= db {
                        let mid = b.r0 + dr / 2;
                        cubes.push(Box3D { r1: mid, ..b });
                        b2.r0 = mid;
                        cubes.push(b2);
                    } else if dg >= dr && dg >= db {
                        let mid = b.g0 + dg / 2;
                        cubes.push(Box3D { g1: mid, ..b });
                        b2.g0 = mid;
                        cubes.push(b2);
                    } else {
                        let mid = b.b0 + db / 2;
                        cubes.push(Box3D { b1: mid, ..b });
                        b2.b0 = mid;
                        cubes.push(b2);
                    }
                }
                None => break,
            }
        }

        let mut palette_colors = Vec::with_capacity(cubes.len());
        for c in &cubes {
            let w = self.cube_sum(&self.weights, c);
            if w > 0.0 {
                let r = (self.cube_sum(&self.mr, c) / w).clamp(0.0, 255.0) as u8;
                let g = (self.cube_sum(&self.mg, c) / w).clamp(0.0, 255.0) as u8;
                let b = (self.cube_sum(&self.mb, c) / w).clamp(0.0, 255.0) as u8;
                let a = (self.cube_sum(&self.ma, c) / w).clamp(0.0, 255.0) as u8;
                palette_colors.push(RGBA::new(r, g, b, a));
            }
        }

        if palette_colors.is_empty() {
            palette_colors.push(RGBA::new(0, 0, 0, 255));
        }

        Palette::new(palette_colors)
    }
}
""",

    "src/dither.rs": """use crate::color::RGBA;
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
""",

    "src/png.rs": """use crate::color::RGBA;

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
""",

    "src/lib.rs": """mod color;
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
""",

    "demo.html": """<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <title>自研前端 WASM PNG 压缩测试</title>
</head>
<body>
    <input type="file" id="upload" accept="image/png, image/jpeg" />
    <br><br>
    <button id="btn" disabled>压缩并下载 8-bit PNG</button>
    <p id="info"></p>

    <script type="module">
        import init, { compress_png } from './pkg/wasm_imagequant.js';
        let wasmReady = false;
        let imgBitmap = null;

        init().then(() => {
            wasmReady = true;
            console.log("WASM 核心已就绪！");
        });

        document.getElementById('upload').addEventListener('change', async (e) => {
            const file = e.target.files[0];
            if (!file) return;
            imgBitmap = await createImageBitmap(file);
            document.getElementById('btn').disabled = !wasmReady;
            document.getElementById('info').innerText = `已载入: ${file.name} (${imgBitmap.width}x${imgBitmap.height}), 原始体积: ${(file.size / 1024).toFixed(1)} KB`;
        });

        document.getElementById('btn').addEventListener('click', () => {
            const canvas = document.createElement('canvas');
            canvas.width = imgBitmap.width;
            canvas.height = imgBitmap.height;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(imgBitmap, 0, 0);
            const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);

            console.time("WASM压缩耗时");
            const result = compress_png(imgData.data, canvas.width, canvas.height, 256, 0.8);
            console.timeEnd("WASM压缩耗时");

            const blob = new Blob([result.data], { type: 'image/png' });
            document.getElementById('info').innerText += ` -> 压缩后: ${(blob.size / 1024).toFixed(1)} KB`;

            const link = document.createElement('a');
            link.href = URL.createObjectURL(blob);
            link.download = "compressed.png";
            link.click();
        });
    </script>
</body>
</html>
""",

    ".gitignore": """/target
/pkg
**/*.rs.bk
Cargo.lock
"""
}

# 自动生成所有文件
os.makedirs(f"{project_name}/src", exist_ok=True)
for path, content in files.items():
    full_path = os.path.join(project_name, path)
    with open(full_path, "w", encoding="utf-8") as f:
        f.write(content)
    print(f" 已生成: {full_path}")

print("\n 项目骨架生成完毕！请进入项目目录：")
print(f"cd {project_name}")