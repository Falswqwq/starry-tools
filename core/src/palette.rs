//! 主题色提取。
//!
//! 「色彩分析 · 提取调色板」背后有三种算法，各有各的脾气：
//!
//! * **K-Means + OKLab** —— 在**感知均匀**的 OKLab 空间里聚类。颜色之间的距离和人眼
//!   看到的一致（暗红和亮红不会被算得很远、而两种鲜艳色的差别不会被算得很近），
//!   所以对比度高的颜色也能凭自己的「分量」占到一席之地，不只是被像素多少决定。
//! * **Median Cut** —— 经典的中位切分，很快，像素画 / 索引色转换的老朋友。
//!   在 RGB 里按像素数切，谁的像素多谁说了算。
//! * **K-Medoids + OKLab** —— 和 K-Means 一样在 OKLab 里聚类，但每簇的代表必须是
//!   **原图里真实存在的颜色**（medoid），而不是算出来的平均色。适合要「颜色自然」的场合。
//!
//! 后两种之外，颜色太多时会先把颜色按分位压到一个样本上限再聚类，保证速度。

use std::collections::HashMap;

use image::DynamicImage;

use crate::png_quant;

/// 聚类时的采样上限：颜色比这还多就先压一压，免得聚类慢到不像话。
const SAMPLE_CAP: usize = 4096;
/// 迭代上限。收敛就提前停。
const MAX_ITER: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    KMeans,
    MedianCut,
    KMedoids,
}

impl Algorithm {
    /// 从参数里的字符串解回算法。认不出来就当 K-Means。
    pub fn parse(value: &str) -> Self {
        match value {
            "medianCut" => Algorithm::MedianCut,
            "kMedoids" => Algorithm::KMedoids,
            _ => Algorithm::KMeans,
        }
    }
}

/// 从图像里提取至多 `colors` 种主题色（RGB）。
///
/// 全透明的像素不算；半透明的按 RGB 算。颜色本来就不多于 `colors` 时原样列出。
pub fn extract(image: &DynamicImage, colors: usize, algorithm: Algorithm) -> Vec<[u8; 3]> {
    // Median Cut 走老的量化器 —— 一模一样的东西没必要写两遍。
    if algorithm == Algorithm::MedianCut {
        return png_quant::representative_colors(image, colors);
    }

    let sample = weighted_colors(image);
    if sample.is_empty() {
        return Vec::new();
    }
    let points: Vec<[f32; 3]> = sample.iter().map(|(rgb, _)| oklab(*rgb)).collect();
    let weights: Vec<f32> = sample.iter().map(|(_, weight)| *weight).collect();
    let k = colors.clamp(2, 256).min(points.len());

    match algorithm {
        Algorithm::KMedoids => {
            // medoid 本身就是原图里的点，直接把它的 RGB 拿回来。
            k_medoids(&points, &weights, k)
                .into_iter()
                .map(|index| sample[index].0)
                .collect()
        }
        _ => k_means(&points, &weights, k)
            .into_iter()
            .map(oklab_to_rgb)
            .collect(),
    }
}

