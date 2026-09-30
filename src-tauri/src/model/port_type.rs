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

/// 一个端口的类型。
///
/// 序列化后长这样：`"any"`、`"text"`、`"number"`、`"bool"`、`{"image":"png"}`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortType {
    /// 通配：什么值都能接。编辑期一律放行，运行期再要求实际值是个具体类型。
    /// 「重命名」这类什么都不管的节点用它，将来加非图像的工具也靠它。
    Any,
    Text,
    Number,
    Bool,
    Image(ImageFormat),
}

impl PortType {
    /// 类型族，界面上用来决定配色分组。
    pub fn family(self) -> &'static str {
        match self {
            PortType::Any => "any",
            PortType::Text => "text",
            PortType::Number => "number",
            PortType::Bool => "bool",
            PortType::Image(_) => "image",
        }
    }

    /// 运行期真的拿到它时，这个类型算不算「说清楚了」。
    /// 通配本身不是具体类型，格式未知的图像也不是。
    pub fn is_concrete(self) -> bool {
        !matches!(self, PortType::Any | PortType::Image(ImageFormat::Any))
    }

    pub fn is_image(self) -> bool {
        matches!(self, PortType::Image(_))
    }

    pub fn image_format(self) -> Option<ImageFormat> {
        match self {
            PortType::Image(f) => Some(f),
            _ => None,
        }
    }

    /// 端口徽标：图像用格式短名，其余用中文字。
    pub fn badge(self) -> &'static str {
        match self {
            PortType::Any => "ANY",
            PortType::Text => "TXT",
            PortType::Number => "NUM",
            PortType::Bool => "BOOL",
            PortType::Image(f) => f.badge(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PortType::Any => "任意值",
            PortType::Text => "文本",
            PortType::Number => "数字",
            PortType::Bool => "布尔",
            PortType::Image(f) => f.label(),
        }
    }

    /// 编辑期检查：`self` 作为目标端口，能否接受 `source` 端口的数据。
    ///
    /// 比 [`PortType::strictly_accepts`] 宽松的地方：通配一头出现就放行，
    /// 格式未知的图像可以接到任何图像端口上 —— 都等运行期再确认。
    pub fn accepts(self, source: PortType) -> bool {
        use PortType::*;
        if self == Any || source == Any {
            return true;
        }
        match (self, source) {
            (Text, Text) | (Number, Number) | (Bool, Bool) => true,
            (Image(ImageFormat::Any), Image(_)) => true,
            (Image(target), Image(actual)) => target == actual || actual == ImageFormat::Any,
            _ => false,
        }
    }

    /// 运行期检查：端口声明 `self`，实际拿到的值是 `source`，是否严格匹配。
    pub fn strictly_accepts(self, source: PortType) -> bool {
        use PortType::*;
        match (self, source) {
            (Text, Text) | (Number, Number) | (Bool, Bool) => true,
            // 声明得宽，实际值就得是个说得清楚的类型。
            (Any, actual) => actual.is_concrete(),
            // 目标声明得很宽，任何具体图像都行，但「格式未知」不是具体格式。
            (Image(ImageFormat::Any), Image(actual)) => actual.is_concrete(),
            (Image(target), Image(actual)) => target.is_concrete() && target == actual,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_stable() {
        assert_eq!(serde_json::to_string(&PortType::Text).unwrap(), "\"text\"");
        assert_eq!(serde_json::to_string(&PortType::Any).unwrap(), "\"any\"");
        assert_eq!(
            serde_json::to_string(&PortType::Image(ImageFormat::Png)).unwrap(),
            "{\"image\":\"png\"}"
        );
        assert_eq!(
            serde_json::to_string(&PortType::Image(ImageFormat::Jpeg)).unwrap(),
            "{\"image\":\"jpeg\"}"
        );
    }

    #[test]
    fn any_image_only_flows_into_any_image() {
        let png = PortType::Image(ImageFormat::Png);
        let jpg = PortType::Image(ImageFormat::Jpeg);
        let any = PortType::Image(ImageFormat::Any);

        assert!(any.accepts(png));
        assert!(any.accepts(jpg));
        assert!(png.accepts(png));
        assert!(!png.accepts(jpg));
        assert!(!PortType::Text.accepts(png));
        assert!(!png.accepts(PortType::Text));

        // 编辑期放行「格式未知」，运行期不放行。
        assert!(png.accepts(any));
        assert!(!png.strictly_accepts(any));
        assert!(any.strictly_accepts(jpg));
        assert!(!any.strictly_accepts(any));
        assert!(png.strictly_accepts(png));
        assert!(!png.strictly_accepts(jpg));
    }

    #[test]
    fn wildcard_port_takes_anything() {
        let any = PortType::Any;
        let sources = [
            PortType::Text,
            PortType::Number,
            PortType::Bool,
            PortType::Image(ImageFormat::Png),
            PortType::Image(ImageFormat::Any),
        ];
        for source in sources {
            assert!(any.accepts(source), "编辑期通配什么都能接：{source:?}");
            assert!(
                source.accepts(any),
                "通配送出来的东西，编辑期也能接到任何端点上"
            );
        }

        // 运行期要的是一个说得清的实例。
        assert!(any.strictly_accepts(PortType::Text));
        assert!(any.strictly_accepts(PortType::Image(ImageFormat::Png)));
        assert!(!any.strictly_accepts(PortType::Any));
        assert!(!any.strictly_accepts(PortType::Image(ImageFormat::Any)));
    }

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
