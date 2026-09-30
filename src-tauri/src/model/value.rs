//! 沿连线流动的值。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::NodeError;
use crate::image_io::ImageValue;
use crate::model::port_type::{ImageFormat, PortType};

/// 下游写盘时该用的文件名 —— 「重命名」节点留下的提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputName {
    /// 用户写的文件名主干。
    pub stem: String,
    /// 要不要按值实际的格式自动补上扩展名。
    pub auto_extension: bool,
}

impl OutputName {
    /// 给定图像实际的格式，算出最终的文件名。
    pub fn file_name(&self, format: ImageFormat) -> String {
        if self.auto_extension {
            format!("{}.{}", self.stem, format.extension())
        } else {
            self.stem.clone()
        }
    }
}

/// 外面套了一层名字的值。
#[derive(Debug, Clone)]
pub struct NamedValue {
    pub name: OutputName,
    pub inner: Value,
}

#[derive(Debug, Clone)]
pub enum Value {
    Text(Arc<str>),
    Number(f64),
    Bool(bool),
    Image(ImageValue),
    /// 带着名字的值 —— 「重命名」节点的产物。里面是什么类型都行。
    Named(Box<NamedValue>),
}

impl Value {
    pub fn text(value: impl Into<String>) -> Self {
        Value::Text(Arc::from(value.into()))
    }

    /// 剥掉外层的名字，拿到真正的值。
    pub fn inner(&self) -> &Value {
        let mut current = self;
        while let Value::Named(named) = current {
            current = &named.inner;
        }
        current
    }

    /// 这个值身上挂着的名字。
    pub fn name_hint(&self) -> Option<&OutputName> {
        match self {
            Value::Named(named) => Some(&named.name),
            _ => None,
        }
    }

    /// 给它挂上（或摘掉）一个名字。原来挂着的会被替掉，不会套成好几层。
    pub fn with_name_hint(self, hint: Option<OutputName>) -> Value {
        let inner = match self {
            Value::Named(named) => named.inner,
            other => other,
        };
        match hint {
            Some(name) => Value::Named(Box::new(NamedValue { name, inner })),
            None => inner,
        }
    }

    /// 值自身的类型，运行期校验就用它。
    pub fn port_type(&self) -> PortType {
        match self {
            // 名字不算类型，得看里面包的是什么。
            Value::Named(named) => named.inner.port_type(),
            Value::Text(_) => PortType::Text,
            Value::Number(_) => PortType::Number,
            Value::Bool(_) => PortType::Bool,
            Value::Image(image) => PortType::Image(image.format()),
        }
    }

    /// 界面上的一行摘要。
    pub fn describe(&self) -> String {
        match self {
            Value::Named(named) => format!("{} · {}", named.name.stem, named.inner.describe()),
            Value::Text(text) => {
                let flattened = text.replace(['\n', '\r'], " ");
                // 长文本把中段省掉，两头都留着 —— 路径这类东西，尾巴比开头有用。
                const HEAD: usize = 26;
                const TAIL: usize = 20;
                let characters: Vec<char> = flattened.chars().collect();
                if characters.len() <= HEAD + TAIL + 1 {
                    flattened
                } else {
                    let head: String = characters[..HEAD].iter().collect();
                    let tail: String = characters[characters.len() - TAIL..].iter().collect();
                    format!("{head}…{tail}")
                }
            }
            Value::Number(number) => {
                if number.fract() == 0.0 && number.abs() < 1e15 {
                    format!("{}", *number as i64)
                } else {
                    format!("{number}")
                }
            }
            Value::Bool(value) => (if *value { "是" } else { "否" }).to_string(),
            Value::Image(image) => match image.dimensions() {
                Ok((width, height)) => format!(
                    "{width} × {height} · {} · {}",
                    image.format().badge(),
                    human_size(image.byte_len())
                ),
                Err(_) => format!("{} · {} 字节", image.format().badge(), image.byte_len()),
            },
        }
    }

    pub fn as_image(&self) -> Result<&ImageValue, NodeError> {
        match self {
            Value::Named(named) => named.inner.as_image(),
            Value::Image(image) => Ok(image),
            other => Err(NodeError::new(format!(
                "需要一个图像，但拿到的是{}",
                other.port_type().label()
            ))),
        }
    }

    pub fn as_text(&self) -> Result<String, NodeError> {
        match self {
            Value::Named(named) => named.inner.as_text(),
            Value::Text(text) => Ok(text.to_string()),
            Value::Number(number) => Ok(number.to_string()),
            Value::Bool(value) => Ok(if *value { "true" } else { "false" }.to_string()),
            other => Err(NodeError::new(format!(
                "需要一段文本，但拿到的是{}",
                other.port_type().label()
            ))),
        }
    }

    pub fn as_number(&self) -> Result<f64, NodeError> {
        match self {
            Value::Named(named) => named.inner.as_number(),
            Value::Number(number) => Ok(*number),
            Value::Text(text) => text
                .trim()
                .parse()
                .map_err(|_| NodeError::new(format!("「{text}」不是一个数字"))),
            other => Err(NodeError::new(format!(
                "需要一个数字，但拿到的是{}",
                other.port_type().label()
            ))),
        }
    }

    pub fn as_bool(&self) -> Result<bool, NodeError> {
        match self {
            Value::Named(named) => named.inner.as_bool(),
            Value::Bool(value) => Ok(*value),
            other => Err(NodeError::new(format!(
                "需要一个布尔值，但拿到的是{}",
                other.port_type().label()
            ))),
        }
    }
}

pub fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// 一个节点所有输出端口的结果，按端口 id 索引。
pub type ValueMap = BTreeMap<String, Value>;

/// 节点执行时能拿到的东西。
pub struct NodeArgs<'a> {
    pub node_id: &'a str,
    pub params: &'a crate::model::params::Params,
    pub inputs: &'a ValueMap,
    pub warnings: &'a mut Vec<String>,
}

impl NodeArgs<'_> {
    pub fn input(&self, port: &str) -> Result<&Value, NodeError> {
        self.inputs
            .get(port)
            .ok_or_else(|| NodeError::new(format!("输入端口「{port}」没有连上任何数据")))
    }

    pub fn image(&self, port: &str) -> Result<&ImageValue, NodeError> {
        self.input(port)?.as_image()
    }

    /// 输入端口上跟着的名字。转换类节点重新构造值之后，用它把名字接着传下去。
    pub fn input_name(&self, port: &str) -> Option<OutputName> {
        self.inputs
            .get(port)
            .and_then(|value| value.name_hint())
            .cloned()
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}
