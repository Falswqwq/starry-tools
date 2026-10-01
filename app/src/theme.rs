//! 设计令牌 —— 浅色、极简：内容只用黑 / 白 / 灰，蓝是唯一的主题色。卡片与各种浮层共用
//! 同一套表面（白底 + 发丝边 + 同一个圆角和阴影），整块屏幕看起来才是一件东西。

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};
use egui::epaint::Shadow;
use starrytools_core::model::port_type::{ImageFormat, PortType};

// 画布
pub const CANVAS: Color32 = Color32::from_rgb(0xfb, 0xfb, 0xfc);
pub const CANVAS_DOT: Color32 = Color32::from_rgb(0xd7, 0xdb, 0xe1);

// 面：卡片和浮层共用这一套
pub const SURFACE: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const SURFACE_2: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf8);
pub const SURFACE_3: Color32 = Color32::from_rgb(0xec, 0xee, 0xf1);
pub const HAIRLINE: Color32 = Color32::from_rgb(0xe4, 0xe7, 0xeb);
pub const HAIRLINE_STRONG: Color32 = Color32::from_rgb(0xcc, 0xd2, 0xda);

// 字
pub const INK: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const INK_2: Color32 = Color32::from_rgb(0x4b, 0x55, 0x63);
pub const INK_3: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa1);

// 主题蓝
pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);
/// 浅一号的蓝：**参数端口**用它 —— 可选的东西，存在感比必填的端口弱一档。
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x8f, 0xb5, 0xf7);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x1d, 0x4e, 0xd8);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xef, 0xf4, 0xff);
pub const ACCENT_LINE: Color32 = Color32::from_rgb(0xc3, 0xd6, 0xfe);

// 类型：蓝灰一条梯度，具体格式是蓝，通配和其余几种是灰
//
// 这几个是**徽标专用**的配色，只回答「这个端口是什么类型」一个问题。特意不复用
// `ACCENT`（那个蓝表示选中 / 焦点 / 端点），也不跟节点的状态色（报错 / 运行）混用 ——
// 类型是类型，状态是状态。
pub const TYPE_IMAGE: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const TYPE_UNKNOWN: Color32 = Color32::from_rgb(0x94, 0xa3, 0xb8);
pub const TYPE_TEXT: Color32 = Color32::from_rgb(0x33, 0x41, 0x55);
pub const TYPE_NUMBER: Color32 = Color32::from_rgb(0x47, 0x55, 0x69);
pub const TYPE_BOOL: Color32 = Color32::from_rgb(0x64, 0x74, 0x8b);

/// 紫色：**阻塞节点**（要你动手的节点）的边框色。它不是状态色，是「这类节点长这样」。
pub const PURPLE: Color32 = Color32::from_rgb(0x7c, 0x3a, 0xed);
pub const PURPLE_SOFT: Color32 = Color32::from_rgb(0xf5, 0xf1, 0xff);
pub const PURPLE_LINE: Color32 = Color32::from_rgb(0xdd, 0xd0, 0xfb);

pub const DANGER: Color32 = Color32::from_rgb(0xdc, 0x26, 0x26);
pub const DANGER_SOFT: Color32 = Color32::from_rgb(0xfe, 0xf3, 0xf2);
pub const DANGER_LINE: Color32 = Color32::from_rgb(0xf3, 0xc6, 0xc2);
pub const WARN: Color32 = Color32::from_rgb(0xb4, 0x53, 0x09);
pub const WARN_SOFT: Color32 = Color32::from_rgb(0xff, 0xf7, 0xed);
pub const WARN_LINE: Color32 = Color32::from_rgb(0xf3, 0xd3, 0xac);
pub const OK: Color32 = Color32::from_rgb(0x15, 0x80, 0x3d);

/// 默认连线。
pub const WIRE: Color32 = Color32::from_rgb(0xc3, 0xc9, 0xd2);

pub const R_CARD: u8 = 8;
pub const R_CTL: u8 = 6;

