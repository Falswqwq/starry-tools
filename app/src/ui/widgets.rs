//! 自己画的按钮与表面 —— 因为 egui 的内置按钮给不出原始设计里那几种变体
//! （实心 / 幽灵 / 主色 / 危险），也给不了每个按钮自己的卡片阴影。

use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui::epaint::RectShape;

use crate::ui::theme;

/// 一个图标绘制闭包：往 `rect` 里用 `color` 画。
/// 用 `&dyn` 是为了能传拼接了相位的 loader（普通图标传函数引用就行）。
pub type IconDraw<'a> = &'a dyn Fn(&egui::Painter, Rect, Color32);

/// 按钮的外观变体。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// 白底 + 发丝边 + 卡片阴影（`.ui-btn--solid`）。
    Solid,
    /// 没底没边，悬停才浮出灰底（`.ui-btn--ghost`）。
    Ghost,
    /// 蓝底白字（`.ui-btn--primary`）。
    Primary,
    /// 红字红边（`.ui-btn--danger`）。
    Danger,
}

/// 按钮的尺寸。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// 常规：左右 11、高 27。
    Md,
    /// 小一号：左右 9、高 25。
    Sm,
    /// 方形图标按钮：29×29。
    Icon,
    /// 小方形图标按钮：21×21。
    IconSm,
}

impl Size {
    fn square(self) -> Option<f32> {
        match self {
            Size::Icon => Some(29.0),
            Size::IconSm => Some(21.0),
            _ => None,
        }
    }

    fn height(self) -> f32 {
        match self {
            // 顶栏一排按钮取同一个高度，芯片和图标按钮才能对齐。
            Size::Md => 29.0,
            Size::Sm => 25.0,
            Size::Icon => 29.0,
            Size::IconSm => 21.0,
        }
    }

    fn pad_x(self) -> f32 {
        match self {
            Size::Md => 11.0,
            Size::Sm => 9.0,
            _ => 0.0,
        }
    }

    fn icon_box(self) -> f32 {
        match self {
            Size::Md | Size::Icon => 14.0,
            Size::Sm | Size::IconSm => 12.0,
        }
    }
}

/// 浮层统一外壳：白底 + 发丝边 + 卡片圆角 + 大投影（`.popover`）。
pub fn panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::HAIRLINE))
        .corner_radius(CornerRadius::same(theme::R_CARD))
        .shadow(theme::shadow_pop())
}

/// 卡片阴影（`--shadow-card`）：一层小模糊。
pub fn card_shadow(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.add(
        RectShape::filled(
            rect.translate(Vec2::new(0.0, 1.0)),
            CornerRadius::same(radius),
            Color32::from_rgba_premultiplied(0x10, 0x18, 0x28, 0x16),
        )
        .with_blur_width(3.0),
    );
}

/// 把一段已排版文本的**墨迹**竖直中心对到 `center_y` 时，该给的左上角 y。
///
/// egui 的 galley 高度含字体上下留白：汉字落在框里偏下（`运行`@11 比框中低约 1.5px），
/// 而拉丁大写偏上（`PNG`@8.5 比框中高约 1px）。若两个元素各自按框高居中，
/// 汉字那一侧就会显得「下端对齐」（图标 / 胶囊与汉字标签对不齐）。按墨迹居中才对得齐。
pub fn ink_top(galley: &egui::Galley, center_y: f32) -> f32 {
    let center = if galley.mesh_bounds.height() > 0.0 {
        galley.mesh_bounds.center().y
    } else {
        galley.size().y * 0.5
    };
    center_y - center
}

/// 一段文本墨迹的竖直中心（相对 galley 顶部的偏移量）。
pub fn ink_center(galley: &egui::Galley) -> f32 {
    if galley.mesh_bounds.height() > 0.0 {
        galley.mesh_bounds.center().y
    } else {
        galley.size().y * 0.5
    }
}

