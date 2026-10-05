//! 「颜色剔除」节点：把跟目标色相近的像素抠成透明（像一个色键 / 去底色）。
//!
//! 两种模式：
//!
//! * **全图剔除** 只要颜色像就剔，不管在哪儿 —— 适合「这个颜色本来就是错的」。
//! * **背景剔除** 只剔**与画面外圈相连**的那些像素 —— 中间的白色眼睛、白衬衫不会被
//!   无辜抠掉。这靠一次从边框发起的洪水填充；已经是透明的像素也当作背景（能让填充穿过
//!   透明边距），边缘上介于背景和主体之间的过渡像素还会按接近程度调成半透明，不留一圈
//!   底色杂边。

use std::sync::Arc;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::palette::parse_color;
use crate::registry::NodeSpec;

pub const KIND: &str = "remove_color";

const PARAM_MODE: &str = "mode";
const PARAM_COLOR: &str = "color";
const PARAM_THRESHOLD: &str = "threshold";

const MODE_WHOLE: &str = "whole";
const MODE_BACKGROUND: &str = "background";

/// 背景剔除时，边缘像素距目标色不到「阈值 + 这个数」就按接近程度调成半透明。
/// 太小盖不住抗锯齿的杂边，太大会啃到主体。
const FEATHER: u8 = 24;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "颜色剔除".into(),
            category: "图像".into(),
            description: "把跟目标色相近的像素抠成透明 —— 去底色、做色键都靠它。".into(),
            is_source: false,
            // 输入不锁格式：去底色的场合很多，没必要逼着先转 PNG。
            inputs: vec![
                PortDef::new("image", "图像", PortType::Image(ImageFormat::Any)).required(),
            ],
            outputs: vec![PortDef::new(
                "image",
                "PNG 图像",
                PortType::Image(ImageFormat::Png),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_MODE,
                    "模式",
                    ParamSpec::Select {
                        default: MODE_WHOLE.into(),
                        options: vec![
                            SelectOption::new(MODE_WHOLE, "全图剔除")
                                .hint("只要颜色像就剔，不管在哪儿"),
                            SelectOption::new(MODE_BACKGROUND, "背景剔除")
                                .hint("只剔和画面外圈相连的（中间的洞留着）"),
                        ],
                    },
                )
                .described("去底色一般用「背景剔除」。"),
                ParamDef::new(
                    PARAM_COLOR,
                    "目标色",
                    ParamSpec::Color {
                        default: "#ffffff".into(),
                    },
                )
                .described("要被抠掉的颜色。也能接色板（取第一个色）。"),
                ParamDef::new(
                    PARAM_THRESHOLD,
                    "相近色剔除阈值",
                    ParamSpec::Number {
                        default: 16.0,
                        min: 0.0,
                        max: 255.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .described("每个分量差在这个范围内的颜色都一起剔掉。0 就是只剔一模一样的。"),
            ],
            notes: vec![
                "相近的判定是**逐分量**的最大差值：|ΔR|、|ΔG|、|ΔB| 都不超过阈值就算同一种。\
                 阈值 0 只剔完全一样的颜色。"
                    .into(),
                "背景剔除从画面外圈开始，沿着「像目标色」的像素往里漫延；被主体包住、端不到\
                 外圈的同类颜色不会被抠掉（比如白底 sprite 里白色的眼睛）。"
                    .into(),
                "背景剔除还把已经透明的像素当成背景（填充能穿过透明边距），并把背景与主体之间\
                 那一圈过渡像素按接近程度调成半透明，不留底色杂边。"
                    .into(),
                "只改 alpha —— 被剔的像素变成全透明，颜色一个字节不动；本来就透明的像素不受影响。"
                    .into(),
                "目标色默认是白色（最常见的底色）。画面里没有白色就把它改成真正的底色，\
                 或者接一个「色彩分析」把主色送过来。"
                    .into(),
                "输出一律是 PNG（要带透明通道）。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆只是复制一个 Arc；后面 warn 之后还要用它带出处。
    let (source, name) = args.image_in("image")?;

    let target = parse_color(&params::string(args.params, PARAM_COLOR, "#ffffff"));
    if target[3] == 0 {
        args.warn("目标色是透明的，没有东西可剔除，原样通过");
        return Ok(one_output(
            "image",
            Value::Image(source).with_name_hint(name),
        ));
    }
    let threshold = params::integer(args.params, PARAM_THRESHOLD, 16).clamp(0, 255) as u8;

    let decoded = source.decode()?;
    let mut rgba = decoded.to_rgba8();
    let background = params::string(args.params, PARAM_MODE, MODE_WHOLE) == MODE_BACKGROUND;
    let removed = if background {
        remove_background(&mut rgba, target, threshold)
    } else {
        remove_whole(&mut rgba, target, threshold)
    };

    if removed == 0 {
        if background {
            args.warn("画面外圈上没有目标色，没找到背景，原样通过");
        } else {
            args.warn("没有像素命中目标色，原样通过");
        }
    } else if background {
        args.warn(format!("从外圈剔除了 {removed} 个像素"));
    } else {
        args.warn(format!("剔除了 {removed} 个像素"));
    }

    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(image::DynamicImage::ImageRgba8(rgba)),
        EncodeOptions::default(),
    )?
    .inherit_provenance(&source);

    Ok(one_output(
        "image",
        Value::Image(value).with_name_hint(name),
    ))
}

/// 全图剔除：只要颜色像就剔，不管在哪儿。返回剔掉（原本不透明）的像素数。
fn remove_whole(rgba: &mut image::RgbaImage, target: [u8; 4], threshold: u8) -> u64 {
    let mut removed = 0;
    for pixel in rgba.pixels_mut() {
        if pixel.0[3] != 0 && matches_target(pixel.0, target, threshold) {
            pixel.0[3] = 0;
            removed += 1;
        }
    }
    removed
}

/// 背景剔除：从画面外圈洪水填充，只剔与外界相连的像素。
///
/// * **4 连通** —— 8 连通会让填充从对角的一个像素缝里钻进去，把本该留住的东西抹掉。
/// * 已经是透明的像素也算背景，填充能穿过透明边距。
/// * 最后把紧邻背景的那圈过渡像素按接近程度调成半透明，去处抗锯齿留下的底色杂边。
fn remove_background(rgba: &mut image::RgbaImage, target: [u8; 4], threshold: u8) -> u64 {
    let (width, height) = (rgba.width(), rgba.height());
    if width == 0 || height == 0 {
        return 0;
    }
    let index = |x: u32, y: u32| (y * width + x) as usize;
    // 能被填充穿过的像素：像目标色，或者本来就是透明的。
    let passable = |x: u32, y: u32| {
        let pixel = rgba.get_pixel(x, y).0;
        pixel[3] == 0 || matches_target(pixel, target, threshold)
    };

    let mut background = vec![false; (width * height) as usize];
    let mut stack: Vec<(u32, u32)> = Vec::new();
    let seed = |background: &mut Vec<bool>, stack: &mut Vec<(u32, u32)>, x: u32, y: u32| {
        let slot = index(x, y);
        if !background[slot] && passable(x, y) {
            background[slot] = true;
            stack.push((x, y));
        }
    };
    for x in 0..width {
        seed(&mut background, &mut stack, x, 0);
        seed(&mut background, &mut stack, x, height - 1);
    }
    for y in 0..height {
        seed(&mut background, &mut stack, 0, y);
        seed(&mut background, &mut stack, width - 1, y);
    }

    while let Some((x, y)) = stack.pop() {
        let visit = |nx: u32, ny: u32, background: &mut Vec<bool>, stack: &mut Vec<(u32, u32)>| {
            let slot = index(nx, ny);
            if !background[slot] && passable(nx, ny) {
                background[slot] = true;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            visit(x - 1, y, &mut background, &mut stack);
        }
        if y > 0 {
            visit(x, y - 1, &mut background, &mut stack);
        }
        if x + 1 < width {
            visit(x + 1, y, &mut background, &mut stack);
        }
        if y + 1 < height {
            visit(x, y + 1, &mut background, &mut stack);
        }
    }

    let mut removed = 0;
    for (slot, pixel) in rgba.pixels_mut().enumerate() {
        if background[slot] && pixel.0[3] != 0 {
            pixel.0[3] = 0;
            removed += 1;
        }
    }

    // 边缘羽化：紧邻背景、颜色又“差一点点”的过渡像素调成半透明。
    for (slot, pixel) in rgba.pixels_mut().enumerate() {
        if background[slot] || pixel.0[3] == 0 {
            continue;
        }
        let (x, y) = ((slot as u32) % width, (slot as u32) / width);
        let touches_background = (x > 0 && background[index(x - 1, y)])
            || (y > 0 && background[index(x, y - 1)])
            || (x + 1 < width && background[index(x + 1, y)])
            || (y + 1 < height && background[index(x, y + 1)]);
        if !touches_background {
            continue;
        }
        let distance = channel_distance(pixel.0, target);
        if distance > threshold && distance <= threshold.saturating_add(FEATHER) {
            let span = u32::from(FEATHER).max(1);
            let alpha = (u32::from(distance - threshold) * 255 / span).min(255) as u8;
            pixel.0[3] = pixel.0[3].min(alpha);
        }
    }
    removed
}

/// 逐分量的最大差值。
fn channel_distance(pixel: [u8; 4], target: [u8; 4]) -> u8 {
    (0..3)
        .map(|channel| pixel[channel].abs_diff(target[channel]))
        .max()
        .unwrap_or(0)
}

fn matches_target(pixel: [u8; 4], target: [u8; 4], threshold: u8) -> bool {
    channel_distance(pixel, target) <= threshold
}

#[cfg(test)]
mod tests {
    use super::{remove_background, remove_whole};
    use image::{Rgba, RgbaImage};

    /// 5×5 白底，中间一个红框圈住一颗白心（背景剔不应该把它抠掉）。
    fn donut() -> RgbaImage {
        let mut image = RgbaImage::from_pixel(5, 5, Rgba([255, 255, 255, 255]));
        for (x, y) in [
            (1, 1),
            (2, 1),
            (3, 1),
            (1, 2),
            (3, 2),
            (1, 3),
            (2, 3),
            (3, 3),
        ] {
            image.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        }
        image
    }

    #[test]
    fn a_near_colour_is_removed_only_within_the_threshold() {
        let base = RgbaImage::from_pixel(2, 1, Rgba([255, 255, 255, 255]));
        let mut image = base.clone();
        image.put_pixel(1, 0, Rgba([240, 250, 255, 255]));

        // 阈值 0：只有纯白被剔。
        let mut exact = image.clone();
        assert_eq!(remove_whole(&mut exact, [255, 255, 255, 255], 0), 1);
        // 阈值够大：两个都剔。
        let mut loose = image.clone();
        assert_eq!(remove_whole(&mut loose, [255, 255, 255, 255], 16), 2);
    }

    #[test]
    fn whole_mode_removes_every_matching_pixel() {
        let mut image = donut();
        remove_whole(&mut image, [255, 255, 255, 255], 0);
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "外圈的白被剔");
        assert_eq!(image.get_pixel(2, 2).0[3], 0, "全图模式连包住的白心也剔");
    }

    #[test]
    fn background_mode_keeps_colours_not_connected_to_the_outside() {
        let mut image = donut();
        remove_background(&mut image, [255, 255, 255, 255], 0);
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "外圈的白是背景");
        assert_eq!(image.get_pixel(2, 2).0[3], 255, "被红框包住的白心留着");
        assert_eq!(image.get_pixel(2, 1).0, [255, 0, 0, 255], "红框不受影响");
    }

    #[test]
    fn background_mode_softens_the_transition_ring() {
        // 白底 + 一块紧邻的浅灰：距离在 (阈值, 阈值 + 羽化] 之间，应该被调成半透明。
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([255, 255, 255, 255]));
        image.put_pixel(1, 0, Rgba([245, 245, 245, 255]));
        remove_background(&mut image, [255, 255, 255, 255], 0);
        let edge = image.get_pixel(1, 0).0[3];
        assert!(edge > 0 && edge < 255, "边缘该被羽化，实际 alpha = {edge}");
    }

    #[test]
    fn background_mode_walks_through_already_transparent_pixels() {
        // 透明边距 + 里面一圈白：白应当被当成背景剔掉（填充能穿过透明的边框）。
        let mut image = RgbaImage::from_pixel(3, 1, Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        // 整行左侧是透明的，右侧没有东西可走；把白垫在中间、右边也是透明。
        image.put_pixel(2, 0, Rgba([0, 0, 0, 0]));
        remove_background(&mut image, [255, 255, 255, 255], 0);
        assert_eq!(image.get_pixel(1, 0).0[3], 0, "白能经由透明边距连到外圈");
    }
}