/// 图像里出现过的颜色 + 它们的像素数（权重）。颜色太多时先按 4 位/分量分桶压一压。
fn weighted_colors(image: &DynamicImage) -> Vec<([u8; 3], f32)> {
    let rgba = image.to_rgba8();
    let mut counts: HashMap<[u8; 3], u64> = HashMap::new();
    for pixel in rgba.pixels() {
        if pixel.0[3] == 0 {
            continue;
        }
        *counts
            .entry([pixel.0[0], pixel.0[1], pixel.0[2]])
            .or_insert(0) += 1;
    }
    if counts.is_empty() {
        return Vec::new();
    }

    let mut list: Vec<([u8; 3], u64)> = counts.into_iter().collect();
    // 排一下序，保证同样的输入每次得到同样的结果。
    list.sort_unstable();
    if list.len() <= SAMPLE_CAP {
        return list
            .into_iter()
            .map(|(color, count)| (color, count as f32))
            .collect();
    }

    // 太多颜色：按 4 位/分量（16 级）分桶，桶内取加权平均色。
    let mut buckets: HashMap<(u8, u8, u8), ([u64; 3], u64)> = HashMap::new();
    for (color, count) in list {
        let key = (color[0] >> 4, color[1] >> 4, color[2] >> 4);
        let entry = buckets.entry(key).or_insert(([0; 3], 0));
        for (channel, value) in color.iter().enumerate() {
            entry.0[channel] += u64::from(*value) * count;
        }
        entry.1 += count;
    }
    let mut reduced: Vec<([u8; 3], u64)> = buckets
        .into_values()
        .map(|(sums, count)| {
            let average = |index: usize| ((sums[index] + count / 2) / count.max(1)) as u8;
            ([average(0), average(1), average(2)], count)
        })
        .collect();
    reduced.sort_unstable();
    reduced
        .into_iter()
        .map(|(color, count)| (color, count as f32))
        .collect()
}

/// K-Means：一堆点、带权重，聚成 `k` 簇，返回每簇的质心（OKLab）。
fn k_means(points: &[[f32; 3]], weights: &[f32], k: usize) -> Vec<[f32; 3]> {
    let mut centroids: Vec<[f32; 3]> = seeded_indices(points, weights, k)
        .into_iter()
        .map(|index| points[index])
        .collect();
    if centroids.is_empty() {
        return centroids;
    }

    let mut assignment = vec![usize::MAX; points.len()];
    for _ in 0..MAX_ITER {
        let mut changed = false;
        for (index, point) in points.iter().enumerate() {
            let nearest = nearest_centroid(point, &centroids);
            if assignment[index] != nearest {
                assignment[index] = nearest;
                changed = true;
            }
        }

        let mut sums = vec![[0.0f32; 3]; centroids.len()];
        let mut totals = vec![0.0f32; centroids.len()];
        for (index, point) in points.iter().enumerate() {
            let cluster = assignment[index];
            if cluster == usize::MAX {
                continue;
            }
            for channel in 0..3 {
                sums[cluster][channel] += point[channel] * weights[index];
            }
            totals[cluster] += weights[index];
        }
        for (cluster, centroid) in centroids.iter_mut().enumerate() {
            if totals[cluster] > 0.0 {
                for channel in 0..3 {
                    centroid[channel] = sums[cluster][channel] / totals[cluster];
                }
            }
        }

        if !changed {
            break;
        }
    }
    centroids
}

