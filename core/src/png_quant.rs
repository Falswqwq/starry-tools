//! 调色板量化。
//!
//! **这是有损的** —— 颜色数被压下去之后，原来的颜色就找不回来了。所以它只在
//! 「有损」那条路上用，和无损那条（`png_opt`）完全分开，绝不会出现在无损流程里。
//!
//! 用的是最经典的**中位切分**（median cut）：把所有出现过的颜色丢进一个盒子，
//! 反复挑「跨得最开、像素又多」的那个盒子，按最长的那条轴从中间切开，直到盒子
//! 数等于目标颜色数；每个盒子取一个加权平均色当代表。像素颜色本来就不多于目标
//! 颜色数时直接原样返回，一点都不动。

use std::collections::HashMap;

use image::{DynamicImage, Rgba, RgbaImage};

/// 把图像量化到至多 `colors` 种颜色。
pub fn quantize(image: &DynamicImage, colors: usize) -> DynamicImage {
    let target = colors.clamp(2, 256);
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return DynamicImage::ImageRgba8(rgba);
    }

    let mut counts: HashMap<[u8; 4], u32> = HashMap::new();
    for pixel in rgba.pixels() {
        *counts.entry(pixel.0).or_insert(0) += 1;
    }
    // 颜色本来就不多，量化是多余的一步。
    if counts.len() <= target {
        return DynamicImage::ImageRgba8(rgba);
    }

    let mut points: Vec<([u8; 4], u32)> = counts.into_iter().collect();
    // 排一下序，保证同样的输入每次得到同样的调色板。
    points.sort_unstable();

    let mut palette_of: HashMap<[u8; 4], [u8; 4]> = HashMap::new();
    for bucket in median_cut(points, target) {
        let representative = average(&bucket);
        for (color, _) in bucket {
            palette_of.insert(color, representative);
        }
    }

    let mut out = RgbaImage::new(width, height);
    for (destination, source) in out.pixels_mut().zip(rgba.pixels()) {
        *destination = Rgba(palette_of.get(&source.0).copied().unwrap_or(source.0));
    }
    DynamicImage::ImageRgba8(out)
}

/// 反复切盒子，直到切够 `target` 个。
fn median_cut(mut points: Vec<([u8; 4], u32)>, target: usize) -> Vec<Vec<([u8; 4], u32)>> {
    let mut buckets = vec![points.split_off(0)];

    while buckets.len() < target {
        // 挑「跨得最开、又装得最多」的那个盒子来切 —— 只按跨度挑的话，
        // 一个只有两个颜色的盒子也会一直抢着被切。
        let Some(index) = buckets
            .iter()
            .enumerate()
            .filter(|(_, bucket)| bucket.len() > 1)
            .max_by_key(|(_, bucket)| {
                let weight: u64 = bucket.iter().map(|(_, count)| u64::from(*count)).sum();
                u64::from(widest_span(bucket).1) * weight
            })
            .map(|(index, _)| index)
        else {
            break;
        };

        let mut bucket = buckets.swap_remove(index);
        let axis = widest_span(&bucket).0;
        bucket.sort_unstable_by_key(|(color, _)| color[axis]);

        // 按「像素数」而不是「颜色数」在中位处切，这样稀疏但占地的颜色不会被整块丢掉。
        let total: u64 = bucket.iter().map(|(_, count)| u64::from(*count)).sum();
        let mut running = 0u64;
        let mut split = 1;
        for (index, (_, count)) in bucket.iter().enumerate() {
            running += u64::from(*count);
            if running * 2 >= total {
                split = (index + 1).min(bucket.len() - 1).max(1);
                break;
            }
        }

        let right = bucket.split_off(split);
        buckets.push(bucket);
        buckets.push(right);
    }

    buckets
}

/// 盒子在哪条轴上跨得最开，以及跨了多少。
fn widest_span(bucket: &[([u8; 4], u32)]) -> (usize, u8) {
    let mut low = [u8::MAX; 4];
    let mut high = [u8::MIN; 4];
    for (color, _) in bucket {
        for channel in 0..4 {
            low[channel] = low[channel].min(color[channel]);
            high[channel] = high[channel].max(color[channel]);
        }
    }
    (0..4)
        .map(|channel| (channel, high[channel] - low[channel]))
        .max_by_key(|(_, span)| *span)
        .unwrap_or((0, 0))
}

/// 盒子里的代表色：按出现次数加权平均。
fn average(bucket: &[([u8; 4], u32)]) -> [u8; 4] {
    let mut sums = [0u64; 4];
    let mut total = 0u64;
    for (color, count) in bucket {
        for channel in 0..4 {
            sums[channel] += u64::from(color[channel]) * u64::from(*count);
        }
        total += u64::from(*count);
    }
    if total == 0 {
        return [0, 0, 0, 255];
    }
    let mut out = [0u8; 4];
    for channel in 0..4 {
        out[channel] = ((sums[channel] + total / 2) / total) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageBuffer};
    use std::collections::HashSet;

    /// 一张颜色又多又散、不透明的图。
    fn noisy() -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_fn(64, 64, |x, y| {
            let mut value = x
                .wrapping_mul(374_761_393)
                .wrapping_add(y.wrapping_mul(668_265_263));
            value ^= value >> 13;
            value = value.wrapping_mul(1_274_126_177);
            let value = value ^ (value >> 16);
            Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
        }))
    }

    fn distinct(image: &DynamicImage) -> usize {
        image
            .to_rgba8()
            .pixels()
            .map(|pixel| pixel.0)
            .collect::<HashSet<_>>()
            .len()
    }

    #[test]
    fn quantizing_brings_the_colour_count_down() {
        let image = noisy();
        assert!(distinct(&image) > 1000);

        let quantized = quantize(&image, 16);
        assert!(
            distinct(&quantized) <= 16,
            "量化后还剩 {} 种颜色",
            distinct(&quantized)
        );
        assert_eq!(quantized.dimensions(), image.dimensions(), "尺寸不能变");
    }

    #[test]
    fn an_image_already_within_the_limit_is_returned_untouched() {
        // 颜色本来就不多于上限时，量化应该是一个像素都不动。
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(8, 8, |x, y| {
            Rgba([(x % 3) as u8 * 40, (y % 2) as u8 * 60, 7, 255])
        }));
        let quantized = quantize(&image, 64);
        assert_eq!(
            quantized.to_rgba8().as_raw(),
            image.to_rgba8().as_raw(),
            "没超上限就不该改"
        );
    }

    #[test]
    fn the_same_input_gives_the_same_palette() {
        let image = noisy();
        let first = quantize(&image, 32).to_rgba8();
        let second = quantize(&image, 32).to_rgba8();
        assert_eq!(first.as_raw(), second.as_raw(), "结果得是确定的");
    }

    #[test]
    fn the_colour_limit_is_clamped_to_something_sane() {
        let image = noisy();
        assert!(distinct(&quantize(&image, 1)) <= 2, "最少也得给两种颜色");
        assert!(distinct(&quantize(&image, 9999)) <= 256, "最多 256 种");
    }
}
