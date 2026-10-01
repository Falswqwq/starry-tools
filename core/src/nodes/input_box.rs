//! 「输入框」节点 —— 一块大输入区。
//!
//! 把文件拖进来、`Ctrl+V` 粘贴，或者点一下直接打字：文本就显示文本，图片就直接把
//! 图片显示出来。它是给「手边就有这段东西」准备的入口 —— 不用先去磁盘上找个路径。
//!
//! 值存在参数 `value` 里，是一个 JSON：
//!
//! * `""`            —— 还空着；
//! * `"一段文字"`     —— 文本；
//! * `{"file":"…"}`  —— 一个文件（工作流只记路径，不把内容存下来）。

use std::path::Path;

use serde_json::Value as Json;

use crate::error::NodeError;
use crate::image_io::{self, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params::Params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "input_box";

/// 参数 id。前端往里写的就是它。
pub const PARAM_VALUE: &str = "value";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "输入框".into(),
            category: "来源".into(),
            description: "一块输入区：把文件拖进来、Ctrl+V 粘贴，或者点一下直接打字。\
                          文本就显示文本，图片就直接显示图片。"
                .into(),
            is_source: true,
            inputs: vec![],
            outputs: vec![PortDef::new("out", "值", PortType::Any)],
            params: vec![ParamDef::new(PARAM_VALUE, "", ParamSpec::DropZone)],
            notes: vec![
                "把图片文件拖到框里，或者在框上 `Ctrl+V` 粘贴文件路径，就能装进一张图片。".into(),
                "点一下空框会变成一个文本框，直接打字；打完点别处就留下文本，没打就变回原样。"
                    .into(),
                "输出类型跟着内容走：文本就是 `Text`，图片按文件头的真实格式。".into(),
                "工作流里只记文件路径，不会把图片内容一起存下来。".into(),
            ],
        },
        output_ports,
        run,
    )
}

/// 输入区里的内容。
pub enum Content {
    Text(String),
    File(String),
}

/// 从参数里读出内容；空着返回 `None`。
pub fn content(params: &Params) -> Option<Content> {
    match params.get(PARAM_VALUE)? {
        Json::String(text) if !text.is_empty() => Some(Content::Text(text.clone())),
        Json::Object(map) => map
            .get("file")
            .and_then(Json::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| Content::File(path.to_string())),
        _ => None,
    }
}

fn output_ports(params: &Params) -> Vec<PortDef> {
    let port_type = match content(params) {
        Some(Content::Text(_)) => PortType::Text,
        Some(Content::File(path)) => match format_of(Path::new(&path)) {
            ImageFormat::Any => PortType::Any,
            format => PortType::Image(format),
        },
        None => PortType::Any,
    };
    vec![PortDef::new("out", port_type.label(), port_type)]
}

/// 文件头优先，扩展名兜底。
fn format_of(path: &Path) -> ImageFormat {
    image_io::guess_format_from_path(path)
        .or_else(|| ImageFormat::from_path(path))
        .unwrap_or(ImageFormat::Any)
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let value = match content(args.params) {
        Some(Content::Text(text)) => Value::text(text),
        Some(Content::File(path)) => Value::Image(ImageValue::open(Path::new(&path))?),
        None => return Err(NodeError::new("输入框还是空的")),
    };

    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), value);
    Ok(outputs)
}
