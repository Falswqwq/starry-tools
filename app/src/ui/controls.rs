//! 参数控件与颜色系统。
//!
//! 「一个参数长什么样、怎么交互」的组件都在这里：数字 / 文本 / 下拉 / 开关 / 文件 /
//! 颜色，以及它们共用的外壳、取色器和取色条。节点只要声明一个 `ParamSpec`，界面就按
//! 同一套映射长出控件 —— 和具体工具无关，也和画布无关。`graph` 那边只剩画布自己的事。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
};
use serde_json::Value;
use starrytools_core::model::params::Params;

use crate::catalog::{Control, Param};
use crate::ui::icons;
use crate::ui::theme;
use crate::ui::widgets;

/// 单个控件一行的高度（流坐标）—— 取色条也用它。
pub(crate) const PARAM_CONTROL_H: f32 = 24.0;

/// 一个控件这一帧发生了什么。
///
/// 文件框**不在这里弹对话框** —— 弹原生框会阻塞整帧，所以只把「用户想选文件」
/// 这件事报出来，由画布拿后台选择器去开（见 [`crate::state::dialog`]）。
pub(crate) enum ControlEvent {
    /// 参数被改了。
    Changed(Value),
    /// 用户点了文件 / 目录框，要开一个选择框。
    PickFile {
        title: String,
        extensions: Vec<String>,
        directory: bool,
    },
}

/// 画一个参数的控件；返回值表示这一帧它发生了什么。
pub(crate) fn control(
    ui: &mut egui::Ui,
    node_id: &str,
    param: &Param,
    rect: Rect,
    params: &Params,
    zoom: f32,
    disabled: bool,
) -> Option<ControlEvent> {
    // 控件的 id 用**节点自己的 id**（而不是下标）—— 节点置顶 / 删除会重排下标，
    // 用下标做 id 会让同一个节点的控件被当成新控件，正在编辑的文本框、展开的下拉都会丢状态。
    let id = ui.id().with(("param", node_id, param.id.as_str()));
    let current = params.get(&param.id);

    match &param.control {
        Control::Bool => {
            let mut value = current.and_then(Value::as_bool).unwrap_or(false);
            // 禁用态：开关还是开关，只是灰下去、不再响应 —— 而不是换成一个灰块。
            if disabled {
                toggle_disabled(ui.painter(), rect, value);
                return None;
            }
            widgets::switch(ui, id, rect, &mut value)
                .then(|| ControlEvent::Changed(serde_json::json!(value)))
        }

        Control::Number {
            min,
            max,
            integer,
            unit,
            ..
        } => number_field(
            ui,
            id,
            rect,
            current.and_then(Value::as_f64).unwrap_or(*min),
            *min,
            *max,
            *integer,
            unit.as_deref(),
            zoom,
            disabled,
        )
        .map(ControlEvent::Changed),

        Control::Slider {
            min,
            max,
            integer,
            unit,
            ..
        } => {
            let value = current.and_then(Value::as_f64).unwrap_or(*min);
            if disabled {
                disabled_shell(ui.painter(), rect, theme::R_CTL);
                muted_value(
                    ui.painter(),
                    rect,
                    &format!(
                        "{}{}",
                        format_number(value, *integer),
                        unit.as_deref().unwrap_or("")
                    ),
                    zoom,
                );
                return None;
            }
            widgets::slider(
                ui,
                id,
                rect,
                value as f32,
                *min as f32,
                *max as f32,
                *integer,
                unit.as_deref().unwrap_or(""),
            )
            .map(|next| ControlEvent::Changed(serde_json::json!(next as f64)))
        }

        Control::Text {
            multiline,
            placeholder,
        } => text_field(
            ui,
            id,
            rect,
            current,
            *multiline,
            placeholder.as_deref(),
            zoom,
            disabled,
        )
        .map(ControlEvent::Changed),

        Control::Select { options } => {
            select_field(ui, id, rect, current, options, zoom, disabled).map(ControlEvent::Changed)
        }

        Control::Color => {
            color_field(ui, id, rect, current, zoom, disabled).map(ControlEvent::Changed)
        }

        Control::File { .. } => file_field(ui, id, rect, current, &param.control, zoom, disabled),
    }
}

/// 控件外壳：一块圆角底色 + 发丝边。可编辑 / 禁用两种状态只是底色不同，
/// 所以共用同一个函数 —— 想调边线或圆角只改这里一处。
pub(crate) fn shell(painter: &egui::Painter, rect: Rect, radius: u8, fill: Color32) {
    painter.rect_filled(rect, CornerRadius::same(radius), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );
}

/// 禁用态的外壳：灰底 + 发丝边。参数被上游接管（或控件本身不可用时）用它，
/// 控件还是控件的样子，只是不再响应、颜色弱下去 —— 不再用统一灰块代替。
pub(crate) fn disabled_shell(painter: &egui::Painter, rect: Rect, radius: u8) {
    shell(painter, rect, radius, theme::SURFACE_2);
}