/// 单行按墨迹居中；会折行的多行文本退回 `fallback_top`，
/// 免得整块被往上顶出自己那一行、蹭到上一行。
pub fn label_top(galley: &egui::Galley, center_y: f32, fallback_top: f32) -> f32 {
    if galley.rows.len() <= 1 {
        ink_top(galley, center_y)
    } else {
        fallback_top
    }
}

/// 一条交叉过渡的两段相位：前半段 `text` 淡出，后半段 `icon` 淡入 —— 两者不重叠。
///
/// 用于「文字 → 箭头」的替换：先看见字消失，再看见箭头浮出，途中不会糊在一起。
pub fn overlay_phases(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let text = (1.0 - t * 2.0).clamp(0.0, 1.0);
    let icon = ((t - 0.5) * 2.0).clamp(0.0, 1.0);
    (text, icon)
}

fn fade(color: Color32, factor: f32) -> Color32 {
    let a = (f32::from(color.a()) * factor) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// 一个自绘按钮。`label` 为空且给了图标时就是一个方形图标按钮。
pub fn button(
    ui: &mut Ui,
    label: &str,
    icon: Option<IconDraw<'_>>,
    variant: Variant,
    size: Size,
    enabled: bool,
) -> Response {
    button_ex(ui, label, icon, None, variant, size, enabled)
}

/// 盖在按钮**文字上**的图标：用于「文字 → 箭头」的交叉淡入 + 翻转。
///
/// 它不占额外宽度 —— 只是盖在原来的文字位置上，所以按钮不会因为多出一个箭头而变形。
pub struct Overlay<'a> {
    pub icon: IconDraw<'a>,
    /// 0 = 只看得到文字，1 = 只看得到图标。
    pub t: f32,
}

