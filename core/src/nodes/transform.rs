//! 「图像变换」节点（PNG → PNG）。
//!
//! 只做几何变换，不碰颜色：翻转（左右 / 上下）、旋转（90° / 180° / 270°）
//! 与按长宽比例分别拉伸。翻转和旋转逐像素搬运，所以无损；拉伸使用邻近采样，
//! 不引入新颜色，但会改变像素数量。
//!
//! 顺序是**先翻转、后旋转** —— 两者不交换，参数区里也就按这个顺序排。
//!
//! 输入输出都锁在 `Image(Png)`，和「缩放图像」「图像裁切」一样，要改用别的格式
//! 得先接一个「图像格式转换」。

use std::sync::Arc;

use image::imageops::FilterType;
use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "transform_image";

const PARAM_MODE: &str = "mode";
const PARAM_FLIP_H: &str = "flipHorizontal";
const PARAM_FLIP_V: &str = "flipVertical";
const PARAM_ROTATE: &str = "rotate";
const PARAM_STRETCH_W: &str = "stretchWidth";
const PARAM_STRETCH_H: &str = "stretchHeight";

const MODE_FLIP: &str = "flip";
const MODE_STRETCH: &str = "stretch";

/// 和「缩放图像」一致的安全上限，避免异常参数把内存撑爆。
const MAX_OUTPUT_PIXELS: u64 = 64_000_000;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "图像变换".into(),
            category: "图像".into(),
            description: "翻转、旋转，或按长宽比例分别拉伸。翻转 / 旋转逐像素搬运，PNG 无损。"
                .into(),
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
                    "处理方式",
                    ParamSpec::Select {
                        default: MODE_FLIP.into(),
                        options: vec![
                            SelectOption::new(MODE_FLIP, "翻转 / 旋转").hint("镜像与旋转"),
                            SelectOption::new(MODE_STRETCH, "图像拉伸")
                                .hint("分别调整长宽比例"),
                        ],
                    },
                )
                .described("选择翻转旋转，或按长宽比例拉伸。"),
                ParamDef::new(PARAM_FLIP_H, "左右翻转", ParamSpec::Bool { default: false })
                    .described("沿竖轴镜像。")
                    .visible_when(PARAM_MODE, &[MODE_FLIP]),
                ParamDef::new(PARAM_FLIP_V, "上下翻转", ParamSpec::Bool { default: false })
                    .described("沿横轴镜像。")
                    .visible_when(PARAM_MODE, &[MODE_FLIP]),
                ParamDef::new(
                    PARAM_ROTATE,
                    "旋转",
                    ParamSpec::Select {
                        default: "0".into(),
                        options: vec![
                            SelectOption::new("0", "不旋转"),
                            SelectOption::new("90", "顺时针 90°"),
                            SelectOption::new("180", "180°"),
                            SelectOption::new("270", "顺时针 270°"),
                        ],
                    },
                )
                .described("90° / 270° 会把长宽对调。")
                .visible_when(PARAM_MODE, &[MODE_FLIP]),
                ParamDef::new(
                    PARAM_STRETCH_W,
                    "宽度拉伸",
                    ParamSpec::Number {
                        default: 100.0,
                        min: 1.0,
                        max: 1600.0,
                        step: 1.0,
                        integer: false,
                        unit: Some("%".into()),
                    },
                )
                .described("100% 表示宽度不变。")
                .visible_when(PARAM_MODE, &[MODE_STRETCH]),
                ParamDef::new(
                    PARAM_STRETCH_H,
                    "高度拉伸",
                    ParamSpec::Number {
                        default: 100.0,
                        min: 1.0,
                        max: 1600.0,
                        step: 1.0,
                        integer: false,
                        unit: Some("%".into()),
                    },
                )
                .described("100% 表示高度不变。")
                .visible_when(PARAM_MODE, &[MODE_STRETCH]),
            ],
            notes: vec![
                "翻转模式顺序是**先翻转、后旋转**：例如「左右翻转 + 顺时针 90°」会先镜像，再把结果转过去。"
                    .into(),
                "90° / 270° 会让宽高对调；180° 不改变尺寸。".into(),
                "拉伸模式分别按宽度 / 高度百分比缩放，100% 表示该方向不处理。".into(),
                "翻转 / 旋转逐像素搬运，PNG 无损；拉伸使用邻近采样，不引入新颜色。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面 warn 之后还要用它带出处。
    let (source, name) = args.image_in("image")?;
    let decoded = source.decode()?;

    let mode = params::string(args.params, PARAM_MODE, MODE_FLIP);
    let mut image: DynamicImage = (*decoded).clone();

    if mode == MODE_STRETCH {
        let width_percent = params::number(args.params, PARAM_STRETCH_W, 100.0).clamp(1.0, 1600.0);
        let height_percent = params::number(args.params, PARAM_STRETCH_H, 100.0).clamp(1.0, 1600.0);
        if (width_percent - 100.0).abs() < f64::EPSILON
            && (height_percent - 100.0).abs() < f64::EPSILON
        {
            args.warn("长宽拉伸都是 100%，图像原样通过");
        } else {
            let (width, height) = (image.width(), image.height());
            let target_width = ((width as f64 * width_percent / 100.0).round() as u32).max(1);
            let target_height = ((height as f64 * height_percent / 100.0).round() as u32).max(1);
            if u64::from(target_width) * u64::from(target_height) > MAX_OUTPUT_PIXELS {
                return Err(NodeError::new(format!(
                    "{target_width} × {target_height} 太大了，超过 {MAX_OUTPUT_PIXELS} 像素的上限"
                )));
            }
            // 像素素材用邻近采样：边缘干脆，也不会混出原图没有的颜色。
            image = image.resize_exact(target_width, target_height, FilterType::Nearest);
        }
    } else {
        let flip_h = params::boolean(args.params, PARAM_FLIP_H, false);
        let flip_v = params::boolean(args.params, PARAM_FLIP_V, false);
        let rotate = params::string(args.params, PARAM_ROTATE, "0");

        if !flip_h && !flip_v && rotate == "0" {
            args.warn("没有选任何变换，图像原样通过");
        }

        if flip_h {
            image = image.fliph();
        }
        if flip_v {
            image = image.flipv();
        }
        // `image` 的 rotateN 都是「顺时针 N 度」，和下拉框里的说法一致。
        image = match rotate.as_str() {
            "90" => image.rotate90(),
            "180" => image.rotate180(),
            "270" => image.rotate270(),
            _ => image,
        };
    }

    let value =
        ImageValue::from_image(ImageFormat::Png, Arc::new(image), EncodeOptions::default())?
            .inherit_provenance(&source);

    Ok(one_output(
        "image",
        Value::Image(value).with_name_hint(name),
    ))
}