/// 禁用态里那行读不切的字：左对齐，和可编辑时的内边距一致，剪在框里。
pub(crate) fn muted_value(painter: &egui::Painter, rect: Rect, text: &str, zoom: f32) {
    let inner = rect.shrink2(egui::vec2(8.0 * zoom, 0.0));
    painter.with_clip_rect(inner).text(
        egui::pos2(inner.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(11.5 * zoom),
        theme::INK_3,
    );
}

pub(crate) fn input_shell(painter: &egui::Painter, rect: Rect, radius: u8) {
    shell(painter, rect, radius, theme::SURFACE);
}

/// 悬停 / 聚焦时叠上去的那圈边（画在内容之上，不会盖住字）。
pub(crate) fn input_shell_state(
    painter: &egui::Painter,
    rect: Rect,
    radius: u8,
    hovered: bool,
    focused: bool,
) {
    let color = if focused {
        theme::ACCENT
    } else if hovered {
        theme::HAIRLINE_STRONG
    } else {
        return;
    };
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}

/// 数字 → 文本。整数不留小数点，小数最多两位。
pub(crate) fn format_number(value: f64, integer: bool) -> String {
    if integer {
        format!("{}", value.round() as i64)
    } else {
        let rounded = (value * 100.0).round() / 100.0;
        if rounded.fract() == 0.0 {
            format!("{}", rounded as i64)
        } else {
            format!("{rounded}")
        }
    }
}

/// 数字输入框：带边框的可输入框 + 可选的单位小框。
#[allow(clippy::too_many_arguments)]
pub(crate) fn number_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    value: f64,
    min: f64,
    max: f64,
    integer: bool,
    unit: Option<&str>,
    zoom: f32,
    disabled: bool,
) -> Option<Value> {
    let radius = theme::R_CTL;
    let unit_w = unit.map_or(0.0, |text| {
        text.chars().count() as f32 * 7.0 * zoom + 16.0 * zoom
    });
    let gap = if unit_w > 0.0 { 4.0 * zoom } else { 0.0 };
    let input = Rect::from_min_size(
        rect.min,
        egui::vec2((rect.width() - unit_w - gap).max(10.0), rect.height()),
    );

    // 禁用态：数字框还是数字框，只是灰下去、不再可编辑。
    if disabled {
        disabled_shell(ui.painter(), input, radius);
        muted_value(ui.painter(), input, &format_number(value, integer), zoom);
        if let Some(unit) = unit {
            let box_rect = Rect::from_min_size(
                egui::pos2(input.right() + gap, rect.top()),
                egui::vec2(unit_w, rect.height()),
            );
            disabled_shell(ui.painter(), box_rect, radius);
            ui.painter().text(
                box_rect.center(),
                Align2::CENTER_CENTER,
                unit,
                FontId::monospace(11.0 * zoom),
                theme::INK_3,
            );
        }
        return None;
    }

    input_shell(ui.painter(), input, radius);

    // 正在编辑时用草稿；不在编辑就跟着外部值走 ——
    // 这样既能敲 ".5" 这种中间状态，载入工作流时也能立刻跟过去。
    let editing = ui.ctx().memory(|memory| memory.focused()) == Some(id);
    let shown = format_number(value, integer);
    let mut draft = if editing {
        ui.data_mut(|data| data.get_temp::<String>(id).unwrap_or_else(|| shown.clone()))
    } else {
        shown
    };
    let inner = input.shrink2(egui::vec2(8.0 * zoom, 3.0 * zoom));
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(&mut draft)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::monospace(11.5 * zoom))
            .desired_width(inner.width()),
    );
    if ui.ctx().memory(|memory| memory.focused()) == Some(id) {
        ui.data_mut(|data| data.insert_temp(id, draft.clone()));
    }
    input_shell_state(
        ui.painter(),
        input,
        radius,
        resp.hovered(),
        resp.has_focus(),
    );

    if let Some(unit) = unit {
        let box_rect = Rect::from_min_size(
            egui::pos2(input.right() + gap, rect.top()),
            egui::vec2(unit_w, rect.height()),
        );
        input_shell(ui.painter(), box_rect, radius);
        ui.painter().text(
            box_rect.center(),
            Align2::CENTER_CENTER,
            unit,
            FontId::monospace(11.0 * zoom),
            theme::INK_3,
        );
    }

    if resp.changed() {
        if let Ok(parsed) = draft.trim().parse::<f64>() {
            let mut value = parsed.clamp(min, max);
            if integer {
                value = value.round();
            }
            return Some(serde_json::json!(value));
        }
    }
    None
}

