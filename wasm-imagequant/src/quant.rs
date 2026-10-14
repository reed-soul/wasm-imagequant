use crate::color::{ColorOklab, RGBA};

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
