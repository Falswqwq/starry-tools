//! 参数控件、参数事件及说明文字缓存；控件与参数端口共用 layout 排版。
use super::{
    Graph, MODEL_PANEL_H, NODE_W, PANEL_INPUT_H, PARAM_BOOL_H, PARAM_LABEL_H, PARAM_LOD_ZOOM,
    TOOL_PANEL_H,
};
use crate::canvas::layout::*;
use crate::canvas::node::{kind_of, refresh_ports};
use crate::state::dialog;
use crate::ui::controls::{control, ControlEvent};
use crate::ui::theme;
use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, Stroke, StrokeKind};
use starrytools_core::model::node_kind::{NodeKindInfo, ParamDef, ParamSpec, PortDef};

/// 参数行右侧给面板输出留的宽度（Exec 徽标 + 序号 + 与圆点的间隔）。
const PANEL_OUT_GUTTER: f32 = 52.0;

impl Graph {
    /// 量一遍所有参数说明的高度（流坐标）。
    ///
    /// 宽度固定（`NODE_W - 20`）、字号固定（9.5），所以按说明文本本身做键就够了 ——
    /// 画的时候整体乘 `zoom`，和这里的值一一对应。说明是随节点类型静态不变的，
    /// 量过的就不再算。
    /// 把参数说明与运行提示都折好行、量好高（流坐标），缓存起来。
    ///
    /// 宽度、字号都固定（`NODE_W - 20` / 提示少一个图标的位置，`NOTE_FONT`），
    /// 所以按文本本身做键就够了 —— 画的时候整体乘 `zoom`。折行在这里定死，
    /// 缩放时不再重排，文字就不会跳。
    pub(super) fn sync_notes(&mut self, ui: &egui::Ui, kinds: &[NodeKindInfo]) {
        let painter = ui.painter();
        for kind in kinds {
            for param in &kind.kind.params {
                let Some(note) = &param.description else {
                    continue;
                };
                if self.notes.contains_key(note) {
                    continue;
                }
                let block = measure_block(painter, note, NODE_W - 20.0, theme::INK_3);
                self.notes.insert(note.clone(), block);
            }
        }

        // 运行提示也会折行，而且高度直接决定卡片多高 —— 同样量一遍。
        // （它们比参数说明多占一个图标的位置，所以按窄一点的宽度量；
        //  错误那一行没有图标，按整宽量。）
        let mut warnings: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for node in &self.nodes {
            let Some(run) = &node.run else { continue };
            warnings.extend(run.warnings.iter().cloned());
            errors.extend(run.error.iter().cloned());
        }
        for warning in warnings {
            if self.notes.contains_key(&warning) {
                continue;
            }
            let block = measure_block(painter, &warning, notice_text_width(), theme::INK_2);
            self.notes.insert(warning, block);
        }
        for error in errors {
            if self.notes.contains_key(&error) {
                continue;
            }
            let block = measure_block(painter, &error, NODE_W - 20.0, theme::DANGER);
            self.notes.insert(error, block);
        }
    }

    /// 一个参数控件这一帧发生了什么的统一处理（画布上所有控件都走它）。
    ///
    /// 返回「参数是否被改了」。文件框不在控件里弹对话框，而是把请求交给
    /// 后台选择器 [`dialog::Picker`]。
    pub(super) fn handle_control(
        &mut self,
        i: usize,
        param: &ParamDef,
        event: Option<ControlEvent>,
    ) -> bool {
        match event {
            Some(ControlEvent::Changed(value)) => {
                self.nodes[i].params.insert(param.id.clone(), value);
                refresh_ports(&mut self.nodes[i]);
                self.interaction.connect = None;
                self.interaction.hovered_port = None;
                true
            }

            Some(ControlEvent::PickFile {
                title,
                extensions,
                directory,
            }) => {
                self.picker.open(
                    dialog::Action::SetParam {
                        node_id: self.nodes[i].id.clone(),
                        param_id: param.id.clone(),
                    },
                    dialog::Spec {
                        title,
                        extensions,
                        mode: if directory {
                            dialog::Mode::Folder
                        } else {
                            dialog::Mode::File
                        },
                    },
                );
                false
            }
            None => false,
        }
    }

