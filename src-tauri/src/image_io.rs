//! 图像数据的读写与编码。
//!
//! [`ImageValue`] 是管线里流动的图像值。它以**编码后的字节**为准，像素只在
//! 真正需要时解码一次并缓存起来。这样做有两个好处：
//!
//! * 节点之间传递图像很便宜（只复制一个 `Arc`），输入节点选完文件后完全
//!   不用解码，直到有节点真的要看像素；
//! * 「转成 PNG」这类节点是货真价实地在编码，而不是把标签改一改。

use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{
    CompressionType as PngCompressionType, FilterType as PngFilterType, PngEncoder,
};
use image::{DynamicImage, ImageReader};
use serde::Serialize;

use crate::error::{AppError, NodeError};
use crate::model::port_type::ImageFormat;

/// PNG 压缩档位，对应「图像格式转换」节点上的下拉框。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngCompression {
    Fast,
    Default,
    Best,
}

impl PngCompression {
    pub fn parse(value: &str) -> Self {
        match value {
            "fast" => PngCompression::Fast,
            "best" => PngCompression::Best,
            _ => PngCompression::Default,
        }
    }

    fn to_image(self) -> PngCompressionType {
        match self {
            PngCompression::Fast => PngCompressionType::Fast,
            PngCompression::Default => PngCompressionType::Default,
            PngCompression::Best => PngCompressionType::Best,
        }
    }
}

/// 编码图像时用到的设置。
#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    pub png_compression: PngCompression,
    pub jpeg_quality: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            png_compression: PngCompression::Default,
            jpeg_quality: 90,
        }
    }
}

/// 管线里流动的图像值。
#[derive(Clone)]
pub struct ImageValue {
    format: ImageFormat,
    origin: Option<PathBuf>,
    bytes: Arc<Vec<u8>>,
    decoded: Arc<OnceLock<Arc<DynamicImage>>>,
}

impl ImageValue {
    pub fn from_bytes(format: ImageFormat, bytes: Vec<u8>) -> Self {
        Self {
            format,
            origin: None,
            bytes: Arc::new(bytes),
            decoded: Arc::new(OnceLock::new()),
        }
    }

    /// 从磁盘读入。格式优先按文件头判断，其次才看扩展名 —— 扩展名经常是错的。
    pub fn open(path: &Path) -> Result<Self, NodeError> {
        let bytes = std::fs::read(path)
            .map_err(|err| NodeError::new(format!("无法读取文件 {}：{err}", path.display())))?;
        let format = guess_format(&bytes)
            .or_else(|| ImageFormat::from_path(path))
            .unwrap_or(ImageFormat::Any);
        Ok(Self {
            format,
            origin: Some(path.to_path_buf()),
            bytes: Arc::new(bytes),
            decoded: Arc::new(OnceLock::new()),
        })
    }

    /// 把已经在内存里的像素编码成 `format`，得到一个新的图像值。
    ///
    /// 用 `Arc` 收下像素，是为了让调用方在「原样透传」时不必真的复制一遍大图。
    /// 编码完顺手把它塞进解码缓存，后面就不用再解一遍了。
    pub fn from_image(
        format: ImageFormat,
        image: Arc<DynamicImage>,
        options: EncodeOptions,
    ) -> Result<Self, NodeError> {
        let bytes = encode_image(format, &image, options)?;
        let cache = OnceLock::new();
        let _ = cache.set(image);
        Ok(Self {
            format,
            origin: None,
            bytes: Arc::new(bytes),
            decoded: Arc::new(cache),
        })
    }

    pub fn format(&self) -> ImageFormat {
        self.format
    }

    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    pub fn with_origin(mut self, origin: Option<PathBuf>) -> Self {
        self.origin = origin;
        self
    }

    /// 把另一个图像值的「出处」搬过来：原始文件。
    ///
    /// 转换类节点重新编码之后要把这带上，否则下游的「保存到目录」就不知道
    /// 这张图是从哪来的，只能起个默认名。
    pub fn inherit_provenance(mut self, source: &ImageValue) -> Self {
        self.origin = source.origin.clone();
        self
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// 取出像素，必要时解码一次并缓存。
    pub fn decode(&self) -> Result<Arc<DynamicImage>, NodeError> {
        if let Some(cached) = self.decoded.get() {
            return Ok(cached.clone());
        }
        let reader = ImageReader::new(Cursor::new(self.bytes.as_slice()))
            .with_guessed_format()
            .map_err(|err| NodeError::new(format!("无法识别图像格式：{err}")))?;
        let image = reader
            .decode()
            .map_err(|err| NodeError::new(format!("无法解码图像：{err}")))?;
        let image = Arc::new(image);
        let _ = self.decoded.set(image.clone());
        Ok(image)
    }

    /// 只要尺寸的话不必解完整张图，读个头就够了。
    pub fn dimensions(&self) -> Result<(u32, u32), NodeError> {
        if let Some(cached) = self.decoded.get() {
            return Ok((cached.width(), cached.height()));
        }
        let reader = ImageReader::new(Cursor::new(self.bytes.as_slice()))
            .with_guessed_format()
            .map_err(|err| NodeError::new(format!("无法识别图像格式：{err}")))?;
        reader
            .into_dimensions()
            .map_err(|err| NodeError::new(format!("无法读取图像尺寸：{err}")))
    }

    pub fn save_to(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.bytes.as_slice())?;
        Ok(())
    }

