//! 「图像变换」节点（PNG → PNG）。
//!
//! 只做几何变换，不碰颜色：翻转（左右 / 上下）与旋转（90° / 180° / 270°）。
//! 像素是一个一个搬过去的，所以无损。
//!
//! 顺序是**先翻转、后旋转** —— 两者不交换，参数区里也就按这个顺序排。
//!
//! 输入输出都锁在 `Image(Png)`，和「缩放图像」「图像裁切」一样，要改用别的格式
//! 得先接一个「图像格式转换」。

use std::sync::Arc;

use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "transform_image";

const PARAM_FLIP_H: &str = "flipHorizontal";
const PARAM_FLIP_V: &str = "flipVertical";
const PARAM_ROTATE: &str = "rotate";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "图像变换".into(),
            category: "图像".into(),
            description: "翻转（左右 / 上下）与旋转（90° / 180° / 270°），逐像素搬运，PNG 无损。"
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
                ParamDef::new(PARAM_FLIP_H, "左右翻转", ParamSpec::Bool { default: false })
                    .described("沿竖轴镜像。"),
                ParamDef::new(PARAM_FLIP_V, "上下翻转", ParamSpec::Bool { default: false })
                    .described("沿横轴镜像。"),
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
                .described("90° / 270° 会把长宽对调。"),
            ],
            notes: vec![
                "顺序是**先翻转、后旋转**：例如「左右翻转 + 顺时针 90°」会先镜像，再把结果转过去。"
                    .into(),
                "90° / 270° 会让宽高对调；180° 不改变尺寸。".into(),
                "纯几何变换，像素一个不少、颜色一点不改，PNG 转 PNG 是无损的。".into(),
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

    let flip_h = params::boolean(args.params, PARAM_FLIP_H, false);
    let flip_v = params::boolean(args.params, PARAM_FLIP_V, false);
    let rotate = params::string(args.params, PARAM_ROTATE, "0");

    if !flip_h && !flip_v && rotate == "0" {
        args.warn("没有选任何变换，图像原样通过");
    }

    let mut image: DynamicImage = (*decoded).clone();
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

    let value =
        ImageValue::from_image(ImageFormat::Png, Arc::new(image), EncodeOptions::default())?
            .inherit_provenance(&source);

    let mut outputs = ValueMap::new();
    outputs.insert(
        "image".to_string(),
        Value::Image(value).with_name_hint(name),
    );
    Ok(outputs)
}
