//! 「边框/边距」节点（PNG → PNG）。
//!
//! 给图像加一圈边框。关键是**沿着内容描边，而不是沿着画布**：
//!
//! * 图是不透明的（整张都是内容）→ 相当于给图像套一个画框：画布四边各扩 `粗细` 像素，
//!   新边距填上挑的颜色；
//! * 图有透明像素 → 描的是**非透明像素的边界**：把不透明区域向外「膨胀」`粗细` 像素，
//!   膨胀出来的、原本透明的像素涂上颜色。中心对称的**洞**（甜甜圈形的 sprite）同样会被
//!   从内侧描到 —— 因为它周围的实体同样会向洞里膨胀。
//!
//! 画布只在**需要时**才扩：某一侧的内容离边不到 `粗细`，描边会被切掉，那一侧才补出来。
//! 内容本来就在画布里头的话尺寸不动 —— 这样游戏素材的一帧不会被无辜地改大小。
//!
//! 「颜色」用的是通用调色板控件，可以挑透明色 —— 那时就是纯加边距。

use std::sync::Arc;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::palette::parse_color;
use crate::registry::NodeSpec;

pub const KIND: &str = "border_image";

const PARAM_THICKNESS: &str = "thickness";
const PARAM_COLOR: &str = "color";

/// 粗细的上限，防止手滑填个几千把图撑爆。
const MAX_THICKNESS: u32 = 256;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "边框/边距".into(),
            category: "图像".into(),
            description: "给图像加一圈边框：沿着非透明像素的边界描边，透明的洞也描；\
                          整张不透明时就是在四周加边距。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "PNG 图像", PortType::Image(ImageFormat::Png)).required(),
            ],
            outputs: vec![PortDef::new(
                "image",
                "PNG 图像",
                PortType::Image(ImageFormat::Png),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_THICKNESS,
                    "粗细",
                    ParamSpec::Number {
                        default: 1.0,
                        min: 0.0,
                        max: MAX_THICKNESS as f64,
                        step: 1.0,
                        integer: true,
                        unit: Some("px".into()),
                    },
                )
                .described("边框的宽度。0 就是什么都不做。"),
                ParamDef::new(
                    PARAM_COLOR,
                    "颜色",
                    ParamSpec::Color {
                        default: "#000000".into(),
                    },
                )
                .described("点开是个取色器；把不透明度拉到 0 就只加边距、不画颜色。"),
            ],
            notes: vec![
                "描边认的是 alpha：alpha 不为 0 的像素算内容，边框画在内容之外。".into(),
                "透明的洞（甜甜圈形的 sprite）会从内侧被描上 —— 洞四周的实体同样向洞里膨胀。"
                    .into(),
                "画布只在描边会被切掉时才向外扩，所以有透明边距的 sprite 尺寸不变；\
                 整张不透明的图会在四边各加「粗细」像素。"
                    .into(),
                "边框颜色可以是透明的 —— 那时它就是一个纯粹的「加边距」节点。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面 warn 之后还要用它带出处。
    let (source, name) = args.image_in("image")?;

    let thickness =
        params::integer(args.params, PARAM_THICKNESS, 1).clamp(0, i64::from(MAX_THICKNESS)) as u32;
    if thickness == 0 {
        args.warn("粗细是 0，图像原样通过");
        return Ok(one_output(
            "image",
            Value::Image(source).with_name_hint(name),
        ));
    }

    let color = parse_color(&params::string(args.params, PARAM_COLOR, "#000000"));

    let decoded = source.decode()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(NodeError::new("图像是空的，没有可描边的内容"));
    }

    // 内容 = alpha 不为 0 的像素。先找出它的外接矩形，决定画布要往哪几边扩。
    let solid: Vec<bool> = rgba.pixels().map(|pixel| pixel.0[3] != 0).collect();
    let Some((min_x, min_y, max_x, max_y)) = content_bounds(&solid, width, height) else {
        args.warn("整张图都是透明的，没有可描边的内容，原样通过");
        return Ok(one_output(
            "image",
            Value::Image(source).with_name_hint(name),
        ));
    };

    let need_left = thickness.saturating_sub(min_x);
    let need_top = thickness.saturating_sub(min_y);
    let need_right = thickness.saturating_sub(width - 1 - max_x);
    let need_bottom = thickness.saturating_sub(height - 1 - max_y);

    let out_width = width + need_left + need_right;
    let out_height = height + need_top + need_bottom;

    // 先把内容整块搬进新画布（新出来的边距默认透明）。
    let mut out = image::RgbaImage::new(out_width, out_height);
    for (x, y, pixel) in rgba.enumerate_pixels() {
        out.put_pixel(x + need_left, y + need_top, *pixel);
    }

    // 再在新画布上算「每个透明像素离内容有多远」，把 `粗细` 以内的涂上颜色。
    let solid: Vec<bool> = out.pixels().map(|pixel| pixel.0[3] != 0).collect();
    let distance = outside_distance(&solid, out_width as usize, out_height as usize);
    let limit = thickness as f32 + 0.5;
    let painted = color[3] != 0;
    for (index, pixel) in out.pixels_mut().enumerate() {
        if solid[index] || pixel.0[3] != 0 {
            continue;
        }
        if distance[index] <= limit {
            *pixel = image::Rgba(color);
        }
    }
    if !painted {
        args.warn("边框颜色是透明的，只加了边距");
    }

    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(image::DynamicImage::ImageRgba8(out)),
        EncodeOptions::default(),
    )?
    .inherit_provenance(&source);

    Ok(one_output(
        "image",
        Value::Image(value).with_name_hint(name),
    ))
}

/// 内容（`solid` 为真的像素）的外接矩形：`(min_x, min_y, max_x, max_y)`。没内容返回 `None`。
fn content_bounds(solid: &[bool], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut min_x, mut min_y) = (width, height);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            if !solid[(y * width + x) as usize] {
                continue;
            }
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

/// 每个像素到最近内容像素的近似距离（chamfer 两遍扫描，权重 1 / √2）。
///
/// 内容像素自身是 0。比逐像素扫一个 `(2t+1)²` 的窗口便宜得多，粗细再大也是 O(像素数)。
fn outside_distance(solid: &[bool], width: usize, height: usize) -> Vec<f32> {
    const INF: f32 = 1.0e9;
    const DIAG: f32 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> = solid
        .iter()
        .map(|&is_solid| if is_solid { 0.0 } else { INF })
        .collect();

    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let mut best = d[i];
            if x > 0 {
                best = best.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                best = best.min(d[i - width] + 1.0);
                if x > 0 {
                    best = best.min(d[i - width - 1] + DIAG);
                }
                if x + 1 < width {
                    best = best.min(d[i - width + 1] + DIAG);
                }
            }
            d[i] = best;
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let i = y * width + x;
            let mut best = d[i];
            if x + 1 < width {
                best = best.min(d[i + 1] + 1.0);
            }
            if y + 1 < height {
                best = best.min(d[i + width] + 1.0);
                if x + 1 < width {
                    best = best.min(d[i + width + 1] + DIAG);
                }
                if x > 0 {
                    best = best.min(d[i + width - 1] + DIAG);
                }
            }
            d[i] = best;
        }
    }
    d
}
