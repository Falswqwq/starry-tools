//! PNG 无损优化。
//!
//! 目标只有一个：**在像素一个都不变的前提下把文件弄到最小**。为此把手脚都动一遍，
//! 最后只留真的最小的那一个：
//!
//! * **颜色类型 / 位深** —— 分析实际用到的颜色与 Alpha：没有透明就不用 RGBA，
//!   是灰度就存灰度，颜色少到装得进调色板就转索引色，并按颜色数挑 1 / 2 / 4 / 8 位；
//! * **逐行 filter** —— 交给 `png` 的 `MinEntropy` / `Adaptive` 逐行挑，不同的行
//!   可以用不同的 filter，压完再比大小；
//! * **DEFLATE** —— 快速 / 均衡 / 极限三档，最后还能再叠一个 zopfli；
//! * **元数据** —— 默认只留影响颜色显示的块（iCCP / sRGB / gAMA / cHRM），
//!   要更干净就整个剥掉；
//! * **不划算就不做** —— 每个候选都真的编出来比大小，比原图大就原样返回原图。
//!
//! 收尾一定逐像素验一遍：把结果解回 RGBA，和原图逐字节比。对不上就当作这次优化
//! 没发生 —— 宁可没优化，也不能悄悄改了图。
//!
//! 这里只做**无损**。调色板量化那种有损的活在 `png_quant` 里，两条路互不掺和。

use std::collections::HashMap;
use std::io::Read;
use std::num::NonZeroU64;

use image::{ColorType as ImageColorType, DynamicImage, GenericImageView};
// 不带前缀的 `ColorType` 一律指 PNG 的颜色类型 —— 这份代码里要写的都是它。
// 判断「源图是不是 16 位」那种场合才用 `ImageColorType`（image crate 的那套）。
use png::ColorType;

use crate::error::NodeError;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// 大到这个像素数就不再「多方案都试一遍」了 —— 内存和 CPU 都吃不消，
/// 退回到最稳的那一种编码。像素画远够不着这条线。
const EXHAUSTIVE_PIXEL_CAP: u64 = 16_000_000;

/// 输入大到这个字节数时，zopfli 少磨几轮，免得一次优化要等上好几分钟。
const ZOPFLI_PATIENCE_BYTES: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// 方案
// ---------------------------------------------------------------------------

/// 压缩方案：越往后越小、越慢。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// 图快，压得轻。
    Fast,
    /// 日常用这个。
    Balanced,
    /// 慢慢磨，尽量小。
    Maximum,
    /// 极限：在「极限」的基础上再拿 zopfli 重压一遍 IDAT。
    Zopfli,
}

impl Scheme {
    pub fn parse(value: &str) -> Self {
        match value {
            "fast" => Scheme::Fast,
            "maximum" => Scheme::Maximum,
            "zopfli" => Scheme::Zopfli,
            _ => Scheme::Balanced,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scheme::Fast => "fast",
            Scheme::Balanced => "balanced",
            Scheme::Maximum => "maximum",
            Scheme::Zopfli => "zopfli",
        }
    }

    /// DEFLATE 档位。
    fn compression(self) -> png::Compression {
        match self {
            // 走 fdeflate，快得多，压得也不差。
            Scheme::Fast => png::Compression::Fast,
            Scheme::Balanced => png::Compression::Balanced,
            // zopfli 那档也先用 flate2 的最高档比出最优候选，再交给 zopfli 重压。
            Scheme::Maximum | Scheme::Zopfli => png::Compression::High,
        }
    }

    /// 逐行 filter 的候选。都会真的编出来比大小。
    fn filters(self) -> &'static [png::Filter] {
        match self {
            Scheme::Fast => &[png::Filter::Adaptive],
            Scheme::Balanced => &[png::Filter::MinEntropy, png::Filter::Adaptive],
            Scheme::Maximum | Scheme::Zopfli => &[
                png::Filter::MinEntropy,
                png::Filter::Adaptive,
                png::Filter::Paeth,
            ],
        }
    }

    /// 用不用 zopfli 重压。
    fn uses_zopfli(self) -> bool {
        matches!(self, Scheme::Zopfli)
    }
}

