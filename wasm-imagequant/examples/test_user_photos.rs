//! 针对用户真实高分辨率照片的基准测试与画质对比程序
//! 测试图片路径: /mnt/c/Users/Sutpc/Pictures/Camera Roll/
//! 1. bingimg_20230504_UHD.jpg (1080p)
//! 2. bingimg_20230806_UHD.jpg (4K UHD)
//! 3. 【哲风壁纸】海边-自然-风景.png (1200万像素 大图)

use std::fs;
use std::path::Path;
use std::time::Instant;
use wasm_imagequant::{
    encode_indexed_png, DitherEngine, Histogram, PerceptualQuantizer, RGBA,
};

#[allow(dead_code)]
struct BenchmarkResult {
    name: String,
    dimensions: (u32, u32),
    raw_size_mb: f64,
    
    // wasm-imagequant
    our_time_ms: f64,
    our_png_size_kb: f64,
    our_mse: f64,
    our_psnr: f64,
    
    // libimagequant
    liq_time_ms: f64,
    liq_png_size_kb: f64,
    liq_mse: f64,
    liq_psnr: f64,
}

fn calculate_metrics(orig: &[RGBA], remapped: &[RGBA]) -> (f64, f64) {
    let mut total_mse = 0.0f64;
    let mut total_srgb_sq = 0.0f64;
    let n = orig.len() as f64;

    for (p1, p2) in orig.iter().zip(remapped.iter()) {
        let ok1 = p1.to_oklab();
        let ok2 = p2.to_oklab();
        let d = ok1.perceptual_distance_sq(&ok2);
        total_mse += d as f64;

        let dr = (p1.r as f64) - (p2.r as f64);
        let dg = (p1.g as f64) - (p2.g as f64);
        let db = (p1.b as f64) - (p2.b as f64);
        let da = (p1.a as f64) - (p2.a as f64);
        total_srgb_sq += (dr * dr + dg * dg + db * db + da * da) / 4.0;
    }

    let oklab_mse = total_mse / n;
    let srgb_mse = total_srgb_sq / n;
    let psnr = if srgb_mse <= 1e-10 {
        99.0
    } else {
        10.0 * (255.0 * 255.0 / srgb_mse).log10()
    };

    (oklab_mse, psnr)
}

fn test_image(path: &str, output_dir: &str) -> BenchmarkResult {
    let file_name = Path::new(path).file_name().unwrap().to_str().unwrap().to_string();
    println!("\n=======================================================");
    println!("正在处理测试照片: {}", file_name);
    println!("=======================================================");

    let img_reader = image::io::Reader::open(path)
        .expect("Failed to open file")
        .with_guessed_format()
        .expect("Failed to guess format");
    let img = img_reader.decode().expect("Failed to decode image").to_rgba8();
    let (width, height) = img.dimensions();
    let raw_bytes = img.into_raw();
    let num_pixels = (width * height) as usize;
    let raw_size_mb = (raw_bytes.len() as f64) / (1024.0 * 1024.0);

    println!("图像规格: {}x{} 像素 (共 {:.2} 百万像素, RGBA 原始大小 {:.2} MB)",
        width, height, num_pixels as f64 / 1_000_000.0, raw_size_mb);

    let orig_pixels: Vec<RGBA> = raw_bytes
        .chunks_exact(4)
        .map(|c| RGBA::new(c[0], c[1], c[2], c[3]))
        .collect();

    // -------------------------------------------------------------
    // 1. wasm-imagequant
    // -------------------------------------------------------------
    println!("  -> [1/2] 运行 wasm-imagequant (Oklab 自研引擎)...");
    let t_start = Instant::now();
    let t_h_start = Instant::now();
    let mut hist = Histogram::build(&orig_pixels, width as usize, height as usize);
    let t_hist = t_h_start.elapsed();

    let t_q_start = Instant::now();
    let pal = PerceptualQuantizer::quantize(&mut hist, 256);
    let t_quant = t_q_start.elapsed();

    let t_d_start = Instant::now();
    let indices = DitherEngine::dither(&orig_pixels, width as usize, height as usize, &pal, 0.8);
    let t_dither = t_d_start.elapsed();
    let our_remap_time = t_start.elapsed();

    let t_p_start = Instant::now();
    let our_png = encode_indexed_png(&indices, width, height, &pal.entries).unwrap();
    let t_png = t_p_start.elapsed();
    let our_total_time = t_start.elapsed();
    let our_png_size_kb = our_png.len() as f64 / 1024.0;

    let our_remapped: Vec<RGBA> = indices.iter().map(|&i| pal.entries[i as usize]).collect();
    let (our_mse, our_psnr) = calculate_metrics(&orig_pixels, &our_remapped);

    println!("     完成！纯量化重映射耗时: {:.1} ms (直方图: {:.1}ms, 聚类: {:.1}ms, 抖动重映射: {:.1}ms)",
        our_remap_time.as_secs_f64() * 1000.0,
        t_hist.as_secs_f64() * 1000.0,
        t_quant.as_secs_f64() * 1000.0,
        t_dither.as_secs_f64() * 1000.0);
    println!("     PNG 编码耗时: {:.1} ms, 全流程总耗时: {:.1} ms",
        t_png.as_secs_f64() * 1000.0, our_total_time.as_secs_f64() * 1000.0);
    println!("     输出体积: {:.2} KB (压缩率: {:.2}%) | Oklab MSE: {:.6} | PSNR: {:.2} dB",
        our_png_size_kb, (1.0 - (our_png.len() as f64 / raw_bytes.len() as f64)) * 100.0, our_mse, our_psnr);

    // 保存 wasm-imagequant 压缩后的图片
    let our_out_path = format!("{}/{}_wasm_quant.png", output_dir, file_name);
    fs::write(&our_out_path, &our_png).expect("Failed to write our PNG");

    // -------------------------------------------------------------
    // 2. libimagequant (官方 Rust 版本对比)
    // -------------------------------------------------------------
    println!("  -> [2/2] 运行 libimagequant (对比基准)...");
    let t_liq_start = Instant::now();
    let mut liq_attr = imagequant::new();
    liq_attr.set_max_colors(256).unwrap();

    let liq_pixels: Vec<imagequant::RGBA> = raw_bytes
        .chunks_exact(4)
        .map(|c| imagequant::RGBA::new(c[0], c[1], c[2], c[3]))
        .collect();

    let mut liq_img = liq_attr.new_image(&liq_pixels[..], width as usize, height as usize, 0.0).unwrap();
    let mut liq_res = liq_attr.quantize(&mut liq_img).unwrap();
    liq_res.set_dithering_level(0.8).unwrap();

    let mut liq_indices = vec![std::mem::MaybeUninit::uninit(); num_pixels];
    liq_res.remap_into(&mut liq_img, &mut liq_indices).unwrap();
    let liq_remap_time = t_liq_start.elapsed();

    let liq_pal_raw = liq_res.palette();
    let liq_pal_rgba: Vec<RGBA> = liq_pal_raw.iter().map(|c| RGBA::new(c.r, c.g, c.b, c.a)).collect();
    let init_indices: Vec<u8> = liq_indices.iter().map(|idx| unsafe { idx.assume_init() }).collect();

    let liq_png = encode_indexed_png(&init_indices, width, height, &liq_pal_rgba).unwrap();
    let liq_png_size_kb = liq_png.len() as f64 / 1024.0;

    let liq_remapped: Vec<RGBA> = init_indices.iter().map(|&i| liq_pal_rgba[i as usize]).collect();
    let (liq_mse, liq_psnr) = calculate_metrics(&orig_pixels, &liq_remapped);

    println!("     完成！纯量化重映射耗时: {:.1} ms", liq_remap_time.as_secs_f64() * 1000.0);
    println!("     输出体积: {:.2} KB (压缩率: {:.2}%) | Oklab MSE: {:.6} | PSNR: {:.2} dB",
        liq_png_size_kb, (1.0 - (liq_png.len() as f64 / raw_bytes.len() as f64)) * 100.0, liq_mse, liq_psnr);

    // 保存 libimagequant 压缩后的图片
    let liq_out_path = format!("{}/{}_libimagequant.png", output_dir, file_name);
    fs::write(&liq_out_path, &liq_png).expect("Failed to write liq PNG");

    BenchmarkResult {
        name: file_name,
        dimensions: (width, height),
        raw_size_mb,
        our_time_ms: our_remap_time.as_secs_f64() * 1000.0,
        our_png_size_kb,
        our_mse,
        our_psnr,
        liq_time_ms: liq_remap_time.as_secs_f64() * 1000.0,
        liq_png_size_kb,
        liq_mse,
        liq_psnr,
    }
}

