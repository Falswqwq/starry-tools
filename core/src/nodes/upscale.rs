//! 「缩放图像」节点。
//!
//! 输入输出都锁定在 `Image(Png)`：想把 JPEG 缩放，得先接一个「图像格式转换」。
//! 这正是类型系统存在的意义 —— 连线的时候就知道这一步过不去。
//!
//! 默认用最近邻插值，也就是像素画软件里那种「一个像素变成一个方块」的放大。

use std::sync::Arc;

use image::imageops::FilterType;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "upscale";

/// 输出尺寸的上限，防止手滑填个 1600% 把内存吃光。
const MAX_OUTPUT_PIXELS: u64 = 64_000_000;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            // id 保留 `upscale` 不变：它在存档里，改了老工作流就打不开了。
            // 显示名才是给人看的那个。
            id: KIND.into(),
            name: "缩放图像".into(),
            category: "图像".into(),
            description: "把 PNG 图像缩放到原来的百分之 n。像素画常配「邻近」插值做整数倍放大，\
                          放大后每个像素还是干净的方块。"
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
                    "percent",
                    "缩放比例",
                    ParamSpec::Number {
                        default: 200.0,
                        min: 1.0,
                        max: 1600.0,
                        step: 1.0,
                        integer: false,
                        unit: Some("%".into()),
                    },
                )
                .described("100% 是原始大小。整数倍放大就填 200 / 300 / 400。"),
                ParamDef::new(
                    "filter",
                    "插值算法",
                    ParamSpec::Select {
                        default: "nearest".into(),
                        options: vec![
                            SelectOption::new("nearest", "邻近").hint("像素画首选，边缘保持硬朗"),
                            SelectOption::new("triangle", "线性"),
                            SelectOption::new("catmullRom", "Catmull-Rom"),
                            SelectOption::new("gaussian", "高斯"),
                            SelectOption::new("lanczos3", "Lanczos3").hint("照片放大最锐利"),
                        ],
                    },
                ),
            ],
            notes: vec![
                "输出尺寸 = 原尺寸 × 比例 ÷ 100，四舍五入到整数像素。100% 就是原样通过。".into(),
                "像素画用「邻近」并取整数倍（200% / 300% / 400%），放大后每个像素还是干净的方块。"
                    .into(),
                "照片类图像换成「Lanczos3」更锐利，代价是硬边缘旁边会出现轻微光晕。".into(),
                "输出像素总数超过 6400 万会直接报错，免得把内存吃光。".into(),
            ],
        },
        run,
    )
}

fn filter_of(value: &str) -> FilterType {
    match value {
        "triangle" => FilterType::Triangle,
        "catmullRom" => FilterType::CatmullRom,
        "gaussian" => FilterType::Gaussian,
        "lanczos3" => FilterType::Lanczos3,
        _ => FilterType::Nearest,
    }
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面 warn 之后还要用它带出处。
    let source = args.image("image")?.clone();
    let name = args.input_name("image");
    let image = source.decode()?;

    let percent = params::number(args.params, "percent", 200.0).clamp(1.0, 1600.0);
    let (width, height) = (image.width(), image.height());
    let target_width = ((width as f64 * percent / 100.0).round() as u32).max(1);
    let target_height = ((height as f64 * percent / 100.0).round() as u32).max(1);

    if u64::from(target_width) * u64::from(target_height) > MAX_OUTPUT_PIXELS {
        return Err(NodeError::new(format!(
            "{target_width} × {target_height} 太大了，超过 {} 像素的上限",
            MAX_OUTPUT_PIXELS
        )));
    }

    let filter = filter_of(&params::string(args.params, "filter", "nearest"));
    let resized = image.resize_exact(target_width, target_height, filter);

    if (percent - 100.0).abs() < f64::EPSILON {
        args.warn("比例是 100%，这一步没有改变图像");
    } else if percent < 100.0 {
        args.warn("比例小于 100%，这一步实际上是在缩小图像");
    }

    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(resized),
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
