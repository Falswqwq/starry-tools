//! 端口类型系统。
//!
//! 数据在工作流里沿着连线流动，每条连线的两端都带类型。类型同时服务于两件事：
//!
//! * **编辑期的静态检查** —— 连线是否合法、节点是否跑得起来；
//! * **运行期的严格校验** —— 上游真正产出的值是否满足下游端口的声明。
//!
//! 两者故意宽松程度不同：编辑期允许「格式未知的图像」（[`ImageFormat::Any`]）
//! 接到任何图像端口上，运行期则要求格式完全一致，因为那时格式已经确定了。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 图像容器格式。
///
/// `Any` 只作为**静态类型**里的通配符出现（例如「还没选文件的输入节点」），
/// 运行时产出的值永远是 [`ImageFormat::CONCRETE`] 中的一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Any,
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Tiff,
    Ico,
    Qoi,
    Tga,
    Pnm,
}

impl ImageFormat {
    /// 可以解码的具体格式，顺序即编辑器里的展示顺序。
    pub const CONCRETE: [ImageFormat; 10] = [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Gif,
        ImageFormat::Webp,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::Ico,
        ImageFormat::Qoi,
        ImageFormat::Tga,
        ImageFormat::Pnm,
    ];

    /// 选格式时跟在选项后面的短句。
    pub fn note(self) -> Option<&'static str> {
        Some(match self {
            ImageFormat::Png => "无损，支持透明",
            ImageFormat::Jpeg => "有损，体积小，不支持透明",
            ImageFormat::Webp => "这里用无损编码",
            ImageFormat::Gif => "索引色，只取第一帧",
            ImageFormat::Bmp => "不压缩，体积大",
            ImageFormat::Tiff => "无损，体积大",
            _ => return None,
        })
    }

    /// 能编码出来的格式，也是「图像格式转换」节点下拉框里的选项。
    ///
    /// 比 [`ImageFormat::CONCRETE`] 短：ICO 是图标容器、TGA / PNM / QOI 偏门，
    /// 都能读进来，但不是一般人想转出去的目标，列在框里只是噪音。
    pub const ENCODABLE: [ImageFormat; 6] = [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Webp,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
    ];

    /// 序列化时用的名字，也是参数里存的值。
    /// 和 `#[serde(rename_all = "lowercase")]` 必须一致；
    /// `names_match_serde` 那个测试盯着这件事。
    pub fn name(self) -> &'static str {
        match self {
            ImageFormat::Any => "any",
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Gif => "gif",
            ImageFormat::Webp => "webp",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Ico => "ico",
            ImageFormat::Qoi => "qoi",
            ImageFormat::Tga => "tga",
            ImageFormat::Pnm => "pnm",
        }
    }

    /// 把参数里的字符串解回格式。认不出来返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "any" => ImageFormat::Any,
            "png" => ImageFormat::Png,
            "jpeg" => ImageFormat::Jpeg,
            "gif" => ImageFormat::Gif,
            "webp" => ImageFormat::Webp,
            "bmp" => ImageFormat::Bmp,
            "tiff" => ImageFormat::Tiff,
            "ico" => ImageFormat::Ico,
            "qoi" => ImageFormat::Qoi,
            "tga" => ImageFormat::Tga,
            "pnm" => ImageFormat::Pnm,
            _ => return None,
        })
    }

    pub fn is_concrete(self) -> bool {
        self != ImageFormat::Any
    }

    /// 端口徽标上的短标签。
    pub fn badge(self) -> &'static str {
        match self {
            ImageFormat::Any => "IMG",
            ImageFormat::Png => "PNG",
            ImageFormat::Jpeg => "JPG",
            ImageFormat::Gif => "GIF",
            ImageFormat::Webp => "WEBP",
            ImageFormat::Bmp => "BMP",
            ImageFormat::Tiff => "TIFF",
            ImageFormat::Ico => "ICO",
            ImageFormat::Qoi => "QOI",
            ImageFormat::Tga => "TGA",
            ImageFormat::Pnm => "PNM",
        }
    }

    /// 界面上完整一点的名字。
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Any => "图像",
            ImageFormat::Png => "PNG 图像",
            ImageFormat::Jpeg => "JPEG 图像",
            ImageFormat::Gif => "GIF 图像",
            ImageFormat::Webp => "WebP 图像",
            ImageFormat::Bmp => "BMP 图像",
            ImageFormat::Tiff => "TIFF 图像",
            ImageFormat::Ico => "ICO 图像",
            ImageFormat::Qoi => "QOI 图像",
            ImageFormat::Tga => "TGA 图像",
            ImageFormat::Pnm => "PNM 图像",
        }
    }

    /// 写盘时使用的扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Any | ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Gif => "gif",
            ImageFormat::Webp => "webp",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Ico => "ico",
            ImageFormat::Qoi => "qoi",
            ImageFormat::Tga => "tga",
            ImageFormat::Pnm => "pnm",
        }
    }

    /// 按文件扩展名猜测格式。
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        match ext.as_str() {
            "png" | "apng" => Some(ImageFormat::Png),
            "jpg" | "jpeg" | "jpe" | "jfif" => Some(ImageFormat::Jpeg),
            "gif" => Some(ImageFormat::Gif),
            "webp" => Some(ImageFormat::Webp),
            "bmp" | "dib" => Some(ImageFormat::Bmp),
            "tif" | "tiff" => Some(ImageFormat::Tiff),
            "ico" => Some(ImageFormat::Ico),
            "qoi" => Some(ImageFormat::Qoi),
            "tga" | "icb" | "vda" | "vst" => Some(ImageFormat::Tga),
            "pnm" | "pbm" | "pgm" | "ppm" | "pam" => Some(ImageFormat::Pnm),
            _ => None,
        }
    }

    /// 按路径的扩展名猜测格式。
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        path.as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }

    pub fn from_image_format(format: image::ImageFormat) -> Option<Self> {
        match format {
            image::ImageFormat::Png => Some(ImageFormat::Png),
            image::ImageFormat::Jpeg => Some(ImageFormat::Jpeg),
            image::ImageFormat::Gif => Some(ImageFormat::Gif),
            image::ImageFormat::WebP => Some(ImageFormat::Webp),
            image::ImageFormat::Bmp => Some(ImageFormat::Bmp),
            image::ImageFormat::Tiff => Some(ImageFormat::Tiff),
            image::ImageFormat::Ico => Some(ImageFormat::Ico),
            image::ImageFormat::Qoi => Some(ImageFormat::Qoi),
            image::ImageFormat::Tga => Some(ImageFormat::Tga),
            image::ImageFormat::Pnm => Some(ImageFormat::Pnm),
            _ => None,
        }
    }

    pub fn to_image_format(self) -> Option<image::ImageFormat> {
        match self {
            ImageFormat::Any => None,
            ImageFormat::Png => Some(image::ImageFormat::Png),
            ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
            ImageFormat::Gif => Some(image::ImageFormat::Gif),
            ImageFormat::Webp => Some(image::ImageFormat::WebP),
            ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
            ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
            ImageFormat::Ico => Some(image::ImageFormat::Ico),
            ImageFormat::Qoi => Some(image::ImageFormat::Qoi),
            ImageFormat::Tga => Some(image::ImageFormat::Tga),
            ImageFormat::Pnm => Some(image::ImageFormat::Pnm),
        }
    }

    /// 所有支持的文件扩展名，供文件对话框过滤。
    pub fn all_extensions() -> Vec<String> {
        Self::CONCRETE
            .iter()
            .map(|f| f.extension().to_string())
            .collect()
    }
}

