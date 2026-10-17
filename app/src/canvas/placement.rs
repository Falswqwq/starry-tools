//! 节点、端口、操作按钮与连线的坐标定位、命中测试及视野适配。
use super::{Graph, HEADER_H, NODE_W, PORT_HIT, PORT_ROW_H, RUN_ACTIONS_H};
use crate::canvas::geometry::Cubic;
use crate::canvas::layout::{height_of_node, panel_input_offset, panel_output_offset, param_slots};
use crate::canvas::node::{kind_of, port_index, PortRef, Wire, TOOL_SIZE};
use eframe::egui::{self, Pos2, Rect};
use starrytools_core::model::node_kind::NodeKindInfo;

impl Graph {
    pub(super) fn flow_rect(&self, kinds: &[NodeKindInfo], i: usize) -> Rect {
        Rect::from_min_size(
            self.nodes[i].pos,
            egui::vec2(NODE_W, height_of_node(kinds, &self.nodes[i], &self.notes)),
        )
    }

    pub(super) fn screen_rect(&self, kinds: &[NodeKindInfo], i: usize) -> Rect {
        let r = self.flow_rect(kinds, i);
        Rect::from_min_max(self.view.to_screen(r.min), self.view.to_screen(r.max))
    }

    /// 第 `k` 个输入端口（流坐标）。
    ///
    /// 参数端口不在端口列里 —— 它跟着那个参数的行走（浅蓝的小圆点）。
    pub(super) fn in_port(&self, kinds: &[NodeKindInfo], i: usize, k: usize) -> Pos2 {
        let r = self.flow_rect(kinds, i);
        let node = &self.nodes[i];
        if node.inputs.get(k).is_some_and(|port| port.exec) {
            let column = node.inputs[..k]
                .iter()
                .filter(|candidate| candidate.exec)
                .count();

            return egui::pos2(
                r.left(),
                r.top() + HEADER_H + PORT_ROW_H * (column as f32 + 0.5),
            );
        }
        // 参数区下方的输入行（「设定值」）：位置读 layout，和绘制共用一份排版。
        if let Some(row) = node.inputs.get(k).and_then(|port| port.panel_input_row) {
            if let Some(kind) = kind_of(kinds, node) {
                return egui::pos2(
                    r.left(),
                    r.top() + panel_input_offset(node, kind, &self.notes, row),
                );
            }
        }
        let param_id = node.inputs.get(k).and_then(|port| port.param.as_deref());
        match param_id.and_then(|id| self.param_port_offset(kinds, i, id)) {
            Some(offset) => egui::pos2(r.left(), r.top() + offset),
            None => {
                // 端口列只数**非参数端口**（对称于 `out_port`）：
                // 即使参数端口夹在中间，位置也不会错。
                let column = node.inputs[..k.min(node.inputs.len())]
                    .iter()
                    .filter(|port| !port.exec && !port.is_param() && port.panel_input_row.is_none())
                    .count()
                    + node.inputs.iter().filter(|port| port.exec).count();
                egui::pos2(
                    r.left(),
                    r.top() + HEADER_H + PORT_ROW_H * (column as f32 + 0.5),
                )
            }
        }
    }

    pub(super) fn out_port(&self, kinds: &[NodeKindInfo], i: usize, k: usize) -> Pos2 {
        let r = self.flow_rect(kinds, i);
        let node = &self.nodes[i];
        if node
            .outputs
            .get(k)
            .is_some_and(|port| port.exec && port.panel_row.is_none())
        {
            let column = node.outputs[..k]
                .iter()
                .filter(|port| port.exec && port.panel_row.is_none())
                .count();
            return egui::pos2(
                r.right(),
                r.top() + HEADER_H + PORT_ROW_H * (column as f32 + 0.5),
            );
        }
        // 挂在参数栏上的输出（动态端口）：位置跟着参数那一行。
        if let Some(row) = node.outputs.get(k).and_then(|port| port.panel_row) {
            if let Some(kind) = kind_of(kinds, node) {
                if let Some(offset) = panel_output_offset(kind, node, &self.notes, row) {
                    return egui::pos2(r.right(), r.top() + offset);
                }
            }
        }
        // 常规输出：按顶部端口列里**非面板**输出的序号排。
        let column = node.outputs[..k.min(node.outputs.len())]
            .iter()
            .filter(|port| !port.exec && port.panel_row.is_none())
            .count()
            + node
                .outputs
                .iter()
                .filter(|port| port.exec && port.panel_row.is_none())
                .count();
        egui::pos2(
            r.right(),
            r.top() + HEADER_H + PORT_ROW_H * (column as f32 + 0.5),
        )
    }

    /// 某个参数行上那个小圆点相对卡片顶部的高度（流坐标）。
    ///
    /// 参数不可见、或者这个参数根本没有端口时就返回 `None`。位置直接读
    /// [`param_slots`] —— 和绘制控件用的是同一份排版。
    pub(super) fn param_port_offset(
        &self,
        kinds: &[NodeKindInfo],
        i: usize,
        param_id: &str,
    ) -> Option<f32> {
        let node = &self.nodes[i];
        let kind = kind_of(kinds, node)?;
        param_slots(node, kind, &self.notes)
            .into_iter()
            .find(|slot| slot.id == param_id)
            .map(|slot| slot.dot_y)
    }

