//! 底部那颗状态药丸。跑完工作流后点开，向上弹出运行记录。

use eframe::egui::{self, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind};

use starrytools_core::engine::{NodeStatus, RunReport};

use crate::ui::icons;
use crate::state::run::Runner;
use crate::ui::theme;
use crate::ui::widgets::{self, Size, Variant};

/// 交给调用方的动作（选中某个节点要在画布上做）。
pub enum Action {
    Select(String),
}

#[derive(Default)]
pub struct Report {
    /// 运行记录展开着吗。
    open: bool,
}

const MARGIN: f32 = 12.0;
const DOT: f32 = 6.0;
const WIDTH: f32 = 620.0;

/// 药丸当前该显示的样子。
struct Status {
    text: String,
    /// `None` = 普通灰点；`Some(color)` = 上色。
    dot: Color32,
    busy: bool,
    kind: Kind,
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Idle,
    Info,
    Error,
    /// 紫色节点拦住了运行，在等用户操作。
    Waiting,
}

impl Report {
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        runner: &mut Runner,
        _runnable: bool,
        _errors: usize,
    ) -> Option<Action> {
        let mut action = None;

        let running = runner.is_running();
        let report = runner.report();
        let status = self.status(runner, running, report);
        let has_report = report.is_some();

        // ---- 药丸本体 ----
        egui::Area::new(egui::Id::new("status-pill"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -MARGIN))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (_rect, resp) = self.draw_pill(ui, &status, report, running, has_report);
                if resp.clicked() && has_report {
                    self.open = !self.open;
                }
                if has_report && (resp.hovered() || self.open) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            });

        // ---- 运行记录（向上弹出） ----
        if !self.open {
            return action;
        }
        let Some(report) = runner.report() else {
            self.open = false;
            return action;
        };

        let mut close = false;
        let mut clear = false;

        egui::Area::new(egui::Id::new("run-log"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -(MARGIN + 30.0 + 10.0)),
            )
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                widgets::panel_frame().show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // 顶栏
                    let head = egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 8,
                            top: 8,
                            bottom: 8,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("运行记录")
                                        .color(theme::INK)
                                        .size(11.5)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{} 个节点", report.nodes.len()))
                                        .color(theme::INK_3)
                                        .size(10.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
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
                                            clear = true;
                                        }
                                        if let Some(dir) = &report.output_dir {
                                            let dir = dir.clone();
                                            if widgets::button(
                                                ui,
                                                "",
                                                Some(&icons::folder_open),
                                                Variant::Ghost,
                                                Size::IconSm,
                                                true,
                                            )
                                            .clicked()
                                            {
                                                let _ = open::that(dir);
                                            }
                                        }
                                    },
                                );
                            });
                        });
                    ui.painter().line_segment(
                        [
                            egui::pos2(head.response.rect.left(), head.response.rect.bottom()),
                            egui::pos2(head.response.rect.right(), head.response.rect.bottom()),
                        ],
                        Stroke::new(1.0, theme::HAIRLINE),
                    );

                    if report.nodes.is_empty() {
                        egui::Frame::default()
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                for issue in &report.issues {
                                    ui.label(
                                        egui::RichText::new(&issue.message)
                                            .color(theme::DANGER)
                                            .size(10.5),
                                    );
                                }
                            });
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(340.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (index, node) in report.nodes.iter().enumerate() {
                                    if log_row(ui, index, node) {
                                        action = Some(Action::Select(node.node_id.clone()));
                                        close = true;
                                    }
                                }
                            });
                    }
                    ui.add_space(6.0);
                });
            });

        if clear {
            runner.clear();
            self.open = false;
        } else if close {
            self.open = false;
        }

        action
    }

    fn status(&self, runner: &Runner, running: bool, report: Option<&RunReport>) -> Status {
        if let Some(waiting) = runner.waiting() {
            return Status {
                text: format!("等待「{}」操作…", waiting.node_name),
                dot: theme::PURPLE,
                busy: true,
                kind: Kind::Waiting,
            };
        }
        if running {
            return Status {
                text: "正在运行…".to_string(),
                dot: theme::ACCENT,
                busy: true,
                kind: Kind::Idle,
            };
        }
        if let Some(error) = runner.error() {
            return Status {
                text: error.to_string(),
                dot: theme::DANGER,
                busy: false,
                kind: Kind::Error,
            };
        }
        if let Some(report) = report {
            let failed = report
                .nodes
                .iter()
                .filter(|node| matches!(node.status, NodeStatus::Failed))
                .count();
            return if failed == 0 {
                Status {
                    text: "完成".to_string(),
                    dot: theme::ACCENT,
                    busy: false,
                    kind: Kind::Info,
                }
            } else {
                Status {
                    text: format!("{failed} 个失败"),
                    dot: theme::DANGER,
                    busy: false,
                    kind: Kind::Error,
                }
            };
        }
        Status {
            text: "就绪".to_string(),
            dot: theme::INK_3,
            busy: false,
            kind: Kind::Idle,
        }
    }

    fn draw_pill(
        &self,
        ui: &mut egui::Ui,
        status: &Status,
        report: Option<&RunReport>,
        running: bool,
        enabled: bool,
    ) -> (Rect, egui::Response) {
        let text_color = match status.kind {
            Kind::Error => theme::DANGER,
            Kind::Waiting => theme::PURPLE,
            _ => theme::INK_2,
        };
        let stat = report.map(|report| {
            let failed = report
                .nodes
                .iter()
                .filter(|node| !matches!(node.status, NodeStatus::Ok))
                .count();
            let mut text = format!("{} 节点 · {}ms", report.nodes.len(), report.duration_ms);
            if failed > 0 {
                text.push_str(&format!(" · {failed} 未通过"));
            }
            text
        });

        let galley =
            ui.painter()
                .layout_no_wrap(status.text.clone(), FontId::monospace(11.0), text_color);
        let gap = 8.0;
        let mut width = 12.0 + DOT + gap + galley.size().x;
        let stat_galley = stat.as_ref().map(|text| {
            let g =
                ui.painter()
                    .layout_no_wrap(text.clone(), FontId::monospace(10.0), theme::INK_3);
            width += gap + 1.0 + gap + g.size().x;
            g
        });
        if enabled {
            // 小箭头右边再留一截空白，视觉上才平衡。
            width += gap + 12.0;
        }
        width += 12.0;

        let height = 30.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
        if !enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }

        let cr = CornerRadius::same((height / 2.0) as u8);
        widgets::card_shadow(ui.painter(), rect, cr.nw);
        let painter = ui.painter();
        let hovered = enabled && resp.hovered();
        painter.rect(
            rect,
            cr,
            theme::SURFACE,
            Stroke::new(
                1.0,
                if hovered {
                    theme::HAIRLINE_STRONG
                } else {
                    theme::HAIRLINE
                },
            ),
            StrokeKind::Inside,
        );

        let mut x = rect.left() + 12.0;
        let cy = rect.center().y;

        // 圆点。运行中时闪。
        let dot_alpha = if status.busy {
            let pulse = 0.35 + 0.65 * ((ui.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.5);
            Color32::from_rgba_unmultiplied(
                status.dot.r(),
                status.dot.g(),
                status.dot.b(),
                (pulse * 255.0) as u8,
            )
        } else {
            status.dot
        };
        painter.circle_filled(egui::pos2(x + DOT / 2.0, cy), DOT / 2.0, dot_alpha);
        x += DOT + gap;

        painter.galley(
            egui::pos2(x, cy - galley.size().y / 2.0),
            galley.clone(),
            text_color,
        );
        x += galley.size().x;

        if let Some(g) = stat_galley {
            x += gap;
            painter.line_segment(
                [egui::pos2(x, cy - 8.0), egui::pos2(x, cy + 8.0)],
                Stroke::new(1.0, theme::HAIRLINE),
            );
            x += 1.0 + gap;
            painter.galley(
                egui::pos2(x, cy - g.size().y / 2.0),
                g.clone(),
                theme::INK_3,
            );
            x += g.size().x;
        }

        if enabled {
            x += gap;
            let icon_rect = Rect::from_center_size(egui::pos2(x + 6.0, cy), egui::vec2(12.0, 12.0));
            icons::chevron_up(painter, icon_rect, theme::INK_3);
        }
        let _ = running;

        (rect, resp)
    }
}