/// 文本输入框：带边框的可输入框（多行则高一些）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    multiline: bool,
    placeholder: Option<&str>,
    zoom: f32,
    disabled: bool,
) -> Option<Value> {
    let radius = theme::R_CTL;
    let value = current.and_then(Value::as_str).unwrap_or("");

    // 禁用态：文本框还是文本框，只是灰下去、不再可编辑。
    if disabled {
        disabled_shell(ui.painter(), rect, radius);
        let inner = rect.shrink2(egui::vec2(8.0 * zoom, 3.0 * zoom));
        let painter = ui.painter().with_clip_rect(inner);
        let galley = painter.layout(
            value.to_string(),
            FontId::monospace(11.5 * zoom),
            theme::INK_3,
            inner.width(),
        );
        painter.galley(inner.min, galley, theme::INK_3);
        return None;
    }

    input_shell(ui.painter(), rect, radius);

    let mut value = value.to_string();
    let inner = rect.shrink2(egui::vec2(8.0 * zoom, 3.0 * zoom));
    // 内容超过最多行数：框不再长高，改成框内滚动。
    let scrolling = multiline && crate::canvas::layout::multiline_scrolls(&value);
    // 滚动时右侧要让出一条滚动条的位置，文字按窄一点折行，免得被挡。
    let text_width = if scrolling {
        (inner.width() - 12.0 * zoom).max(40.0)
    } else {
        inner.width()
    };
    let mut widget = if multiline {
        // 行数跟着内容走（含换行后的折行）—— 长文本会把框撑高，不会溢出下边框。
        // 用和估高同一套行数（见 `layout::multiline_rows`），两边才对得上。
        let rows = crate::canvas::layout::multiline_rows(&value);
        egui::TextEdit::multiline(&mut value).desired_rows(rows)
    } else {
        egui::TextEdit::singleline(&mut value)
    };
    if let Some(hint) = placeholder {
        widget = widget.hint_text(hint);
    }
    let widget = widget
        .id(id)
        .frame(egui::Frame::NONE)
        .font(FontId::monospace(11.5 * zoom))
        .desired_width(text_width);
    let resp = if scrolling {
        // 把文本框放进一个纵向滚动区，高度定死在框的大小 —— 内容再多就在框内滚。
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| ui.add(widget))
                .inner
        })
        .inner
    } else {
        ui.put(inner, widget)
    };
    input_shell_state(ui.painter(), rect, radius, resp.hovered(), resp.has_focus());

    if resp.changed() {
        Some(serde_json::json!(value))
    } else {
        None
    }
}

/// 颜色控件（收起时）那条色条的高度（流坐标）。
pub(crate) const COLOR_BAR_H: f32 = PARAM_CONTROL_H;
/// 弹出的取色器宽度与内部各段高度（屏幕像素，不跟缩放走 —— 取色要看得清）。
pub(crate) const PICKER_W: f32 = 236.0;
pub(crate) const PICKER_SV_H: f32 = 132.0;
pub(crate) const PICKER_HUE_H: f32 = 16.0;

/// 颜色控件占多高（流坐标）—— 收起时就是一条色条。
pub(crate) fn color_height() -> f32 {
    COLOR_BAR_H
}

/// 颜色控件：收起时是一条显示当前颜色的色条，点开是一个**通用取色器**
/// —— 取色区（x = 饱和度，y = 亮度）、色相条、R/G/B、Hex、不透明度，双向同步。
///
/// 通用可复用：哪个节点声明一个 `Color` 参数就能用上它。
pub(crate) fn color_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    zoom: f32,
    disabled: bool,
) -> Option<Value> {
    let rgba = color_rgba(current.and_then(Value::as_str).unwrap_or("#000000"));
    let radius = CornerRadius::same(theme::R_CTL);

    let resp = ui.interact(rect, id.with("bar"), Sense::click());
    if !disabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fade = if disabled { 0.4 } else { 1.0 };
    paint_color_chip(ui.painter(), rect, rgba, radius, fade);
    // 色条上把 hex 写出来，不点开也一眼看得到是什么颜色。
    ui.painter().text(
        egui::pos2(rect.right() - 8.0 * zoom, rect.center().y),
        Align2::RIGHT_CENTER,
        color_string(rgba),
        FontId::monospace(10.0 * zoom),
        readable_ink(rgba).gamma_multiply(fade),
    );

    if disabled {
        return None;
    }

    let ctx = ui.ctx().clone();
    let open_id = id.with("open");
    let mut open = temp_bool(&ctx, open_id).unwrap_or(false);
    if resp.clicked() {
        open = !open;
    }

    let changed = if open {
        color_picker(ui, id, rect, rgba)
    } else {
        None
    };

    // 点别处就收起来。点在色条上不算「别处」—— 那是在切换开合。
    if open && ctx.input(|input| input.pointer.any_click()) {
        if let Some(point) = ctx.input(|input| input.pointer.interact_pos()) {
            let inside_bar = rect.contains(point);
            let inside_popup =
                temp_rect(&ctx, id.with("popup-rect")).is_some_and(|r| r.contains(point));
            if !inside_bar && !inside_popup {
                open = false;
            }
        }
    }
    ctx.data_mut(|data| data.insert_temp(open_id, open));

    changed
}

