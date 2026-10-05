//! 「背景移除」节点：用 u2net 这类模型把主体抠出来，背景变透明。
//!
//! 比之前的「颜色剔除」聪明得多 —— 它不是按颜色抠，而是**认出主体**，所以头发丝、
//! 半透明的纱、和背景同色的部分都能保住。
//!
//! 应用**不自带模型**：模型放在 `<数据目录>/models/` 下，没下载的话这个节点在画布上
//! 是禁用的，卡片上挂一个「下载模型」按钮（可选原始源 / 镜像源）。

use std::sync::Arc;

use crate::bg_model;
use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;
use crate::rembg;

pub const KIND: &str = "background_removal";

const PARAM_MODEL: &str = "model";

pub fn spec() -> NodeSpec {
    let options = bg_model::MODELS
        .iter()
        .map(|model| SelectOption::new(model.id, model.name).hint(model.note))
        .collect();

    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "背景移除".into(),
            category: "图像".into(),
            description: "用 AI 模型认出主体、把背景抠成透明。需要先下载模型（模型不带在应用里）。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any)).required(),
            ],
            outputs: vec![PortDef::new(
                "image",
                "PNG 图像",
                PortType::Image(ImageFormat::Png),
            )],
            params: vec![ParamDef::new(
                PARAM_MODEL,
                "模型",
                ParamSpec::Select {
                    default: bg_model::default_model().id.into(),
                    options,
                },
            )
            .described("没下载的话，卡片上会出现「下载模型」按钮。")],
            notes: vec![
                "模型不在应用里，第一次用要先下载 —— 卡片上的「下载模型」按钮里能挑原始源或镜像源。"
                    .into(),
                "下载存在 `<数据目录>/models/` 下；下好了这个节点就自动能用了。".into(),
                "U²-Netp 小、快，日常够用；U²-Net 更大更准，但慢不少。".into(),
                "每次运行都会重新读模型（模型大的话这一步就不便宜），结果一律是 PNG。".into(),
            ],
        },
        run,
    )
    .needs_model(PARAM_MODEL)
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面还要用它带出处。
    let (source, name) = args.image_in("image")?;

    let model_id = params::string(args.params, PARAM_MODEL, bg_model::default_model().id);
    let model = bg_model::find(&model_id)
        .ok_or_else(|| NodeError::new(format!("不认识的模型：{model_id}")))?;
    let model_path = bg_model::path_for(model);
    if !model_path.is_file() {
        return Err(NodeError::new(format!(
            "模型「{}」还没下载 —— 在卡片上点「下载模型」",
            model.name
        )));
    }

    let decoded = source.decode()?;
    let rgba = decoded.to_rgba8();
    let cut = rembg::remove_background(&rgba, &model_path).map_err(NodeError::new)?;

    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(image::DynamicImage::ImageRgba8(cut)),
        EncodeOptions::default(),
    )?
    .inherit_provenance(&source);

    Ok(one_output(
        "image",
        Value::Image(value).with_name_hint(name),
    ))
}
