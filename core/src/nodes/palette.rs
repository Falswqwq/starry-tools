//! 「色彩分析」节点：从图里抽出一个**色板**。
//!
//! 色板就是一段文本 —— 每行一个 hex 颜色（`#rrggbb`）。所以它的输出是一个普通的
//! 文本，可以直接接给「边框/边距」这类有「颜色」参数的节点（那时取第一个颜色），
//! 也可以再接别的色板节点。
//!
//! 两种模式：
//!
//! * **提取调色板** 用中位切分把图里的颜色归成 N 种主色，按占比从多到少；
//! * **数量统计** 数每种颜色有多少像素，把相近色（分量差在阈值内）并成一种，
//!   按像素数从上到下排。
//!
//! 输入不锁格式：`Image(Any)` —— 任何图片都行。

use std::collections::HashMap;

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{palette_text, NodeArgs, Value, ValueMap};
use crate::palette::{self, Algorithm};
use crate::registry::NodeSpec;

pub const KIND: &str = "color_analysis";

const PARAM_MODE: &str = "mode";
const PARAM_ALGORITHM: &str = "algorithm";
const PARAM_COLORS: &str = "colors";
const PARAM_THRESHOLD: &str = "threshold";

const MODE_EXTRACT: &str = "extract";
const MODE_COUNT: &str = "count";