/// K-Medoids：和 K-Means 一样迭代，但每簇的代表是簇里**真实存在**的那个点。
fn k_medoids(points: &[[f32; 3]], weights: &[f32], k: usize) -> Vec<usize> {
    let mut medoids = seeded_indices(points, weights, k);
    if medoids.is_empty() {
        return medoids;
    }

    for _ in 0..MAX_ITER {
        // 每个点归到最近的 medoid。
        let mut clusters: Vec<Vec<usize>> = vec![Vec::new(); medoids.len()];
        for (index, point) in points.iter().enumerate() {
            let nearest = medoids
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    dist2(point, &points[**a])
                        .partial_cmp(&dist2(point, &points[**b]))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(slot, _)| slot)
                .unwrap_or(0);
            clusters[nearest].push(index);
        }

        let mut changed = false;
        for (slot, members) in clusters.iter().enumerate() {
            // 挑「到同簇其他点的加权距离和」最小的那个当新 medoid。
            let Some(&best) = members.iter().min_by(|a, b| {
                let cost = |candidate: usize| -> f64 {
                    members
                        .iter()
                        .map(|member| {
                            f64::from(dist2(&points[candidate], &points[*member]))
                                * f64::from(weights[*member])
                        })
                        .sum()
                };
                cost(**a)
                    .partial_cmp(&cost(**b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) else {
                continue;
            };
            if medoids[slot] != best {
                medoids[slot] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    medoids
}

/// 加权 k-means++ 的初始中心：第一个取像素最多的颜色，之后按「离已选中心越远、
/// 像素越多越容易被选中」抽样。对比度高的颜色因此更容易自己占一席。
///
/// 用固定种子的 PRNG，所以同样的输入每次都得到同样的结果。
fn seeded_indices(points: &[[f32; 3]], weights: &[f32], k: usize) -> Vec<usize> {
    if points.is_empty() || k == 0 {
        return Vec::new();
    }
    let first = points
        .iter()
        .enumerate()
        .max_by(|(i, _), (j, _)| {
            weights[*i]
                .partial_cmp(&weights[*j])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);

    let mut chosen = vec![first];
    let mut nearest: Vec<f32> = points
        .iter()
        .map(|point| dist2(point, &points[first]))
        .collect();

    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    while chosen.len() < k {
        let total: f64 = nearest
            .iter()
            .zip(weights)
            .map(|(distance, weight)| f64::from(*distance) * f64::from(*weight))
            .sum();
        if total <= 0.0 {
            break;
        }
        let mut target = f64::from(rng.next_f32()) * total;
        let mut pick = chosen[0];
        for (index, distance) in nearest.iter().enumerate() {
            target -= f64::from(*distance) * f64::from(weights[index]);
            if target <= 0.0 {
                pick = index;
                break;
            }
        }
        // 别选到已经选过的。
        while chosen.contains(&pick) {
            pick = (pick + 1) % points.len();
        }
        chosen.push(pick);
        for (index, point) in points.iter().enumerate() {
            nearest[index] = nearest[index].min(dist2(point, &points[pick]));
        }
    }
    chosen
}

fn nearest_centroid(point: &[f32; 3], centroids: &[[f32; 3]]) -> usize {
    centroids
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            dist2(point, a)
                .partial_cmp(&dist2(point, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let (dl, da, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dl * dl + da * da + db * db
}

/// 一个确定性 PRNG（SplitMix64）—— 只为 k-means++ 抽样用，不引入依赖。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [0, 1) 的浮点。
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------------------
// OKLab
// ---------------------------------------------------------------------------

/// 把颜色字符串解成 RGBA。`transparent` → 全透明；坏值 → 黑。
///
/// 值可能是一整段**色板文本**（「色彩分析」的输出，hex 一行一个）—— 那时取第一行：
/// 需要一个颜色、却拿到一板多色的，就用第一个。这个规则对所有「颜色」参数都成立。
pub fn parse_color(value: &str) -> [u8; 4] {
    let value = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if value.eq_ignore_ascii_case("transparent") || value.is_empty() {
        return [0, 0, 0, 0];
    }
    let hex = value.trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return [0, 0, 0, 255];
    }
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
    match hex.len() {
        6 => [byte(0), byte(2), byte(4), 255],
        8 => [byte(0), byte(2), byte(4), byte(6)],
        _ => [0, 0, 0, 255],
    }
}

/// sRGB（字节）→ OKLab。OKLab 是个感知均匀的空间：数值上的距离≈人眼看到的差别。
pub fn oklab(rgb: [u8; 3]) -> [f32; 3] {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(rgb[0]), linear(rgb[1]), linear(rgb[2]));

    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    ]
}

/// OKLab → sRGB（字节）。超出色域的会被夹回 0–255。
pub fn oklab_to_rgb(lab: [f32; 3]) -> [u8; 3] {
    let l_ = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m_ = lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2];
    let s_ = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];

    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

    let encode = |value: f32| {
        let value = value.clamp(0.0, 1.0);
        let srgb = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [encode(r), encode(g), encode(b)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn image_of(colors: &[([u8; 3], usize)]) -> RgbaImage {
        let total: usize = colors.iter().map(|(_, n)| n).sum();
        let mut image = RgbaImage::new(total as u32, 1);
        let mut x = 0;
        for (color, count) in colors {
            for _ in 0..*count {
                image.put_pixel(x, 0, Rgba([color[0], color[1], color[2], 255]));
                x += 1;
            }
        }
        image
    }

    fn extract(colors: &[([u8; 3], usize)], n: usize, algorithm: Algorithm) -> Vec<[u8; 3]> {
        let image = DynamicImage::ImageRgba8(image_of(colors));
        super::extract(&image, n, algorithm)
    }

    #[test]
    fn colours_parse_and_bad_values_fall_back_to_black() {
        assert_eq!(parse_color("#ff0000"), [255, 0, 0, 255]);
        assert_eq!(parse_color("transparent"), [0, 0, 0, 0]);
        assert_eq!(parse_color("#11223344"), [0x11, 0x22, 0x33, 0x44]);
        // 色板文本取第一行。
        assert_eq!(parse_color("#abcdef\n#123456"), [0xab, 0xcd, 0xef, 255]);
        assert_eq!(parse_color("\n  #ff0000  \n#00ff00"), [255, 0, 0, 255]);
        assert_eq!(parse_color("nonsense"), [0, 0, 0, 255]);
    }

    #[test]
    fn oklab_round_trips_through_its_inverse() {
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [12, 200, 91],
            [123, 45, 210],
            [200, 200, 200],
        ] {
            assert_eq!(oklab_to_rgb(oklab(rgb)), rgb, "{rgb:?}");
        }
    }

    #[test]
    fn every_algorithm_finds_distinct_theme_colours() {
        // 三撮颜色：红、绿、蓝，量差不多。三种算法都该把它们分出来。
        let colors = [
            ([230, 20, 20], 100),
            ([20, 230, 20], 100),
            ([20, 20, 230], 100),
        ];
        for algorithm in [Algorithm::KMeans, Algorithm::MedianCut, Algorithm::KMedoids] {
            let palette = extract(&colors, 3, algorithm);
            assert_eq!(palette.len(), 3, "{algorithm:?}：{palette:?}");
        }
    }

    #[test]
    fn a_small_but_very_different_colour_still_gets_a_slot() {
        // 90% 浅灰 + 10% 鲜红：红的量少，但对比度高，K-Means 不该把它吞掉。
        let colors = [([200, 200, 200], 900), ([255, 0, 0], 100)];
        let palette = extract(&colors, 2, Algorithm::KMeans);
        assert_eq!(palette.len(), 2);
        let has_red = palette.iter().any(|c| c[0] > 180 && c[1] < 80 && c[2] < 80);
        assert!(has_red, "红色该占到一席：{palette:?}");
    }

    #[test]
    fn kmedoids_only_returns_colours_from_the_image() {
        let colors = [([10, 20, 30], 50), ([200, 100, 40], 40), ([3, 3, 3], 30)];
        let palette = extract(&colors, 3, Algorithm::KMedoids);
        for color in &palette {
            assert!(
                colors.iter().any(|(original, _)| original == color),
                "{color:?} 不是原图里的颜色"
            );
        }
    }

    #[test]
    fn algorithms_are_deterministic() {
        let colors: Vec<([u8; 3], usize)> = (0..64)
            .map(|i| ([i as u8 * 4, (i * 7) as u8, (i * 13) as u8], i + 1))
            .collect();
        for algorithm in [Algorithm::KMeans, Algorithm::KMedoids, Algorithm::MedianCut] {
            let first = extract(&colors, 8, algorithm);
            let second = extract(&colors, 8, algorithm);
            assert_eq!(first, second, "{algorithm:?} 得是确定的");
        }
    }

    #[test]
    fn transparent_pixels_do_not_count() {
        let mut image = RgbaImage::from_pixel(8, 1, Rgba([0, 0, 0, 0]));
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let palette = super::extract(&DynamicImage::ImageRgba8(image), 4, Algorithm::KMeans);
        assert_eq!(palette, vec![[255, 0, 0]]);
    }
}
