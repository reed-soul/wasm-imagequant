//! 核心色彩量化引擎 (Color Quantization Engine)
//!
//! 结合改良方差中位切分 (Variance-based Median Cut)、Lloyd 沃罗诺伊质心松弛 (K-Means Voronoi Iteration)
//! 与孤立点动态回填 (Outlier Recycling)，确保提取的调色板达到全局感知最优。

use crate::color::{ColorOklab, RGBA};
use crate::hist::{HistItem, Histogram};
use crate::vptree::VpTree;

/// 优化后的调色板
#[derive(Clone, Debug)]
pub struct Palette {
    pub entries: Vec<RGBA>,
    pub oklab_entries: Vec<ColorOklab>,
    pub tree: VpTree,
}

impl Palette {
    pub fn new(mut colors: Vec<RGBA>) -> Self {
        if colors.is_empty() {
            colors.push(RGBA::new(0, 0, 0, 255));
        }

        // 调色板排序：透明颜色优先排在前面，以减小 PNG tRNS 块尺寸
        colors.sort_by_key(|c| (c.a != 0, c.a < 255, c.a));

        let oklab_entries: Vec<ColorOklab> = colors.iter().map(|c| c.to_oklab()).collect();
        let tree = VpTree::new(oklab_entries.clone());

        Self {
            entries: colors,
            oklab_entries,
            tree,
        }
    }

    #[inline(always)]
    pub fn find_nearest(&self, target_oklab: &ColorOklab, last_match: u8) -> (u8, f32) {
        self.tree.find_nearest(target_oklab, last_match)
    }
}

/// 聚类方差分析盒
struct BoxCluster {
    items: Vec<usize>,
    center: ColorOklab,
    variance: [f32; 4],
    total_variance: f32,
    weight: f32,
}

pub struct PerceptualQuantizer;

impl PerceptualQuantizer {
    /// 计算某一组直方图条目的加权均值与方差
    fn compute_cluster_stats(
        item_indices: &[usize],
        hist: &[HistItem],
    ) -> (ColorOklab, [f32; 4], f32, f32) {
        let mut total_w = 0.0f32;
        let mut sum_l = 0.0f32;
        let mut sum_a = 0.0f32;
        let mut sum_b = 0.0f32;
        let mut sum_alpha = 0.0f32;

        for &idx in item_indices {
            let item = &hist[idx];
            let w = item.weight;
            total_w += w;
            sum_l += item.color.l * w;
            sum_a += item.color.a * w;
            sum_b += item.color.b * w;
            sum_alpha += item.color.alpha * w;
        }

        if total_w <= 0.0 {
            return (ColorOklab::default(), [0.0; 4], 0.0, 0.0);
        }

        let mean_l = sum_l / total_w;
        let mean_a = sum_a / total_w;
        let mean_b = sum_b / total_w;
        let mean_alpha = sum_alpha / total_w;
        let center = ColorOklab::new(mean_l, mean_a, mean_b, mean_alpha);

        let mut var_l = 0.0f32;
        let mut var_a = 0.0f32;
        let mut var_b = 0.0f32;
        let mut var_alpha = 0.0f32;

        for &idx in item_indices {
            let item = &hist[idx];
            let w = item.weight;
            let dl = item.color.l - mean_l;
            let da = item.color.a - mean_a;
            let db = item.color.b - mean_b;
            let dalpha = item.color.alpha - mean_alpha;

            var_l += dl * dl * w;
            var_a += da * da * w;
            var_b += db * db * w;
            var_alpha += dalpha * dalpha * w * 2.0;
        }

        let total_var = var_l + var_a + var_b + var_alpha;
        (
            center,
            [var_l, var_a, var_b, var_alpha],
            total_var,
            total_w,
        )
    }

