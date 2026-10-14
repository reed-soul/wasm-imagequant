#[derive(Copy, Clone, Debug, Default, PartialEq)]
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
