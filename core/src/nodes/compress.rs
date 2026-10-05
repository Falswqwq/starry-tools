//! 「图像压缩」节点。
//!
//! 两条完全分开的路：
//!
//! * **无损** 走 [`crate::png_opt`] —— 颜色类型 / 位深 / 调色板 / 逐行 filter /
//!   DEFLATE / zopfli / 元数据全都过一遍，只留最小的那一份，出来必然还是逐像素
//!   一模一样的 PNG；
//! * **有损** 有两种：转 JPEG（照片），或者量化成调色板 PNG（像素画）。
//!   后者先把颜色数压下去再交给无损那条路编码 —— 量化本身是有损的，所以它绝不会
//!   出现在无损模式里。
//!
//! 输出端口跟着参数变：无损和有损调色板都是 `Image(Png)`，有损 JPEG 是 `Image(Jpeg)`。

use std::sync::Arc;

use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{flatten_onto_white, has_transparency, EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{human_size, one_output, NodeArgs, Value, ValueMap};
use crate::png_opt;
use crate::png_quant;
use crate::registry::NodeSpec;

pub const KIND: &str = "compress_image";

const PARAM_MODE: &str = "mode";
const PARAM_SCHEME: &str = "scheme";
const PARAM_STRIP: &str = "strip";
const PARAM_LOSSY_FORMAT: &str = "lossyFormat";
const PARAM_LOSS: &str = "loss";
const PARAM_COLORS: &str = "colors";

const MODE_LOSSLESS: &str = "lossless";
const MODE_LOSSY: &str = "lossy";
const LOSSY_JPEG: &str = "jpeg";
const LOSSY_PALETTE: &str = "palette";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "图像压缩".into(),
            category: "图像".into(),
            description: "把图像编码得更小。无损模式会挨个试颜色类型、位深、调色板、\
                          逐行滤波和压缩级别，只留最小的那一份；有损模式转 JPEG，\
                          或者把颜色数压到调色板里。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any))
                    .required()
                    .hint("任何可解码的图像"),
            ],
            // 声明里的输出是泛化的「图像」（卡片上显示 `IMG`）——
            // 真正的输出格式由 `output_ports` 按压缩方式现算。
            outputs: vec![PortDef::new(
                "image",
                "图像",
                PortType::Image(ImageFormat::Any),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_MODE,
                    "压缩方式",
                    ParamSpec::Select {
                        default: MODE_LOSSLESS.into(),
                        options: vec![
                            SelectOption::new(MODE_LOSSLESS, "无损")
                                .hint("像素一个都不改，输出 PNG"),
                            SelectOption::new(MODE_LOSSY, "有损")
                                .hint("更小，但像素会变，输出 JPEG 或调色板 PNG"),
                        ],
                    },
                )
                .described("输出端口跟着它变。"),
                ParamDef::new(
                    PARAM_SCHEME,
                    "优化力度",
                    ParamSpec::Select {
                        default: png_opt::Scheme::Balanced.name().into(),
                        options: vec![
                            SelectOption::new(png_opt::Scheme::Fast.name(), "快速")
                                .hint("压得快，压得轻"),
                            SelectOption::new(png_opt::Scheme::Balanced.name(), "均衡")
                                .hint("日常用这个"),
                            SelectOption::new(png_opt::Scheme::Maximum.name(), "最大")
                                .hint("几种滤波都试一遍，慢但更小"),
                            SelectOption::new(png_opt::Scheme::Zopfli.name(), "Zopfli")
                                .hint("极限离线优化，可能要等一会儿"),
                        ],
                    },
                )
                .described("越大越慢越小。任何一档都不会动像素。")
                .visible_when(PARAM_MODE, &[MODE_LOSSLESS]),
                ParamDef::new(
                    PARAM_STRIP,
                    "元数据",
                    ParamSpec::Select {
                        default: png_opt::Strip::Safe.name().into(),
                        options: vec![
                            SelectOption::new(png_opt::Strip::Safe.name(), "保留颜色信息")
                                .hint("留住 ICC / sRGB / gAMA，去掉文本、时间等"),
                            SelectOption::new(png_opt::Strip::All.name(), "全部剥离")
                                .hint("连颜色描述一起去掉，游戏素材用"),
                        ],
                    },
                )
                .described("ICC 这类影响颜色显示的默认留着，不当垃圾删。")
                .visible_when(PARAM_MODE, &[MODE_LOSSLESS]),
                ParamDef::new(
                    PARAM_LOSSY_FORMAT,
                    "有损方式",
                    ParamSpec::Select {
                        default: LOSSY_JPEG.into(),
                        options: vec![
                            SelectOption::new(LOSSY_JPEG, "JPEG 图像").hint("适合照片，存不下透明"),
                            SelectOption::new(LOSSY_PALETTE, "PNG 调色板")
                                .hint("适合像素画：把颜色数压下来"),
                        ],
                    },
                )
                .visible_when(PARAM_MODE, &[MODE_LOSSY]),
                ParamDef::new(
                    PARAM_LOSS,
                    "允许的损耗率",
                    ParamSpec::Slider {
                        default: 15.0,
                        min: 0.0,
                        max: 100.0,
                        step: 1.0,
                        integer: true,
                        unit: Some("%".into()),
                    },
                )
                .described("0 最保真，100 最狠。它换算成 JPEG 的画质参数。")
                .visible_when(PARAM_MODE, &[MODE_LOSSY])
                .and_visible_when(PARAM_LOSSY_FORMAT, &[LOSSY_JPEG]),
                ParamDef::new(
                    PARAM_COLORS,
                    "颜色数上限",
                    ParamSpec::Slider {
                        default: 64.0,
                        min: 2.0,
                        max: 256.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .described("量化到这么多种颜色。颜色本来就不多时不会被改动。")
                .visible_when(PARAM_MODE, &[MODE_LOSSY])
                .and_visible_when(PARAM_LOSSY_FORMAT, &[LOSSY_PALETTE]),
            ],
            notes: vec![
                "无损模式不改任何像素：先分析这张图实际用到什么 —— 没有透明就不用 RGBA，\
                 是灰度就存灰度，颜色少到装得进调色板就转索引色，再配上合适的位深。"
                    .into(),
                "调色板转换是逐像素精确的：颜色对不上就不会用。每一步最后都会解回 RGBA \
                 和原图逐像素比对，对不上就整个放弃，宁可不优化。"
                    .into(),
                "优化力度越大越慢。「最大」会把几种逐行滤波都编一遍比大小；\
                 「Zopfli」还要再花时间重压一遍压缩数据，适合丢在后台慢慢跑。"
                    .into(),
                "「保留颜色信息」是指留着 ICC / sRGB / gAMA 这些决定颜色怎么显示的块，\
                 只去掉文本、时间这类没用的。要连颜色描述一起去掉就选「全部剥离」。"
                    .into(),
                "有损的「PNG 调色板」是量化，属于有损压缩 —— 和无损那条路完全分开，\
                 无损模式绝不会偷偷用它。JPEG 存不下透明，转过去时透明会被压到白底上。"
                    .into(),
                "本来就有损的图（比如 JPEG）选无损转成 PNG 只会更大；这种时候会原样返回。".into(),
            ],
        },
        output_ports,
        run,
    )
}

