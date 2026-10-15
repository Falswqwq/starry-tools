//! 设计令牌 —— 浅色、极简：内容只用黑 / 白 / 灰，蓝是唯一的主题色。卡片与各种浮层共用
//! 同一套表面（白底 + 发丝边 + 同一个圆角和阴影），整块屏幕看起来才是一件东西。

use eframe::egui::{self, CornerRadius, FontId, Stroke, TextStyle};
use egui::epaint::Shadow;

mod colors;
mod fonts;
mod metrics;
pub use colors::*;
pub use fonts::{FONT_BUTTON, FONT_CONTROL, FONT_HEADING, FONT_SMALL};
pub use metrics::*;

pub fn shadow_pop() -> Shadow {
    Shadow {
        offset: POP_SHADOW_OFFSET,
        blur: POP_SHADOW_BLUR,
        spread: 0,
        color: SHADOW_POP,
    }
}

/// 把令牌灌进 egui 的 `Visuals` / `Spacing`，让默认控件也长得对。
pub fn apply(ctx: &egui::Context) {
    fonts::install(ctx);
    ctx.set_theme(egui::Theme::Light);

    ctx.style_mut_of(egui::Theme::Light, |style| {
        style.animation_time = crate::ui::motion::QUICK;
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
            (TextStyle::Heading, FontId::monospace(FONT_HEADING)),
            (TextStyle::Body, FontId::monospace(FONT_CONTROL)),
            (TextStyle::Monospace, FontId::monospace(FONT_CONTROL)),
            (TextStyle::Button, FontId::monospace(FONT_BUTTON)),
            (TextStyle::Small, FontId::monospace(FONT_SMALL)),
        ]
        .into();

        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(CONTROL_PAD_X, CONTROL_PAD_Y);
        style.spacing.interact_size.y = CONTROL_H;
        style.spacing.window_margin = egui::Margin::same(0);
        style.spacing.scroll.bar_width = 8.0;
    });
}
