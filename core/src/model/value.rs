//! 沿连线流动的值。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::NodeError;
use crate::image_io::ImageValue;
use crate::interaction::{Interaction, InteractionKind, InteractionResponse};
use crate::media::MediaValue;
use crate::model::port_type::PortType;
use crate::progress::{NodeStep, Progress};

/// 下游写盘时该用的文件名 —— 「重命名」节点留下的提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputName {
    /// 用户写的文件名主干。
    pub stem: String,
    /// 要不要按值实际的格式自动补上扩展名。
    pub auto_extension: bool,
}

impl OutputName {
    /// 给定写盘用的扩展名，算出最终的文件名。
    pub fn file_name(&self, extension: &str) -> String {
        if self.auto_extension {
            format!("{}.{}", self.stem, extension)
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
    /// 非图像的产物（视频……）。与图像一样都是「能写盘的东西」。
    Media(MediaValue),
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
            Value::Media(media) => media.port_type(),
        }
    }

    /// 界面上的一行摘要。
    pub fn describe(&self) -> String {
        match self {
            Value::Named(named) => format!("{} · {}", named.name.stem, named.inner.describe()),
            Value::Text(text) => {
                // 色板（hex 一行一个）单独说，不然一长串颜色会被截成没意义的文字。
                if let Some(palette) = parse_palette(text) {
                    return format!("色板 · {} 色", palette.len());
                }
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
            Value::Media(media) => media.describe(),
        }
    }

    /// 这个值当作**参数**用时，转成参数的 JSON 形态；转不了返回 `None`。
    ///
    /// 参数端口把上游的值覆盖到参数上时用它 —— 能让参数吃下的就三种基本类型，
    /// 图像不行。
    pub fn to_param_json(&self) -> Option<serde_json::Value> {
        Some(match self.inner() {
            Value::Text(text) => serde_json::json!(text.as_ref()),
            Value::Number(number) => serde_json::json!(number),
            Value::Bool(value) => serde_json::json!(value),
            // `inner()` 已经把名字剥光了，这里只是给编译器一个交代。
            Value::Named(named) => named.inner.to_param_json()?,
            Value::Image(_) | Value::Media(_) => return None,
        })
    }

    /// 取产物字节，连同它写盘该用的扩展名。图像和通用媒体都算产物。
    pub fn product(&self) -> Option<(&[u8], &'static str)> {
        match self.inner() {
            Value::Image(image) => Some((image.bytes(), image.format().extension())),
            Value::Media(media) => Some((media.bytes(), media.extension())),
            _ => None,
        }
    }

    /// 产物原来的出处（文件路径）—— 写盘起名时用。
    pub fn product_origin(&self) -> Option<&std::path::Path> {
        match self.inner() {
            Value::Image(image) => image.origin(),
            Value::Media(media) => media.origin(),
            _ => None,
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

/// 把一段文本当成**色板**解析：每行一个颜色，`#rrggbb` 或 `#rrggbbaa`。
///
/// 只要有一行不是颜色或者一行都没有，就返回 `None`（那就只是一段普通文本）。
/// 返回的是规范化后的颜色串（小写、带 `#`）。
pub fn parse_palette(text: &str) -> Option<Vec<String>> {
    let mut colors = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let hex = line.strip_prefix('#')?;
        if !matches!(hex.len(), 6 | 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        colors.push(format!("#{}", hex.to_ascii_lowercase()));
    }
    if colors.is_empty() {
        None
    } else {
        Some(colors)
    }
}

/// 色板 → 文本：一行一个颜色。
pub fn palette_text(colors: &[[u8; 3]]) -> String {
    colors
        .iter()
        .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
        .collect::<Vec<_>>()
        .join("\n")
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

/// 只有一个输出端口时，省掉手写 `ValueMap::new()` + `insert` 的样板。
pub fn one_output(port: &str, value: Value) -> ValueMap {
    let mut outputs = ValueMap::new();
    outputs.insert(port.to_string(), value);
    outputs
}

/// 节点执行时能拿到的东西。
pub struct NodeArgs<'a> {
    pub node_id: &'a str,
    /// 节点类型名（「图像裁切」这种）。阻塞节点弹窗时要用它报出是谁在等。
    pub node_name: &'a str,
    pub params: &'a crate::model::params::Params,
    pub inputs: &'a ValueMap,
    pub warnings: &'a mut Vec<String>,
    /// 与界面通话的通道。没有界面时（脱离 GUI 直接调 `run`）是 `None`。
    pub interaction: Option<&'a Interaction>,
    /// 运行进度通道（只有长任务会用到）。脱离 GUI 直接调 `run` 时是 `None`。
    pub progress: Option<&'a Progress>,
}

impl NodeArgs<'_> {
    /// 停下来问界面一次，阻塞到用户给出答案。
    ///
    /// 只有紫色节点会用到；没有界面时直接报错，而不是静默地瞎猜一个结果。
    pub fn ask(&self, kind: InteractionKind) -> Result<InteractionResponse, NodeError> {
        let Some(interaction) = self.interaction else {
            return Err(NodeError::new("这个节点需要用户操作，但当前没有可用的界面"));
        };
        interaction.ask(self.node_id, self.node_name, kind)
    }
    pub fn input(&self, port: &str) -> Result<&Value, NodeError> {
        self.inputs
            .get(port)
            .ok_or_else(|| NodeError::new(format!("输入端口「{port}」没有连上任何数据")))
    }

    pub fn image(&self, port: &str) -> Result<&ImageValue, NodeError> {
        self.input(port)?.as_image()
    }

    /// 取图像输入，连同它带着的名字。
    ///
    /// 图像节点几乎都这么开头：把输入克隆出来（只是复制一个 `Arc`），
    /// 记住它的名字，处理完再把名字挂回输出。
    pub fn image_in(&self, port: &str) -> Result<(ImageValue, Option<OutputName>), NodeError> {
        let value = self.input(port)?;
        Ok((value.as_image()?.clone(), value.name_hint().cloned()))
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

    /// 报一次运行中的进度 —— 界面会在卡片上画进度条与帧计数。
    /// 没有界面时什么也不做。
    pub fn report(&self, step: NodeStep) {
        if let Some(progress) = self.progress {
            progress.step(self.node_id, step);
        }
    }
}
