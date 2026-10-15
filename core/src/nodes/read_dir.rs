//! 「读取目录」节点 —— 从硬盘上的一个目录读出一批文件，装成一个**列表**。
//!
//! 这是第一个产出 `List[T]` 的节点，和「[遍历](iterate.rs)」正好配上：
//! 读一个装满图的目录 → 拿到 `List[Img]` → 交给「遍历」一项项解包。
//!
//! 和「读取」一样是**起点**节点（因此自动带一个空输入，可以挂到逻辑输出上排先后）。
//! 工作流里只记目录路径，不存文件内容 —— 目录变了重新跑就行。

use std::path::{Path, PathBuf};

use crate::error::NodeError;
use crate::image_io::ImageValue;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "read_dir";

const VALUE_TYPE_IMAGE: &str = "image";
const VALUE_TYPE_TEXT: &str = "text";

/// 一次最多读进来多少个文件 —— 指到一整个乱七八糟的大目录时不至于把内存吃光。
const MAX_FILES: usize = 1024;

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "读取目录".into(),
            category: "来源".into(),
            description: "把一个目录里的文件读成一个列表：图像装成 `List[Img]`，\
                          文本装成 `List[Text]`。接给「遍历」就能一项项处理。"
                .into(),
            is_source: true,
            inputs: vec![],
            // 声明里给泛化的样子（节点库卡片）；真正的元素类型由 `output_ports` 现算。
            outputs: vec![PortDef::new("out", "列表", PortType::any_list())],
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
                    "目录",
                    ParamSpec::File {
                        default: String::new(),
                        dialog_title: "选择目录".into(),
                        extensions: Vec::new(),
                        directory: true,
                    },
                ),
                ParamDef::new(
                    "recursive",
                    "包含子目录",
                    ParamSpec::Bool { default: false },
                )
                .described("打开时连子目录里的文件一起读，按文件名的字典序排。"),
            ],
            notes: vec![
                "目录里的文件按**文件名的字典序**排，所以列表的顺序是稳定的、可预期的。".into(),
                "图像按文件头解码，认不出的文件直接跳过（会报出跳过了几个）。".into(),
                "一次最多读 1024 个，多了会截断并提示 —— 免得指到一个大目录把内存吃光。".into(),
                "和「读取」一样，工作流里只记路径，不把文件内容一起存下来。".into(),
            ],
        },
        output_ports,
        run,
    )
}

/// 输出是一个列表：图像模式是 `List[Img]`，文本模式是 `List[Text]`。
fn output_ports(params: &Params) -> Vec<PortDef> {
    let element = if params::string(params, "valueType", VALUE_TYPE_IMAGE) == VALUE_TYPE_TEXT {
        PortType::Text
    } else {
        PortType::trait_("img")
    };
    vec![PortDef::new("out", "列表", PortType::list_of(element))]
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let dir =
        params::string_opt(args.params, "path").ok_or_else(|| NodeError::new("还没有选择目录"))?;
    let dir = Path::new(&dir);
    if !dir.is_dir() {
        return Err(NodeError::new(format!("{} 不是一个目录", dir.display())));
    }

    let text_mode = params::string(args.params, "valueType", VALUE_TYPE_IMAGE) == VALUE_TYPE_TEXT;
    let recursive = params::boolean(args.params, "recursive", false);

    let mut files: Vec<PathBuf> = Vec::new();
    collect(dir, recursive, text_mode, &mut files)?;
    files.sort();
    if files.len() > MAX_FILES {
        args.warn(format!(
            "目录里有 {} 个文件，只读前 {MAX_FILES} 个",
            files.len()
        ));
        files.truncate(MAX_FILES);
    }

    let mut items: Vec<Value> = Vec::new();
    let mut skipped = 0usize;
    for path in &files {
        match load(path, text_mode) {
            Ok(value) => items.push(value),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        args.warn(format!("跳过了 {skipped} 个读不出来的文件"));
    }
    if items.is_empty() {
        return Err(NodeError::new("这个目录里没有找到可读的文件"));
    }

    args.warn(format!("读到 {} 个文件", items.len()));
    Ok(one_output("out", Value::List(items)))
}

/// 走一遍目录，把该读的文件收进 `out`。子目录按需要递归。
fn collect(
    dir: &Path,
    recursive: bool,
    text_mode: bool,
    out: &mut Vec<PathBuf>,
) -> Result<(), NodeError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| NodeError::new(format!("读不出目录 {}：{err}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if recursive {
                collect(&path, true, text_mode, out)?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if readable(&path, text_mode) {
            out.push(path);
        }
    }
    Ok(())
}

/// 这个扩展名归不归我们要读的那一类。
fn readable(path: &Path, text_mode: bool) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if text_mode {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "txt"
                | "md"
                | "markdown"
                | "json"
                | "csv"
                | "tsv"
                | "toml"
                | "yaml"
                | "yml"
                | "xml"
                | "ini"
                | "log"
                | "rs"
                | "js"
                | "ts"
                | "py"
                | "sh"
                | "c"
                | "h"
                | "cpp"
                | "css"
                | "html"
        )
    } else {
        ImageFormat::from_extension(ext).is_some()
    }
}

fn load(path: &Path, text_mode: bool) -> Result<Value, NodeError> {
    if text_mode {
        let text = std::fs::read_to_string(path)
            .map_err(|err| NodeError::new(format!("读不出文本 {}：{err}", path.display())))?;
        Ok(Value::text(text))
    } else {
        Ok(Value::Image(ImageValue::open(path)?))
    }
}
