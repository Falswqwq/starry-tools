//! 节点库浮层：分类 / 卡片 / 展开动画 / 拖出到画布。
//!
//! 卡片上的每一个字都来自 [`crate::catalog`]（也就是 core 的注册表）——
//! 这里没有任何写死的节点名或分类。
//!
//! 触发按钮不在这里：它由左上角的 [`crate::chrome`] 画，只翻转这里的 `open`。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
};
use egui::epaint::RectShape;

use crate::catalog::{self, Kind};
use crate::icons;
use crate::theme;
use crate::widgets;

/// 折叠时的高度；展开后的高度按内容行数算。
const CARD_H: f32 = 66.0;
const LINE_H: f32 = 15.0;
const RAIL_W: f32 = 88.0;
const PANEL_W: f32 = 560.0;
const LIST_W: f32 = PANEL_W - RAIL_W - 1.0;
const MAX_LIST_H: f32 = 460.0;
const DRAG_THRESHOLD: f32 = 5.0;
/// 卡片内容的左右内边距：文字从 `left + CARD_PAD` 起，折到 `right - CARD_PAD`。
const CARD_PAD: f32 = 11.0;

pub struct Library {
    pub open: bool,
    category: usize,
    expanded: Option<usize>,
    /// 正在被拖出浮层的卡片（是 `kinds` 里的下标，不是过滤后列表的下标）。
    dragging: Option<usize>,
    /// 吸附在鼠标上的那枚胶囊（正在拖 / 刚松手正在淡出）。
    ghost: Option<Ghost>,
    /// 浮层这一帧所在的矩形，用来判断「拖出去了没有」。
    panel_rect: Option<Rect>,
    kinds: Vec<Kind>,
    categories: Vec<String>,
}

/// 拖拽时吸附在指针上的胶囊。
struct Ghost {
    kind: usize,
    pos: Pos2,
    /// 出现时刻 —— 用来做淡入。
    shown: f64,
    /// 松手时刻 —— 有值就表示正在淡出。
    leaving: Option<f64>,
}

/// 拖出时返回给调用方：要把哪个节点放到画布哪个屏幕位置。
pub struct DropOut {
    pub kind: usize,
    pub screen_pos: Pos2,
}

/// 展开卡片里的一行。
enum Row {
    Section(String),
    Text {
        text: String,
        color: Color32,
        indent: f32,
    },
    Port {
        badge: String,
        color: Color32,
        text: String,
        flag: Option<String>,
        hint: Option<String>,
    },
}

impl Library {
    pub fn new(kinds: Vec<Kind>) -> Self {
        let categories = catalog::categories(&kinds);
        Self {
            open: false,
            category: 0,
            expanded: None,
            dragging: None,
            ghost: None,
            panel_rect: None,
            kinds,
            categories,
        }
    }

    fn visible(&self, i: usize) -> bool {
        self.category == 0 || self.kinds[i].category == self.categories[self.category]
    }

    fn category_count(&self, index: usize) -> usize {
        if index == 0 {
            self.kinds.len()
        } else {
            let name = &self.categories[index];
            self.kinds
                .iter()
                .filter(|kind| &kind.category == name)
                .count()
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<DropOut> {
        let mut drop_out = None;

        // 关掉的时候还要再淡出一下，所以不能立刻停画。
        let anim = ctx.animate_bool_with_time(egui::Id::new("lib-anim"), self.open, 0.13);
        if !self.open && anim <= 0.01 {
            self.panel_rect = None;
            self.dragging = None;
            return None;
        }

        // 浮层靠左展开，不挡视线：左上角按钮的下方。
        let pos = egui::pos2(12.0, 12.0 + 27.0 + 8.0);
        egui::Area::new(egui::Id::new("lib-panel"))
            .fixed_pos(pos + egui::vec2(0.0, (1.0 - anim) * -4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(anim);
                widgets::panel_frame().show(ui, |ui| {
                    ui.set_width(PANEL_W);
                    self.panel_rect = Some(ui.min_rect());

                    // 顶栏
                    egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 8,
                            bottom: 8,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("节点库")
                                        .color(theme::INK)
                                        .size(11.5)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{} 个", self.kinds.len()))
                                        .color(theme::INK_3)
                                        .size(10.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new("拖到画布上放置")
                                                .color(theme::INK_3)
                                                .size(10.0),
                                        );
                                    },
                                );
                            });
                        });
                    ui.painter().line_segment(
                        [
                            egui::pos2(ui.min_rect().left(), ui.min_rect().top()),
                            egui::pos2(ui.min_rect().right(), ui.min_rect().top()),
                        ],
                        Stroke::new(1.0, theme::HAIRLINE),
                    );

                    let body_h = self.body_height();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        self.rail(ui, body_h);
                        // 竖直分隔线
                        let (line, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, body_h), Sense::hover());
                        ui.painter()
                            .rect_filled(line, CornerRadius::ZERO, theme::HAIRLINE);

