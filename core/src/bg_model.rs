//! 背景移除用的 ONNX 模型：清单、本地路径、下载。
//!
//! 应用**不自带模型**（u2net 那几个都太大了），模型放在
//! `<数据目录>/models/` 下，用的时候没有就给节点挂一个「下载模型」按钮。
//!
//! 每个模型给了两个源：**原始源**（GitHub release）和**镜像源**（hf-mirror）。
//! 下载先写 `.part` 临时文件，成功再改名 —— 中途断了不会留下半截文件冒充「已下载」。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use crate::paths::models_dir;

/// 下载用的 HTTP 客户端。只设一个**连接超时** —— 服务器连不上时快速失败，
/// 而不是挂在那里等；下载本身可能很久，所以不设全局超时。
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .build()
        .into()
});

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct Source {
    /// 给人看的名字。
    pub label: &'static str,
    pub url: &'static str,
}

/// 一个模型。
#[derive(Debug, Clone, Copy)]
pub struct Model {
    /// 稳定 id，参数里存的就是它。
    pub id: &'static str,
    /// 给人看的名字。
    pub name: &'static str,
    /// 本地文件名。
    pub file: &'static str,
    /// 大概多大（只用于显示）。
    pub size: &'static str,
    /// 一句话说明。
    pub note: &'static str,
    pub sources: &'static [Source],
}

const U2NETP_SOURCES: &[Source] = &[
    Source {
        label: "GitHub（原始源）",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
    },
    Source {
        label: "hf-mirror（镜像源）",
        url: "https://hf-mirror.com/tomjackson2023/rembg/resolve/main/u2netp.onnx",
    },
];

const U2NET_SOURCES: &[Source] = &[
    Source {
        label: "hf-mirror（镜像源）",
        url: "https://hf-mirror.com/tomjackson2023/rembg/resolve/main/u2net.onnx",
    },
    Source {
        label: "GitHub（原始源）",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2net.onnx",
    },
];

/// 内置的模型清单。
pub const MODELS: &[Model] = &[
    Model {
        id: "u2netp",
        name: "U²-Netp",
        file: "u2netp.onnx",
        size: "≈ 4.7 MB",
        note: "小、快，一般够用",
        sources: U2NETP_SOURCES,
    },
    Model {
        id: "u2net",
        name: "U²-Net",
        file: "u2net.onnx",
        size: "≈ 176 MB",
        note: "更大更准，慢一些",
        sources: U2NET_SOURCES,
    },
];

/// 按 id 找模型。
pub fn find(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.id == id)
}

/// 默认用哪个。
pub fn default_model() -> &'static Model {
    &MODELS[0]
}

/// 某个模型在本地应该放哪儿。
pub fn path_for(model: &Model) -> PathBuf {
    models_dir().join(model.file)
}

/// 某个模型在本地吗（按 id）。
pub fn is_downloaded(id: &str) -> bool {
    find(id).is_some_and(|model| path_for(model).is_file())
}

/// 从 `url` 把模型下到本地（`model` 对应的位置）。
///
/// `cancel` 一置真就中止并清掉临时文件；`on_progress` 每收到一块就调一次，
/// 参数是「已下多少字节、总共多少字节（不知道就是 `None`）」。
pub fn download(
    model: &Model,
    url: &str,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(u64, Option<u64>),
) -> Result<PathBuf, String> {
    use std::io::{Read, Write};

    let target = path_for(model);
    let response = AGENT
        .get(url)
        .call()
        .map_err(|err| format!("连接失败：{err}"))?;
    let total = response.body().content_length();
    let mut reader = response.into_body().into_reader();

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("建目录失败：{err}"))?;
    }
    // 先写 .part，成功了再改名 —— 中途断了不会留下半截文件被当成「已下载」。
    let temp = target.with_extension("onnx.part");
    let mut file = std::fs::File::create(&temp).map_err(|err| format!("建文件失败：{err}"))?;

    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&temp);
            return Err("已取消".to_string());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("下载中断：{err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("写文件失败：{err}"))?;
        written += read as u64;
        on_progress(written, total);
    }
    drop(file);

    std::fs::rename(&temp, &target).map_err(|err| format!("收尾失败：{err}"))?;
    Ok(target)
}

/// 把模型文件删掉（「重新下载」用）。
pub fn remove(model: &Model) -> std::io::Result<()> {
    let path: &Path = &path_for(model);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_has_a_file_a_source_and_lives_under_the_models_dir() {
        assert!(!MODELS.is_empty());
        for model in MODELS {
            assert!(model.file.ends_with(".onnx"), "{}", model.file);
            assert!(!model.sources.is_empty(), "{} 没有下载源", model.id);
            for source in model.sources {
                assert!(
                    source.url.starts_with("https://"),
                    "{} 的源不是 https：{}",
                    model.id,
                    source.url
                );
            }
            assert!(path_for(model).starts_with(crate::paths::models_dir()));
        }
    }

    #[test]
    fn unknown_ids_are_not_downloaded() {
        assert!(find("nope").is_none());
        assert!(!is_downloaded("nope"));
    }

    #[test]
    fn the_default_model_exists() {
        assert!(find(default_model().id).is_some());
    }
}