/// 弹出的取色器。返回 `Some(值)` 表示这一帧把颜色改了。
pub(crate) fn color_picker(
    ui: &mut egui::Ui,
    id: egui::Id,
    bar: Rect,
    rgba: [u8; 4],
) -> Option<Value> {
    let ctx = ui.ctx().clone();
    let hue_id = id.with("hue");
    let (h, s, _) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);
    // 灰色的颜色里 h 无从得知，用上一次记住的色相，取色区才不会蹦到红。
    let mut hue = if s > 0.0 {
        h
    } else {
        temp_f32(&ctx, hue_id).unwrap_or(h)
    };

    let original = rgba;
    let mut rgba = rgba;
    let mut dirty = false;

    let area = egui::Area::new(id.with("popup"))
        .order(egui::Order::Tooltip)
        .fixed_pos(egui::pos2(bar.left(), bar.bottom() + 6.0))
        .show(&ctx, |ui| {
            widgets::panel_frame().show(ui, |ui| {
                ui.set_width(PICKER_W);
                egui::Frame::default()
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        let inner_w = PICKER_W - 20.0;
                        let (_, sat, val) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);

                        // ---- 取色区：x = 饱和度，y = 亮度 ----
                        let (sv, sv_resp) = ui.allocate_exact_size(
                            egui::vec2(inner_w, PICKER_SV_H),
                            Sense::click_and_drag(),
                        );
                        draw_sv_square(ui.painter(), sv, hue);
                        if let Some(point) = dragged_point(&sv_resp) {
                            let new_sat = ((point.x - sv.left()) / sv.width()).clamp(0.0, 1.0);
                            let new_val =
                                1.0 - ((point.y - sv.top()) / sv.height()).clamp(0.0, 1.0);
                            let (r, g, b) = hsv_to_rgb(hue, new_sat, new_val);
                            rgba = [r, g, b, rgba[3]];
                            dirty = true;
                        }
                        let cursor = egui::pos2(
                            sv.left() + sat * sv.width(),
                            sv.top() + (1.0 - val) * sv.height(),
                        );
                        ui.painter()
                            .circle_stroke(cursor, 5.0, Stroke::new(2.0, Color32::WHITE));
                        ui.painter().circle_stroke(
                            cursor,
                            6.5,
                            Stroke::new(1.0, Color32::from_black_alpha(140)),
                        );

                        ui.add_space(8.0);

                        // ---- 色相条 ----
                        let (hue_rect, hue_resp) = ui.allocate_exact_size(
                            egui::vec2(inner_w, PICKER_HUE_H),
                            Sense::click_and_drag(),
                        );
                        draw_hue_bar(ui.painter(), hue_rect);
                        if let Some(point) = dragged_point(&hue_resp) {
                            hue = ((point.x - hue_rect.left()) / hue_rect.width()).clamp(0.0, 1.0)
                                * 360.0;
                            let (_, sat, val) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);
                            let (r, g, b) = hsv_to_rgb(hue, sat, val);
                            rgba = [r, g, b, rgba[3]];
                            dirty = true;
                        }
                        let marker_x = hue_rect.left() + (hue / 360.0) * hue_rect.width();
                        ui.painter().rect_stroke(
                            Rect::from_center_size(
                                egui::pos2(marker_x, hue_rect.center().y),
                                egui::vec2(5.0, hue_rect.height() + 4.0),
                            ),
                            CornerRadius::same(3),
                            Stroke::new(2.0, Color32::WHITE),
                            StrokeKind::Outside,
                        );

                        ui.add_space(10.0);

                        // ---- Hex + 不透明度 ----
                        picker_row(ui, 22.0, |ui, row| {
                            picker_label(ui, row, 0.0, "Hex");
                            let hex_rect = Rect::from_min_size(
                                egui::pos2(row.left() + 30.0, row.top()),
                                egui::vec2(84.0, row.height()),
                            );
                            if let Some(text) =
                                hex_field(ui, id.with("hex"), hex_rect, &color_string(rgba))
                            {
                                rgba = color_rgba(&text);
                                dirty = true;
                            }
                            let alpha_rect = Rect::from_min_size(
                                egui::pos2(hex_rect.right() + 8.0, row.top()),
                                egui::vec2(
                                    (row.right() - hex_rect.right() - 8.0).max(40.0),
                                    row.height(),
                                ),
                            );
                            if let Some(next) = widgets::slider(
                                ui,
                                id.with("alpha"),
                                alpha_rect,
                                rgba[3] as f32,
                                0.0,
                                255.0,
                                true,
                                "",
                            ) {
                                rgba[3] = next.round().clamp(0.0, 255.0) as u8;
                                dirty = true;
                            }
                        });

                        ui.add_space(6.0);

                        // ---- R / G / B ----
                        picker_row(ui, 22.0, |ui, row| {
                            for (index, label) in ["R", "G", "B"].into_iter().enumerate() {
                                let x = row.left() + index as f32 * 72.0;
                                picker_label(ui, row, index as f32 * 72.0, label);
                                let field = Rect::from_min_size(
                                    egui::pos2(x + 14.0, row.top()),
                                    egui::vec2(42.0, row.height()),
                                );
                                if let Some(value) = number_field(
                                    ui,
                                    id.with(label),
                                    field,
                                    rgba[index] as f64,
                                    0.0,
                                    255.0,
                                    true,
                                    None,
                                    1.0,
                                    false,
                                ) {
                                    if let Some(byte) = value.as_u64() {
                                        rgba[index] = byte as u8;
                                        dirty = true;
                                    }
                                }
                            }
                        });
                    });
            });
        });
    ctx.data_mut(|data| data.insert_temp(id.with("popup-rect"), area.response.rect));

    // 记住色相供下次使用（全灰的颜色里 h 丢了）。
    let (h, s, _) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);
    let remembered = if s > 0.0 { h } else { hue };
    ctx.data_mut(|data| data.insert_temp(hue_id, remembered));

    if dirty && color_string(rgba) != color_string(original) {
        Some(serde_json::json!(color_string(rgba)))
    } else {
        None
    }
}

