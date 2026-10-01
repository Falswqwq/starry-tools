//! 背景移除的推理：跑 u2net 那几个 ONNX 模型。
//!
//! 这里是 rembg 那条流水线的 Rust 版：把图缩到 320×320、按 ImageNet 的均值方差
//! 归一化、喂给模型，再把模型吐出来的那张遮罩缩回原尺寸当 alpha。
//!
//! 不启用 `onnx` 特性时只有一个会报错的桩 —— 节点会给出一句明确的提示，而不是静默地
//! 什么都干不了。

use std::path::Path;

use image::{imageops::FilterType, GrayImage, Luma, RgbaImage};
#[cfg(feature = "onnx")]
use tract_onnx::prelude::tract_ndarray;

/// 模型输入的边长（u2net 系列固定是 320）。
const SIZE: usize = 320;
/// rembg 用的 ImageNet 归一化参数。
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// 把背景抠掉：跑模型、拿遮罩、当成 alpha 盖回去。
#[cfg(feature = "onnx")]
pub fn remove_background(image: &RgbaImage, model_path: &Path) -> Result<RgbaImage, String> {
    use tract_onnx::prelude::*;

    let model = tract_onnx::onnx()
        .model_for_path(model_path)
        .map_err(|err| format!("读模型失败：{err:?}"))?
        .into_optimized()
        .map_err(|err| format!("优化模型失败：{err}"))?
        .into_runnable()
        .map_err(|err| format!("准备模型失败：{err}"))?;

    let input: Tensor = preprocess(image).into();
    let outputs = model
        .run(tvec!(input.into()))
        .map_err(|err| format!("推理失败：{err}"))?;
    let mask = outputs[0]
        .to_plain_array_view::<f32>()
        .map_err(|err| format!("读输出失败：{err}"))?
        .into_dimensionality::<tract_ndarray::Ix4>()
        .map_err(|err| format!("输出形状不对：{err}"))?;
    Ok(apply_mask(image, &mask))
}

/// 没开 `onnx` 特性时的桩。
#[cfg(not(feature = "onnx"))]
pub fn remove_background(_image: &RgbaImage, _model_path: &Path) -> Result<RgbaImage, String> {
    Err("这次构建没有启用 ONNX 推理（用 `--features onnx` 重新构建）".to_string())
}

/// 缩到 320×320、按 rembg 的方式来一遍归一化，得到一个 `[1, 3, 320, 320]` 的张量。
#[cfg(feature = "onnx")]
fn preprocess(image: &RgbaImage) -> tract_ndarray::Array4<f32> {
    let resized = image::imageops::resize(image, SIZE as u32, SIZE as u32, FilterType::Triangle);
    // rembg 先整图除以最大值，再按 ImageNet 的均值方差归一化。
    let peak = resized
        .pixels()
        .flat_map(|pixel| pixel.0[..3].iter())
        .map(|channel| f32::from(*channel) / 255.0)
        .fold(0.0f32, f32::max)
        .max(f32::MIN_POSITIVE);

    tract_ndarray::Array4::from_shape_fn((1, 3, SIZE, SIZE), |(_, channel, y, x)| {
        let pixel = resized.get_pixel(x as u32, y as u32).0;
        let value = f32::from(pixel[channel]) / 255.0 / peak;
        (value - MEAN[channel]) / STD[channel]
    })
}

/// 把模型的遮罩盖回原图的 alpha 上。
///
/// 先按最大最小把遮罩拉回 0–1（模型输出不一定在范围内），再缩放到原尺寸 ——
/// 和 rembg 一致。alpha 就是遮罩值：背景处接近 0、主体处接近 255。
#[cfg(feature = "onnx")]
fn apply_mask(image: &RgbaImage, mask: &tract_ndarray::ArrayView4<f32>) -> RgbaImage {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for value in mask.iter() {
        min = min.min(*value);
        max = max.max(*value);
    }
    // 遮罩整个是一个常数时（全 0 或全 1），min-max 拉伸没有意义 —— 直接用它本身。
    let constant = max - min <= 1e-6;
    let span = (max - min).max(1e-6);

    let (mask_h, mask_w) = (mask.shape()[2], mask.shape()[3]);
    let mut gray = GrayImage::new(mask_w as u32, mask_h as u32);
    for y in 0..mask_h {
        for x in 0..mask_w {
            let raw = mask[[0, 0, y, x]];
            let value = if constant {
                raw.clamp(0.0, 1.0)
            } else {
                ((raw - min) / span).clamp(0.0, 1.0)
            };
            gray.put_pixel(x as u32, y as u32, Luma([(value * 255.0).round() as u8]));
        }
    }

    let scaled =
        image::imageops::resize(&gray, image.width(), image.height(), FilterType::Triangle);
    let mut out = image.clone();
    for (pixel, mask) in out.pixels_mut().zip(scaled.pixels()) {
        pixel.0[3] = mask.0[0];
    }
    out
}

#[cfg(all(test, feature = "onnx"))]
mod tests {
    use super::*;

    /// 真跑一遍模型（需要一个真实的模型文件，所以默认忽略）。
    ///
    /// 验证：能在本地拿到模型 → 能加载 → 能推理 → 输出能变成合理的 alpha。
    #[test]
    #[ignore = "需要一个真实的 onnx 模型"]
    fn smoke_real_model() {
        let model_path = crate::bg_model::path_for(crate::bg_model::default_model());
        if !model_path.is_file() {
            eprintln!("跳过：本机没有 {}", model_path.display());
            return;
        }
        // 左半白、右半黑的简单图：期望左、右两边的 alpha 明显不同。
        let mut image = RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
        for y in 0..64 {
            for x in 32..64 {
                image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let out = remove_background(&image, &model_path).expect("跑通推理");
        assert_eq!(out.dimensions(), image.dimensions(), "尺寸不变");
        let left = out.get_pixel(4, 4).0[3];
        let right = out.get_pixel(60, 60).0[3];
        eprintln!("alpha 左={left} 右={right}");
        assert!(left != right, "两边应该不一样（模型确实在做事）");
    }

    /// 遮罩归一化 + 缩放：全 0 的遮罩会把整张图变透明，全 1 的会全部留下。
    #[test]
    fn the_mask_becomes_the_alpha() {
        let image = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));

        let transparent = tract_ndarray::Array4::<f32>::zeros((1, 1, SIZE, SIZE));
        let out = apply_mask(&image, &transparent.view());
        assert!(
            out.pixels().all(|pixel| pixel.0[3] == 0),
            "全 0 遮罩 → 全透明"
        );
        assert_eq!(out.get_pixel(0, 0).0[..3], [10, 20, 30], "RGB 不动");

        let opaque = tract_ndarray::Array4::<f32>::ones((1, 1, SIZE, SIZE));
        let out = apply_mask(&image, &opaque.view());
        assert!(
            out.pixels().all(|pixel| pixel.0[3] == 255),
            "全 1 遮罩 → 全保留"
        );
    }
}