/// 类型徽标的颜色 —— **全应用唯一的一处**。端口列上的徽标、参数名前的徽标、
/// 节点库卡片和详情里的徽标，全部走它，因此**同一类型的徽标在哪儿都同色**。
///
/// 它只取决于**类型本身**（`PortType`），与节点、与选中 / 悬停 / 报错等状态无关，
/// 也不借用 `ACCENT`（那个蓝表示选中 / 焦点）—— 徽标标的是类型，不是状态。
/// 到底是 PNG 还是 JPG，由徽标上的字来说，颜色只分「图像 / 文本 / 数字 / 布尔 / 通配」。
pub fn badge_color(ty: PortType) -> Color32 {
    match ty {
        // 格式未知的通配图像，和 `Any` 一样留在灰阶里。
        PortType::Image(ImageFormat::Any) | PortType::Any => TYPE_UNKNOWN,
        PortType::Image(_) => TYPE_IMAGE,
        PortType::Text => TYPE_TEXT,
        PortType::Number => TYPE_NUMBER,
        PortType::Bool => TYPE_BOOL,
    }
}

/// 强调色的半透明版本，刀光的渐隐与发光用。
pub fn accent_alpha(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(0x25, 0x63, 0xeb, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// 危险色的半透明版本，将被切断的连线和断口闪光用。
pub fn danger_alpha(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(0xdc, 0x26, 0x26, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// `--shadow-pop`：浮层用的大模糊。
pub fn shadow_pop() -> Shadow {
    Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_rgba_premultiplied(0x10, 0x18, 0x28, 0x22),
    }
}

/// 界面主字体的候选：(路径, 字面索引)。JetBrains Mono 和原设计那款等宽西文神韵最接近。
pub const UI_FONT_CANDIDATES: [(&str, u32); 3] = [
    (
        "/usr/share/fonts/truetype/jetbrains/JetBrainsMonoNerdFont-Regular.ttf",
        0,
    ),
    (
        "/usr/share/fonts/truetype/jetbrains/JetBrainsMonoNLNerdFont-Regular.ttf",
        0,
    ),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0),
];

/// 中文字体候选：(路径, 字体集合里的字面索引)。索引只对 `.ttc` 有意义，单字面文件填 0。
///
/// `NotoSansCJK-Regular.ttc` 里装了 10 个字面，2 = Noto Sans CJK SC，
/// 7 = Noto Sans Mono CJK SC。两者都含全部汉字，但比例字面看着更舒服。
pub const CJK_CANDIDATES: [(&str, u32); 3] = [
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 7),
    (
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        0,
    ),
];

fn find_font(candidates: &[(&str, u32)]) -> Option<(u32, Vec<u8>)> {
    let (path, index) = candidates
        .iter()
        .find(|(path, _)| std::path::Path::new(path).is_file())?;
    let bytes = std::fs::read(path).ok()?;
    Some((*index, bytes))
}

/// 挂字体：主字体用 JetBrains Mono，汉字交给 Noto Sans CJK。
///
/// egui 用 `ab_glyph` 解析字体，**只认 TTF/OTF**。
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut changed = false;

    if let Some((index, bytes)) = find_font(&UI_FONT_CANDIDATES) {
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        fonts
            .font_data
            .insert("ui".to_owned(), std::sync::Arc::new(data));
        // 放在字族**最前面**：西文和数字走它。
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "ui".to_owned());
        }
        changed = true;
    } else {
        eprintln!("[theme] 找不到 JetBrains Mono，西文将用内置字体");
    }

    if let Some((index, bytes)) = find_font(&CJK_CANDIDATES) {
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        fonts
            .font_data
            .insert("cjk".to_owned(), std::sync::Arc::new(data));
        // 挂在字族**末尾**：只接默认字族里没有的字形（也就是汉字）。
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        changed = true;
    } else {
        eprintln!("[theme] 找不到中文字体，中文会显示成方块");
    }

    if changed {
        ctx.set_fonts(fonts);
    }
}