/// 取色器里的一行：把内容包进一个**定死 rect** 的子 scope。
///
/// 里面的输入框用的是 `ui.put`，它会顺手推进父级光标；若直接摊在父级的
/// `horizontal` 里排，每个输入框的宽度会被算两次 —— 一行很快挤爆，字母和框就叠上了。
/// 包一层定死 rect 的 scope 后，父级只按这一行的尺寸推进一次，里面怎么放都不影响外面。
pub(crate) fn picker_row(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui, Rect)) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
        // 占下整行的位置，让父级按这一行的尺寸推进（而不是按内容拼出来的那块）。
        let _ = ui.allocate_rect(row, Sense::hover());
        add(ui, row);
    });
}

/// 取色器一行的左侧小标签（在行内坐标里，`offset` 是相对行左边的偏移）。
pub(crate) fn picker_label(ui: &egui::Ui, row: Rect, offset: f32, text: &str) {
    ui.painter().text(
        egui::pos2(row.left() + offset, row.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(10.0),
        theme::INK_3,
    );
}

/// 取色 / 拖动时指针落在控件上的那一点。
pub(crate) fn dragged_point(resp: &egui::Response) -> Option<Pos2> {
    (resp.dragged() || resp.clicked())
        .then(|| resp.interact_pointer_pos())
        .flatten()
}

/// 取色区：白色→色相 的横向渐变，叠上 透明→黑 的纵向渐变。
/// x 是饱和度、y 是亮度，所以左上角是白、右上角是纯色、底部是黑。
pub(crate) fn draw_sv_square(painter: &egui::Painter, rect: Rect, hue: f32) {
    let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
    let hue_color = Color32::from_rgb(r, g, b);
    let clear = Color32::from_rgba_unmultiplied(0, 0, 0, 0);
    let black = Color32::from_rgb(0, 0, 0);

    let mut horizontal = egui::Mesh::default();
    push_quad(
        &mut horizontal,
        rect,
        [Color32::WHITE, hue_color, hue_color, Color32::WHITE],
    );
    painter.add(egui::Shape::mesh(horizontal));

    let mut vertical = egui::Mesh::default();
    push_quad(&mut vertical, rect, [clear, clear, black, black]);
    painter.add(egui::Shape::mesh(vertical));

    // 取色区 / 色相条不加圆角：渐变是方角的网格，裁不出圆角，索性就方着来。
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );
}

/// 色相条：红→黄→绿→青→蓝→品红→红的一条渐变。
pub(crate) fn draw_hue_bar(painter: &egui::Painter, rect: Rect) {
    const STOPS: [(f32, [u8; 3]); 7] = [
        (0.0, [255, 0, 0]),
        (1.0 / 6.0, [255, 255, 0]),
        (2.0 / 6.0, [0, 255, 0]),
        (3.0 / 6.0, [0, 255, 255]),
        (4.0 / 6.0, [0, 0, 255]),
        (5.0 / 6.0, [255, 0, 255]),
        (1.0, [255, 0, 0]),
    ];
    let mut mesh = egui::Mesh::default();
    for (t, [r, g, b]) in STOPS {
        let x = rect.left() + t * rect.width();
        let color = Color32::from_rgb(r, g, b);
        mesh.vertices
            .push(solid_vertex(egui::pos2(x, rect.top()), color));
        mesh.vertices
            .push(solid_vertex(egui::pos2(x, rect.bottom()), color));
    }
    for pair in (0..STOPS.len() - 1).map(|i| (i as u32 * 2, i as u32 * 2 + 1)) {
        let (a, b) = pair;
        mesh.indices
            .extend_from_slice(&[a, b, a + 3, a, a + 3, a + 2]);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );
}

/// 一个用顶点颜色着色的四边形（不采样贴图，所以 uv 指向字体的白点）。
/// 顶点顺序：左上、右上、右下、左下。
pub(crate) fn push_quad(mesh: &mut egui::Mesh, rect: Rect, colors: [Color32; 4]) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    for (point, color) in corners.into_iter().zip(colors) {
        mesh.vertices.push(solid_vertex(point, color));
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
}

pub(crate) fn solid_vertex(pos: Pos2, color: Color32) -> egui::epaint::Vertex {
    egui::epaint::Vertex {
        pos,
        uv: egui::epaint::WHITE_UV,
        color,
    }
}

