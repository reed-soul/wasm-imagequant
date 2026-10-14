//! 图像感知直方图与特征采样模块
//!
//! 将数百万原始像素聚合为数千个代表性特征簇（Bins），并保留亚像素精度的色彩均值，
//! 为后续的中位切分与 K-Means 沃罗诺伊松弛提供极速的运算支持（将运算量降低 99% 以上）。

use crate::color::{ColorOklab, RGBA};

/// 直方图采样单元
#[derive(Clone, Debug)]
pub struct HistItem {
    pub color: ColorOklab,
    pub rgba: RGBA,
    pub weight: f32,
    pub likely_cluster: u8,
}

/// 图像特征色彩直方图
pub struct Histogram {
    pub items: Vec<HistItem>,
    pub total_weight: f32,
}

impl Histogram {
    /// 从原始 RGBA 像素缓冲区构建感知加权直方图
    pub fn build(pixels: &[RGBA], width: usize, height: usize) -> Self {
        // 使用 19-bit 哈希索引（R: 5-bit, G: 5-bit, B: 5-bit, A: 4-bit）
        const TOTAL_BINS: usize = 1 << 19; // 524,288

        let mut bin_counts: Vec<f32> = vec![0.0; TOTAL_BINS];
        let mut bin_sum_r: Vec<f32> = vec![0.0; TOTAL_BINS];
        let mut bin_sum_g: Vec<f32> = vec![0.0; TOTAL_BINS];
        let mut bin_sum_b: Vec<f32> = vec![0.0; TOTAL_BINS];
        let mut bin_sum_a: Vec<f32> = vec![0.0; TOTAL_BINS];
        let mut active_bins: Vec<usize> = Vec::with_capacity(8192);

        let mut total_weight = 0.0f32;

        for y in 0..height {
            let row_offset = y * width;
            for x in 0..width {
                let p = pixels[row_offset + x];

                // 完全透明像素归一化合并到 bin 0
                let bin_idx = if p.a <= 1 {
                    0
                } else {
                    let r5 = (p.r >> 3) as usize;
                    let g5 = (p.g >> 3) as usize;
                    let b5 = (p.b >> 3) as usize;
                    let a4 = (p.a >> 4) as usize;
                    (r5 << 14) | (g5 << 9) | (b5 << 4) | a4
                };

                // 感知权重计算：
                // 不透明度较低的像素权重按比例调整，平滑半透明边缘
                let alpha_ratio = p.a as f32 / 255.0;
                let weight = 0.1 + 0.9 * alpha_ratio;

                if bin_counts[bin_idx] == 0.0 {
                    active_bins.push(bin_idx);
                }

                bin_counts[bin_idx] += weight;
                bin_sum_r[bin_idx] += p.r as f32 * weight;
                bin_sum_g[bin_idx] += p.g as f32 * weight;
                bin_sum_b[bin_idx] += p.b as f32 * weight;
                bin_sum_a[bin_idx] += p.a as f32 * weight;
                total_weight += weight;
            }
        }

        let mut items = Vec::with_capacity(active_bins.len());
        for &idx in &active_bins {
            let count = bin_counts[idx];
            if count > 0.0 {
                let avg_r = (bin_sum_r[idx] / count).round().clamp(0.0, 255.0) as u8;
                let avg_g = (bin_sum_g[idx] / count).round().clamp(0.0, 255.0) as u8;
                let avg_b = (bin_sum_b[idx] / count).round().clamp(0.0, 255.0) as u8;
                let avg_a = (bin_sum_a[idx] / count).round().clamp(0.0, 255.0) as u8;

                let rgba = RGBA::new(avg_r, avg_g, avg_b, avg_a);
                let oklab = rgba.to_oklab();

                items.push(HistItem {
                    color: oklab,
                    rgba,
                    weight: count,
                    likely_cluster: 0,
                });
            }
        }

        Self {
            items,
            total_weight,
        }
    }
}
