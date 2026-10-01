//! 画布上的两组浮动控件：左上角是「这个工作流」，右上角是「拿它做什么」。
//!
//! 不做胶囊外框，按钮自己就是圆角矩形，排成一行。
//! 浮层（说明 / 加载）都靠在触发按钮下面展开。

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind};

use crate::icons;
use crate::library::Library;
use crate::settings::{Settings, FPS_MAX, FPS_MIN};
use crate::theme;
use crate::widgets::{self, Size, Variant};
use crate::workspace::Workspace;

pub enum Action {
    Save,
    New,
    Load(String),
    Run,
    /// 跳到某个节点（右上角「N 处问题」会用它）。
    Select(String),
}

#[derive(Default)]
pub struct Chrome {
    /// 「说明」面板开着吗。
    describing: bool,
    /// 「加载」面板开着吗。
    browsing: bool,
    /// 「设置」面板开着吗。
    settings_open: bool,
    /// 设置改过了、还没落盘（等鼠标松开再写，免得拖动时每帧都写文件）。
    settings_dirty: bool,
    /// 正在确认删除哪一个存档。
    confirming: Option<String>,
}

const MARGIN: f32 = 12.0;
const NAME_W: f32 = 176.0;
const PANEL_GAP: f32 = 8.0;

impl Chrome {
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        workspace: &mut Workspace,
        library: &mut Library,
        settings: &mut Settings,
        dirty: bool,
        running: bool,
        errors: usize,
        first_bad: Option<&str>,
        nodes: usize,
        edges: usize,
    ) -> Option<Action> {
        let mut action = None;

        // ---- 左上角 ----
        let (_name_rect, desc_rect, gear_rect) = egui::Area::new(egui::Id::new("chrome-left"))
            .fixed_pos(egui::pos2(MARGIN, MARGIN))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.horizontal_centered(|ui| {
                    // 展开时，「节点库」这三个字会被一个淡入 + 翻转的下箭头盖住 ——
                    // 不另开一个箭头，按钮宽度也不会因此改变。
                    let open_t = ctx.animate_bool_with_time(
                        egui::Id::new("library-caret"),
                        library.open,
                        0.24,
                    );
                    let caret = move |painter: &egui::Painter, rect: Rect, color: Color32| {
                        // 旋转跟着图标的淡入走（后半段），不跟文字重叠。
                        let (_, icon_alpha) = widgets::overlay_phases(open_t);
                        icons::chevron_down_turned(
                            painter,
                            rect,
                            color,
                            (1.0 - icon_alpha) * std::f32::consts::PI,
                        )
                    };
                    let lib = widgets::button_ex(
                        ui,
                        "节点库",
                        Some(&icons::blocks),
                        Some(widgets::Overlay {
                            icon: &caret,
                            t: open_t,
                        }),
                        Variant::Solid,
                        Size::Md,
                        true,
                    );
                    if lib.clicked() {
                        library.open = !library.open;
                        // 几个浮层互斥：开一个就关掉别个。
                        self.describing = false;
                        self.browsing = false;
                        self.settings_open = false;
                    }

                    let name = name_field(ui, workspace.name_mut());
                    let name_rect = name.rect;

                    let desc = widgets::button(
                        ui,
                        "",
                        Some(&icons::file_text),
                        Variant::Solid,
                        Size::Icon,
                        true,
                    );
                    if desc.clicked() {
                        self.describing = !self.describing;
                        self.browsing = false;
                        self.settings_open = false;
                        library.open = false;
                    }

                    // 设置：齿轮。排在「说明」后面。
                    let gear = widgets::button(
                        ui,
                        "",
                        Some(&icons::settings),
                        Variant::Solid,
                        Size::Icon,
                        true,
                    );
                    if gear.clicked() {
                        self.settings_open = !self.settings_open;
                        self.describing = false;
                        self.browsing = false;
                        library.open = false;
                    }
                    (name_rect, desc.rect, gear.rect)
                })
                .inner
            })
            .inner;

        // ---- 右上角 ----
        let (load_rect, _save_rect) = egui::Area::new(egui::Id::new("chrome-right"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-MARGIN, MARGIN))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.horizontal_centered(|ui| {
                    if errors > 0 {
                        let chip = widgets::button(
                            ui,
                            &format!("{errors} 处问题"),
                            None,
                            Variant::Danger,
                            Size::Md,
                            true,
                        );
                        if chip.clicked() {
                            if let Some(id) = first_bad {
                                action = Some(Action::Select(id.to_string()));
                            }
                        }
                    }

                    let load = widgets::button(
                        ui,
                        "",
                        Some(&icons::folder_open),
                        Variant::Solid,
                        Size::Icon,
                        true,
                    );
                    if load.clicked() {
                        self.browsing = !self.browsing;
                        self.describing = false;
                        self.settings_open = false;
                        library.open = false;
                        self.confirming = None;
                    }

                    let save = widgets::button(
                        ui,
                        "",
                        Some(&icons::save),
                        Variant::Solid,
                        Size::Icon,
                        true,
                    );
                    if save.clicked() {
                        action = Some(Action::Save);
                    }
                    // 未保存的小蓝点：挂在「保存」下面，像个强调号。
                    if dirty {
                        ui.painter().circle_filled(
                            egui::pos2(save.rect.center().x, save.rect.bottom() + 7.0),
                            2.5,
                            theme::ACCENT,
                        );
                    }

                    if running {
                        let phase = (ctx.input(|i| i.time) as f32 * 1.2) % 1.0;
                        let spinner = move |painter: &egui::Painter, rect: Rect, color: Color32| {
                            icons::loader(painter, rect, color, phase)
                        };
                        widgets::button(
                            ui,
                            "运行中",
                            Some(&spinner),
                            Variant::Primary,
                            Size::Md,
                            false,
                        );
                    } else {
                        let run = widgets::button(
                            ui,
                            "运行",
                            Some(&icons::play),
                            Variant::Primary,
                            Size::Md,
                            errors == 0,
                        );
                        if run.clicked() {
                            action = Some(Action::Run);
                        }
                    }

                    (load.rect, save.rect)
                })
                .inner
            })
            .inner;

        // ---- 说明面板 ----
        let desc_anim =
            ctx.animate_bool_with_time(egui::Id::new("desc-anim"), self.describing, 0.13);
        if desc_anim > 0.01 {
            let pos = egui::pos2(desc_rect.left(), desc_rect.bottom() + PANEL_GAP);
            let mut close = false;
            panel(ctx, "workflow-description", pos, 340.0, desc_anim, |ui| {
                header(
                    ui,
                    "说明",
                    &format!("更新于 {}", format_time(now_millis())),
                    |ui| {
                        if widgets::button(
                            ui,
                            "",
                            Some(&icons::x),
                            Variant::Ghost,
                            Size::IconSm,
                            true,
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    },
                );
                let inner = egui::Frame::default()
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 10,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("这个工作流是做什么的")
                                .color(theme::INK_3)
                                .size(10.0),
                        );
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::multiline(workspace.description_mut())
                                .desired_rows(4)
                                .desired_width(f32::INFINITY)
                                .hint_text("例如：把 webp 素材转成 png，再放大到四倍给像素画用…"),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            fact(ui, "节点", &nodes.to_string());
                            ui.add_space(8.0);
                            fact(ui, "连线", &edges.to_string());
                        });
                    });
                ui.allocate_rect(inner.response.rect, Sense::hover());
            });
            if close {
                self.describing = false;
            }
        }

        // ---- 设置面板 ----
        let mut settings_changed = false;
        let settings_anim =
            ctx.animate_bool_with_time(egui::Id::new("settings-anim"), self.settings_open, 0.13);
        if settings_anim > 0.01 {
            let pos = egui::pos2(gear_rect.left(), gear_rect.bottom() + PANEL_GAP);
            let mut close = false;
            panel(ctx, "app-settings", pos, 320.0, settings_anim, |ui| {
                header(ui, "设置", "外观与性能", |ui| {
                    if widgets::button(ui, "", Some(&icons::x), Variant::Ghost, Size::IconSm, true)
                        .clicked()
                    {
                        close = true;
                    }
                });
                let inner = egui::Frame::default()
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 10,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("帧率上限")
                                    .color(theme::INK_2)
                                    .size(11.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let (row, _) = ui.allocate_exact_size(
                                        egui::vec2(200.0, 24.0),
                                        Sense::hover(),
                                    );
                                    if let Some(next) = widgets::slider(
                                        ui,
                                        egui::Id::new("settings-fps"),
                                        row,
                                        settings.max_fps,
                                        FPS_MIN,
                                        FPS_MAX,
                                        true,
                                        " fps",
                                    ) {
                                        settings.max_fps = next;
                                        settings_changed = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new("拖动画布、动画都按这个上限节流；调低更省电。")
                                .color(theme::INK_3)
                                .size(9.5),
                        );

                        ui.add_space(12.0);

                        let (row, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 24.0),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            egui::pos2(row.left(), row.center().y),
                            Align2::LEFT_CENTER,
                            "显示 fps 计数器",
                            FontId::monospace(11.0),
                            theme::INK_2,
                        );
                        let mut show = settings.show_fps;
                        if widgets::switch(ui, egui::Id::new("settings-show-fps"), row, &mut show) {
                            settings.show_fps = show;
                            settings_changed = true;
                        }
                    });
                ui.allocate_rect(inner.response.rect, Sense::hover());
            });
            if close {
                self.settings_open = false;
            }
        }
        // 设置改过就记一笔；等鼠标松开再落盘 —— 拖滑杆时每帧写文件既没必要也磕手。
        self.settings_dirty |= settings_changed;
        if self.settings_dirty && !ctx.input(|input| input.pointer.any_down()) {
            settings.save();
            self.settings_dirty = false;
        }

        // ---- 加载面板 ----
        let load_anim = ctx.animate_bool_with_time(egui::Id::new("load-anim"), self.browsing, 0.13);
        if load_anim > 0.01 {
            let width = 360.0;
            let pos = egui::pos2(load_rect.right() - width, load_rect.bottom() + PANEL_GAP);
            let summaries = workspace.list();
            let open_id = workspace.id().to_string();
            let mut close = false;

            panel(ctx, "workflow-list", pos, width, load_anim, |ui| {
                header(
                    ui,
                    "加载",
                    &format!("{} 个存档", summaries.len()),
                    |ui| {
                        if widgets::button(
                            ui,
                            "新建",
                            Some(&icons::plus),
                            Variant::Ghost,
                            Size::Sm,
                            true,
                        )
                        .clicked()
                        {
                            action = Some(Action::New);
                            close = true;
                        }
                        if widgets::button(
                            ui,
                            "",
                            Some(&icons::x),
                            Variant::Ghost,
                            Size::IconSm,
                            true,
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    },
                );

                if summaries.is_empty() {
                    egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 6,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "还没有存档。按右上角的保存按钮，把当前工作流存下来。",
                                )
                                .color(theme::INK_3)
                                .size(10.5),
                            );
                        });
                } else {
                    egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: 8,
                            right: 8,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(420.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.add_space(6.0);
                                    for summary in &summaries {
                                        if let Some(next) = loader_row(
                                            ui,
                                            summary,
                                            summary.id == open_id,
                                            self.confirming.as_deref() == Some(summary.id.as_str()),
                                        ) {
                                            match next {
                                                LoaderAction::Open(id) => {
                                                    action = Some(Action::Load(id));
                                                    close = true;
                                                }
                                                LoaderAction::AskDelete(id) => {
                                                    self.confirming = Some(id)
                                                }
                                                LoaderAction::Cancel => self.confirming = None,
                                                LoaderAction::Delete(id) => {
                                                    if workspace.remove(&id) {
                                                        self.confirming = None;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    ui.add_space(6.0);
                                });
                        });
                }

                let foot = egui::Frame::default()
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        let link = ui.add(
                            egui::Label::new(
                                egui::RichText::new("打开存档目录")
                                    .color(theme::INK_3)
                                    .size(10.0)
                                    .underline(),
                            )
                            .sense(Sense::click()),
                        );
                        if link.clicked() {
                            let _ = open::that(workspace.workflow_dir());
                        }
                    });
                ui.allocate_rect(foot.response.rect, Sense::hover());
            });
            if close {
                self.browsing = false;
            }
        }

        action
    }
}