    /// 连线的四个控制点（**流坐标**）。
    ///
    /// 一定要在缩放之前算：否则 `reach` 的那个最小值会被缩放影响，
    /// 同一张图在不同缩放下的形状会不一样。
    pub(super) fn wire_points(&self, kinds: &[NodeKindInfo], wire: &Wire) -> Cubic {
        let source = &self.nodes[wire.from];
        let target = &self.nodes[wire.to];
        let a = self.out_port(
            kinds,
            wire.from,
            port_index(&source.outputs, &wire.from_port),
        );
        let b = self.in_port(kinds, wire.to, port_index(&target.inputs, &wire.to_port));
        let reach = ((b.x - a.x).abs() * 0.55).max(48.0);
        [
            a,
            egui::pos2(a.x + reach, a.y),
            egui::pos2(b.x - reach, b.y),
            b,
        ]
    }

    /// 命中测试：后画的在上面，所以从后往前找。
    pub(super) fn hit(&self, kinds: &[NodeKindInfo], flow: Pos2) -> Option<usize> {
        (0..self.nodes.len())
            .rev()
            .find(|&i| self.flow_rect(kinds, i).contains(flow))
    }

    pub(crate) fn can_run_node(&self, kinds: &[NodeKindInfo], i: usize) -> bool {
        self.nodes
            .get(i)
            .and_then(|node| kind_of(kinds, node))
            .is_some_and(|kind| kind.pure)
    }

    /// 卡片右上角两个工具按钮的矩形（屏幕坐标）：返回 `(运行至此, 删除)`。
    pub(super) fn node_tool_rects(&self, kinds: &[NodeKindInfo], i: usize) -> (Rect, Rect) {
        let r = self.screen_rect(kinds, i);
        let z = self.view.zoom;
        let s = TOOL_SIZE * z;
        let pad = 5.0 * z;
        let gap = 1.0 * z;
        let y = r.top() + (HEADER_H * z - s) * 0.5;
        let del = Rect::from_min_size(egui::pos2(r.right() - pad - s, y), egui::vec2(s, s));
        let play = Rect::from_min_size(egui::pos2(del.left() - gap - s, y), egui::vec2(s, s));
        (play, del)
    }

    /// 指针现在悬在哪个节点上（决定要不要露出工具按钮）。
    pub(super) fn hovered_node(
        &self,
        kinds: &[NodeKindInfo],
        pointer: Option<Pos2>,
    ) -> Option<usize> {
        pointer.and_then(|p| self.hit(kinds, self.view.to_flow(p)))
    }

    /// 某个端口在流坐标里的位置。
    pub(super) fn port_pos(&self, kinds: &[NodeKindInfo], port: PortRef) -> Pos2 {
        if port.input {
            self.in_port(kinds, port.node, port.port)
        } else {
            self.out_port(kinds, port.node, port.port)
        }
    }

    /// 指针底下有没有端口。判定半径是**屏幕**像素 —— 缩放不该让端口更难或更好点。
    pub(super) fn port_at(&self, kinds: &[NodeKindInfo], flow: Pos2) -> Option<PortRef> {
        let reach = PORT_HIT / self.view.zoom;
        let mut best: Option<(f32, PortRef)> = None;

        for node in 0..self.nodes.len() {
            let ports = (0..self.nodes[node].outputs.len())
                .map(|port| PortRef {
                    node,
                    port,
                    input: false,
                })
                .chain((0..self.nodes[node].inputs.len()).map(|port| PortRef {
                    node,
                    port,
                    input: true,
                }));

            for candidate in ports {
                let distance = (self.port_pos(kinds, candidate) - flow).length();
                if distance <= reach && best.is_none_or(|(best, _)| distance < best) {
                    best = Some((distance, candidate));
                }
            }
        }

        best.map(|(_, port)| port)
    }

    /// 「另存为」：把产物拷到用户挑的位置。
    /// 指针现在悬在哪个节点上（决定要不要露出工具按钮）。
    /// 卡片底部「产物操作行」两个按钮的矩形：`(在文件夹中显示, 另存为)`。
    pub(super) fn node_action_rects(&self, kinds: &[NodeKindInfo], i: usize) -> (Rect, Rect) {
        let r = self.screen_rect(kinds, i);
        let z = self.view.zoom;
        let s = TOOL_SIZE * z;
        let pad = 7.0 * z;
        let y = r.bottom() - RUN_ACTIONS_H * z + (RUN_ACTIONS_H * z - s) * 0.5;
        let reveal = Rect::from_min_size(egui::pos2(r.left() + pad, y), egui::vec2(s, s));
        let save = Rect::from_min_size(egui::pos2(reveal.right() + 2.0 * z, y), egui::vec2(s, s));
        (reveal, save)
    }

    /// 以画布中央为锚点缩放。
    pub(super) fn zoom_by(&mut self, factor: f32, canvas: Rect) {
        let anchor = self.view.to_flow(canvas.center());
        self.view.zoom = (self.view.zoom * factor).clamp(0.15, 2.0);
        self.view.pan = canvas.center().to_vec2() - anchor.to_vec2() * self.view.zoom;
    }

    /// 把所有节点框进视野。
    pub(super) fn fit_view(&mut self, canvas: Rect, kinds: &[NodeKindInfo]) {
        if self.nodes.is_empty() {
            self.view.zoom = 1.0;
            self.view.pan = egui::vec2(40.0, 30.0);
            return;
        }
        let mut bounds: Option<Rect> = None;
        for node in &self.nodes {
            let rect = Rect::from_min_size(
                node.pos,
                egui::vec2(NODE_W, height_of_node(kinds, node, &self.notes)),
            );
            bounds = Some(match bounds {
                Some(b) => b.union(rect),
                None => rect,
            });
        }
        let Some(bounds) = bounds else { return };
        let scale = (canvas.width() / bounds.width().max(1.0))
            .min(canvas.height() / bounds.height().max(1.0));
        self.view.zoom = (scale * 0.75).clamp(0.2, 1.05);
        self.view.pan = canvas.center().to_vec2() - bounds.center().to_vec2() * self.view.zoom;
    }
}
