//! 「输入」节点 —— 工作流的起点。
//!
//! 它是唯一一个输出端口**动态**的节点：用户选了 JPEG 文件，它的输出类型就
//! 是 `Image(Jpeg)`；只选了「文本」，输出类型就是 `Text`。类型跟着数据走，
//! 所以下游的检查是准的。

use std::path::Path;

use crate::error::NodeError;
use crate::image_io::{self, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "input";

const VALUE_TYPE_IMAGE: &str = "image";
const VALUE_TYPE_TEXT: &str = "text";
const VALUE_TYPE_NUMBER: &str = "number";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "输入".into(),
            category: "来源".into(),
            description: "每个工作流的起点。从磁盘读入一张图像，或者直接给出文本 / 数字。\
                          可以放多个输入节点，让几条支线各走各的。"
                .into(),
            is_source: true,
            inputs: vec![],
            outputs: vec![PortDef::new("out", "值", PortType::Image(ImageFormat::Any))],
            params: vec![
                ParamDef::new(
                    "valueType",
                    "类型",
                    ParamSpec::Select {
                        default: VALUE_TYPE_IMAGE.into(),
                        options: vec![
                            SelectOption::new(VALUE_TYPE_IMAGE, "图像文件"),
                            SelectOption::new(VALUE_TYPE_TEXT, "文本"),
                            SelectOption::new(VALUE_TYPE_NUMBER, "数字"),
                        ],
                    },
                ),
                ParamDef::new(
                    "path",
                    "图像文件",
                    ParamSpec::File {
                        default: String::new(),
                        dialog_title: "选择图像文件".into(),
                        extensions: ImageFormat::all_extensions(),
                        directory: false,
                    },
                )
                .visible_when("valueType", &[VALUE_TYPE_IMAGE]),
                ParamDef::new(
                    "text",
                    "文本内容",
                    ParamSpec::Text {
                        default: String::new(),
                        multiline: true,
                        placeholder: Some("在这里输入文本…".into()),
                    },
                )
                .visible_when("valueType", &[VALUE_TYPE_TEXT]),
                ParamDef::new(
                    "number",
                    "数值",
                    ParamSpec::Number {
                        default: 0.0,
                        min: -1e12,
                        max: 1e12,
                        step: 1.0,
                        integer: false,
                        unit: None,
                    },
                )
                .visible_when("valueType", &[VALUE_TYPE_NUMBER]),
            ],
            notes: vec![
                "每个工作流都从「输入」节点开始；一个工作流可以放多个输入，几条支线各走各的。"
                    .into(),
                "选「图像文件」时，输出类型跟着文件格式走 —— 选了 JPEG，输出就是 Image(Jpeg)，\
                 这样下游的检查才是准的。"
                    .into(),
                "工作流里只记路径，不会把图片内容一起存下来。文件挪走之后要重新选一次。".into(),
            ],
        },
        output_ports,
        run,
    )
}

fn output_ports(params: &Params) -> Vec<PortDef> {
    let port_type = match params::string(params, "valueType", VALUE_TYPE_IMAGE).as_str() {
        VALUE_TYPE_TEXT => PortType::Text,
        VALUE_TYPE_NUMBER => PortType::Number,
        _ => PortType::Image(selected_image_format(params)),
    };
    vec![PortDef::new("out", port_type.label(), port_type)]
}

/// 格式以文件头为准，扩展名只在读不出文件头时兜底。
fn selected_image_format(params: &Params) -> ImageFormat {
    let Some(path) = params::string_opt(params, "path") else {
        return ImageFormat::Any;
    };
    let path = Path::new(&path);
    image_io::guess_format_from_path(path)
        .or_else(|| ImageFormat::from_path(path))
        .unwrap_or(ImageFormat::Any)
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let value = match params::string(args.params, "valueType", VALUE_TYPE_IMAGE).as_str() {
        VALUE_TYPE_TEXT => Value::text(params::string(args.params, "text", "")),
        VALUE_TYPE_NUMBER => Value::Number(params::number(args.params, "number", 0.0)),
        _ => {
            let path = params::string_opt(args.params, "path")
                .ok_or_else(|| NodeError::new("还没有选择图像文件"))?;
            Value::Image(ImageValue::open(Path::new(&path))?)
        }
    };

    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), value);
    Ok(outputs)
}