/// 元数据清理力度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strip {
    /// 只留影响颜色怎么显示的块，文本 / 时间 / EXIF 之类都去掉。
    Safe,
    /// 连颜色块一起去掉 —— 给游戏素材用的「榨干」模式。
    All,
}

impl Strip {
    pub fn parse(value: &str) -> Self {
        if value == "all" {
            Strip::All
        } else {
            Strip::Safe
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Strip::Safe => "safe",
            Strip::All => "all",
        }
    }

    /// 这个辅助块能不能原样搬过去。
    ///
    /// 只搬颜色相关的：像素值一个都没改，颜色空间的描述当然还是对的。
    /// 别的块（bKGD / hIST / sBIT 这些）和颜色类型绑死，换过类型就不敢搬了。
    fn keeps(self, kind: &[u8; 4]) -> bool {
        match self {
            Strip::All => false,
            Strip::Safe => matches!(kind, b"iCCP" | b"sRGB" | b"gAMA" | b"cHRM"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub scheme: Scheme,
    pub strip: Strip,
    /// 逐像素校验。默认开着 —— 这是「无损」这个承诺的最后一道保险。
    pub verify: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            scheme: Scheme::Balanced,
            strip: Strip::Safe,
            verify: true,
        }
    }
}

impl Options {
    pub fn new(scheme: Scheme, strip: Strip) -> Self {
        Self {
            scheme,
            strip,
            verify: true,
        }
    }
}

/// 优化结果。
#[derive(Debug, Clone)]
pub struct Optimized {
    /// 最终写出去的字节。
    pub bytes: Vec<u8>,
    /// 原 PNG 的字节数；输入本来就不是 PNG 时是 `None`。
    pub before: Option<usize>,
    /// 选中了哪套方案，写进运行记录用。
    pub strategy: String,
    /// 是不是真的比原 PNG 小。原图更小时会把原图原样吐回来。
    pub improved: bool,
    /// 逐像素校验过了没。
    pub verified: bool,
}

impl Optimized {
    pub fn after(&self) -> usize {
        self.bytes.len()
    }

    /// 省下来的比例（0.0—1.0）。没有可比的原图时是 `None`。
    pub fn saved_ratio(&self) -> Option<f64> {
        let before = self.before?;
        if before == 0 {
            return None;
        }
        Some(1.0 - self.after() as f64 / before as f64)
    }
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

/// 把一张图编成尽可能小的无损 PNG。
///
/// `source_png` 是输入本来就是 PNG 时的原始字节：只用来搬运颜色相关的元数据块，
/// 以及和原始体积作对比。输入不是 PNG 就传 `None`。
pub fn optimize(
    image: &DynamicImage,
    source_png: Option<&[u8]>,
    options: &Options,
) -> Result<Optimized, NodeError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(NodeError::new("图像是空的，没法编码"));
    }

    let before = source_png.map(<[u8]>::len);
    let analysis = Analysis::of(image);
    let mut specs = analysis.specs();
    if u64::from(image.width()) * u64::from(image.height()) > EXHAUSTIVE_PIXEL_CAP {
        // 太大就别穷举了，留一个最稳的。
        specs.truncate(1);
    }

    let filters = options.scheme.filters();
    let compression = options.scheme.compression();

    // 每个颜色方案 × 每种 filter 都真的编一遍，留最小的那个。
    let mut best: Option<(Vec<u8>, String)> = None;
    for spec in &specs {
        let candidate = analysis.materialize(spec);
        for filter in filters {
            let Ok(bytes) = encode(&candidate, *filter, compression) else {
                continue;
            };
            let better = best
                .as_ref()
                .is_none_or(|(current, _)| bytes.len() < current.len());
            if better {
                let label = if filters.len() > 1 {
                    format!("{} · {}", candidate.label(), filter_name(*filter))
                } else {
                    candidate.label()
                };
                best = Some((bytes, label));
            }
        }
    }
    let Some((mut best, mut strategy)) = best else {
        return Err(NodeError::new("没有可用的 PNG 编码方案"));
    };

