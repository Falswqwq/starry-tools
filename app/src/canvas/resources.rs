//! 后台文件选择结果与模型/外部工具面板；耗时复制不占用 UI 线程。
use super::{
    Graph, MODEL_ACTION_GAP, MODEL_ACTION_H, MODEL_NOTE_H, MODEL_PAD, MODEL_SELECT_H, MODEL_TITLE_H,
};
use crate::canvas::node::refresh_ports;
use crate::catalog::KindRequirements;
use crate::state::dialog;
use crate::ui::controls::select_field;
use crate::ui::{icons, theme};
use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind};
use starrytools_core::model::node_kind::{NodeKindInfo, SelectOption};

impl Graph {
    /// 只在工具探测完成时刷新依赖，不碰模型磁盘或动态端口。
    pub(super) fn sync_tool_requirements(&mut self) {
        let revision = crate::catalog::tool_revision();
        if self.tool_revision == Some(revision) {
            return;
        }
        for node in &mut self.nodes {
            crate::catalog::refresh_tool_requirement(&node.kind, &mut node.requirements);
        }
        self.tool_revision = Some(revision);
    }
    /// 把后台选好的文件 / 目录贴到它该去的地方。
    ///
    /// 选择框是在后台线程上开的（见 [`crate::state::dialog`]），结果用通道送回来；
    /// 这里每帧收一次。
    pub(super) fn apply_pick(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        while let Some((action, path)) = self.picker.poll() {
            match action {
                dialog::Action::SetParam { node_id, param_id } => {
                    if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
                        node.params
                            .insert(param_id, serde_json::json!(path.to_string_lossy()));
                        // 类型可能跟着文件变（「读取」这类），端口重算一遍。
                        refresh_ports(node);
                        changed = true;
                    }
                }
                dialog::Action::SaveAs { source } => {
                    // 产物可能很大（视频），拷贝不能放在界面线程上 —— 否则会瞬间冻住窗口。
                    // 失败一直静默（和原来一致）；这里只管不卡。
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let _ = std::fs::copy(source, &path);
                        ctx.request_repaint();
                    });
                }
            }
        }
        if changed {
            self.interaction.connect = None;
            self.interaction.hovered_port = None;
            self.touch_content();
        }
    }

    /// 画「缺外部程序」那块面板：一个警示图标 + 一句提示（底色已由 `draw_node` 铺好）。
    pub(super) fn draw_tool_panel(
        &self,
        ui: &egui::Ui,
        kind: &NodeKindInfo,
        rect: Rect,
        zoom: f32,
    ) {
        let tool = kind.requires_tool.as_deref().unwrap_or("外部程序");
        let painter = ui.painter().with_clip_rect(rect);
        let icon = Rect::from_min_size(
            egui::pos2(rect.left() + 10.0 * zoom, rect.center().y - 6.0 * zoom),
            egui::Vec2::splat(12.0 * zoom),
        );
        icons::info(&painter, icon, theme::WARN);
        let message = format!("未找到 {tool}，先装好它并放进 PATH");
        painter.text(
            egui::pos2(icon.right() + 6.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            message,
            FontId::monospace(9.5 * zoom),
            theme::WARN,
        );
    }

    /// 画「模型还没下载」那块面板：模型名 + 大小 + 一句说明 + 源下拉 + 下载 / 进度 / 重试。
    ///
    /// 面板底色由 [`Self::draw_node`] 铺在参数区下面，这里只画内容。
    pub(super) fn draw_model_panel(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        kind: &NodeKindInfo,
        rect: Rect,
        zoom: f32,
        now: f64,
    ) {
        let node_id = self.nodes[i].id.clone();
        let Some(model) = kind.model(&self.nodes[i].params) else {
            return;
        };

        let pad = MODEL_PAD * zoom;
        let left = rect.left() + pad;
        let width = rect.width() - pad * 2.0;
        let top = rect.top() + pad;
        let note_y = top + MODEL_TITLE_H * zoom;
        let select_y = top + (MODEL_TITLE_H + MODEL_NOTE_H) * zoom;
        let select_h = MODEL_SELECT_H * zoom;
        let action_h = MODEL_ACTION_H * zoom;
        let action_y = select_y + select_h + MODEL_ACTION_GAP * zoom;

        // 当前状态：正在下 / 失败 / 待下载。
        let busy = self.downloads.busy(&node_id);
        let failed = self
            .downloads
            .get(&node_id)
            .and_then(|task| task.failed.clone());
        let fraction = self
            .downloads
            .get(&node_id)
            .and_then(crate::state::models::Download::fraction);
        let source = self.downloads.source(&node_id);

        // 标题：模型名 + 大小。
        ui.painter().text(
            egui::pos2(left, top),
            Align2::LEFT_TOP,
            format!("{}  {}", model.name, model.size),
            FontId::monospace(11.0 * zoom),
            theme::INK,
        );
        // 第二行：失败原因（红）优先，否则那句说明（灰）。剪在面板里，长了也不出框。
        let (note, note_color) = match &failed {
            Some(err) => (err.as_str(), theme::DANGER),
            None => (model.note, theme::INK_3),
        };
        ui.painter().with_clip_rect(rect).text(
            egui::pos2(left, note_y),
            Align2::LEFT_TOP,
            note,
            FontId::monospace(9.5 * zoom),
            note_color,
        );

        // 源下拉。正在下载时锁住 —— 别下半道再换源。
        let options: Vec<SelectOption> = model
            .sources
            .iter()
            .enumerate()
            .map(|(k, source)| SelectOption {
                value: k.to_string(),
                label: source.label.to_string(),
                hint: None,
            })
            .collect();
        if !options.is_empty() {
            let select_rect =
                Rect::from_min_size(egui::pos2(left, select_y), egui::vec2(width, select_h));
            let current = serde_json::json!(source.to_string());
            if let Some(value) = select_field(
                ui,
                ui.id().with(("model-source", &node_id)),
                select_rect,
                Some(&current),
                &options,
                zoom,
                busy,
            ) {
                if let Some(picked) = value.as_str().and_then(|text| text.parse::<usize>().ok()) {
                    self.downloads.set_source(&node_id, picked);
                }
            }
        }

        // 底下一整行：待下载 → 主色按钮；正在下 → 进度条（点一下取消）；失败 → 重试。
        let action = Rect::from_min_size(egui::pos2(left, action_y), egui::vec2(width, action_h));
        let resp = ui.interact(
            action,
            ui.id().with(("model-action", &node_id)),
            Sense::click(),
        );
        let hot = resp.hovered();
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let cr = CornerRadius::same(theme::R_CTL);
        let painter = ui.painter();

        if busy {
            painter.rect_filled(action, cr, theme::SURFACE);
            painter.rect_stroke(
                action,
                cr,
                Stroke::new(1.0, theme::ACCENT_LINE),
                StrokeKind::Inside,
            );
            match fraction {
                Some(fraction) => {
                    let mut fill = action;
                    fill.set_right(action.left() + action.width() * fraction);
                    painter.rect_filled(fill, cr, theme::ACCENT_SOFT);
                }
                None => {
                    // 不知道总量：一段来回走的浅蓝，表示「在下但说不准还剩多少」。
                    let width = action.width() * 0.35;
                    let t = (now * 0.7).rem_euclid(1.0) as f32;
                    let x0 = (action.left() - width + (action.width() + width) * t)
                        .max(action.left() + 2.0);
                    let x1 = (x0 + width).min(action.right() - 2.0);
                    if x1 > x0 {
                        painter.rect_filled(
                            Rect::from_min_max(
                                egui::pos2(x0, action.top() + 2.0),
                                egui::pos2(x1, action.bottom() - 2.0),
                            ),
                            CornerRadius::same((theme::R_CTL as f32 * 0.7) as u8),
                            theme::ACCENT_SOFT,
                        );
                    }
                }
            }
            let label = if hot {
                "取消下载".to_string()
            } else {
                match fraction {
                    Some(fraction) => format!("下载中 {}%", (fraction * 100.0).round() as i64),
                    None => "下载中…".to_string(),
                }
            };
            painter.text(
                action.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(10.5 * zoom),
                theme::ACCENT,
            );
            if resp.clicked() {
                self.downloads.cancel(&node_id);
            }
        } else {
            let fill = if hot {
                theme::ACCENT_HOVER
            } else {
                theme::ACCENT
            };
            painter.rect_filled(action, cr, fill);
            let label = if failed.is_some() {
                "重试下载"
            } else {
                "下载模型"
            };
            let galley = painter.layout_no_wrap(
                label.to_string(),
                FontId::monospace(10.5 * zoom),
                theme::INK_ON_COLOR,
            );
            let icon_box = 12.0 * zoom;
            let gap = 5.0 * zoom;
            let content = icon_box + gap + galley.size().x;
            let x = action.center().x - content * 0.5;
            icons::download(
                painter,
                Rect::from_min_size(
                    egui::pos2(x, action.center().y - icon_box * 0.5),
                    egui::vec2(icon_box, icon_box),
                ),
                theme::INK_ON_COLOR,
            );
            painter.galley(
                egui::pos2(
                    x + icon_box + gap,
                    crate::ui::widgets::ink_top(&galley, action.center().y),
                ),
                galley,
                theme::INK_ON_COLOR,
            );
            if resp.clicked() {
                self.downloads.start(&node_id, model, source);
            }
        }
    }
}
