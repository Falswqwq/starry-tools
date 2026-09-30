//! 「图像格式转换」节点。
//!
//! 以前只会输出 PNG，现在输出格式是个参数 —— 所以它的输出端口是**动态**的：
//! 选了 JPEG，输出就是 `Image(Jpeg)`，下游的类型检查因此仍然是准的。
//!
//! 输入声明成 `Image(Any)`：能解码的都收。产物是真的用目标格式的编码器编出来的
//! 字节，不是把标签改一改。

use std::sync::Arc;

use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{flatten_onto_white, EncodeOptions, ImageValue, PngCompression};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

/// 节点 id。改过一次（原本叫 `convert_to_png`），老存档靠 `registry` 里的别名表兜住。
pub const KIND: &str = "convert_image";

pub const PARAM_FORMAT: &str = "format";
pub const PARAM_QUALITY: &str = "quality";
pub const PARAM_COMPRESSION: &str = "compression";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "图像格式转换".into(),
            category: "图像".into(),
            description: "在常见图像格式之间转。输入收下任何能解码的格式，\
                          输出格式由「目标格式」决定。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any))
                    .required()
                    .hint("任何可解码的图像"),
            ],
            outputs: vec![PortDef::new(
                "image",
                "PNG 图像",
                PortType::Image(ImageFormat::Png),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_FORMAT,
                    "目标格式",
                    ParamSpec::Select {
                        default: ImageFormat::Png.name().into(),
                        options: ImageFormat::ENCODABLE
                            .iter()
                            .map(|format| {
                                let option = SelectOption::new(format.name(), format.badge());
                                match format.note() {
                                    Some(note) => option.hint(note),
                                    None => option,
                                }
                            })
                            .collect(),
                    },
                )
                .described("输出端口的类型跟着它变。"),
                ParamDef::new(
                    PARAM_QUALITY,
                    "画质",
                    ParamSpec::Number {
                        default: 90.0,
                        min: 1.0,
                        max: 100.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("%".into()),
                    },
                )
                .described("越小体积越小、块状越明显。")
                .visible_when(PARAM_FORMAT, &[ImageFormat::Jpeg.name()]),
                ParamDef::new(
                    PARAM_COMPRESSION,
                    "压缩档位",
                    ParamSpec::Select {
                        default: "default".into(),
                        options: vec![
                            SelectOption::new("fast", "快速").hint("写盘最快"),
                            SelectOption::new("default", "均衡"),
                            SelectOption::new("best", "最小体积").hint("写盘最慢"),
                        ],
                    },
                )
                .described("PNG 是无损的，这里调的只是文件大小和写盘速度。")
                .visible_when(PARAM_FORMAT, &[ImageFormat::Png.name()]),
                ParamDef::new(
                    "colorMode",
                    "色彩模式",
                    ParamSpec::Select {
                        default: "keep".into(),
                        options: vec![
                            SelectOption::new("keep", "保持原始"),
                            SelectOption::new("rgb8", "8 位 RGB").hint("透明区域填白"),
                            SelectOption::new("gray8", "8 位灰度"),
                        ],
                    },
                )
                .described("目标格式容纳不下的时候，编码器会自己挑一个能用的表示。"),
            ],
            notes: vec![
                "输入收下任何能解码的格式：PNG、JPEG、GIF、WebP、BMP、TIFF、ICO、QOI、TGA、PNM。"
                    .into(),
                "JPEG 存不下透明通道 —— 转过去的时候透明的地方会被压成背景色。".into(),
                "WebP 这里走的是无损编码（`image` crate 目前只提供这一种），所以没有画质可调。"
                    .into(),
                "GIF 只取第一帧，动图会变成静态图。".into(),
                "「最小体积」档写盘最慢，批量处理时先用「快速」试跑。".into(),
            ],
        },
        output_ports,
        run,
    )
}

/// 输出端口跟着「目标格式」走。
fn output_ports(params: &Params) -> Vec<PortDef> {
    let format = target_format(params);
    vec![PortDef::new(
        "image",
        format.label(),
        PortType::Image(format),
    )]
}

fn target_format(params: &Params) -> ImageFormat {
    let name = params::string(params, PARAM_FORMAT, ImageFormat::Png.name());
    ImageFormat::from_name(&name)
        .filter(|format| format.to_image_format().is_some())
        .unwrap_or(ImageFormat::Png)
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let target = target_format(args.params);

    // 先把输入拿在手里（克隆只是复制一个 Arc），后面 warn 之后还要用它带出处。
    let source = args.image("image")?.clone();
    let name = args.input_name("image");
    let source_format = source.format();
    let decoded = source.decode()?;

    if target == source_format {
        args.warn(format!("输入已经是 {}，只是重新编码了一遍", target.badge()));
    }

    let converted = match params::string(args.params, "colorMode", "keep").as_str() {
        "rgb8" => Arc::new(DynamicImage::ImageRgb8(flatten_onto_white(&decoded))),
        "gray8" => Arc::new(DynamicImage::ImageLuma8(decoded.to_luma8())),
        // 透传时只复制一个 Arc，不复制像素。
        _ => decoded,
    };

    let options = EncodeOptions {
        png_compression: PngCompression::parse(&params::string(
            args.params,
            PARAM_COMPRESSION,
            "default",
        )),
        jpeg_quality: params::integer(args.params, PARAM_QUALITY, 90).clamp(1, 100) as u8,
    };

    let value = ImageValue::from_image(target, converted, options)?.inherit_provenance(&source);

    let mut outputs = ValueMap::new();
    outputs.insert(
        "image".to_string(),
        Value::Image(value).with_name_hint(name),
    );
    Ok(outputs)
}