    // 极限档：把 IDAT 整条解出来交给 zopfli 重压。
    if options.scheme.uses_zopfli() {
        if let Ok(packed) = recompress_with_zopfli(&best) {
            if packed.len() < best.len() {
                best = packed;
                strategy = format!("{strategy} + zopfli");
            }
        }
    }

    // 搬元数据。
    best = splice_metadata(&best, source_png, options.strip);

    // 最后一道保险：逐像素比对。
    let verified = verify(image, &best);
    if options.verify && !verified {
        return Err(NodeError::new("优化后的像素和原图对不上，这次优化已放弃"));
    }

    // 比原图大就别折腾了，原样返回。
    if let (Some(before), Some(original)) = (before, source_png) {
        if best.len() >= before {
            return Ok(Optimized {
                bytes: original.to_vec(),
                before: Some(before),
                strategy: "原样保留（优化后反而更大）".into(),
                improved: false,
                verified: true,
            });
        }
    }

    Ok(Optimized {
        before,
        strategy,
        improved: true,
        verified,
        bytes: best,
    })
}

fn filter_name(filter: png::Filter) -> &'static str {
    match filter {
        png::Filter::NoFilter => "不滤波",
        png::Filter::Sub => "Sub",
        png::Filter::Up => "Up",
        png::Filter::Avg => "Avg",
        png::Filter::Paeth => "Paeth",
        png::Filter::Adaptive => "逐行自适应",
        png::Filter::MinEntropy => "逐行最小熵",
        // `png::Filter` 是 non_exhaustive 的，将来加了新花样也不至于漏掉。
        _ => "其它滤波",
    }
}

// ---------------------------------------------------------------------------
// 分析：这张图到底用到了什么
// ---------------------------------------------------------------------------

/// 调色板：颜色按字典序排好，顺带一张「颜色 → 下标」的表。
struct Palette {
    colors: Vec<[u8; 4]>,
    index_of: HashMap<[u8; 4], u8>,
}

struct Analysis {
    width: u32,
    height: u32,
    /// 8 位源：展开成 RGBA 的字节。
    rgba8: Option<Vec<u8>>,
    /// 16 位源：展开成 RGBA 的 u16。
    rgba16: Option<Vec<u16>>,
    has_alpha: bool,
    is_gray: bool,
    /// 灰度能用的最小位深（1 / 2 / 4 / 8）。不是「不透明的灰度图」就是 `None`。
    gray_depth: Option<u8>,
    /// 唯一色不超过 256 时才有。
    palette: Option<Palette>,
}

impl Analysis {
    fn of(image: &DynamicImage) -> Self {
        let (width, height) = image.dimensions();
        let wide = matches!(
            image.color(),
            ImageColorType::L16
                | ImageColorType::La16
                | ImageColorType::Rgb16
                | ImageColorType::Rgba16
        );
        if wide {
            Self::wide(image, width, height)
        } else {
            Self::narrow(image, width, height)
        }
    }

    fn narrow(image: &DynamicImage, width: u32, height: u32) -> Self {
        let raw = image.to_rgba8().into_raw();

        let mut has_alpha = false;
        let mut is_gray = true;
        let mut unique: Vec<[u8; 4]> = Vec::new();
        let mut seen: HashMap<[u8; 4], ()> = HashMap::new();
        let mut too_many = false;

        for pixel in raw.chunks_exact(4) {
            let color = [pixel[0], pixel[1], pixel[2], pixel[3]];
            if color[3] != 255 {
                has_alpha = true;
            }
            if color[0] != color[1] || color[1] != color[2] {
                is_gray = false;
            }
            if !too_many && seen.insert(color, ()).is_none() {
                unique.push(color);
                if unique.len() > 256 {
                    too_many = true;
                }
            }
        }

        let palette = if too_many {
            None
        } else {
            unique.sort_unstable();
            let index_of = unique
                .iter()
                .enumerate()
                .map(|(index, color)| (*color, index as u8))
                .collect();
            Some(Palette {
                colors: unique,
                index_of,
            })
        };

        // 灰度位深只在「不透明 + 灰度」时才有意义：
        // PNG 的「灰度 + Alpha」只允许 8 / 16 位，压不到 1/2/4。
        let gray_depth = (is_gray && !has_alpha).then(|| {
            let values: Vec<u8> = match &palette {
                Some(palette) => palette.colors.iter().map(|color| color[0]).collect(),
                None => raw.chunks_exact(4).map(|pixel| pixel[0]).collect(),
            };
            gray_bit_depth(&values)
        });

        Self {
            width,
            height,
            rgba8: Some(raw),
            rgba16: None,
            has_alpha,
            is_gray,
            gray_depth,
            palette,
        }
    }

