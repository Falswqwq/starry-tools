//! 「图像分割」节点 —— 按透明部分把一张图切成多张。
//!
//! 把**不透明**的像素按连通关系分成一块块，每块裁出一个外框、各成一张图，装成一个
//! `List[Png]`。sprite 图（一整张拼图里放了很多小图）的常见处理就是它。
//!
//! 输出是列表，所以通常接给「[遍历](iterate.rs)」逐个处理。

use std::sync::Arc;

use image::{DynamicImage, RgbaImage};

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, OutputName, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "split_image";

const PARAM_CONNECTIVITY: &str = "connectivity";
const PARAM_MIN_PIXELS: &str = "minPixels";
const PARAM_PADDING: &str = "padding";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "图像分割".into(),
            category: "图像".into(),
            description: "按透明部分把一张图切成多张：每一块连续的不透明像素各成一张图。\
                          sprite 图处理常用，输出是一个列表。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any)).required(),
            ],
            outputs: vec![PortDef::new(
                "out",
                "图像列表",
                PortType::list_of(PortType::Image(ImageFormat::Png)),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_CONNECTIVITY,
                    "邻接方式",
                    ParamSpec::Select {
                        default: "8".into(),
                        options: vec![
                            SelectOption::new("8", "八邻接").hint("斜着挨着的算同一块"),
                            SelectOption::new("4", "四邻接").hint("只有上下左右挨着才算"),
                        ],
                    },
                )
                .described("决定两块像素怎么算「连在一起」。"),
                ParamDef::new(
                    PARAM_MIN_PIXELS,
                    "最小像素数",
                    ParamSpec::Number {
                        default: 0.0,
                        min: 0.0,
                        max: 1_000_000.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("px".into()),
                    },
                )
                .described("比这还小的碎块丢掉（去噪）。0 表示不过滤。"),
                ParamDef::new(
                    PARAM_PADDING,
                    "每张留边",
                    ParamSpec::Number {
                        default: 0.0,
                        min: 0.0,
                        max: 512.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("px".into()),
                    },
                )
                .described("每张切出来的图四周多留这么多透明边。"),
            ],
            notes: vec![
                "透明（alpha = 0）的像素不算内容；不透明的像素按邻接方式连成一块块，每块裁一个外框。".into(),
                "结果按**从上到下、从左到右**排，顺序稳定。".into(),
                "输出是 `List[Png]`（都带透明通道）—— 接给「遍历」就能逐张处理。".into(),
                "整张图没有任何不透明像素（或没有透明部分可切）时，原样返回一张图并给出提示。".into(),
                "每张图的名字是「原名-序号」，下游「保存到目录」写盘时不会互相盖。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let (source, name) = args.image_in("image")?;
    let decoded = source.decode()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();

    let eight = params::string(args.params, PARAM_CONNECTIVITY, "8") != "4";
    let min_pixels = params::integer(args.params, PARAM_MIN_PIXELS, 0).max(0) as u64;
    let padding = params::integer(args.params, PARAM_PADDING, 0).max(0) as u32;

    let boxes = components(&rgba, eight, min_pixels);
    if boxes.is_empty() {
        args.warn("没有找到不透明的区域，原样返回一张图");
        let value = ImageValue::from_image(
            ImageFormat::Png,
            Arc::new(DynamicImage::ImageRgba8(rgba)),
            EncodeOptions::default(),
        )?;
        return Ok(one_output(
            "out",
            Value::List(vec![Value::Image(value).with_name_hint(name)]),
        ));
    }

    let mut items = Vec::new();
    for (index, &(x0, y0, x1, y1)) in boxes.iter().enumerate() {
        let (cx, cy, cw, ch) = padded_bounds(x0, y0, x1, y1, padding, width, height);
        let cropped = image::imageops::crop_imm(&rgba, cx, cy, cw, ch).to_image();
        let value = ImageValue::from_image(
            ImageFormat::Png,
            Arc::new(DynamicImage::ImageRgba8(cropped)),
            EncodeOptions::default(),
        )?
        .inherit_provenance(&source);
        // 每张写上「原名-序号」，下游写盘才分得开。
        let hint = name.as_ref().map(|base| OutputName {
            stem: format!("{}-{}", base.stem, index + 1),
            auto_extension: true,
        });
        items.push(Value::Image(value).with_name_hint(hint));
    }

    args.warn(format!("切出 {} 张图", items.len()));
    Ok(one_output("out", Value::List(items)))
}