/// 带一个「盖住文字的图标」的按钮（比如展开时盖住「节点库」的那个下箭头）。
#[allow(clippy::too_many_arguments)]
pub fn button_ex(
    ui: &mut Ui,
    label: &str,
    icon: Option<IconDraw<'_>>,
    overlay: Option<Overlay<'_>>,
    variant: Variant,
    size: Size,
    enabled: bool,
) -> Response {
    let font = egui::FontId::monospace(if size == Size::Sm { 10.5 } else { 11.0 });
    let color = egui::Color32::PLACEHOLDER;
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    let text_h = galley.size().y;
    let text_w = if label.is_empty() {
        0.0
    } else {
        galley.size().x
    };
    // 有文字时，图标方块按文字行高取 —— 两者上下沿完全对齐。
    let icon_box = if label.is_empty() {
        size.icon_box()
    } else {
        text_h.max(12.0)
    };
    let gap = if icon.is_some() && !label.is_empty() {
        5.0
    } else {
        0.0
    };
    let icon_w = if icon.is_some() { icon_box + gap } else { 0.0 };
    let height = size.height();

    let width = match size.square() {
        Some(side) => side,
        None => size.pad_x() * 2.0 + icon_w + text_w,
    };

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    if !enabled {
        // 光标不变，也不给悬停反馈。
    } else if resp.has_focus() || resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let hovered = enabled && resp.hovered();
    let (fill, stroke, fg) = match (variant, hovered) {
        (Variant::Solid, false) => (theme::SURFACE, theme::HAIRLINE, theme::INK_2),
        (Variant::Solid, true) => (theme::SURFACE, theme::HAIRLINE_STRONG, theme::INK),
        (Variant::Ghost, false) => (Color32::TRANSPARENT, Color32::TRANSPARENT, theme::INK_2),
        (Variant::Ghost, true) => (theme::SURFACE_3, Color32::TRANSPARENT, theme::INK),
        (Variant::Primary, false) => (theme::ACCENT, theme::ACCENT, Color32::WHITE),
        (Variant::Primary, true) => (theme::ACCENT_HOVER, theme::ACCENT_HOVER, Color32::WHITE),
        (Variant::Danger, false) => (Color32::TRANSPARENT, theme::DANGER_LINE, theme::DANGER),
        (Variant::Danger, true) => (theme::DANGER_SOFT, theme::DANGER, theme::DANGER),
    };
    let factor = if enabled { 1.0 } else { 0.45 };

    let painter = ui.painter();
    if variant == Variant::Solid && enabled {
        card_shadow(painter, rect, theme::R_CTL);
    }
    let cr = CornerRadius::same(theme::R_CTL);
    if fill.a() > 0 {
        painter.rect_filled(rect, cr, fade(fill, factor));
    }
    if stroke.a() > 0 {
        painter.rect_stroke(
            rect,
            cr,
            Stroke::new(1.0, fade(stroke, factor)),
            StrokeKind::Inside,
        );
    }

    let fg = fade(fg, factor);
    // 文字淡出在前半、图标淡入在后半，两段不重叠。
    let t = overlay
        .as_ref()
        .map_or(0.0, |overlay| overlay.t.clamp(0.0, 1.0));
    let (text_alpha, icon_alpha) = overlay_phases(t);
    let content_w = icon_w + text_w;
    let cy = rect.center().y;
    let mut x = rect.center().x - content_w * 0.5;
    if let Some(icon) = icon {
        let box_rect =
            Rect::from_min_size(Pos2::new(x, cy - icon_box * 0.5), Vec2::splat(icon_box));
        icon(painter, box_rect, fg);
        x += icon_box + gap;
    }
    if !label.is_empty() {
        let label_rect =
            Rect::from_min_size(Pos2::new(x, cy - text_h * 0.5), Vec2::new(text_w, text_h));
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::FontId::monospace(if size == Size::Sm { 10.5 } else { 11.0 }),
            fade(fg, text_alpha),
        );
        // 按墨迹居中，图标和汉字才对得齐（见 `ink_top`）。
        painter.galley(
            Pos2::new(x, ink_top(&galley, cy)),
            galley,
            fade(fg, text_alpha),
        );
        // 图标盖在文字这一块上（不额外撑宽按钮）。
        if let Some(overlay) = overlay {
            if icon_alpha > 0.01 {
                let side = label_rect.height().max(12.0);
                let box_rect = Rect::from_center_size(label_rect.center(), Vec2::splat(side));
                (overlay.icon)(painter, box_rect, fade(fg, icon_alpha));
            }
        }
    }

    resp
}

/// 便捷：一个方形图标按钮里，图标该占的方块。
pub fn icon_rect(rect: Rect, box_size: f32) -> Rect {
    Rect::from_center_size(rect.center(), Vec2::splat(box_size))
}

/// 自己画的开关（轨道 + 滑动的圆点）。
///
/// `rect` 是**整行的可点区域**：轨道画在它的**右侧**、高度按 `rect.height()` 定 ——
/// 所以节点里的参数开关和设置面板里的开关是同一份实现，缩放 / 大小一致。
/// 返回这一下有没有把它翻过来。
pub fn switch(ui: &mut Ui, id: egui::Id, rect: Rect, value: &mut bool) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.has_focus() || resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let changed = resp.clicked();
    if changed {
        *value = !*value;
    }
    let t = ui.ctx().animate_bool_with_time(id, *value, 0.16);

    let height = rect.height() * 0.8;
    let width = (height * 1.8).min(rect.width());
    let track = Rect::from_min_size(
        Pos2::new(rect.right() - width, rect.center().y - height * 0.5),
        Vec2::new(width, height),
    );
    let radius = CornerRadius::same((height * 0.5) as u8);
    let accent = if *value {
        theme::ACCENT
    } else {
        theme::HAIRLINE_STRONG
    };

    let painter = ui.painter();
    painter.rect_filled(
        track,
        radius,
        if *value {
            theme::ACCENT
        } else {
            theme::SURFACE_3
        },
    );
    painter.rect_stroke(track, radius, Stroke::new(1.0, accent), StrokeKind::Inside);

    let pad = height * 0.15;
    let dot = height * 0.5 - pad;
    let travel = (track.width() - 2.0 * (dot + pad)).max(0.0);
    let x = track.left() + dot + pad + travel * t;
    painter.circle_filled(Pos2::new(x, track.center().y), dot, theme::SURFACE);

    changed
}