    fn wide(image: &DynamicImage, width: u32, height: u32) -> Self {
        let raw = image.to_rgba16().into_raw();

        let mut has_alpha = false;
        let mut is_gray = true;
        for pixel in raw.chunks_exact(4) {
            if pixel[3] != u16::MAX {
                has_alpha = true;
            }
            if pixel[0] != pixel[1] || pixel[1] != pixel[2] {
                is_gray = false;
            }
        }

        Self {
            width,
            height,
            rgba8: None,
            rgba16: Some(raw),
            has_alpha,
            is_gray,
            // 16 位的调色板不是一回事（PNG 的调色板永远是 8 位），不趟这浑水。
            gray_depth: None,
            palette: None,
        }
    }

    /// 有哪些值得试的颜色类型 / 位深组合。
    fn specs(&self) -> Vec<Spec> {
        let mut specs = Vec::new();

        if self.rgba16.is_some() {
            if self.has_alpha {
                specs.push(Spec::new(
                    "RGBA 16 位",
                    ColorType::Rgba,
                    png::BitDepth::Sixteen,
                    Shape::Rgba16,
                ));
                if self.is_gray {
                    specs.push(Spec::new(
                        "灰度 + 透明 16 位",
                        ColorType::GrayscaleAlpha,
                        png::BitDepth::Sixteen,
                        Shape::LumaAlpha16,
                    ));
                }
            } else {
                specs.push(Spec::new(
                    "RGB 16 位",
                    ColorType::Rgb,
                    png::BitDepth::Sixteen,
                    Shape::Rgb16,
                ));
                if self.is_gray {
                    specs.push(Spec::new(
                        "灰度 16 位",
                        ColorType::Grayscale,
                        png::BitDepth::Sixteen,
                        Shape::Luma16,
                    ));
                }
            }
            return specs;
        }

        if self.has_alpha {
            specs.push(Spec::new(
                "RGBA 8 位",
                ColorType::Rgba,
                png::BitDepth::Eight,
                Shape::Rgba,
            ));
            if self.is_gray {
                specs.push(Spec::new(
                    "灰度 + 透明 8 位",
                    ColorType::GrayscaleAlpha,
                    png::BitDepth::Eight,
                    Shape::LumaAlpha,
                ));
            }
        } else {
            specs.push(Spec::new(
                "RGB 8 位",
                ColorType::Rgb,
                png::BitDepth::Eight,
                Shape::Rgb,
            ));
            if self.is_gray {
                let bits = self.gray_depth.unwrap_or(8);
                if bits < 8 {
                    specs.push(Spec::new(
                        &format!("灰度 {bits} 位"),
                        ColorType::Grayscale,
                        bit_depth(bits),
                        Shape::GrayBits(bits),
                    ));
                } else {
                    specs.push(Spec::new(
                        "灰度 8 位",
                        ColorType::Grayscale,
                        png::BitDepth::Eight,
                        Shape::Luma,
                    ));
                }
            }
        }

        // 索引色：按实际颜色数挑装得下的最小位深。
        if let Some(palette) = &self.palette {
            let bits = min_index_bits(palette.colors.len());
            let mut trns: Vec<u8> = palette.colors.iter().map(|color| color[3]).collect();
            // 末尾那几个不透明的（255）不用写：tRNS 没写到的就是全不透明。
            while trns.last() == Some(&255) {
                trns.pop();
            }
            specs.push(Spec {
                label: format!("索引色 {} 位（{} 色）", bits, palette.colors.len()),
                color: ColorType::Indexed,
                depth: bit_depth(bits),
                shape: Shape::Indexed(bits),
                palette: Some(
                    palette
                        .colors
                        .iter()
                        .flat_map(|color| [color[0], color[1], color[2]])
                        .collect(),
                ),
                trns: (!trns.is_empty()).then_some(trns),
            });
        }

        specs
    }