/// 输出端口跟着「压缩方式」和「有损方式」走。
fn output_ports(params: &Params) -> Vec<PortDef> {
    let format = output_format(params);
    vec![PortDef::new(
        "image",
        format.label(),
        PortType::Image(format),
    )]
}

fn is_lossy(params: &Params) -> bool {
    params::string(params, PARAM_MODE, MODE_LOSSLESS) == MODE_LOSSY
}

fn output_format(params: &Params) -> ImageFormat {
    if !is_lossy(params) {
        return ImageFormat::Png;
    }
    if params::string(params, PARAM_LOSSY_FORMAT, LOSSY_JPEG) == LOSSY_PALETTE {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    }
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 先把输入拿在手里（克隆只是复制一个 Arc），后面 warn 之后还要用它。
    let (source, name) = args.image_in("image")?;
    let decoded = source.decode()?;

    let value = if is_lossy(args.params) {
        lossy(args, &source, &decoded)?
    } else {
        lossless(args, &source, &decoded)?
    };

    Ok(one_output(
        "image",
        Value::Image(value).with_name_hint(name),
    ))
}

/// 无损：整条 PNG 优化管线跑一遍。
fn lossless(
    args: &mut NodeArgs<'_>,
    source: &ImageValue,
    decoded: &DynamicImage,
) -> Result<ImageValue, NodeError> {
    let scheme = png_opt::Scheme::parse(&params::string(
        args.params,
        PARAM_SCHEME,
        png_opt::Scheme::Balanced.name(),
    ));
    let strip = png_opt::Strip::parse(&params::string(
        args.params,
        PARAM_STRIP,
        png_opt::Strip::Safe.name(),
    ));

    // 输入本来就是 PNG 的话，把原始字节给它：既能搬颜色相关的块，也能比大小。
    let source_png = (source.format() == ImageFormat::Png).then(|| source.bytes());
    let optimized = png_opt::optimize(decoded, source_png, &png_opt::Options::new(scheme, strip))?;

    match optimized.saved_ratio() {
        Some(ratio) if optimized.improved => args.warn(format!(
            "{} → {}（省下 {:.0}%）· {}",
            human_size(optimized.before.unwrap_or(0)),
            human_size(optimized.after()),
            ratio * 100.0,
            optimized.strategy,
        )),
        Some(_) => args.warn("已经压得很紧了，优化后反而更大，原样保留"),
        None => args.warn(format!(
            "输入不是 PNG，直接算出一份最优 PNG：{} · {}",
            human_size(optimized.after()),
            optimized.strategy,
        )),
    }

    Ok(ImageValue::from_bytes(ImageFormat::Png, optimized.bytes).inherit_provenance(source))
}