/// 把 `rect` 的四个圆角补成 `bg`：直角矩形（棋盘格、图片）会盖到圆角外面，
/// 四角于是露出灰角。这里在四个角各补一小块「方角减四分之一圆」的月亮形，
/// 把溢出圆角的那部分盖回背景色。用一小片顶点色网格拼出来（`radius` 是圆角半径）。
pub(crate) fn mask_corners(painter: &egui::Painter, rect: Rect, radius: f32, bg: Color32) {
    let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    if r <= 0.5 {
        return;
    }
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    // 每个角：方角顶点 + 圆心 + 圆弧的起 / 止角（屏幕 y 向下）。
    let corners = [
        (
            rect.left_top(),
            egui::pos2(rect.left() + r, rect.top() + r),
            PI,
            PI + FRAC_PI_2,
        ),
        (
            rect.right_top(),
            egui::pos2(rect.right() - r, rect.top() + r),
            PI + FRAC_PI_2,
            TAU,
        ),
        (
            rect.right_bottom(),
            egui::pos2(rect.right() - r, rect.bottom() - r),
            0.0,
            FRAC_PI_2,
        ),
        (
            rect.left_bottom(),
            egui::pos2(rect.left() + r, rect.bottom() - r),
            FRAC_PI_2,
            PI,
        ),
    ];

    let steps = ((r * 2.0) as usize).clamp(4, 16);
    let mut mesh = egui::Mesh::default();
    for (corner, center, start, end) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.push(solid_vertex(corner, bg));
        for step in 0..=steps {
            let angle = start + (end - start) * (step as f32 / steps as f32);
            let point = egui::pos2(center.x + r * angle.cos(), center.y + r * angle.sin());
            mesh.vertices.push(solid_vertex(point, bg));
        }
        // 从方角顶点扇形铺到这段圆弧上。
        for step in 0..steps {
            mesh.indices
                .extend_from_slice(&[base, base + 1 + step as u32, base + 2 + step as u32]);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// 色条：一块浅灰圆角底，再盖上颜色。半透明的颜色透出底，看起来就是「没铺满」。
///
/// 不用棋盘格：棋盘格是一堆方角小方块，盖不住圆角 —— 四角会从圆弧外面冒出来。
/// 浅灰底 + 整块圆角颜色就完全落在圆角里，而且不管衬在什么背景上都对。
pub(crate) fn paint_color_chip(
    painter: &egui::Painter,
    rect: Rect,
    rgba: [u8; 4],
    radius: CornerRadius,
    fade: f32,
) {
    painter.rect_filled(rect, radius, theme::SURFACE_3.gamma_multiply(fade));
    painter.rect_filled(
        rect,
        radius,
        Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]).gamma_multiply(fade),
    );
    // 完全透明时画一道斜杠，一眼能看出「这里没颜色」。
    if rgba[3] == 0 {
        painter.line_segment(
            [
                egui::pos2(rect.left() + 6.0, rect.bottom() - 6.0),
                egui::pos2(rect.right() - 6.0, rect.top() + 6.0),
            ],
            Stroke::new(1.5, theme::ACCENT_LINE.gamma_multiply(fade)),
        );
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::HAIRLINE.gamma_multiply(fade)),
        StrokeKind::Inside,
    );
}

/// 色块上的字用黑还是白：按亮度挑，保证读得清。
pub(crate) fn readable_ink(rgba: [u8; 4]) -> Color32 {
    let luma = 0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32;
    // 半透明时底下透出的是浅灰底，亮色或透明都用深色字。
    if rgba[3] < 128 || luma > 150.0 {
        theme::INK
    } else {
        Color32::WHITE
    }
}

/// 一个小的十六进制输入框（类似 `number_field`，但它收的是文本）。
pub(crate) fn hex_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: &str,
) -> Option<String> {
    input_shell(ui.painter(), rect, theme::R_CTL);
    let editing = ui.ctx().memory(|memory| memory.focused()) == Some(id);
    let mut draft = if editing {
        ui.data_mut(|data| data.get_temp::<String>(id))
            .unwrap_or_else(|| current.to_string())
    } else {
        current.to_string()
    };
    let inner = rect.shrink2(egui::vec2(7.0, 3.0));
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(&mut draft)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::monospace(11.0))
            .desired_width(inner.width()),
    );
    if ui.ctx().memory(|memory| memory.focused()) == Some(id) {
        ui.data_mut(|data| data.insert_temp(id, draft.clone()));
    }
    input_shell_state(
        ui.painter(),
        rect,
        theme::R_CTL,
        resp.hovered(),
        resp.has_focus(),
    );
    resp.changed().then_some(draft)
}

pub(crate) fn temp_bool(ctx: &egui::Context, id: egui::Id) -> Option<bool> {
    ctx.data_mut(|data| data.get_temp::<bool>(id))
}

pub(crate) fn temp_f32(ctx: &egui::Context, id: egui::Id) -> Option<f32> {
    ctx.data_mut(|data| data.get_temp::<f32>(id))
}

pub(crate) fn temp_rect(ctx: &egui::Context, id: egui::Id) -> Option<Rect> {
    ctx.data_mut(|data| data.get_temp::<Rect>(id))
}

/// RGB → HSV。h 是 0–360，s / v 是 0–1。
pub(crate) fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let sat = if max <= f32::EPSILON {
        0.0
    } else {
        delta / max
    };
    (hue, sat, max)
}