    /// 画一个节点的参数控件。**在它的卡片刚画完之后**调用 ——
    /// 这样叠放顺序才和卡片一致（下面那张卡的控件不会盖到上面那张卡上）。
    pub(super) fn draw_node_controls(
        &mut self,
        ui: &mut egui::Ui,
        kinds: &[NodeKindInfo],
        i: usize,
        now: f64,
    ) -> bool {
        let zoom = self.view.zoom;
        // 缩得太小时控件会挤成一团，索性只留标题和端口（LOD）。
        if zoom < PARAM_LOD_ZOOM {
            return false;
        }
        let Some(kind) = kind_of(kinds, &self.nodes[i]).cloned() else {
            return false;
        };
        let node_id = self.nodes[i].id.clone();
        let showing: Vec<usize> = (0..kind.kind.params.len())
            .filter(|&k| kind.kind.params[k].visible(&self.nodes[i].params))
            .collect();
        // 要模型但本地还没下载：整个节点禁用，参数全部灰下去 —— **只有那个「模型」
        // 下拉框还留着**，因为用户正是靠它挑要下哪个模型。缺外部程序（如 ffmpeg）时
        // 也整个禁用，但没有任何参数需要留着。
        let model_missing = self.nodes[i].requirements.model_missing;
        let tool_missing = self.nodes[i].requirements.tool_missing;
        // 面板布局也消费实例端口，避免后台解析后的类型和标签与控件错位。
        let panel_inputs: Vec<PortDef> = self.nodes[i]
            .inputs
            .iter()
            .filter(|port| port.panel_input_row.is_some())
            .cloned()
            .collect();
        if showing.is_empty() && panel_inputs.is_empty() && !model_missing && !tool_missing {
            return false;
        }

        let card = self.screen_rect(kinds, i);
        let left = card.left() + 10.0 * zoom;
        let width = card.width() - 20.0 * zoom;
        // 每个可见参数块占哪几行 —— 绘制读它，端口圆点也读它（`param_slots`）。
        let slots = param_slots(&self.nodes[i], &kind, &self.notes);
        let mut changed = false;

        for (slot_index, (k, slot)) in showing.iter().copied().zip(slots.iter()).enumerate() {
            let param = &kind.kind.params[k];
            // 这一行上挂了输出端口（顺序执行 / 遍历）：控件右侧留出徽标与序号的位置，
            // 标签由 `draw_ports` 跟着端口圆点逐个绘制。
            let has_panel_output = self.nodes[i]
                .outputs
                .iter()
                .any(|port| port.panel_row == Some(slot_index));
            let panel_gutter = if has_panel_output {
                PANEL_OUT_GUTTER * zoom
            } else {
                0.0
            };
            // 这个参数有没有可选输入端口 —— 有的话在参数名前面画一个类型徽标，
            // 接上之后控件就不可用了（值改从上游取）。
            let port = self.nodes[i]
                .inputs
                .iter()
                .find(|port| port.param.as_deref() == Some(param.id.as_str()))
                .cloned();
            let linked = port
                .as_ref()
                .is_some_and(|port| self.port_linked(i, &port.id, true));
            // 缺模型时：除「模型」参数外全部禁用。缺外部程序时全部禁用。
            let is_model_param = kind.model_param.as_deref() == Some(param.id.as_str());
            let disabled = linked || tool_missing || (model_missing && !is_model_param);

            // 开关：标签和开关同一行。
            if matches!(param.spec, ParamSpec::Bool { .. }) {
                let row = Rect::from_min_size(
                    egui::pos2(left, card.top() + slot.control_y * zoom),
                    egui::vec2(width, PARAM_BOOL_H * zoom),
                );
                let mut label_x = row.left();
                if let Some(port) = &port {
                    label_x += type_chip(ui.painter(), port, row.left(), row.center().y, zoom);
                }
                if has_label(param) {
                    ui.painter().text(
                        egui::pos2(label_x, row.center().y),
                        Align2::LEFT_CENTER,
                        &param.label,
                        FontId::monospace(10.0 * zoom),
                        theme::INK_3,
                    );
                }
                let switch = Rect::from_min_size(
                    egui::pos2(row.right() - 30.0 * zoom - panel_gutter, row.top()),
                    egui::vec2(30.0 * zoom, row.height()),
                );
                let event = control(
                    ui,
                    &node_id,
                    param,
                    switch,
                    &self.nodes[i].params,
                    zoom,
                    disabled,
                );
                if self.handle_control(i, param, event) {
                    changed = true;
                }
                continue;
            }

            // 标签一行（没有标签就整行省掉）。
            if let Some(label_y) = slot.label_y {
                let label_top = card.top() + label_y * zoom;
                let line = PARAM_LABEL_H * zoom;
                let mut label_x = left;
                if let Some(port) = &port {
                    label_x += type_chip(ui.painter(), port, left, label_top + line * 0.5, zoom);
                }
                ui.painter().text(
                    egui::pos2(label_x, label_top),
                    Align2::LEFT_TOP,
                    &param.label,
                    FontId::monospace(10.0 * zoom),
                    theme::INK_3,
                );
            }

            // 控件一行。没有标签行时，类型徽标没有地方放 —— 挤到控件左侧，
            // 控件相应让出这点宽度（字面量的「值」就是这种无标签参数）。
            let control_y = card.top() + slot.control_y * zoom;
            let height = slot.control_h * zoom;
            let mut control_left = left;
            if slot.label_y.is_none() {
                if let Some(port) = &port {
                    control_left +=
                        type_chip(ui.painter(), port, left, control_y + height * 0.5, zoom);
                }
            }
            let control_rect = Rect::from_min_size(
                egui::pos2(control_left, control_y),
                egui::vec2(width - (control_left - left) - panel_gutter, height),
            );
            let event = control(
                ui,
                &node_id,
                param,
                control_rect,
                &self.nodes[i].params,
                zoom,
                disabled,
            );
            if self.handle_control(i, param, event) {
                changed = true;
            }

            // 说明一行（自动折行）。
            if let (Some(note_y), Some(note)) = (slot.note_y, &param.description) {
                if let Some(block) = self.notes.get(note) {
                    draw_block(
                        ui.painter(),
                        block,
                        zoom,
                        left,
                        card.top() + note_y * zoom,
                        theme::INK_3,
                    );
                }
            }
        }

        // ---- 参数区下方的输入行（「设定值」） ----
        if !panel_inputs.is_empty() {
            let top = card.top()
                + (node_body_top(&self.nodes[i])
                    + params_height(&kind, &self.nodes[i].params, &self.notes))
                    * zoom;
            for port in &panel_inputs {
                let row = port.panel_input_row.unwrap_or(0);
                let center = top + (row as f32 + 0.5) * PANEL_INPUT_H * zoom;
                // 类型徽标 + 「设定值」；圆点由端口绘制负责（卡片左边缘）。
                let chip = type_chip(ui.painter(), port, left, center, zoom);
                ui.painter().text(
                    egui::pos2(left + chip, center),
                    Align2::LEFT_CENTER,
                    &port.label,
                    FontId::monospace(10.0 * zoom),
                    theme::INK_3,
                );
            }
        }

        // ---- 模型下载面板（紧贴参数区：挑源 + 下载 / 进度 / 重试） ----
        if model_missing {
            let top = card.top()
                + (node_body_top(&self.nodes[i])
                    + params_height(&kind, &self.nodes[i].params, &self.notes))
                    * zoom;
            let rect = Rect::from_min_size(
                egui::pos2(card.left(), top),
                egui::vec2(card.width(), MODEL_PANEL_H * zoom),
            );
            self.draw_model_panel(ui, i, &kind, rect, zoom, now);
        } else if tool_missing {
            let top = card.top()
                + (node_body_top(&self.nodes[i])
                    + params_height(&kind, &self.nodes[i].params, &self.notes))
                    * zoom;
            let rect = Rect::from_min_size(
                egui::pos2(card.left(), top),
                egui::vec2(card.width(), TOOL_PANEL_H * zoom),
            );
            self.draw_tool_panel(ui, &kind, rect, zoom);
        }

        changed
    }
}