/// 找出所有不透明连通块的外框 `(x0, y0, x1, y1)`（含端点），按从上到下、从左到右排。
fn components(rgba: &RgbaImage, eight: bool, min_pixels: u64) -> Vec<(u32, u32, u32, u32)> {
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let opaque = |x: u32, y: u32| rgba.get_pixel(x, y).0[3] != 0;
    let mut visited = vec![false; (width * height) as usize];
    let slot = |x: u32, y: u32| (y * width + x) as usize;

    let steps: &[(i64, i64)] = if eight {
        &[
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ]
    } else {
        &[(0, -1), (-1, 0), (1, 0), (0, 1)]
    };

    let mut boxes = Vec::new();
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for sy in 0..height {
        for sx in 0..width {
            if visited[slot(sx, sy)] || !opaque(sx, sy) {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx, sy);
            let mut area = 0u64;
            visited[slot(sx, sy)] = true;
            stack.push((sx, sy));
            while let Some((x, y)) = stack.pop() {
                area += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                for &(dx, dy) in steps {
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let (nx, ny) = (nx as u32, ny as u32);
                    if visited[slot(nx, ny)] || !opaque(nx, ny) {
                        continue;
                    }
                    visited[slot(nx, ny)] = true;
                    stack.push((nx, ny));
                }
            }
            if area >= min_pixels.max(1) {
                boxes.push((x0, y0, x1, y1));
            }
        }
    }

    boxes.sort_by_key(|&(x0, y0, _, _)| (y0, x0));
    boxes
}

/// 把外框按 `padding` 向外扩一圈（夹在图像范围内），返回 `(x, y, 宽, 高)`。
fn padded_bounds(
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    padding: u32,
    width: u32,
    height: u32,
) -> (u32, u32, u32, u32) {
    let left = x0.saturating_sub(padding);
    let top = y0.saturating_sub(padding);
    let right = (x1 + 1 + padding).min(width);
    let bottom = (y1 + 1 + padding).min(height);
    (left, top, right - left, bottom - top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 一张 6×6，里面两块分开的不透明方块。
    fn sprite_sheet() -> RgbaImage {
        let mut image = RgbaImage::from_pixel(6, 6, Rgba([0, 0, 0, 0]));
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            image.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        }
        for (x, y) in [(4, 4), (5, 4), (4, 5), (5, 5)] {
            image.put_pixel(x, y, Rgba([0, 0, 255, 255]));
        }
        image
    }

    #[test]
    fn two_separate_blocks_become_two_boxes_in_reading_order() {
        let boxes = components(&sprite_sheet(), true, 0);
        assert_eq!(boxes, vec![(0, 0, 1, 1), (4, 4, 5, 5)]);
    }

    #[test]
    fn a_tiny_block_is_dropped_by_the_minimum() {
        let boxes = components(&sprite_sheet(), true, 4);
        assert_eq!(boxes.len(), 2);
        // 只留一个连通块的面积阈值。
        assert!(components(&sprite_sheet(), true, 5).is_empty());
    }

    #[test]
    fn padding_expands_the_box_and_clamps_to_the_image() {
        // 6×6 图里，一个 (4,4)-(5,5) 的块向外扩 1：夹到边界后是 (3,3) 起、3×3。
        assert_eq!(padded_bounds(4, 4, 5, 5, 1, 6, 6), (3, 3, 3, 3));
        // 顶到左上角就不用再往外。
        assert_eq!(padded_bounds(0, 0, 1, 1, 3, 6, 6), (0, 0, 5, 5));
    }
}