/// 工作流名输入框：外框由我们画，字由 egui 的 `TextEdit` 编。
fn name_field(ui: &mut egui::Ui, name: &mut String) -> egui::Response {
    let frame = egui::Frame::default()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::HAIRLINE))
        .corner_radius(CornerRadius::same(theme::R_CTL))
        .inner_margin(egui::Margin::symmetric(9, 6));
    let inner = frame.show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(name)
                .frame(egui::Frame::NONE)
                .desired_width(NAME_W)
                .font(FontId::monospace(12.0))
                .hint_text("未命名工作流"),
        )
    });
    let resp = inner.inner;
    if resp.has_focus() {
        ui.painter().rect_stroke(
            inner.response.rect,
            CornerRadius::same(theme::R_CTL),
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
    resp
}

/// 浮层外壳 + 固定位置。
fn panel(
    ctx: &egui::Context,
    id: &str,
    pos: egui::Pos2,
    width: f32,
    anim: f32,
    contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Area::new(egui::Id::new(id))
        .fixed_pos(pos + egui::vec2(0.0, (1.0 - anim) * -4.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_opacity(anim);
            widgets::panel_frame().show(ui, |ui| {
                ui.set_width(width);
                contents(ui);
            });
        });
}

/// 浮层顶栏：标题 + 灰色小字 + 右侧内容，底下一根发丝线。
fn header(ui: &mut egui::Ui, title: &str, meta: &str, right: impl FnOnce(&mut egui::Ui)) {
    let inner = egui::Frame::default()
        .inner_margin(egui::Margin {
            left: 12,
            right: 8,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .color(theme::INK)
                        .size(11.5)
                        .strong(),
                );
                ui.label(egui::RichText::new(meta).color(theme::INK_3).size(10.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
        });
    let rect = inner.response.rect;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, theme::HAIRLINE),
    );
}