/// HSV → RGB。
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// 把颜色字符串解成 RGBA。`transparent` → 全透明；坏值 → 黑。
///
/// 值可能是一整段**色板文本**（hex 一行一个）—— 那种情况下取第一行：
/// 需要一种颜色、却拿到一板多色的，就用第一个。
pub(crate) fn color_rgba(value: &str) -> [u8; 4] {
    let value = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if value.eq_ignore_ascii_case("transparent") {
        return [0, 0, 0, 0];
    }
    let hex = value.trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return [0, 0, 0, 255];
    }
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
    match hex.len() {
        6 => [byte(0), byte(2), byte(4), 255],
        8 => [byte(0), byte(2), byte(4), byte(6)],
        _ => [0, 0, 0, 255],
    }
}

/// RGBA → 颜色字符串：不透明用 6 位，有透明度用 8 位。
pub(crate) fn color_string(rgba: [u8; 4]) -> String {
    if rgba[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2])
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            rgba[0], rgba[1], rgba[2], rgba[3]
        )
    }
}

/// 下拉框：带边框的按钮 + 一个弹出的列表（对勾 + 短说明）。
pub(crate) fn select_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    options: &[crate::catalog::Choice],
    zoom: f32,
    disabled: bool,
) -> Option<Value> {
    let radius = theme::R_CTL;
    let chosen = current.and_then(Value::as_str).unwrap_or("");
    let label = options
        .iter()
        .find(|choice| choice.value == chosen)
        .map(|choice| choice.label.clone())
        .unwrap_or_else(|| "—".to_string());

    // 禁用态：还是那个「文字 + 箭头」的下拉框，只是灰下去、点不开。
    if disabled {
        disabled_shell(ui.painter(), rect, radius);
        let label_area = Rect::from_min_max(
            egui::pos2(rect.left() + 8.0 * zoom, rect.top()),
            egui::pos2(rect.right() - 20.0 * zoom, rect.bottom()),
        );
        ui.painter().with_clip_rect(label_area).text(
            egui::pos2(rect.left() + 8.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(11.5 * zoom),
            theme::INK_3,
        );
        icons::chevron_down(
            ui.painter(),
            Rect::from_center_size(
                egui::pos2(rect.right() - 9.0 * zoom, rect.center().y),
                egui::vec2(12.0 * zoom, 12.0 * zoom),
            ),
            theme::HAIRLINE_STRONG,
        );
        return None;
    }

    input_shell(ui.painter(), rect, radius);
    let resp = ui.interact(rect, id, Sense::click());
    // 名称剪到「箭头之前」，长了也不会碰到箭头或出框。
    let label_area = Rect::from_min_max(
        egui::pos2(rect.left() + 8.0 * zoom, rect.top()),
        egui::pos2(rect.right() - 20.0 * zoom, rect.bottom()),
    );
    let label_painter = ui.painter().with_clip_rect(label_area);
    label_painter.text(
        egui::pos2(rect.left() + 8.0 * zoom, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::monospace(11.5 * zoom),
        theme::INK,
    );
    icons::chevron_down(
        ui.painter(),
        Rect::from_center_size(
            egui::pos2(rect.right() - 9.0 * zoom, rect.center().y),
            egui::vec2(12.0 * zoom, 12.0 * zoom),
        ),
        theme::INK_3,
    );
    input_shell_state(ui.painter(), rect, radius, resp.hovered(), false);

    let mut picked = None;
    // 弹层宽度按最宽的一项算 —— 否则窄的下拉框一展开，说明文字就会顶出去。
    let painter = ui.painter();
    let needed = options.iter().fold(rect.width(), |widest, choice| {
        let label = painter
            .layout_no_wrap(choice.label.clone(), FontId::monospace(11.5), theme::INK)
            .size()
            .x;
        let hint = choice.hint.as_ref().map_or(0.0, |hint| {
            painter
                .layout_no_wrap(hint.clone(), FontId::monospace(10.0), theme::INK_3)
                .size()
                .x
                + 10.0
        });
        widest.max(11.0 + 6.0 + label + 6.0 + hint + 18.0)
    });
    egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_width(needed);
            for choice in options {
                if menu_item(
                    ui,
                    &choice.label,
                    choice.hint.as_deref(),
                    choice.value == chosen,
                    zoom,
                ) {
                    picked = Some(choice.value.clone());
                }
            }
        });
    picked.map(|value| serde_json::json!(value))
}

/// 弹出列表里的一行：对勾 + 名称 + 右侧短说明。
pub(crate) fn menu_item(
    ui: &mut egui::Ui,
    label: &str,
    hint: Option<&str>,
    selected: bool,
    zoom: f32,
) -> bool {
    let height = 24.0 * zoom.max(0.8);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::R_CTL), theme::ACCENT_SOFT);
    }
    let ink = if selected { theme::ACCENT } else { theme::INK };
    let check = Rect::from_center_size(
        egui::pos2(rect.left() + 11.0 * zoom.max(0.8), rect.center().y),
        egui::vec2(12.0 * zoom.max(0.8), 12.0 * zoom.max(0.8)),
    );
    if selected {
        icons::check(ui.painter(), check, theme::ACCENT);
    }

    // 给右侧说明留出位置，名称按剩下的宽度折行 —— 两边都不出框。
    let painter = ui.painter();
    let hint_w = hint.map_or(0.0, |text| {
        painter
            .layout_no_wrap(text.to_string(), FontId::monospace(10.0), theme::INK_3)
            .size()
            .x
            + 10.0
    });
    let label_x = check.right() + 6.0;
    let avail = (rect.right() - 8.0 - hint_w - label_x).max(16.0);
    let galley = painter.layout(label.to_string(), FontId::monospace(11.5), ink, avail);
    painter.galley(
        egui::pos2(
            label_x,
            crate::ui::widgets::ink_top(&galley, rect.center().y),
        ),
        galley,
        ink,
    );
    if let Some(hint) = hint {
        painter.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            FontId::monospace(10.0),
            theme::INK_3,
        );
    }
    resp.clicked()
}