/// 自己画的滑杆：左边一根轨道、右边一个读数。
///
/// `rect` 是整行的可点区域（节点里会乘过缩放）。返回 `Some(新值)` 表示这一下改了它。
/// 读数按**最大值**的那份文本预留宽度，所以拖动时轨道和数字都不左右跳。
#[allow(clippy::too_many_arguments)]
pub fn slider(
    ui: &mut Ui,
    id: egui::Id,
    rect: Rect,
    value: f32,
    min: f32,
    max: f32,
    integer: bool,
    unit: &str,
) -> Option<f32> {
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    if resp.has_focus() || resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let font = egui::FontId::monospace((rect.height() * 0.46).max(8.0));
    let painter = ui.painter();
    // 按最大值的文本量一遍宽 —— 轨道宽度定死，数字变长也不滑动。
    let sample = format!("{}{}", slider_text(max, integer), unit);
    let readout_w = painter
        .layout_no_wrap(sample, font.clone(), theme::INK_2)
        .size()
        .x;

    let gap = rect.height() * 0.4;
    let bar_h = (rect.height() * 0.28).max(3.0);
    let radius = bar_h * 0.5;
    let handle_r = (rect.height() * 0.30).max(5.0);
    let cy = rect.center().y;
    let track = Rect::from_min_max(
        Pos2::new(rect.left() + handle_r, cy - bar_h * 0.5),
        Pos2::new(
            (rect.right() - readout_w - gap).max(rect.left() + handle_r * 2.0),
            cy + bar_h * 0.5,
        ),
    );

    let span = (max - min).max(f32::EPSILON);
    let mut value_out = value.clamp(min, max);
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
            let mut next = min + t * span;
            if integer {
                next = next.round();
            }
            value_out = next.clamp(min, max);
        }
    }

    let value_t = ((value_out - min) / span).clamp(0.0, 1.0);
    let handle_x = track.left() + track.width() * value_t;

    // 轨道：未填充部分浅灰，已填充部分主题蓝。
    let round = CornerRadius::same(radius as u8);
    painter.rect_filled(track, round, theme::SURFACE_3);
    if value_t > 0.0 {
        let filled = Rect::from_min_max(track.min, Pos2::new(handle_x, track.max.y));
        painter.rect_filled(filled, round, theme::ACCENT);
    }

    // 圆点：拖 / 悬停时描主题蓝，平静时只是白底加一圈发丝线。
    let active = resp.dragged() || resp.hovered();
    let border = if active {
        theme::ACCENT
    } else {
        theme::HAIRLINE_STRONG
    };
    let knob = Rect::from_center_size(Pos2::new(handle_x, cy), Vec2::splat(handle_r * 2.0));
    let knob_radius = CornerRadius::same(handle_r as u8);
    painter.rect_filled(knob, knob_radius, theme::SURFACE);
    painter.rect_stroke(
        knob,
        knob_radius,
        Stroke::new(if active { 2.0 } else { 1.0 }, border),
        StrokeKind::Inside,
    );

    let galley = painter.layout_no_wrap(
        format!("{}{}", slider_text(value_out, integer), unit),
        font,
        theme::INK_2,
    );
    painter.galley(
        Pos2::new(rect.right() - galley.size().x, cy - galley.size().y * 0.5),
        galley,
        theme::INK_2,
    );

    ((value_out - value).abs() > f32::EPSILON).then_some(value_out)
}

/// 滑杆读数：整数就去零取整，否则保留最多两位小数。
fn slider_text(value: f32, integer: bool) -> String {
    if integer || value.fract() == 0.0 {
        format!("{}", value.round() as i64)
    } else {
        let text = format!("{value:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
