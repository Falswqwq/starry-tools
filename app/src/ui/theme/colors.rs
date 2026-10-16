//! 淡蓝灰工具界面的颜色令牌；类型与类别颜色直接来自 core 注册表。

use eframe::egui::Color32;
use starrytools_core::{model::port_type::PortType, registry::registry, types::TypeColor};

pub const CANVAS: Color32 = Color32::from_rgb(0xfb, 0xfb, 0xfc);
pub const CANVAS_DOT: Color32 = Color32::from_rgb(0xd7, 0xdb, 0xe1);
pub const SURFACE: Color32 = Color32::WHITE;
pub const INK_ON_COLOR: Color32 = Color32::WHITE;
/// 图片与白色图标遮盖贴图的中性 tint，不参与界面调色。
pub const IMAGE_TINT: Color32 = Color32::WHITE;
pub const SURFACE_2: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf8);
pub const SURFACE_3: Color32 = Color32::from_rgb(0xec, 0xee, 0xf1);
pub const HAIRLINE: Color32 = Color32::from_rgb(0xe4, 0xe7, 0xeb);
pub const HAIRLINE_STRONG: Color32 = Color32::from_rgb(0xcc, 0xd2, 0xda);
pub const INK: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const INK_2: Color32 = Color32::from_rgb(0x4b, 0x55, 0x63);
pub const INK_3: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa1);
pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x8f, 0xb5, 0xf7);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x1d, 0x4e, 0xd8);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xef, 0xf4, 0xff);
pub const ACCENT_LINE: Color32 = Color32::from_rgb(0xc3, 0xd6, 0xfe);
/// 「交互」专色的浅色底与描边 —— core 注册的 `interactive` 专色（紫）派生的 tints，
/// 只用于本地的小标记与光晕。节点边线本身走 `interactive_color()`，不从这里取。
pub const PURPLE_SOFT: Color32 = Color32::from_rgb(0xf5, 0xf1, 0xff);
pub const PURPLE_LINE: Color32 = Color32::from_rgb(0xdd, 0xd0, 0xfb);
pub const DANGER: Color32 = Color32::from_rgb(0xdc, 0x26, 0x26);
pub const DANGER_SOFT: Color32 = Color32::from_rgb(0xfe, 0xf3, 0xf2);
pub const DANGER_LINE: Color32 = Color32::from_rgb(0xf3, 0xc6, 0xc2);
pub const WARN: Color32 = Color32::from_rgb(0xb4, 0x53, 0x09);
pub const WARN_SOFT: Color32 = Color32::from_rgb(0xff, 0xf7, 0xed);
pub const WARN_LINE: Color32 = Color32::from_rgb(0xf3, 0xd3, 0xac);
pub const OK: Color32 = Color32::from_rgb(0x15, 0x80, 0x3d);
pub const WIRE: Color32 = Color32::from_rgb(0xc3, 0xc9, 0xd2);
pub const SHADOW_CARD: Color32 = Color32::from_rgba_unmultiplied_const(0x10, 0x18, 0x28, 0x16);
pub const SHADOW_RAISED: Color32 = Color32::from_rgba_unmultiplied_const(0x10, 0x18, 0x28, 0x1e);
pub const SHADOW_POP: Color32 = Color32::from_rgba_unmultiplied_const(0x10, 0x18, 0x28, 0x22);
pub const SCRIM: Color32 = Color32::from_black_alpha(48);
pub const MASK_DIM: Color32 = Color32::from_black_alpha(120);
pub const PICKER_CURSOR_SHADOW: Color32 = Color32::from_black_alpha(140);
pub const DISABLED_ALPHA: f32 = 0.45;
pub const DRAG_CONTENT_ALPHA: f32 = 0.15;

pub fn category_label(category: &str) -> &str {
    registry()
        .category(category)
        .map_or(category, |info| info.label.as_str())
}

/// 某类**节点**的专色（id → 颜色），颜色由 core 的颜色册注册。
///
/// 分类不参与：分类只把节点分组，颜色说的是「这类节点长这样」。
pub fn accent_color(id: &str) -> Option<Color32> {
    registry().accent(id).map(|accent| type_color(accent.color))
}

/// 「当前会拦住运行」的专色 —— core 注册的 `interactive`（紫）。
/// 颜色册里查不到时退回主题蓝，不至于画出错色。
pub fn interactive_color() -> Color32 {
    accent_color(starrytools_core::registry::INTERACTIVE_ACCENT).unwrap_or(ACCENT)
}

pub fn badge_color(ty: PortType) -> Color32 {
    type_color(ty.color())
}

pub fn type_color(TypeColor([r, g, b]): TypeColor) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn alpha(color: Color32, opacity: f32) -> Color32 {
    // Color32 的通道已预乘；直接缩放，避免透明阴影被再次预乘而变黑。
    color.gamma_multiply(opacity.clamp(0.0, 1.0))
}

pub fn white_alpha(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

pub fn accent_alpha(a: f32) -> Color32 {
    alpha(ACCENT, a)
}
pub fn danger_alpha(a: f32) -> Color32 {
    alpha(DANGER, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use starrytools_core::model::port_type::ImageFormat;

    #[test]
    fn opacity_preserves_premultiplied_colors() {
        assert_eq!(alpha(SHADOW_CARD, 1.0), SHADOW_CARD);
        assert_eq!(alpha(SHADOW_CARD, 0.0), Color32::TRANSPARENT);
        assert_eq!(alpha(ACCENT, 2.0), ACCENT);
        assert_eq!(alpha(ACCENT, -1.0), Color32::TRANSPARENT);
        assert_eq!(white_alpha(255), INK_ON_COLOR);
        assert_eq!(white_alpha(0), Color32::TRANSPARENT);
        for a in [40, 56, 220] {
            assert_eq!(
                white_alpha(a),
                Color32::from_rgba_unmultiplied(255, 255, 255, a)
            );
        }
        for shadow in [SHADOW_CARD, SHADOW_RAISED, SHADOW_POP] {
            assert!(
                shadow.r() <= shadow.a() && shadow.g() <= shadow.a() && shadow.b() <= shadow.a()
            );
        }
    }

    #[test]
    fn registry_colors_are_used_without_ui_mirrors() {
        assert_eq!(type_color(TypeColor([3, 5, 7])), Color32::from_rgb(3, 5, 7));
        // 分类只有名字，没有颜色。
        for category in registry().categories() {
            assert_eq!(category_label(&category.id), category.label);
        }
        assert_eq!(category_label("extension-category"), "extension-category");
        // 节点专色走颜色册：`interactive` 是框架注册的那支紫。
        assert_eq!(interactive_color(), Color32::from_rgb(0x7c, 0x3a, 0xed));
        assert_eq!(accent_color("interactive"), Some(interactive_color()));
        assert_eq!(accent_color("not-registered"), None);
        assert_eq!(
            badge_color(PortType::Image(ImageFormat::Png)),
            badge_color(PortType::Image(ImageFormat::Jpeg))
        );
        assert_ne!(badge_color(PortType::Text), badge_color(PortType::Number));
        assert_ne!(badge_color(PortType::Number), badge_color(PortType::Bool));
        assert_ne!(badge_color(PortType::Image(ImageFormat::Png)), ACCENT);
    }
}