    /// 生成一张缩略图，作为 `data:` URL 交给前端直接显示。
    pub fn preview_data_url(&self, max_dim: u32) -> Result<String, NodeError> {
        let image = self.decode()?;
        let (width, height) = (image.width(), image.height());
        let longest = width.max(height);
        // 缩小预览一律用最近邻：像素画不会被糊掉，放大效果在预览里也看得清。
        let preview = if longest > max_dim && longest > 0 {
            let scale = max_dim as f64 / longest as f64;
            let w = ((width as f64 * scale).round() as u32).max(1);
            let h = ((height as f64 * scale).round() as u32).max(1);
            image.resize_exact(w, h, image::imageops::FilterType::Nearest)
        } else {
            (*image).clone()
        };
        let bytes = encode_image(
            ImageFormat::Png,
            &preview,
            EncodeOptions {
                png_compression: PngCompression::Fast,
                ..EncodeOptions::default()
            },
        )?;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

impl fmt::Debug for ImageValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageValue")
            .field("format", &self.format)
            .field("bytes", &self.bytes.len())
            .field("decoded", &self.decoded.get().is_some())
            .finish()
    }
}

/// 按文件头判断格式。认不出来就返回 `None`。
pub fn guess_format(bytes: &[u8]) -> Option<ImageFormat> {
    image::guess_format(bytes)
        .ok()
        .and_then(ImageFormat::from_image_format)
}

/// 只读文件开头的一小段来判断格式，不必把整张图读进内存。
pub fn guess_format_from_path(path: &Path) -> Option<ImageFormat> {
    use std::io::Read as _;

    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 512];
    let read = file.read(&mut header).ok()?;
    guess_format(&header[..read])
}

/// 把像素编码成指定容器格式。
pub fn encode_image(
    format: ImageFormat,
    image: &DynamicImage,
    options: EncodeOptions,
) -> Result<Vec<u8>, NodeError> {
    match format {
        ImageFormat::Png => {
            let mut buffer = Vec::new();
            let encoder = PngEncoder::new_with_quality(
                &mut buffer,
                options.png_compression.to_image(),
                PngFilterType::Adaptive,
            );
            image.write_with_encoder(encoder)?;
            Ok(buffer)
        }
        ImageFormat::Jpeg => {
            let mut buffer = Vec::new();
            let encoder = JpegEncoder::new_with_quality(&mut buffer, options.jpeg_quality);
            image.write_with_encoder(encoder)?;
            Ok(buffer)
        }
        other => {
            let target = other
                .to_image_format()
                .ok_or_else(|| NodeError::new("无法确定输出图像的格式"))?;
            let mut cursor = Cursor::new(Vec::new());
            image.write_to(&mut cursor, target)?;
            Ok(cursor.into_inner())
        }
    }
}

/// 图像是否包含真正的透明像素：有 alpha 通道，且不全是 255。
pub fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && !is_fully_opaque(image)
}

/// 所有像素都是不透明的。没有 alpha 通道的图像恒为真。
pub fn is_fully_opaque(image: &DynamicImage) -> bool {
    if !image.color().has_alpha() {
        return true;
    }
    image.to_rgba8().pixels().all(|pixel| pixel.0[3] == 255)
}

/// 把透明区域合成到白底上再去掉 alpha。
///
/// 直接 `to_rgb8()` 只是把 alpha 丢掉，透明像素底下那些值往往是黑的（或别的垃圾），
/// 转出来就是一圈黑边。合成白底至少结果是可预期的。
pub fn flatten_onto_white(image: &DynamicImage) -> image::RgbImage {
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());

    for (destination, source) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = u16::from(source.0[3]);
        let blend = |channel: u8| -> u8 {
            // 白底：out = 前景 * a + 255 * (1 - a)
            let foreground = u16::from(channel) * alpha;
            let background = 255 * (255 - alpha);
            ((foreground + background) / 255) as u8
        };
        *destination = image::Rgb([blend(source.0[0]), blend(source.0[1]), blend(source.0[2])]);
    }

    rgb
}

/// 供「输入」节点选择文件后展示的摘要信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub file_name: String,
    pub file_size: u64,
    pub preview: Option<String>,
}

impl ImageInfo {
    pub fn inspect(path: &Path, preview_max: u32) -> Result<Self, AppError> {
        let value = ImageValue::open(path).map_err(|err| AppError::msg(err.0))?;
        let (width, height) = value.dimensions().map_err(|err| AppError::msg(err.0))?;
        Ok(Self {
            format: value.format(),
            width,
            height,
            file_name: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
            file_size: value.byte_len() as u64,
            preview: value.preview_data_url(preview_max).ok(),
        })
    }
}
