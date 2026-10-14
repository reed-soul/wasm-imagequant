use wasm_imagequant::*;

#[test]
fn test_color_oklab_roundtrip() {
    let test_colors = [
        RGBA::new(255, 0, 0, 255),
        RGBA::new(0, 255, 0, 255),
        RGBA::new(0, 0, 255, 255),
        RGBA::new(255, 255, 255, 255),
        RGBA::new(0, 0, 0, 255),
        RGBA::new(128, 128, 128, 128),
        RGBA::new(0, 0, 0, 0),
        RGBA::new(255, 200, 100, 200),
    ];

    for orig in test_colors {
        let oklab = orig.to_oklab();
        let recon = oklab.to_rgba();
        if orig.a == 0 {
            assert_eq!(recon.a, 0);
        } else {
            assert!((orig.r as i32 - recon.r as i32).abs() <= 1);
            assert!((orig.g as i32 - recon.g as i32).abs() <= 1);
            assert!((orig.b as i32 - recon.b as i32).abs() <= 1);
            assert!((orig.a as i32 - recon.a as i32).abs() <= 1);
        }
    }
}

#[test]
fn test_vptree_search_accuracy() {
    let mut palette_colors = Vec::new();
    for r in (0..=255).step_by(51) {
        for g in (0..=255).step_by(51) {
            for b in (0..=255).step_by(51) {
                palette_colors.push(RGBA::new(r as u8, g as u8, b as u8, 255));
            }
        }
    }

    let pal = Palette::new(palette_colors);

    // 测试 100 个随机颜色，对比 VP-Tree 与暴力全局搜索的结果是否严格一致
    for i in 0..100 {
        let test_color = RGBA::new(
            ((i * 37 + 13) % 256) as u8,
            ((i * 73 + 47) % 256) as u8,
            ((i * 101 + 89) % 256) as u8,
            255,
        );
        let target_oklab = test_color.to_oklab();

        // 1. VP-Tree 搜索
        let (vp_idx, vp_dist) = pal.find_nearest(&target_oklab, 0);

        // 2. 暴力全局遍历搜索
        let mut brute_idx = 0u8;
        let mut brute_dist = f32::MAX;
        for (idx, color) in pal.oklab_entries.iter().enumerate() {
            let d = target_oklab.perceptual_distance_sq(color);
            if d < brute_dist {
                brute_dist = d;
                brute_idx = idx as u8;
            }
        }

        // 距离应该完全一致
        assert!(
            (vp_dist - brute_dist).abs() < 1e-4,
            "VP-Tree distance mismatch: {} vs {}",
            vp_dist,
            brute_dist
        );
        assert_eq!(
            pal.entries[vp_idx as usize], pal.entries[brute_idx as usize],
            "Selected color mismatch"
        );
    }
}

#[test]
fn test_adaptive_bit_depth_compression() {
    // 1. 双色黑白图像 (应触发 1-bit PNG)
    let width = 64;
    let height = 64;
    let mut pixels_bw = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let val = if (x + y) % 2 == 0 { 255 } else { 0 };
            pixels_bw.extend_from_slice(&[val, val, val, 255]);
        }
    }

    let png_1bit = quantize_and_compress(&pixels_bw, width as u32, height as u32, 2, 0.0)
        .expect("Compression failed");

    // 检查 IHDR 第 24 字节（位深应为 1）
    assert_eq!(png_1bit[24], 1, "Expected 1-bit PNG depth");

    // 2. 彩色丰富渐变图像 (应输出 8-bit PNG)
    let mut pixels_grad = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let r = (x * 255 / width) as u8;
            let g = (y * 255 / height) as u8;
            let b = ((x + y) * 128 / (width + height)) as u8;
            pixels_grad.extend_from_slice(&[r, g, b, 255]);
        }
    }

    let png_8bit = quantize_and_compress(&pixels_grad, width as u32, height as u32, 256, 0.8)
        .expect("Compression failed");

    assert_eq!(png_8bit[24], 8, "Expected 8-bit PNG depth");
    assert!(png_8bit.len() > 0);
}

#[test]
fn test_transparency_handling() {
    let width = 32;
    let height = 32;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let a = if x < 16 { 0 } else { ((y * 255) / height) as u8 };
            pixels.extend_from_slice(&[255, 100, 50, a]);
        }
    }

    let png_out = quantize_and_compress(&pixels, width as u32, height as u32, 64, 0.8)
        .expect("Compression failed");

    // 确保包含 tRNS Chunk
    let has_trns = png_out.windows(4).any(|w| w == b"tRNS");
    assert!(has_trns, "Expected tRNS chunk in transparent PNG");
}
