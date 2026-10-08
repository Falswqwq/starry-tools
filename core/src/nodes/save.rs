//! 「保存到目录」节点 —— 把接进来的产物写到用户挑的目录里。
//!
//! 它接收的是最底层的接口 **FILE**（能写盘的东西）：图像、视频、以后可能的音频
//! 都能保存。字节原样写出，既不重新编码也不改内容 —— 这一步只是「挪个地方」。
//!
//! 它是个出口：输出端口给的是写下的那个路径，方便在运行记录里看一眼。

use std::path::{Path, PathBuf};

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, OutputName, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "save_output";

const PARAM_DIRECTORY: &str = "directory";
const PARAM_OVERWRITE: &str = "overwrite";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "保存到目录".into(),
            category: "产出".into(),
            description: "把接进来的产物写到指定目录，输出是最终写下的路径。\
                          不改内容也不换格式，只是把产物放到你想要的地方。"
                .into(),
            is_source: false,
            inputs: vec![PortDef::new("file", "产物", PortType::File)
                .required()
                .hint("任何能写盘的产物（图像、视频……）")],
            outputs: vec![PortDef::new("path", "保存路径", PortType::Text)],
            params: vec![
                ParamDef::new(
                    PARAM_DIRECTORY,
                    "目标目录",
                    ParamSpec::File {
                        default: String::new(),
                        dialog_title: "选择保存目录".into(),
                        extensions: Vec::new(),
                        directory: true,
                    },
                ),
                ParamDef::new(
                    PARAM_OVERWRITE,
                    "允许覆盖已有产物",
                    ParamSpec::Bool { default: true },
                )
                .described("关掉的话，遇到重名就在名字后面加序号另存，不动已有的文件。"),
            ],
            notes: vec![
                "文件名按这个顺序定：上游「重命名」节点给的名字 → 输入的来源名字 → `output`；\
                 扩展名一律按产物**实际**的类型写。"
                    .into(),
                "目录不存在会自动创建。".into(),
                "这一步不重新编码，写出字节和上游产出的完全一致。".into(),
                "接的是最底层的「文件」类型，所以图像、视频都能往这里送。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 先把值拿在手里（图像只是复制一个 Arc），后面才好拿 `args` 记提示。
    let value = args.input("file")?.clone();
    let (bytes, extension) = value
        .product()
        .ok_or_else(|| NodeError::new("这个值不能写盘（只有图像、视频这类产物可以）"))?;
    let name_hint = value.name_hint().cloned();
    let directory = params::string_opt(args.params, PARAM_DIRECTORY)
        .ok_or_else(|| NodeError::new("还没有选择保存目录"))?;
    let overwrite = params::boolean(args.params, PARAM_OVERWRITE, true);

    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)
        .map_err(|err| NodeError::new(format!("无法创建目录 {}：{err}", directory.display())))?;

    let file_name = file_name_for(value.product_origin(), name_hint.as_ref(), extension);
    let mut target = directory.join(&file_name);
    // 不允许覆盖时，重名就另起一个名字 —— 宁可多一个文件，也不动人家已有的东西。
    if !overwrite && target.exists() {
        let renamed = unique_name(&directory, &file_name);
        args.warn(format!("{file_name} 已存在，改存为 {renamed}"));
        target = directory.join(renamed);
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| NodeError::new(format!("无法创建目录 {}：{err}", parent.display())))?;
    }
    std::fs::write(&target, bytes)
        .map_err(|err| NodeError::new(format!("写入 {} 失败：{err}", target.display())))?;

    let mut outputs = ValueMap::new();
    outputs.insert(
        "path".to_string(),
        Value::text(target.to_string_lossy().to_string()),
    );
    Ok(outputs)
}

/// 文件名怎么定：
///
/// 1. 上游有「重命名」节点留下的提示，就用它（按实际类型决定要不要补扩展名）；
/// 2. 否则沿用来源文件名，但扩展名一律按产物**实际**的类型写 ——
///    上游可能换过格式，来源的扩展名这时候是过期的；
/// 3. 再从文件读来的都没有，就叫 `output`。
fn file_name_for(origin: Option<&Path>, hint: Option<&OutputName>, extension: &str) -> String {
    if let Some(hint) = hint {
        return hint.file_name(extension);
    }
    let stem = origin
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("output");
    format!("{stem}.{extension}")
}

/// 找一个还没被占用的名字：`名字 (2).png`、`名字 (3).png`……
fn unique_name(directory: &Path, file_name: &str) -> String {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("output");
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("png");
    for index in 2..10_000 {
        let candidate = format!("{stem} ({index}).{extension}");
        if !directory.join(&candidate).exists() {
            return candidate;
        }
    }
    format!(
        "{stem} ({}).{extension}",
        crate::model::workflow::now_millis()
    )
}
