//! 「读取」节点 —— 从硬盘上读一个文件。
//!
//! 「类型」参数说这个文件是什么：**图像文件**按图像解码，**文本**当文本读进来。
//! 输出类型跟着文件与类型自动推断（和「输入」原来那套一致）。
//!
//! 没有「数字」这一档 —— 从一个文件里读一个数太拧巴了；要给一个数字，直接用
//! 「数字」字面量节点。

use std::path::Path;

use crate::error::NodeError;
use crate::image_io::{self, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "read";

const VALUE_TYPE_IMAGE: &str = "image";
const VALUE_TYPE_TEXT: &str = "text";

/// 文本文件对话框里认的扩展名。
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "json", "csv", "tsv", "toml", "yaml", "yml", "xml", "ini", "log",
    "rs", "js", "ts", "py", "sh", "c", "h", "cpp", "css", "html",
];

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "读取".into(),
            category: "来源".into(),
            description: "从硬盘上读一个文件：图像按图像解码，文本按文本读进来。\
                          类型跟着文件自动推断。"
                .into(),
            is_source: true,
            inputs: vec![],
            // 声明里放泛化的 `Any`（卡片上显示 `ANY`）；真正的类型由 `output_ports` 现算。
            outputs: vec![PortDef::new("out", "值", PortType::Any)],
            params: vec![
                ParamDef::new(
                    "valueType",
                    "类型",
                    ParamSpec::Select {
                        default: VALUE_TYPE_IMAGE.into(),
                        options: vec![
                            SelectOption::new(VALUE_TYPE_IMAGE, "图像文件"),
                            SelectOption::new(VALUE_TYPE_TEXT, "文本文件"),
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
                    "textPath",
                    "文本文件",
                    ParamSpec::File {
                        default: String::new(),
                        dialog_title: "选择文本文件".into(),
                        extensions: TEXT_EXTENSIONS.iter().map(|ext| ext.to_string()).collect(),
                        directory: false,
                    },
                )
                .visible_when("valueType", &[VALUE_TYPE_TEXT]),
            ],
            notes: vec![
                "「类型」决定怎么读：图像文件按图像解码，文本文件按 UTF-8 文本读进来。".into(),
                "图像格式以**文件头**为准 —— 输出类型跟着它变，下游的类型检查因此是准的。".into(),
                "工作流里只记路径，不会把文件内容一起存下来。文件挪走之后要重新选一次。".into(),
            ],
        },
        output_ports,
        run,
    )
}

/// 输出类型：图像按文件头的真实格式，文本就是 `Text`，还没选文件时是 `Any`。
fn output_ports(params: &Params) -> Vec<PortDef> {
    let port_type = match params::string(params, "valueType", VALUE_TYPE_IMAGE).as_str() {
        VALUE_TYPE_TEXT => PortType::Text,
        _ => match selected_image_format(params) {
            ImageFormat::Any => PortType::Any,
            format => PortType::Image(format),
        },
    };
    vec![PortDef::new("out", port_type.label(), port_type)]
}

/// 格式以文件头为准，扩展名只在读不出文件头时兜底。认不出来返回 [`ImageFormat::Any`]。
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
    let value = if params::string(args.params, "valueType", VALUE_TYPE_IMAGE) == VALUE_TYPE_TEXT {
        let path = params::string_opt(args.params, "textPath")
            .ok_or_else(|| NodeError::new("还没有选择文本文件"))?;
        let text = std::fs::read_to_string(&path)
            .map_err(|err| NodeError::new(format!("读不出文本文件 {path}：{err}")))?;
        Value::text(text)
    } else {
        let path = params::string_opt(args.params, "path")
            .ok_or_else(|| NodeError::new("还没有选择图像文件"))?;
        Value::Image(ImageValue::open(Path::new(&path))?)
    };

    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), value);
    Ok(outputs)
}