/// 运行记录里的一行。返回 `true` 表示这一行的节点名被点了。
fn log_row(
    ui: &mut egui::Ui,
    index: usize,
    node: &starrytools_core::engine::NodeRunResult,
) -> bool {
    let mut clicked = false;
    let (mark, color) = match node.status {
        NodeStatus::Ok => ("完成", theme::OK),
        NodeStatus::Failed => ("出错", theme::DANGER),
        NodeStatus::Skipped => ("跳过", theme::INK_3),
    };

    egui::Frame::default()
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 4,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [22.0, 18.0],
                    egui::Label::new(
                        egui::RichText::new(format!("{:02}", index + 1))
                            .color(theme::INK_3)
                            .size(11.0),
                    ),
                );
                let name = ui.add(
                    egui::Label::new(egui::RichText::new(&node.name).color(theme::INK).size(11.0))
                        .sense(Sense::click()),
                );
                if name.clicked() {
                    clicked = true;
                }
                name.on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(mark).color(color).size(10.0));
                ui.label(
                    egui::RichText::new(format!("{}ms", node.elapsed_ms))
                        .color(theme::INK_3)
                        .size(11.0),
                );
            });

            let has_detail =
                node.error.is_some() || !node.warnings.is_empty() || !node.outputs.is_empty();
            if has_detail {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(30.0);
                    if let Some(error) = &node.error {
                        ui.label(egui::RichText::new(error).color(theme::DANGER).size(10.0));
                    }
                    for warning in &node.warnings {
                        ui.label(egui::RichText::new(warning).color(theme::WARN).size(10.0));
                    }
                    for output in &node.outputs {
                        ui.label(
                            egui::RichText::new(&output.summary)
                                .color(theme::INK_3)
                                .size(10.0),
                        );
                    }
                });
            }
        });

    clicked
}