/// 一个端口的类型 —— 定义在 [`crate::types`]，这里再导出一遍，
/// 让原来 `model::port_type::PortType` 的引用路径继续可用。
///
/// 具体有哪些接口、哪些类型，以及「谁能接到谁上」，都归 [`crate::types`] 管；
/// 这个模块只剩下图像格式 [`ImageFormat`]。
pub use crate::types::PortType;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_serde() {
        // 参数里存的是 name()，落盘和发给前端用的是 serde 的名字，
        // 两者一旦不一致，参数解析就会静默地回退到默认值。
        for format in ImageFormat::CONCRETE.into_iter().chain([ImageFormat::Any]) {
            assert_eq!(
                serde_json::to_string(&format).unwrap(),
                format!("\"{}\"", format.name()),
                "{:?} 的 name() 和 serde 名字对不上",
                format
            );
            assert_eq!(ImageFormat::from_name(format.name()), Some(format));
        }
        assert_eq!(ImageFormat::from_name("jpg"), None, "只认 name()，不认别名");
        assert_eq!(ImageFormat::from_name(""), None);
    }

    #[test]
    fn extensions_round_trip() {
        for f in ImageFormat::CONCRETE {
            assert_eq!(ImageFormat::from_extension(f.extension()), Some(f));
        }
        assert_eq!(ImageFormat::from_extension(".JPG"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("txt"), None);
    }
}