                        // 给右栏一块**固定尺寸**的区域：不这么写，横向布局里
                        // 的 ScrollArea 拿不到宽度，卡片内容会被压成一条。
                        ui.allocate_ui_with_layout(
                            egui::vec2(LIST_W, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(body_h)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        ui.set_width(LIST_W - 20.0);
                                        ui.add_space(8.0);
                                        for i in 0..self.kinds.len() {
                                            if !self.visible(i) {
                                                continue;
                                            }
                                            if let Some(pos) = self.card(ctx, ui, i) {
                                                drop_out = Some(DropOut {
                                                    kind: i,
                                                    screen_pos: pos,
                                                });
                                            }
                                            ui.add_space(5.0);
                                        }
                                        ui.add_space(3.0);
                                    });
                            },
                        );
                    });
                });
            });

        // ---- 拖出时的幽灵 ----
        let now = ctx.input(|input| input.time);
        if let Some(i) = self.dragging {
            if let Some(p) = ctx.input(|input| input.pointer.interact_pos()) {
                let fresh = self
                    .ghost
                    .as_ref()
                    .is_none_or(|g| g.kind != i || g.leaving.is_some());
                if fresh {
                    self.ghost = Some(Ghost {
                        kind: i,
                        pos: p,
                        shown: now,
                        leaving: None,
                    });
                } else if let Some(g) = self.ghost.as_mut() {
                    g.pos = p;
                }
            }
        }
        // 淡出放完就收掉。
        if let Some(g) = &self.ghost {
            if let Some(t0) = g.leaving {
                if now - t0 > 0.24 {
                    self.ghost = None;
                }
            }
        }
        if let Some(g) = &self.ghost {
            let fade_in = ((now - g.shown) / 0.14).clamp(0.0, 1.0);
            let fade_out = g
                .leaving
                .map_or(1.0, |t0| 1.0 - ((now - t0) / 0.24).clamp(0.0, 1.0));
            let alpha = (fade_in.min(fade_out)) as f32;
            let (kind, pos) = (g.kind, g.pos);
            self.ghost_view(ctx, kind, pos, alpha);
        }

        drop_out
    }

    /// 浮层正文的高度：按内容算，封顶到 `MAX_LIST_H`，且不低于左栏本身。
    fn body_height(&self) -> f32 {
        let visible = (0..self.kinds.len()).filter(|&i| self.visible(i)).count();
        let list = 16.0 + visible as f32 * (CARD_H + 5.0);
        let rail = 16.0 + self.categories.len() as f32 * 23.0;
        list.min(MAX_LIST_H).max(rail)
    }

    /// 左栏：分类，名字 + 数量。整栏给一块**固定尺寸**的区域，背景一直铺到底。
    fn rail(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.allocate_ui_with_layout(
            egui::vec2(RAIL_W, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::Frame::default()
                    .fill(theme::SURFACE_2)
                    .corner_radius(CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: theme::R_CARD,
                        se: 0,
                    })
                    .inner_margin(egui::Margin {
                        left: 7,
                        right: 7,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        let width = RAIL_W - 14.0;
                        ui.set_width(width);
                        ui.set_min_height(height - 16.0);
                        // 分类之间留点竖缝，紧靠着不好看。
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for index in 0..self.categories.len() {
                            let on = self.category == index;
                            let label = self.categories[index].clone();
                            let count = self.category_count(index);
                            let (rect, resp) =
                                ui.allocate_exact_size(egui::vec2(width, 22.0), Sense::click());
                            if on {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(theme::R_CTL),
                                    theme::SURFACE,
                                );
                            } else if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(theme::R_CTL),
                                    theme::SURFACE_3,
                                );
                            }
                            let ink = if on {
                                theme::ACCENT
                            } else if resp.hovered() {
                                theme::INK
                            } else {
                                theme::INK_2
                            };
                            ui.painter().text(
                                egui::pos2(rect.left() + 8.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                label,
                                FontId::monospace(11.0),
                                ink,
                            );
                            ui.painter().text(
                                egui::pos2(rect.right() - 8.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                count.to_string(),
                                FontId::monospace(11.0),
                                ink,
                            );
                            if resp.clicked() {
                                self.category = index;
                                self.expanded = None;
                            }
                        }
                    });
            },
        );
    }

    /// 展开卡片时列出来的内容。
    fn details(&self, kind: &Kind) -> Vec<Row> {
        let mut rows = Vec::new();

        if !kind.notes.is_empty() {
            rows.push(Row::Section("注意事项".to_string()));
            for note in &kind.notes {
                rows.push(Row::Text {
                    text: format!("· {note}"),
                    color: theme::INK_2,
                    indent: 2.0,
                });
            }
        }
        if !kind.params.is_empty() {
            rows.push(Row::Section("参数".to_string()));
            for param in &kind.params {
                let text = if param.label.is_empty() {
                    param.summary()
                } else {
                    format!("{}   {}", param.label, param.summary())
                };
                rows.push(Row::Text {
                    text,
                    color: theme::INK,
                    indent: 0.0,
                });
                if let Some(note) = &param.description {
                    rows.push(Row::Text {
                        text: note.clone(),
                        color: theme::INK_2,
                        indent: 0.0,
                    });
                }
                if let crate::catalog::Control::Select { options } = &param.control {
                    let text = options
                        .iter()
                        .map(|option| match &option.hint {
                            Some(hint) => format!("{}（{hint}）", option.label),
                            None => option.label.clone(),
                        })
                        .collect::<Vec<_>>()
                        .join(" · ");
                    rows.push(Row::Text {
                        text,
                        color: theme::INK_3,
                        indent: 0.0,
                    });
                }
            }
        }
        if !kind.inputs.is_empty() || !kind.outputs.is_empty() {
            rows.push(Row::Section("端口".to_string()));
            for port in &kind.inputs {
                rows.push(Row::Port {
                    badge: port.badge.clone(),
                    color: theme::badge_color(port.ty),
                    text: format!("输入 · {}", port.label),
                    flag: port.required.then(|| "必填".to_string()),
                    hint: None,
                });
            }
            for port in &kind.outputs {
                rows.push(Row::Port {
                    badge: port.badge.clone(),
                    color: theme::badge_color(port.ty),
                    text: format!("输出 · {}", port.label),
                    flag: None,
                    hint: None,
                });
            }
        }

        rows
    }

    /// 画一张卡片。返回 `Some(屏幕位置)` 表示它刚被拖出浮层并松手。
    fn card(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, i: usize) -> Option<Pos2> {
        let expanded = self.expanded == Some(i);
        let t = ctx.animate_bool_with_time(egui::Id::new(("lib-card", i)), expanded, 0.2);

        // 内容要按宽度折行，所以先用同一个 painter 量一遍高，再拿这个总高去定卡片高度。
        let card_w = ui.available_width() - 20.0;
        let laid: Vec<(Row, f32)> = {
            let measure = ui.painter();
            self.details(&self.kinds[i])
                .into_iter()
                .map(|row| {
                    let h = measure_row(measure, &row, card_w - CARD_PAD * 2.0);
                    (row, h)
                })
                .collect()
        };
        let details_h: f32 = laid.iter().map(|(_, h)| *h).sum();
        let expanded_h = CARD_H + 8.0 + details_h + 12.0;
        let height = CARD_H + t * (expanded_h - CARD_H);

        let kind = &self.kinds[i];
        let title = kind.name.clone();
        let brief = kind.description.clone();
        let dragging = self.dragging == Some(i);

        let (full, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        // 左右留出内边距，卡片不贴着边栏。
        let rect = full.shrink2(egui::vec2(10.0, 0.0));
        let painter = ui.painter_at(rect);
        let cr = CornerRadius::same(theme::R_CARD);
        let hot = resp.hovered();

        // 展开时整张卡往上浮一点（这里用强调色描边 + 大投影表示）。
        if expanded {
            painter.add(
                RectShape::filled(
                    rect.translate(egui::vec2(0.0, 1.0)),
                    cr,
                    Color32::from_rgba_premultiplied(0x10, 0x18, 0x28, 0x1e),
                )
                .with_blur_width(14.0),
            );
        } else {
            widgets::card_shadow(&painter, rect, theme::R_CARD);
        }
        painter.rect_filled(rect, cr, theme::SURFACE);
        let border = if dragging {
            theme::ACCENT
        } else if expanded {
            theme::ACCENT_LINE
        } else if hot {
            theme::HAIRLINE_STRONG
        } else {
            theme::HAIRLINE
        };
        painter.rect_stroke(rect, cr, Stroke::new(1.0, border), StrokeKind::Inside);

        // 内容（拖出去时退开）
        let content_alpha = if dragging { 0.15 } else { 1.0 };
        let name_ink = theme::INK.gamma_multiply(content_alpha);
        let desc_ink = theme::INK_3.gamma_multiply(content_alpha);

        let name = painter.layout_no_wrap(title, FontId::monospace(12.0), name_ink);
        let mut x = rect.left() + 10.0;
        let top = rect.top() + 9.0;
        painter.galley(egui::pos2(x, top), name.clone(), name_ink);
        x += name.size().x + 6.0;

        // 类型徽标 + 箭头（与节点名按**墨迹**中线对齐，汉字名和拉丁徽标才不显得错位）
        let badges_y = top + crate::widgets::ink_center(&name);
        self.draw_badges(&painter, kind, x, badges_y, content_alpha);

        // 紫色节点（要你动手的）在卡片右上角挂一个小标记。
        if kind.interactive {
            let tag = painter.layout_no_wrap(
                "交互".to_string(),
                FontId::monospace(8.5),
                theme::PURPLE.gamma_multiply(content_alpha),
            );
            let w = tag.size().x + 10.0;
            let h = tag.size().y + 3.0;
            let r = Rect::from_min_size(
                egui::pos2(rect.right() - 10.0 - w, badges_y - h / 2.0),
                egui::vec2(w, h),
            );
            painter.rect_filled(
                r,
                CornerRadius::same(3),
                theme::PURPLE_SOFT.gamma_multiply(content_alpha),
            );
            painter.rect_stroke(
                r,
                CornerRadius::same(3),
                Stroke::new(1.0, theme::PURPLE_LINE.gamma_multiply(content_alpha)),
                StrokeKind::Inside,
            );
            painter.galley(
                egui::pos2(
                    r.center().x - tag.size().x / 2.0,
                    crate::widgets::ink_top(&tag, r.center().y),
                ),
                tag,
                theme::PURPLE.gamma_multiply(content_alpha),
            );
        }

        // 一句话简介（最多两行）
        let desc_y = top + name.size().y + 3.0;
        let desc = painter.layout(
            brief,
            FontId::monospace(10.5),
            desc_ink,
            rect.width() - 20.0,
        );
        let desc_clip = Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, desc_y),
            egui::vec2(rect.width() - 20.0, LINE_H * 2.0),
        );
        ui.painter_at(desc_clip)
            .galley(egui::pos2(rect.left() + 10.0, desc_y), desc, desc_ink);

        // 被拖出时，卡片本体盖一个「向外」的箭头
        if dragging {
            icons::arrow_right(
                &painter,
                Rect::from_center_size(rect.center(), egui::vec2(30.0, 30.0)),
                theme::ACCENT,
            );
        }

        // 展开区
        if t > 0.01 {
            let body = painter.with_clip_rect(rect);
            let mut y = rect.top() + CARD_H + 8.0;
            for (row, h) in &laid {
                self.draw_row(&body, row, rect, y, t, card_w - CARD_PAD * 2.0);
                y += h;
            }
        }

        // ---- 交互 ----
        let mut dropped = None;
        if resp.clicked() {
            self.expanded = if expanded { None } else { Some(i) };
        }
        if resp.drag_started() {
            self.dragging = Some(i);
        }
        if resp.drag_stopped() && self.dragging == Some(i) {
            self.dragging = None;
            let outside = self.outside_of_panel(ctx);
            let now = ctx.input(|input| input.time);
            let at = ctx.input(|input| input.pointer.interact_pos());
            if outside {
                // 停在原地再淡出 —— 看起来像被放进了画布。
                if let Some(at) = at {
                    let shown = self.ghost.as_ref().map_or(now, |g| g.shown);
                    self.ghost = Some(Ghost {
                        kind: i,
                        pos: at,
                        shown,
                        leaving: Some(now),
                    });
                }
                dropped = at;
            } else {
                self.ghost = None;
            }
        }
        // 拖到浮层外面就顺势收起展开的那张。
        if resp.dragged() && self.outside_of_panel(ctx) {
            self.expanded = None;
        }

        dropped
    }

    fn outside_of_panel(&self, ctx: &egui::Context) -> bool {
        ctx.input(|input| input.pointer.interact_pos())
            .zip(self.panel_rect)
            .is_some_and(|(p, r)| !r.expand(DRAG_THRESHOLD).contains(p))
    }

    /// 顶行的类型徽标：`[入] → [出]`，没有输入就写「起点」。
    fn draw_badges(&self, painter: &egui::Painter, kind: &Kind, mut x: f32, cy: f32, alpha: f32) {
        let font = FontId::monospace(8.5);
        let chip = |text: &str, color: Color32, x: &mut f32| {
            let g = painter.layout_no_wrap(text.to_string(), font.clone(), color);
            let w = g.size().x + 8.0;
            let h = g.size().y + 2.0;
            let r = Rect::from_min_size(egui::pos2(*x, cy - h / 2.0), egui::vec2(w, h));
            painter.rect_stroke(
                r,
                CornerRadius::same(3),
                Stroke::new(1.0, color),
                StrokeKind::Inside,
            );
            painter.galley(
                egui::pos2(
                    r.center().x - g.size().x / 2.0,
                    crate::widgets::ink_top(&g, r.center().y),
                ),
                g,
                color,
            );
            *x += w + 3.0;
        };

        if kind.inputs.is_empty() {
            chip("起点", theme::ACCENT.gamma_multiply(alpha), &mut x);
        } else {
            for port in &kind.inputs {
                chip(
                    &port.badge,
                    theme::badge_color(port.ty).gamma_multiply(alpha),
                    &mut x,
                );
            }
        }
        // 箭头
        icons::arrow_right(
            painter,
            Rect::from_center_size(egui::pos2(x + 5.0, cy), egui::vec2(10.0, 10.0)),
            theme::INK_3.gamma_multiply(alpha),
        );
        x += 13.0;
        for port in &kind.outputs {
            chip(
                &port.badge,
                theme::badge_color(port.ty).gamma_multiply(alpha),
                &mut x,
            );
        }
    }

    fn draw_row(&self, painter: &egui::Painter, row: &Row, card: Rect, y: f32, t: f32, wrap: f32) {
        let alpha = t;
        match row {
            Row::Section(text) => {
                let galley = painter.layout(
                    text.clone(),
                    FontId::monospace(9.5),
                    theme::INK_3.gamma_multiply(alpha),
                    wrap,
                );
                painter.galley(
                    egui::pos2(card.left() + 11.0, y),
                    galley,
                    theme::INK_3.gamma_multiply(alpha),
                );
            }
            Row::Text {
                text,
                color,
                indent,
            } => {
                // 按卡片宽度折行 —— 长说明不会顶到右边框。
                let galley = painter.layout(
                    text.clone(),
                    FontId::monospace(10.5),
                    color.gamma_multiply(alpha),
                    wrap - indent,
                );
                painter.galley(
                    egui::pos2(card.left() + 11.0 + indent, y),
                    galley,
                    color.gamma_multiply(alpha),
                );
            }
            Row::Port {
                badge,
                color,
                text,
                flag,
                hint,
            } => {
                let mut x = card.left() + 11.0;
                let color = color.gamma_multiply(alpha);
                let g = painter.layout_no_wrap(badge.clone(), FontId::monospace(8.5), color);
                let w = g.size().x + 8.0;
                let h = g.size().y + 2.0;
                let r = Rect::from_min_size(egui::pos2(x, y + 2.0), egui::vec2(w, h));
                painter.rect_stroke(
                    r,
                    CornerRadius::same(3),
                    Stroke::new(1.0, color),
                    StrokeKind::Inside,
                );
                // 徽标按墨迹在胶囊里居中，右侧文字也按墨迹对齐到胶囊中线。
                let chip_baseline = crate::widgets::ink_top(&g, r.center().y);
                painter.galley(
                    egui::pos2(r.center().x - g.size().x / 2.0, chip_baseline),
                    g,
                    color,
                );
                x += w + 6.0;
                // 标签也折行；「必填」占的地方先扣掉。
                let flag_w = flag.clone().map_or(0.0, |flag| {
                    painter
                        .layout_no_wrap(flag, FontId::monospace(9.0), theme::DANGER)
                        .size()
                        .x
                        + 10.0
                });
                let galley = painter.layout(
                    text.clone(),
                    FontId::monospace(10.5),
                    theme::INK_2.gamma_multiply(alpha),
                    (card.right() - 11.0 - flag_w - x).max(16.0),
                );
                painter.galley(
                    egui::pos2(x, crate::widgets::label_top(&galley, r.center().y, y + 3.0)),
                    galley,
                    theme::INK_2.gamma_multiply(alpha),
                );
                if let Some(flag) = flag {
                    painter.text(
                        egui::pos2(card.right() - 11.0, y + 4.0),
                        Align2::RIGHT_TOP,
                        flag,
                        FontId::monospace(9.0),
                        theme::DANGER.gamma_multiply(alpha),
                    );
                }
                if let Some(hint) = hint {
                    let galley = painter.layout(
                        hint.clone(),
                        FontId::monospace(10.0),
                        theme::INK_3.gamma_multiply(alpha),
                        wrap,
                    );
                    painter.galley(
                        egui::pos2(card.left() + 11.0, y + 3.0 + LINE_H),
                        galley,
                        theme::INK_3.gamma_multiply(alpha),
                    );
                }
            }
        }
    }

    /// 吸附在指针上的胶囊：节点名 + 类型徽标。`alpha` 用来淡入淡出。
    fn ghost_view(&self, ctx: &egui::Context, i: usize, p: Pos2, alpha: f32) {
        let kind = &self.kinds[i];
        let name = kind.name.clone();
        let badges: Vec<(String, Color32)> = kind
            .inputs
            .iter()
            .chain(kind.outputs.iter())
            .map(|port| (port.badge.clone(), theme::badge_color(port.ty)))
            .collect();

        egui::Area::new(egui::Id::new("lib-ghost"))
            .fixed_pos(p + egui::vec2(14.0, 12.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                widgets::panel_frame()
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(name)
                                    .color(theme::INK)
                                    .size(11.5)
                                    .strong(),
                            );
                            for (badge, color) in &badges {
                                let g = ui.painter().layout_no_wrap(
                                    badge.clone(),
                                    FontId::monospace(8.5),
                                    *color,
                                );
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(g.size().x + 8.0, g.size().y + 2.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_stroke(
                                    r,
                                    CornerRadius::same(3),
                                    Stroke::new(1.0, *color),
                                    StrokeKind::Inside,
                                );
                                ui.painter().galley(
                                    egui::pos2(
                                        r.center().x - g.size().x / 2.0,
                                        crate::widgets::ink_top(&g, r.center().y),
                                    ),
                                    g,
                                    *color,
                                );
                            }
                        });
                    });
            });
    }
}

/// 一行（按宽度折行后）占多高。
fn measure_row(painter: &egui::Painter, row: &Row, wrap: f32) -> f32 {
    match row {
        Row::Section(text) => {
            painter
                .layout(text.clone(), FontId::monospace(9.5), theme::INK_3, wrap)
                .size()
                .y
                + 4.0
        }
        Row::Text {
            text,
            color,
            indent,
        } => {
            painter
                .layout(text.clone(), FontId::monospace(10.5), *color, wrap - indent)
                .size()
                .y
        }
        Row::Port { hint, .. } => {
            if hint.is_some() {
                LINE_H * 2.0
            } else {
                LINE_H + 3.0
            }
        }
    }
}
