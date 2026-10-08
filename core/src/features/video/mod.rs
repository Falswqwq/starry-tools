//! 视频处理功能模块。
//!
//! 它给应用补上「视频」这一类：登记 `VID` 接口与一批具体视频格式，并提供节点。
//! 视频的实际编码 / 解码交给外部 `ffmpeg` 可执行文件 —— 不引入任何 crate，
//! 也就没有原生库依赖、没有打包负担。

use serde::{Deserialize, Serialize};

use crate::registry::NodeSpec;
use crate::types::{Registry, TypeColor};

mod compress;
mod ffmpeg;

/// 视频容器格式。
///
/// `Any` 只作为**静态类型**里的「视频」接口出现，运行时产出的值永远是具体格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoFormat {
    Any,
    Mp4,
    Mkv,
    Webm,
    Mov,
    Avi,
}

impl VideoFormat {
    pub const CONCRETE: [VideoFormat; 5] = [
        VideoFormat::Mp4,
        VideoFormat::Mkv,
        VideoFormat::Webm,
        VideoFormat::Mov,
        VideoFormat::Avi,
    ];

    pub fn name(self) -> &'static str {
        match self {
            VideoFormat::Any => "any",
            VideoFormat::Mp4 => "mp4",
            VideoFormat::Mkv => "mkv",
            VideoFormat::Webm => "webm",
            VideoFormat::Mov => "mov",
            VideoFormat::Avi => "avi",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "any" => VideoFormat::Any,
            "mp4" => VideoFormat::Mp4,
            "mkv" => VideoFormat::Mkv,
            "webm" => VideoFormat::Webm,
            "mov" => VideoFormat::Mov,
            "avi" => VideoFormat::Avi,
            _ => return None,
        })
    }

    pub fn badge(self) -> &'static str {
        match self {
            VideoFormat::Any => "VID",
            VideoFormat::Mp4 => "MP4",
            VideoFormat::Mkv => "MKV",
            VideoFormat::Webm => "WEBM",
            VideoFormat::Mov => "MOV",
            VideoFormat::Avi => "AVI",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Any => "视频",
            VideoFormat::Mp4 => "MP4 视频",
            VideoFormat::Mkv => "MKV 视频",
            VideoFormat::Webm => "WebM 视频",
            VideoFormat::Mov => "MOV 视频",
            VideoFormat::Avi => "AVI 视频",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Any | VideoFormat::Mp4 => "mp4",
            VideoFormat::Mkv => "mkv",
            VideoFormat::Webm => "webm",
            VideoFormat::Mov => "mov",
            VideoFormat::Avi => "avi",
        }
    }

    /// 按文件扩展名猜格式。
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        match ext.as_str() {
            "mp4" | "m4v" => Some(VideoFormat::Mp4),
            "mkv" => Some(VideoFormat::Mkv),
            "webm" => Some(VideoFormat::Webm),
            "mov" => Some(VideoFormat::Mov),
            "avi" => Some(VideoFormat::Avi),
            _ => None,
        }
    }

    /// 所有视频扩展名，供文件对话框过滤。
    pub fn all_extensions() -> Vec<String> {
        Self::CONCRETE
            .iter()
            .map(|format| format.extension().to_string())
            .collect()
    }
}

/// 视频格式在类型系统里实现的接口。
const VIDEO_TRAITS: &[&str] = &["vid", "file"];

/// 向类型登记表补上视频的接口与具体格式。
pub fn register(registry: &mut Registry) {
    registry.register_trait("vid", "视频", "VID", TypeColor::Video, &["file"]);
    for format in VideoFormat::CONCRETE {
        registry.register_type(
            format.name(),
            format.label(),
            format.badge(),
            TypeColor::Video,
            VIDEO_TRAITS,
            Some(format.extension()),
        );
    }
}

/// 这个模块提供的节点。
pub fn specs() -> Vec<NodeSpec> {
    vec![compress::spec()]
}

/// 界面想知道「这台机器能不能跑视频」时问它。
pub fn tool_available() -> bool {
    ffmpeg::available()
}