    /// 第一阶段：基于方差最小化的改良中位切分 (Variance-based Median Cut)
    fn initial_variance_split(hist: &Histogram, max_colors: usize) -> Vec<ColorOklab> {
        let n = hist.items.len();
        if n <= max_colors {
            return hist.items.iter().map(|item| item.color).collect();
        }

        let all_indices: Vec<usize> = (0..n).collect();
        let (center, variance, total_variance, weight) =
            Self::compute_cluster_stats(&all_indices, &hist.items);

        let mut clusters = Vec::with_capacity(max_colors);
        clusters.push(BoxCluster {
            items: all_indices,
            center,
            variance,
            total_variance,
            weight,
        });

        while clusters.len() < max_colors {
            // 找出总体方差最大的聚类盒进行拆分
            let mut best_box_idx = None;
            let mut max_var = 0.0f32;

            for (i, c) in clusters.iter().enumerate() {
                if c.items.len() > 1 && c.total_variance > max_var {
                    max_var = c.total_variance;
                    best_box_idx = Some(i);
                }
            }

            let box_idx = match best_box_idx {
                Some(idx) => idx,
                None => break, // 所有簇均不可再分
            };

            let target_cluster = clusters.swap_remove(box_idx);
            let mut items = target_cluster.items;

            // 选取方差最大的通道维度进行排序
            let vars = target_cluster.variance;
            let mut max_dim = 0;
            let mut max_dim_var = vars[0];
            for dim in 1..4 {
                if vars[dim] > max_dim_var {
                    max_dim_var = vars[dim];
                    max_dim = dim;
                }
            }

            // 根据最大方差维度对项目进行排序
            items.sort_unstable_by(|&a, &b| {
                let ca = &hist.items[a].color;
                let cb = &hist.items[b].color;
                let val_a = match max_dim {
                    0 => ca.l,
                    1 => ca.a,
                    2 => ca.b,
                    _ => ca.alpha,
                };
                let val_b = match max_dim {
                    0 => cb.l,
                    1 => cb.a,
                    2 => cb.b,
                    _ => cb.alpha,
                };
                val_a.partial_cmp(&val_b).unwrap_or(std::cmp::Ordering::Equal)
            });

            // 寻找加权中位切分点
            let half_weight = target_cluster.weight * 0.5;
            let mut cum_w = 0.0f32;
            let mut split_pos = items.len() / 2;

            for (i, &item_idx) in items.iter().enumerate() {
                cum_w += hist.items[item_idx].weight;
                if cum_w >= half_weight && i > 0 && i < items.len() {
                    split_pos = i;
                    break;
                }
            }

            let items_right = items.split_off(split_pos);
            let items_left = items;

            let (c1, v1, tv1, w1) = Self::compute_cluster_stats(&items_left, &hist.items);
            let (c2, v2, tv2, w2) = Self::compute_cluster_stats(&items_right, &hist.items);

            clusters.push(BoxCluster {
                items: items_left,
                center: c1,
                variance: v1,
                total_variance: tv1,
                weight: w1,
            });
            clusters.push(BoxCluster {
                items: items_right,
                center: c2,
                variance: v2,
                total_variance: tv2,
                weight: w2,
            });
        }

        clusters.into_iter().map(|c| c.center).collect()
    }

    /// 第二阶段：Lloyd 沃罗诺伊质心松弛 (K-Means Voronoi Relaxation)
    ///
    /// 将中位切分的粗略调色板反复微调，向真实空间质心收敛，
    /// 并对空簇进行异常色彩（Outlier）再注入，确保无调色板插槽浪费。
    fn refine_palette_kmeans(
        initial_centroids: Vec<ColorOklab>,
        hist: &mut Histogram,
        iterations: usize,
    ) -> Vec<RGBA> {
        let k = initial_centroids.len();
        if k == 0 {
            return vec![RGBA::new(0, 0, 0, 255)];
        }

        let mut centroids = initial_centroids;

        for _ in 0..iterations {
            let tree = VpTree::new(centroids.clone());

            let mut sum_l = vec![0.0f32; k];
            let mut sum_a = vec![0.0f32; k];
            let mut sum_b = vec![0.0f32; k];
            let mut sum_alpha = vec![0.0f32; k];
            let mut cluster_weights = vec![0.0f32; k];

            let mut worst_err_sq = 0.0f32;
            let mut worst_item_idx = 0;

            for (item_idx, item) in hist.items.iter_mut().enumerate() {
                let (best_cluster, dist_sq) = tree.find_nearest(&item.color, item.likely_cluster);
                item.likely_cluster = best_cluster;
                let idx = best_cluster as usize;

                let w = item.weight;
                cluster_weights[idx] += w;
                sum_l[idx] += item.color.l * w;
                sum_a[idx] += item.color.a * w;
                sum_b[idx] += item.color.b * w;
                sum_alpha[idx] += item.color.alpha * w;

                if dist_sq > worst_err_sq {
                    worst_err_sq = dist_sq;
                    worst_item_idx = item_idx;
                }
            }

            // 更新质心
            let mut max_shift = 0.0f32;
            for i in 0..k {
                let w = cluster_weights[i];
                if w > 0.0 {
                    let new_center = ColorOklab::new(
                        sum_l[i] / w,
                        sum_a[i] / w,
                        sum_b[i] / w,
                        sum_alpha[i] / w,
                    );
                    let shift = centroids[i].perceptual_distance_sq(&new_center);
                    if shift > max_shift {
                        max_shift = shift;
                    }
                    centroids[i] = new_center;
                } else {
                    // 空聚类簇：重新激活注入误差最大的孤立样本
                    centroids[i] = hist.items[worst_item_idx].color;
                }
            }

            // 质心位移极小，提前收敛
            if max_shift < 1e-5 {
                break;
            }
        }

        centroids.into_iter().map(|ok| ok.to_rgba()).collect()
    }

    /// 执行两阶段色彩量化全流程并生成完整调色板
    pub fn quantize(hist: &mut Histogram, max_colors: usize) -> Palette {
        let initial_centroids = Self::initial_variance_split(hist, max_colors);
        let final_colors = Self::refine_palette_kmeans(initial_centroids, hist, 4);
        Palette::new(final_colors)
    }
}