fn main() {
    let output_dir = "/home/sutpc/code/temp/wasm-imagequant/test_output";
    fs::create_dir_all(output_dir).unwrap();

    let images = [
        "/mnt/c/Users/Sutpc/Pictures/Camera Roll/bingimg_20230504_UHD.jpg",
        "/mnt/c/Users/Sutpc/Pictures/Camera Roll/bingimg_20230806_UHD.jpg",
        "/mnt/c/Users/Sutpc/Pictures/Camera Roll/【哲风壁纸】海边-自然-风景.png",
    ];

    let mut results = Vec::new();
    for img_path in &images {
        if Path::new(img_path).exists() {
            let res = test_image(img_path, output_dir);
            results.push(res);
        } else {
            eprintln!("警告: 图片文件不存在: {}", img_path);
        }
    }

    println!("\n\n=========================================================================================");
    println!("                    真实高分辨率照片综合测试对比总结报告                                  ");
    println!("=========================================================================================");
    println!("{:<24} | {:<10} | {:<16} | {:<16} | {:<12} | {:<10}",
        "测试照片", "分辨率", "wasm-imagequant耗时", "libimagequant耗时", "MSE (保真度)", "体积节省");
    println!("-----------------------------------------------------------------------------------------");

    for r in &results {
        let speed_ratio = r.liq_time_ms / r.our_time_ms;
        let speed_str = if speed_ratio > 1.0 {
            format!("{:.1}ms ({:.2}x快)", r.our_time_ms, speed_ratio)
        } else {
            format!("{:.1}ms ({:.2}x)", r.our_time_ms, speed_ratio)
        };
        let liq_speed_str = format!("{:.1}ms", r.liq_time_ms);
        let mse_comp = if r.our_mse < r.liq_mse {
            format!("优于 -{:.1}%", (1.0 - r.our_mse / r.liq_mse) * 100.0)
        } else {
            format!("+{:.2}%", (r.our_mse / r.liq_mse - 1.0) * 100.0)
        };
        let size_diff = (r.liq_png_size_kb - r.our_png_size_kb) / r.liq_png_size_kb * 100.0;
        let size_str = if size_diff > 0.0 {
            format!("小 {:.1}%", size_diff)
        } else {
            format!("大 {:.1}%", -size_diff)
        };

        println!("{:<24} | {:<4}x{:<5} | {:<19} | {:<17} | {:<14} | {:<10}",
            r.name, r.dimensions.0, r.dimensions.1, speed_str, liq_speed_str, mse_comp, size_str);
    }
    println!("=========================================================================================\n");
}
