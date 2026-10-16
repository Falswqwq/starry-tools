//! 视频处理功能模块。
//!
//! 它给应用补上「视频」这一类：登记 `VID` 接口与一批具体视频格式，并提供节点。
//! 视频的实际编码 / 解码交给外部 `ffmpeg` 可执行文件 —— 不引入任何 crate，
//! 也就没有原生库依赖、没有打包负担。

use serde::{Deserialize, Serialize};

use crate::registry::{CategoryInfo, RegistrationError, RegistryBuilder, ToolSpec};
use crate::types::{PortType, TypeColor};

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
    pub fn port_type(self) -> PortType {
        match self {
            Self::Any => PortType::trait_("vid"),
            concrete => PortType::concrete(concrete.name()),
        }
    }

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

/// 此 feature 自带类别、颜色、工具探测、类型和节点，不污染核心的封闭枚举。
pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistrationError> {
    let color = TypeColor::rgb(0x0d, 0x94, 0x88);
    // 分类只做分组：视频节点走正常主题边线，不额外占用一种节点颜色。
    registry.register_category(CategoryInfo::new("视频", "视频"))?;
    registry.register_tool(ToolSpec::new("ffmpeg", ffmpeg::available))?;
    let types = registry.types_mut();
    types.register_trait("vid", "视频", "VID", color, &["file"])?;
    for format in VideoFormat::CONCRETE {
        types.register_type(
            format.name(),
            format.label(),
            format.badge(),
            color,
            &["vid", "file"],
            Some(format.extension()),
        )?;
    }
    registry.register_node(compress::spec())?;
    Ok(())
}