/// 类型徽标的字号、左右内边距、后面那段间隔。
const CHIP_FONT: f32 = 8.5;
const CHIP_PAD_X: f32 = 4.0;
const CHIP_TRAIL: f32 = 5.0;

/// 类型徽标（chip）：标在参数名前，标出这个参数 / 这个端口的类型。
/// 颜色走 [`theme::badge_color`]，和端口列上的徽标同色。返回它连同后面那段间隔的宽度。
fn type_chip(painter: &egui::Painter, port: &PortDef, x: f32, center_y: f32, zoom: f32) -> f32 {
    let color = theme::badge_color(port.ty);
    let g = painter.layout_no_wrap(
        port.badge().to_owned(),
        FontId::monospace(CHIP_FONT * zoom),
        color,
    );
    let w = g.size().x + CHIP_PAD_X * 2.0 * zoom;
    let h = g.size().y + 2.0 * zoom;
    let r = Rect::from_min_size(egui::pos2(x, center_y - h * 0.5), egui::vec2(w, h));
    painter.rect_stroke(
        r,
        CornerRadius::same((3.0 * zoom) as u8),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(
            r.center().x - g.size().x * 0.5,
            crate::ui::widgets::ink_top(&g, r.center().y),
        ),
        g,
        color,
    );
    w + CHIP_TRAIL * zoom
}