/// 色板最多多少种颜色 —— 和量化器一致的 256。
const MAX_COLORS: i64 = 256;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "色彩分析".into(),
            category: "图像".into(),
            description: "从图里抽出色板：按主色归类，或者数每种颜色占多少像素。\
                          输出是一段 hex 文本，每行一个颜色。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any)).required(),
            ],
            outputs: vec![PortDef::new("palette", "色板", PortType::Text)],
            params: vec![
                ParamDef::new(
                    PARAM_MODE,
                    "模式",
                    ParamSpec::Select {
                        default: MODE_EXTRACT.into(),
                        options: vec![
                            SelectOption::new(MODE_EXTRACT, "提取调色板")
                                .hint("归成 N 种主色，按占比"),
                            SelectOption::new(MODE_COUNT, "数量统计")
                                .hint("数像素，按多少从上到下排"),
                        ],
                    },
                ),
                ParamDef::new(
                    PARAM_ALGORITHM,
                    "提取算法",
                    ParamSpec::Select {
                        default: "kmeans".into(),
                        options: vec![
                            SelectOption::new("kmeans", "K-Means + OKLab")
                                .hint("通用主题色提取（推荐）—— 对比度高的颜色也算数"),
                            SelectOption::new("medianCut", "Median Cut")
                                .hint("最快、经典；谁的像素多谁说了算（像素画常用）"),
                            SelectOption::new("kMedoids", "K-Medoids + OKLab")
                                .hint("取的颜色都来自原图，看着自然、真实"),
                        ],
                    },
                )
                .described("选哪种算法把颜色归成主色。拿不准就用 K-Means。")
                .visible_when(PARAM_MODE, &[MODE_EXTRACT]),
                ParamDef::new(
                    PARAM_COLORS,
                    "颜色数",
                    ParamSpec::Number {
                        default: 8.0,
                        min: 2.0,
                        max: MAX_COLORS as f64,
                        step: 1.0,
                        integer: true,
                        unit: Some("色".into()),
                    },
                )
                .described("归成多少种主色。")
                .visible_when(PARAM_MODE, &[MODE_EXTRACT]),
                ParamDef::new(
                    PARAM_THRESHOLD,
                    "相近色合并阈值",
                    ParamSpec::Number {
                        default: 16.0,
                        min: 0.0,
                        max: 255.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .described("分量差在这个范围内的颜色算同一种。0 就是完全按原色统计。")
                .visible_when(PARAM_MODE, &[MODE_COUNT]),
            ],
            notes: vec![
                "输出是一段文本：每行一个 #rrggbb。它能接到任何「颜色」参数上（取第一个色），\
                 也能当普通文本用。"
                    .into(),
                "提取算法怎么选：要「好看的主题色」用 K-Means + OKLab；要快、或者在处理\
                 像素画，用 Median Cut；要「颜色一定来自原图」，用 K-Medoids + OKLab。"
                    .into(),
                "K-Means / K-Medoids 在 OKLab（感知均匀）空间里聚类 —— 对比度高的颜色会凭自己的\
                 分量占到一席，不会被像素多的颜色淹掉。颜色本来就不多于「颜色数」时原样列出。"
                    .into(),
                "数量统计按分量分桶合并相近色，桶是网格对齐的 —— 正好跨在桶边界上的两种相近色\
                 可能不会并到一起，把阈值调大一点通常就盖住了。"
                    .into(),
                "全透明的像素不算颜色；半透明的像素按它的 RGB 归类，色板里不带 alpha。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；输出之后还要用它把名字接下去。
    let source = args.image("image")?.clone();
    let name = args.input_name("image");
    let decoded = source.decode()?;
    let rgba = decoded.to_rgba8();

    let mode = params::string(args.params, PARAM_MODE, MODE_EXTRACT);
    let colors = match mode.as_str() {
        MODE_COUNT => {
            let threshold = params::integer(args.params, PARAM_THRESHOLD, 16).clamp(0, 255) as u8;
            counted_colors(&rgba, threshold)
        }
        _ => {
            let wanted =
                params::integer(args.params, PARAM_COLORS, 8).clamp(2, MAX_COLORS) as usize;
            let algorithm =
                Algorithm::parse(&params::string(args.params, PARAM_ALGORITHM, "kmeans"));
            palette::extract(&decoded, wanted, algorithm)
        }
    };

    if colors.is_empty() {
        args.warn("整张图都是透明的，没有颜色可分析");
    }

    let mut outputs = ValueMap::new();
    outputs.insert(
        "palette".to_string(),
        Value::text(palette_text(&colors)).with_name_hint(name),
    );
    Ok(outputs)
}

/// 数量统计：按分量分桶把相近色并成一种，再按像素数从多到少排。
fn counted_colors(rgba: &image::RgbaImage, threshold: u8) -> Vec<[u8; 3]> {
    let step = u32::from(threshold).max(1);
    // 桶 → （各分量的和、像素数）。
    let mut buckets: HashMap<(u32, u32, u32), ([u64; 3], u64)> = HashMap::new();
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        if a == 0 {
            continue;
        }
        let key = (
            u32::from(r) / step,
            u32::from(g) / step,
            u32::from(b) / step,
        );
        let entry = buckets.entry(key).or_insert(([0; 3], 0));
        entry.0[0] += u64::from(r);
        entry.0[1] += u64::from(g);
        entry.0[2] += u64::from(b);
        entry.1 += 1;
    }

    let mut colors: Vec<([u8; 3], u64)> = buckets
        .into_values()
        .map(|(sums, count)| {
            let average = |index: usize| ((sums[index] + count / 2) / count.max(1)) as u8;
            ([average(0), average(1), average(2)], count)
        })
        .collect();
    colors.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    colors.into_iter().map(|(color, _)| color).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn count_merges_near_colors_and_sorts_by_how_often_they_appear() {
        let mut image = image::RgbaImage::from_pixel(6, 1, Rgba([255, 0, 0, 255]));
        // 五红一蓝：红占多。
        image.put_pixel(5, 0, Rgba([0, 0, 255, 255]));
        let colors = counted_colors(&image, 0);
        assert_eq!(colors, vec![[255, 0, 0], [0, 0, 255]]);

        // 阈值够大时，红 (255,0,0) 和 (250,0,0) 会并成一种。
        let mut similar = image::RgbaImage::from_pixel(4, 1, Rgba([255, 0, 0, 255]));
        similar.put_pixel(2, 0, Rgba([250, 0, 0, 255]));
        similar.put_pixel(3, 0, Rgba([0, 0, 255, 255]));
        let merged = counted_colors(&similar, 16);
        assert_eq!(merged.len(), 2, "两个相近的红应当并成一种");
    }

    #[test]
    fn fully_transparent_pixels_are_ignored() {
        let image = image::RgbaImage::from_pixel(4, 1, Rgba([0, 0, 0, 0]));
        assert!(counted_colors(&image, 0).is_empty());
    }
}