fn fact(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(label).color(theme::INK_3).size(10.0));
        ui.label(
            egui::RichText::new(value)
                .color(theme::INK)
                .size(13.0)
                .strong(),
        );
    });
}

enum LoaderAction {
    Open(String),
    AskDelete(String),
    Delete(String),
    Cancel,
}

/// 加载列表里的一行。
fn loader_row(
    ui: &mut egui::Ui,
    summary: &starrytools_core::model::workflow::WorkflowSummary,
    current: bool,
    confirming: bool,
) -> Option<LoaderAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if confirming {
            ui.label(
                egui::RichText::new(format!("删除「{}」？", summary.name))
                    .color(theme::DANGER)
                    .size(10.5),
            );
            if widgets::button(
                ui,
                "",
                Some(&icons::trash),
                Variant::Danger,
                Size::IconSm,
                true,
            )
            .clicked()
            {
                action = Some(LoaderAction::Delete(summary.id.clone()));
            }
            if widgets::button(
                ui,
                "",
                Some(&icons::undo),
                Variant::Ghost,
                Size::IconSm,
                true,
            )
            .clicked()
            {
                action = Some(LoaderAction::Cancel);
            }
            return;
        }

        let width = (ui.available_width() - 26.0).max(60.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 42.0), Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::R_CTL), theme::SURFACE_2);
        }
        let name_color = if current { theme::ACCENT } else { theme::INK };
        let name =
            ui.painter()
                .layout_no_wrap(summary.name.clone(), FontId::monospace(11.5), name_color);
        let name_w = name.size().x;
        ui.painter().galley(
            egui::pos2(rect.left() + 8.0, rect.top() + 6.0),
            name,
            name_color,
        );
        if current {
            let badge = Rect::from_min_size(
                egui::pos2(rect.left() + 8.0 + name_w + 6.0, rect.top() + 7.0),
                egui::vec2(30.0, 14.0),
            );
            ui.painter()
                .rect_filled(badge, CornerRadius::same(3), theme::ACCENT_SOFT);
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                "当前",
                FontId::monospace(9.0),
                theme::ACCENT,
            );
        }
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.top() + 24.0),
            Align2::LEFT_TOP,
            format!(
                "{} 节点 · {} 连线 · {}",
                summary.node_count,
                summary.edge_count,
                format_time(summary.updated_at.unwrap_or(0))
            ),
            FontId::monospace(9.5),
            theme::INK_3,
        );
        if resp.clicked() {
            action = Some(LoaderAction::Open(summary.id.clone()));
        }

        if widgets::button(
            ui,
            "",
            Some(&icons::trash),
            Variant::Ghost,
            Size::IconSm,
            true,
        )
        .clicked()
        {
            action = Some(LoaderAction::AskDelete(summary.id.clone()));
        }
    });
    action
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 毫秒时间戳 → 「刚刚 / N 分钟前 / …」。
///
/// 显示相对时间，是为了不引时区库 —— 依赖越少越好。
fn format_time(millis: i64) -> String {
    if millis <= 0 {
        return "—".to_string();
    }
    let minutes = (now_millis() - millis) / 60_000;
    if minutes < 1 {
        "刚刚".to_string()
    } else if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 60 * 24 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / (60 * 24))
    }
}