/// 禁用态的开关：同一个轨道和圆点，只是灰下去、不响应。
pub(crate) fn toggle_disabled(painter: &egui::Painter, rect: Rect, value: bool) {
    let height = rect.height() * 0.8;
    let width = (height * 1.8).min(rect.width());
    let track = Rect::from_min_size(
        egui::pos2(rect.right() - width, rect.center().y - height * 0.5),
        egui::vec2(width, height),
    );
    let radius = CornerRadius::same((height * 0.5) as u8);
    painter.rect_filled(track, radius, theme::SURFACE_3);
    painter.rect_stroke(
        track,
        radius,
        Stroke::new(1.0, theme::HAIRLINE_STRONG),
        StrokeKind::Inside,
    );
    let pad = height * 0.15;
    let dot = height * 0.5 - pad;
    let travel = (track.width() - 2.0 * (dot + pad)).max(0.0);
    let x = track.left() + dot + pad + if value { travel } else { 0.0 };
    painter.circle_filled(egui::pos2(x, track.center().y), dot, theme::SURFACE);
}

/// 文件 / 目录选择。
///
/// 设计上：空着时是个虚线框的按钮；选完之后文件名顶在原来的位置，
/// 鼠标移上去才重新变回「换一个」。
pub(crate) fn file_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    control: &Control,
    zoom: f32,
    disabled: bool,
) -> Option<ControlEvent> {
    let Control::File {
        dialog_title,
        extensions,
        directory,
    } = control
    else {
        return None;
    };

    let path = current.and_then(Value::as_str).unwrap_or("");
    let name = file_name(path);
    let empty = name.is_empty();

    // 禁用态：还是那个文件框，只是灰下去、点不开。
    if disabled {
        let painter = ui.painter();
        disabled_shell(painter, rect, theme::R_CTL);
        let label = if empty {
            if *directory {
                "选择目录…"
            } else {
                "选择文件…"
            }
        } else {
            name.as_str()
        };
        painter.with_clip_rect(rect).text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(10.0 * zoom),
            theme::INK_3,
        );
        return None;
    }

    let resp = ui.interact(rect, id, Sense::click());
    let corner = CornerRadius::same(theme::R_CTL);

    {
        let painter = ui.painter();
        if empty {
            // 虚线框 —— 空着的时候一眼能看出「这里还没填」。
            dashed_rect(painter, rect, zoom, theme::HAIRLINE_STRONG);
        } else {
            painter.rect_filled(rect, corner, theme::SURFACE_2);
            painter.rect_stroke(
                rect,
                corner,
                Stroke::new(1.0, theme::HAIRLINE),
                StrokeKind::Inside,
            );
        }

        let (label, color) = if empty {
            (
                if *directory {
                    "选择目录…".to_string()
                } else {
                    "选择文件…".to_string()
                },
                theme::INK_3,
            )
        } else if resp.hovered() {
            (
                if *directory {
                    "换一个目录".to_string()
                } else {
                    "换一个文件".to_string()
                },
                theme::ACCENT,
            )
        } else {
            (name, theme::INK_2)
        };

        // 文件名可能很长，剪在框里。
        painter.with_clip_rect(rect).text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(10.0 * zoom),
            color,
        );
    }

    if !resp.clicked() {
        return None;
    }

    // 只把「要选」报出去 —— 真正的对话框由画布在后台线程上开（见 `state::dialog`）。
    Some(ControlEvent::PickFile {
        title: dialog_title.clone(),
        extensions: extensions.clone(),
        directory: *directory,
    })
}

/// 路径最后一段。目录选完也是显示最后一段，和文件一样。
pub(crate) fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 虚线矩形。egui 没有内置的虚线描边，就自己按段画。
pub(crate) fn dashed_rect(painter: &egui::Painter, rect: Rect, zoom: f32, color: Color32) {
    let dash = 4.0 * zoom;
    let gap = 3.0 * zoom;
    let stroke = Stroke::new(1.0, color);

    let edge = |a: Pos2, b: Pos2| {
        let total = (b - a).length();
        if total <= 0.0 {
            return;
        }
        let dir = (b - a) / total;
        let mut start = 0.0;
        while start < total {
            let end = (start + dash).min(total);
            painter.line_segment([a + dir * start, a + dir * end], stroke);
            start = end + gap;
        }
    };

    edge(rect.left_top(), rect.right_top());
    edge(rect.right_top(), rect.right_bottom());
    edge(rect.right_bottom(), rect.left_bottom());
    edge(rect.left_bottom(), rect.left_top());
}