/// 把令牌灌进 egui 的 `Visuals` / `Spacing`，让默认控件也长得对。
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Light);

    ctx.style_mut_of(egui::Theme::Light, |style| {
        let v = &mut style.visuals;

        v.panel_fill = CANVAS;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0, HAIRLINE);
        v.window_corner_radius = CornerRadius::same(R_CARD);
        v.window_shadow = shadow_pop();
        v.popup_shadow = shadow_pop();

        v.faint_bg_color = SURFACE_2;
        v.extreme_bg_color = SURFACE;
        v.override_text_color = Some(INK);
        v.hyperlink_color = ACCENT;
        v.warn_fg_color = WARN;
        v.error_fg_color = DANGER;

        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, SURFACE);

        // 控件：从「没碰过」到「按下去」一整套。
        let cr = CornerRadius::same(R_CTL);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = cr;
            w.expansion = 0.0;
            w.bg_stroke = Stroke::new(1.0, HAIRLINE);
        }

        v.widgets.noninteractive.bg_fill = SURFACE_2;
        v.widgets.noninteractive.weak_bg_fill = SURFACE_2;
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_3);

        v.widgets.inactive.bg_fill = SURFACE;
        v.widgets.inactive.weak_bg_fill = SURFACE;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK_2);

        v.widgets.hovered.bg_fill = SURFACE_2;
        v.widgets.hovered.weak_bg_fill = SURFACE_2;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE_STRONG);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, INK);

        v.widgets.active.bg_fill = SURFACE_3;
        v.widgets.active.weak_bg_fill = SURFACE_3;
        v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        v.widgets.active.fg_stroke = Stroke::new(1.0, INK);

        v.widgets.open.bg_fill = SURFACE_2;
        v.widgets.open.weak_bg_fill = SURFACE_2;
        v.widgets.open.fg_stroke = Stroke::new(1.0, INK);

        // 只有一款字体：拉丁字母和数字走等宽（数字能对齐），中文交给上面的回退。
        style.text_styles = [
            (TextStyle::Heading, FontId::monospace(12.0)),
            (TextStyle::Body, FontId::monospace(11.5)),
            (TextStyle::Monospace, FontId::monospace(11.5)),
            (TextStyle::Button, FontId::monospace(11.0)),
            (TextStyle::Small, FontId::monospace(10.5)),
        ]
        .into();

        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.window_margin = egui::Margin::same(0);
        style.spacing.scroll.bar_width = 8.0;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 徽标配色只有 [`badge_color`] 这一处：同一类型任何地方都同色，
    /// 不同类型要能分得开。这条测试守的就是「别再有人往别处塞一个硬编码颜色」。
    #[test]
    fn badge_colors_are_per_type_and_distinct() {
        let png = badge_color(PortType::Image(ImageFormat::Png));
        let jpg = badge_color(PortType::Image(ImageFormat::Jpeg));
        assert_eq!(
            png, jpg,
            "同为具体图像格式，徽标同色 —— 格式由字说，不靠颜色"
        );

        assert_ne!(png, badge_color(PortType::Text));
        assert_ne!(badge_color(PortType::Text), badge_color(PortType::Number));
        assert_ne!(badge_color(PortType::Number), badge_color(PortType::Bool));

        // 格式未知的图像是通配，和 `Any` 一起走灰 —— 与具体格式的蓝分得开。
        let any_image = badge_color(PortType::Image(ImageFormat::Any));
        assert_ne!(png, any_image);
        assert_eq!(any_image, badge_color(PortType::Any));

        // 徽标色与「选中 / 焦点」的主题蓝分开：一个蓝色徽标不该被误读成“选中”。
        assert_ne!(png, ACCENT);
    }

    /// 中文字体能不能真的解析出汉字字形。
    ///
    /// 这一步值得自动守着：`NotoSansCJK-Regular.ttc` 是字体**集合**，索引取错会静默
    /// 拿到别的字面（能画出来但字形不对，比如把简体画成日文写法），索引越界则解析失败 ——
    /// 两种都不会在启动时报错，只会让界面上出现一片方块或怪字。
    ///
    /// 不需要窗口，任何机器上都能跑。
    #[test]
    fn cjk_font_has_han_glyphs() {
        use ab_glyph::Font;

        let Some((path, index)) = CJK_CANDIDATES
            .iter()
            .find(|(path, _)| std::path::Path::new(path).is_file())
        else {
            eprintln!("跳过：这台机器上没有候选中文字体");
            return;
        };

        let bytes = std::fs::read(path).expect("读字体文件");
        let font = ab_glyph::FontRef::try_from_slice_and_index(&bytes, *index)
            .unwrap_or_else(|e| panic!("{path} 的第 {index} 个字面解析失败：{e}"));

        for c in "重命名图像压缩裁切保存目录输入输出参数".chars() {
            assert_ne!(
                font.glyph_id(c).0,
                0,
                "{path} 第 {index} 个字面里没有「{c}」的字形（多半是索引取错了）"
            );
        }
    }
}
