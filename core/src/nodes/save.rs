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

    let file_name = file_name_for(value.product_origin(), name_hint.as_ref(), extension)?;
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)
        .map_err(|err| NodeError::new(format!("无法创建目录 {}：{err}", directory.display())))?;

    let mut target = directory.join(&file_name);
    let mut file = if overwrite {
        if let Ok(meta) = std::fs::symlink_metadata(&target) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(NodeError::new("不能覆盖符号链接或非普通文件"));
            }
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&target)?
    } else {
        let path = Path::new(&file_name);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&file_name);
        let suffix = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| format!(".{ext}"))
            .unwrap_or_default();
        let mut created = None;
        for index in 1..=10_000 {
            let candidate = if index == 1 {
                file_name.clone()
            } else {
                format!("{stem} ({index}){suffix}")
            };
            crate::engine::validate_file_name(&candidate)?;
            target = directory.join(&candidate);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(file) => {
                    if index > 1 {
                        args.warn(format!("{file_name} 已存在，改存为 {candidate}"));
                    }
                    created = Some(file);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        created.ok_or_else(|| NodeError::new("找不到可用的保存文件名"))?
    };
    std::io::Write::write_all(&mut file, bytes)
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
fn file_name_for(
    origin: Option<&Path>,
    hint: Option<&OutputName>,
    extension: &str,
) -> Result<String, NodeError> {
    let name = if let Some(hint) = hint {
        crate::engine::validate_file_name(&hint.stem)?;
        hint.file_name(extension)
    } else {
        let stem = origin
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .unwrap_or("output");
        crate::engine::validate_file_name(stem)?;
        format!("{stem}.{extension}")
    };
    crate::engine::validate_file_name(&name)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_are_validated_even_without_a_rename_node() {
        for stem in [
            "../escape",
            "..\\escape",
            "C:escape",
            "NUL",
            "COM1.png",
            "tail.",
            "tail ",
        ] {
            for auto_extension in [true, false] {
                let hint = OutputName {
                    stem: stem.into(),
                    auto_extension,
                };
                assert!(file_name_for(None, Some(&hint), "png").is_err(), "{stem}");
            }
        }
        assert!(file_name_for(Some(Path::new("NUL.png")), None, "png").is_err());
        assert!(file_name_for(None, None, "../escape").is_err());
        assert_eq!(
            file_name_for(
                None,
                Some(&OutputName {
                    stem: "hero".into(),
                    auto_extension: false
                }),
                "png"
            )
            .unwrap(),
            "hero"
        );
    }
}