    /// 把一个方案真的编出来。
    fn materialize(&self, spec: &Spec) -> Candidate {
        let data = match spec.shape {
            Shape::Rgba => self.rgba8.clone().unwrap_or_default(),
            Shape::Rgb => self
                .rgba8
                .as_ref()
                .map(|raw| {
                    raw.chunks_exact(4)
                        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                        .collect()
                })
                .unwrap_or_default(),
            Shape::Luma => self
                .rgba8
                .as_ref()
                .map(|raw| raw.chunks_exact(4).map(|pixel| pixel[0]).collect())
                .unwrap_or_default(),
            Shape::LumaAlpha => self
                .rgba8
                .as_ref()
                .map(|raw| {
                    raw.chunks_exact(4)
                        .flat_map(|pixel| [pixel[0], pixel[3]])
                        .collect()
                })
                .unwrap_or_default(),
            Shape::GrayBits(bits) => {
                let values: Vec<u8> = self
                    .rgba8
                    .as_ref()
                    .map(|raw| {
                        raw.chunks_exact(4)
                            .map(|pixel| scale_gray(pixel[0], bits))
                            .collect()
                    })
                    .unwrap_or_default();
                pack_rows(&values, self.width, self.height, bits)
            }
            Shape::Indexed(bits) => {
                let palette = self.palette.as_ref();
                let values: Vec<u8> = self
                    .rgba8
                    .as_ref()
                    .map(|raw| {
                        raw.chunks_exact(4)
                            .map(|pixel| {
                                let color = [pixel[0], pixel[1], pixel[2], pixel[3]];
                                palette
                                    .and_then(|palette| palette.index_of.get(&color).copied())
                                    .unwrap_or(0)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                pack_rows(&values, self.width, self.height, bits)
            }
            Shape::Rgba16 => self
                .rgba16
                .as_ref()
                .map(|raw| raw.iter().flat_map(|value| value.to_be_bytes()).collect())
                .unwrap_or_default(),
            Shape::Rgb16 => self
                .rgba16
                .as_ref()
                .map(|raw| {
                    raw.chunks_exact(4)
                        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                        .flat_map(u16::to_be_bytes)
                        .collect()
                })
                .unwrap_or_default(),
            Shape::Luma16 => self
                .rgba16
                .as_ref()
                .map(|raw| {
                    raw.chunks_exact(4)
                        .map(|pixel| pixel[0])
                        .flat_map(u16::to_be_bytes)
                        .collect()
                })
                .unwrap_or_default(),
            Shape::LumaAlpha16 => self
                .rgba16
                .as_ref()
                .map(|raw| {
                    raw.chunks_exact(4)
                        .flat_map(|pixel| [pixel[0], pixel[3]])
                        .flat_map(u16::to_be_bytes)
                        .collect()
                })
                .unwrap_or_default(),
        };

        Candidate {
            label: spec.label.clone(),
            color: spec.color,
            depth: spec.depth,
            palette: spec.palette.clone(),
            trns: spec.trns.clone(),
            width: self.width,
            height: self.height,
            data,
        }
    }
}

/// 颜色数到「装得下它的最小索引位深」。
fn min_index_bits(count: usize) -> u8 {
    match count {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

/// 灰度值能不能用更小的位深表示：1 位是两极，2 位是 0/85/170/255，4 位是 17 的倍数。
fn gray_bit_depth(values: &[u8]) -> u8 {
    if values.iter().all(|value| *value == 0 || *value == 255) {
        return 1;
    }
    if values.iter().all(|value| value % 85 == 0) {
        return 2;
    }
    if values.iter().all(|value| value % 17 == 0) {
        return 4;
    }
    8
}

/// 把 8 位灰度值缩到 `bits` 位能表示的那个值。调用前已经确认能整除。
fn scale_gray(value: u8, bits: u8) -> u8 {
    match bits {
        1 => value / 255,
        2 => value / 85,
        4 => value / 17,
        _ => value,
    }
}

/// 把每行的值按位深紧密排到字节里（PNG 的亚字节位深就是这么存的，高位在前），
/// 每行末尾补到整字节。
fn pack_rows(values: &[u8], width: u32, height: u32, bits: u8) -> Vec<u8> {
    if bits >= 8 {
        return values.to_vec();
    }
    let width = width as usize;
    let mask = (1u16 << bits) - 1;
    let row_bytes = (width * bits as usize).div_ceil(8);
    let mut out = vec![0u8; row_bytes * height as usize];

    for y in 0..height as usize {
        for x in 0..width {
            let value = u16::from(values[y * width + x]) & mask;
            let bit_index = x * bits as usize;
            let shift = 8 - bits as usize - (bit_index % 8);
            out[y * row_bytes + bit_index / 8] |= (value << shift) as u8;
        }
    }

    out
}

fn bit_depth(bits: u8) -> png::BitDepth {
    match bits {
        1 => png::BitDepth::One,
        2 => png::BitDepth::Two,
        4 => png::BitDepth::Four,
        _ => png::BitDepth::Eight,
    }
}

/// 一个候选的颜色类型 / 位深组合（还不含数据）。
#[derive(Clone)]
struct Spec {
    label: String,
    color: ColorType,
    depth: png::BitDepth,
    shape: Shape,
    palette: Option<Vec<u8>>,
    trns: Option<Vec<u8>>,
}

impl Spec {
    fn new(label: &str, color: ColorType, depth: png::BitDepth, shape: Shape) -> Self {
        Self {
            label: label.to_string(),
            color,
            depth,
            shape,
            palette: None,
            trns: None,
        }
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Rgba,
    Rgb,
    Luma,
    LumaAlpha,
    GrayBits(u8),
    Indexed(u8),
    Rgba16,
    Rgb16,
    Luma16,
    LumaAlpha16,
}

/// 真正交给 `png` 去写的东西。
struct Candidate {
    label: String,
    color: ColorType,
    depth: png::BitDepth,
    palette: Option<Vec<u8>>,
    trns: Option<Vec<u8>>,
    width: u32,
    height: u32,
    /// 原始扫描线数据，**不含**每行开头那个 filter 字节 —— 那个由编码器自己挑。
    data: Vec<u8>,
}

impl Candidate {
    fn label(&self) -> String {
        self.label.clone()
    }
}

// ---------------------------------------------------------------------------
// 编码
// ---------------------------------------------------------------------------

fn encode(
    candidate: &Candidate,
    filter: png::Filter,
    compression: png::Compression,
) -> Result<Vec<u8>, NodeError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, candidate.width, candidate.height);
        encoder.set_color(candidate.color);
        encoder.set_depth(candidate.depth);
        if let Some(palette) = &candidate.palette {
            encoder.set_palette(palette.clone());
        }
        if let Some(trns) = &candidate.trns {
            encoder.set_trns(trns.clone());
        }
        encoder.set_filter(filter);
        encoder.set_compression(compression);
        let mut writer = encoder
            .write_header()
            .map_err(|err| NodeError::new(format!("PNG 头写不出去：{err}")))?;
        writer
            .write_image_data(&candidate.data)
            .map_err(|err| NodeError::new(format!("PNG 数据写不出去：{err}")))?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 结构：读块 / 写块 / 搬元数据
// ---------------------------------------------------------------------------

type Chunk = ([u8; 4], Vec<u8>);

/// 把 PNG 拆成块。签名不对或者长度对不上就返回 `None`。
fn read_chunks(png: &[u8]) -> Option<Vec<Chunk>> {
    if png.len() < 8 || png[..8] != SIGNATURE {
        return None;
    }
    let mut chunks = Vec::new();
    let mut position = 8;
    while position + 12 <= png.len() {
        let length = u32::from_be_bytes(png[position..position + 4].try_into().ok()?) as usize;
        let kind: [u8; 4] = png[position + 4..position + 8].try_into().ok()?;
        if position + 12 + length > png.len() {
            return None;
        }
        let data = png[position + 8..position + 8 + length].to_vec();
        let is_end = &kind == b"IEND";
        chunks.push((kind, data));
        position += 12 + length;
        if is_end {
            break;
        }
    }
    Some(chunks)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
}

/// 把原图里该留的辅助块搬到新图上，位置按惯例摆在 IHDR 后面。
fn splice_metadata(optimized: &[u8], source: Option<&[u8]>, strip: Strip) -> Vec<u8> {
    let Some(source) = source else {
        return optimized.to_vec();
    };
    let (Some(source_chunks), Some(own_chunks)) = (read_chunks(source), read_chunks(optimized))
    else {
        return optimized.to_vec();
    };

    let kept: Vec<&Chunk> = source_chunks
        .iter()
        .filter(|(kind, _)| strip.keeps(kind))
        .collect();
    if kept.is_empty() {
        return optimized.to_vec();
    }

    let mut out = Vec::with_capacity(optimized.len() + 128);
    out.extend_from_slice(&SIGNATURE);
    for (kind, data) in &own_chunks {
        write_chunk(&mut out, kind, data);
        if kind == b"IHDR" {
            for (kind, data) in &kept {
                write_chunk(&mut out, kind, data);
            }
        }
    }
    out
}

/// 把 IDAT 解出来交给 zopfli 重压。
///
/// 只换 DEFLATE 的表示，filter 过的那串原始数据一个字节都没动 —— 像素不可能变。
fn recompress_with_zopfli(png: &[u8]) -> Result<Vec<u8>, NodeError> {
    let chunks = read_chunks(png).ok_or_else(|| NodeError::new("PNG 结构读不出来"))?;

    let mut idat = Vec::new();
    for (kind, data) in &chunks {
        if kind == b"IDAT" {
            idat.extend_from_slice(data);
        }
    }
    if idat.is_empty() {
        return Err(NodeError::new("这张 PNG 里没有 IDAT"));
    }

    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(idat.as_slice())
        .read_to_end(&mut raw)
        .map_err(|err| NodeError::new(format!("解开 IDAT 失败：{err}")))?;

    let mut options = zopfli::Options::default();
    if raw.len() > ZOPFLI_PATIENCE_BYTES {
        // 大图就少磨几轮，不然一次优化能等上几分钟。
        options.iteration_count = NonZeroU64::new(5).expect("5 不是零");
        options.maximum_block_splits = 5;
    }

    let mut compressed = Vec::new();
    zopfli::compress(
        options,
        zopfli::Format::Zlib,
        raw.as_slice(),
        &mut compressed,
    )
    .map_err(|err| NodeError::new(format!("zopfli 压缩失败：{err}")))?;

    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&SIGNATURE);
    let mut wrote_idat = false;
    for (kind, data) in &chunks {
        if kind == b"IDAT" {
            if !wrote_idat {
                write_chunk(&mut out, b"IDAT", &compressed);
                wrote_idat = true;
            }
            continue;
        }
        if !wrote_idat && kind == b"IEND" {
            write_chunk(&mut out, b"IDAT", &compressed);
            wrote_idat = true;
        }
        write_chunk(&mut out, kind, data);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 校验
// ---------------------------------------------------------------------------

/// 把优化后的 PNG 解回来，和原图逐像素比。
/// 宽、高、每个像素的 R / G / B / A 都要一模一样才算过。
pub fn verify(original: &DynamicImage, optimized_png: &[u8]) -> bool {
    let Ok(decoded) = image::load_from_memory_with_format(optimized_png, image::ImageFormat::Png)
    else {
        return false;
    };
    if decoded.dimensions() != original.dimensions() {
        return false;
    }

    if is_wide(original.color()) {
        original.to_rgba16().as_raw() == decoded.to_rgba16().as_raw()
    } else {
        original.to_rgba8().as_raw() == decoded.to_rgba8().as_raw()
    }
}

fn is_wide(color: ImageColorType) -> bool {
    matches!(
        color,
        ImageColorType::L16 | ImageColorType::La16 | ImageColorType::Rgb16 | ImageColorType::Rgba16
    )
}

#[cfg(test)]
mod tests;
