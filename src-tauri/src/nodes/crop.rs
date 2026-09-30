//! 「图像裁切」节点 —— 从图上取出一块。
//!
//! 三种方式：
//!
//! * **比例** 按目标长宽比取一块，尽量大；
//! * **大小** 取固定像素尺寸，不够就贴着边取；
//! * **内容** 把四周的透明留白裁掉，内缩到最近的有色像素。
//!
//! 输入输出都锁定在 `Image(Png)`，和「缩放图像」一样，得先转成 PNG 才能接。

use std::sync::Arc;

use image::{DynamicImage, GenericImageView};

use crate::error::NodeError;
use crate::image_io::{has_transparency, EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "crop_image";

const PARAM_MODE: &str = "mode";
const PARAM_RATIO_WIDTH: &str = "ratioWidth";
const PARAM_RATIO_HEIGHT: &str = "ratioHeight";
const PARAM_WIDTH: &str = "width";
const PARAM_HEIGHT: &str = "height";
const PARAM_ANCHOR: &str = "anchor";

const MODE_RATIO: &str = "ratio";
const MODE_SIZE: &str = "size";
const MODE_CONTENT: &str = "content";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "图像裁切".into(),
            category: "图像".into(),
            description: "从图上取出一块：按长宽比取、取固定尺寸，或者把四周的透明留白裁掉。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "PNG 图像", PortType::Image(ImageFormat::Png)).required(),
            ],
            outputs: vec![PortDef::new(
                "image",
                "PNG 图像",
                PortType::Image(ImageFormat::Png),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_MODE,
                    "裁切方式",
                    ParamSpec::Select {
                        default: MODE_RATIO.into(),
                        options: vec![
                            SelectOption::new(MODE_RATIO, "裁切到比例").hint("按目标长宽比取一块"),
                            SelectOption::new(MODE_SIZE, "裁切到大小").hint("取固定的像素尺寸"),
                            SelectOption::new(MODE_CONTENT, "裁切到内容")
                                .hint("裁掉四周的透明留白"),
                        ],
                    },
                ),
                ParamDef::new(
                    PARAM_RATIO_WIDTH,
                    "长宽比 · 宽",
                    ParamSpec::Number {
                        default: 16.0,
                        min: 1.0,
                        max: 1000.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .described("和「长宽比 · 高」一起构成目标比例，例如 16 : 9。")
                .visible_when(PARAM_MODE, &[MODE_RATIO]),
                ParamDef::new(
                    PARAM_RATIO_HEIGHT,
                    "长宽比 · 高",
                    ParamSpec::Number {
                        default: 9.0,
                        min: 1.0,
                        max: 1000.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .visible_when(PARAM_MODE, &[MODE_RATIO]),
                ParamDef::new(
                    PARAM_WIDTH,
                    "宽度",
                    ParamSpec::Number {
                        default: 512.0,
                        min: 1.0,
                        max: 100_000.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("px".into()),
                    },
                )
                .visible_when(PARAM_MODE, &[MODE_SIZE]),
                ParamDef::new(
                    PARAM_HEIGHT,
                    "高度",
                    ParamSpec::Number {
                        default: 512.0,
                        min: 1.0,
                        max: 100_000.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("px".into()),
                    },
                )
                .visible_when(PARAM_MODE, &[MODE_SIZE]),
                ParamDef::new(
                    PARAM_ANCHOR,
                    "保留位置",
                    ParamSpec::Select {
                        default: "center".into(),
                        options: [
                            ("center", "居中"),
                            ("top", "靠上"),
                            ("bottom", "靠下"),
                            ("left", "靠左"),
                            ("right", "靠右"),
                            ("topLeft", "左上角"),
                            ("topRight", "右上角"),
                            ("bottomLeft", "左下角"),
                            ("bottomRight", "右下角"),
                        ]
                        .into_iter()
                        .map(|(value, label)| SelectOption::new(value, label))
                        .collect(),
                    },
                )
                .described("图上被取走的那块靠在哪个角落。")
                .visible_when(PARAM_MODE, &[MODE_RATIO, MODE_SIZE]),
            ],
            notes: vec![
                "裁切到比例会在图上取一块最大的、符合目标长宽比的区域；比例对不上时四边各裁掉一些。"
                    .into(),
                "裁切到大小要的尺寸比原图还大时，会贴着边取到原图那么大，并给出提示。".into(),
                "裁切到内容认的是透明通道：四边向内缩到最近的有色像素。整张图都不透明时无从下手，\
                 会原样放行并给出提示。"
                    .into(),
                "「保留位置」决定取走的是哪一块 —— 默认居中，想留住某个角落就把它挪到对应位置。"
                    .into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面 warn 之后还要用它带出处。
    let source = args.image("image")?.clone();
    let name = args.input_name("image");
    let decoded = source.decode()?;
    let (image_width, image_height) = decoded.dimensions();

    let mode = params::string(args.params, PARAM_MODE, MODE_RATIO);
    let (x, y, width, height) = match mode.as_str() {
        MODE_SIZE => {
            let wanted_width = params::integer(args.params, PARAM_WIDTH, 512).max(1) as u32;
            let wanted_height = params::integer(args.params, PARAM_HEIGHT, 512).max(1) as u32;
            let width = wanted_width.min(image_width);
            let height = wanted_height.min(image_height);
            if width != wanted_width || height != wanted_height {
                args.warn(format!(
                    "想要的 {wanted_width} × {wanted_height} 比原图大，已取到 {width} × {height}"
                ));
            }
            let (x, y) = anchored(
                image_width - width,
                image_height - height,
                &params::string(args.params, PARAM_ANCHOR, "center"),
            );
            (x, y, width, height)
        }
        MODE_CONTENT => match content_bounds(&decoded) {
            Some(bounds) if bounds.2 < image_width || bounds.3 < image_height => {
                args.warn(format!(
                    "裁掉了四周的透明留白，得到 {} × {}",
                    bounds.2, bounds.3
                ));
                bounds
            }
            Some(bounds) => {
                if has_transparency(&decoded) {
                    args.warn("四周没有整块透明的留白，没有可裁的内容，原样通过");
                } else {
                    args.warn("图像没有透明区域，没有可裁的留白，原样通过");
                }
                bounds
            }
            None => {
                args.warn("整张图都是透明的，没有可保留的内容，原样通过");
                (0, 0, image_width, image_height)
            }
        },
        _ => {
            let ratio_width = params::number(args.params, PARAM_RATIO_WIDTH, 16.0).max(0.001);
            let ratio_height = params::number(args.params, PARAM_RATIO_HEIGHT, 9.0).max(0.001);
            let target = ratio_width / ratio_height;
            let current = image_width as f64 / image_height as f64;
            let (width, height) = if current > target {
                (
                    ((image_height as f64 * target).round() as u32).max(1),
                    image_height,
                )
            } else {
                (
                    image_width,
                    ((image_width as f64 / target).round() as u32).max(1),
                )
            };
            let (x, y) = anchored(
                image_width - width,
                image_height - height,
                &params::string(args.params, PARAM_ANCHOR, "center"),
            );
            (x, y, width, height)
        }
    };

    let cropped = decoded.crop_imm(x, y, width, height);
    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(DynamicImage::ImageRgba8(cropped.to_rgba8())),
        EncodeOptions::default(),
    )?
    .inherit_provenance(&source);

    let mut outputs = ValueMap::new();
    outputs.insert(
        "image".to_string(),
        Value::Image(value).with_name_hint(name),
    );
    Ok(outputs)
}

/// 把「剩下多少可以偏」映射成具体坐标。锚点用两个 0 / 0.5 / 1 的分数表示，
/// 左上角是 (0, 0)，右下角是 (1, 1)，居中是 (0.5, 0.5)。
fn anchored(leftover_x: u32, leftover_y: u32, anchor: &str) -> (u32, u32) {
    let (fx, fy) = match anchor {
        "top" => (0.5, 0.0),
        "bottom" => (0.5, 1.0),
        "left" => (0.0, 0.5),
        "right" => (1.0, 0.5),
        "topLeft" => (0.0, 0.0),
        "topRight" => (1.0, 0.0),
        "bottomLeft" => (0.0, 1.0),
        "bottomRight" => (1.0, 1.0),
        _ => (0.5, 0.5),
    };
    (
        (leftover_x as f64 * fx).round() as u32,
        (leftover_y as f64 * fy).round() as u32,
    )
}

/// 有色像素（alpha 不为 0）的外接矩形，格式是 `(x, y, 宽, 高)`。
/// 整张图都透明时返回 `None`。
fn content_bounds(image: &DynamicImage) -> Option<(u32, u32, u32, u32)> {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();

    let (mut min_x, mut min_y) = (width, height);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    let mut found = false;

    for (x, y, pixel) in rgba.enumerate_pixels() {
        if pixel.0[3] == 0 {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    found.then(|| (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}