/// 有损：要么转 JPEG，要么量化成调色板 PNG。
fn lossy(
    args: &mut NodeArgs<'_>,
    source: &ImageValue,
    decoded: &DynamicImage,
) -> Result<ImageValue, NodeError> {
    if params::string(args.params, PARAM_LOSSY_FORMAT, LOSSY_JPEG) == LOSSY_PALETTE {
        return palette(args, source, decoded);
    }

    let loss = params::number(args.params, PARAM_LOSS, 15.0).clamp(0.0, 100.0);
    // 损耗率 0 对应最高画质，100 对应最省 —— 直接翻过来当画质用。
    let quality = (100.0 - loss).round().clamp(1.0, 100.0) as u8;
    if has_transparency(decoded) {
        args.warn("JPEG 存不下透明，透明区域已合成到白底上");
    }

    let flattened = DynamicImage::ImageRgb8(flatten_onto_white(decoded));
    Ok(ImageValue::from_image(
        ImageFormat::Jpeg,
        Arc::new(flattened),
        EncodeOptions {
            jpeg_quality: quality,
            ..EncodeOptions::default()
        },
    )?
    .inherit_provenance(source))
}

/// 有损：量化成调色板，再交给无损那条路去编码（索引色 PNG）。
fn palette(
    args: &mut NodeArgs<'_>,
    source: &ImageValue,
    decoded: &DynamicImage,
) -> Result<ImageValue, NodeError> {
    let limit = params::integer(args.params, PARAM_COLORS, 64).clamp(2, 256) as usize;
    let quantized = png_quant::quantize(decoded, limit);

    // 量化之后颜色很少，无损优化会自己挑中索引色编码。
    let optimized = png_opt::optimize(
        &quantized,
        None,
        &png_opt::Options::new(png_opt::Scheme::Balanced, png_opt::Strip::Safe),
    )?;

    args.warn(format!(
        "已量化到 {} 色以内（有损），{} · {}",
        limit,
        human_size(optimized.after()),
        optimized.strategy,
    ));

    Ok(ImageValue::from_bytes(ImageFormat::Png, optimized.bytes).inherit_provenance(source))
}
