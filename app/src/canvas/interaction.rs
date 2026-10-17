//! 指针、快捷键、菜单与连线操作；不负责卡片绘制或存档转换。
use super::{Graph, Interaction};
use crate::canvas::geometry::{self, curve_maybe_cut};
use crate::canvas::node::{Connect, MenuAction, PortRef, Severing, Slash, Wire};
use crate::state::dialog;
use crate::state::run::Marks;
use crate::ui::controls::file_name;
use crate::ui::motion::{CANVAS_OVERTAKE, CANVAS_SEVER};
use crate::ui::theme;
use eframe::egui::{self, PointerButton};
use starrytools_core::model::node_kind::NodeKindInfo;

impl Graph {
    /// 这一帧画布想要的鼠标光标 —— 给外壳在**所有面板都画完之后**调用。
    ///
    /// 还按着右键（刀光没收）时是十字；悬停在端口上、或正在拉线时是手型；
    /// 其余情况返回 `None`，交给别的控件决定。
    ///
    /// 之所以不能在这里（`ui()` 里）直接设：光标是「后设的赢」，而状态药丸、
    /// 按钮会在画布之后各自设自己的悬停光标，会把十字盖掉。
    pub fn cursor_icon(&self) -> Option<egui::CursorIcon> {
        let holding = self
            .interaction
            .slash
            .as_ref()
            .is_some_and(|slash| slash.release.is_none());
        if holding {
            Some(egui::CursorIcon::Crosshair)
        } else if self.interaction.hovered_port.is_some() || self.interaction.connect.is_some() {
            Some(egui::CursorIcon::PointingHand)
        } else {
            None
        }
    }

    /// 推进两个定时动画：断口回缩、收刀跟出。
    pub(super) fn advance(&mut self, now: f64) {
        self.anim
            .severing
            .retain(|entry| now - entry.started < CANVAS_SEVER);

        if let Some(slash) = &self.interaction.slash {
            if let Some(start) = slash.release {
                if now - start >= CANVAS_OVERTAKE {
                    self.interaction.slash = None;
                }
            }
        }
    }

    pub(super) fn handle_input(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        kinds: &[NodeKindInfo],
        now: f64,
    ) {
        // ---- 滚轮缩放（以指针为锚点） ----
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 && resp.contains_pointer() {
            if let Some(p) = ui.input(|input| input.pointer.hover_pos()) {
                let anchor = self.view.to_flow(p);
                self.view.zoom = (self.view.zoom * (1.0 + scroll * 0.0015)).clamp(0.15, 2.0);
                self.view.pan = p.to_vec2() - anchor.to_vec2() * self.view.zoom;
            }
        }

        // ---- 端口悬停 ----
        self.interaction.hovered_port = ui
            .input(|input| input.pointer.hover_pos())
            .and_then(|p| self.port_at(kinds, self.view.to_flow(p)));

        // ---- 左键：从端口拉线 / 拖节点 / 拖空白平移画布 ----
        if resp.drag_started_by(PointerButton::Primary) {
            // 用**按下时**的位置判定端口，而不是拖过阈值之后的当前位置 ——
            // 等 `drag_started` 触发时指针已经移开好几像素，小小的端口就点不中了，
            // 拖动于是掉给背景去平移画布。`press_origin` 一直停在按下的那一点。
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(p) = origin {
                let flow = self.view.to_flow(p);
                // 落在端口上就是拉线，否则才是拖节点 —— 端口优先。
                match self.port_at(kinds, flow) {
                    Some(from) => {
                        self.interaction.connect = Some(Connect {
                            from,
                            to: p,
                            target: None,
                        });
                        self.interaction.grabbed = None;
                    }
                    None => {
                        // 摇起来的节点提到最上面（最后碰过的在最上层），顺便让它聚焦。
                        let hit = self
                            .hit(kinds, flow)
                            .map(|index| self.bring_to_front(index));
                        if let Some(index) = hit {
                            self.selected = Some(index);
                        }
                        self.interaction.grabbed = hit;
                    }
                }
            }
        }

        if resp.dragged_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some(from) = self
                    .interaction
                    .connect
                    .as_ref()
                    .map(|connect| connect.from)
                {
                    let target = self
                        .port_at(kinds, self.view.to_flow(p))
                        .filter(|candidate| self.can_link(from, *candidate));
                    if let Some(connect) = self.interaction.connect.as_mut() {
                        connect.to = p;
                        connect.target = target;
                    }
                }
            }

            // 正在拉线就别同时拖节点了。
            if self.interaction.connect.is_none() {
                let delta = resp.drag_delta();
                match self.interaction.grabbed {
                    Some(i) => {
                        self.nodes[i].pos += delta / self.view.zoom;
                        self.touch();
                    }
                    None => self.view.pan += delta,
                }
            }
        }

        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Some(connect) = self.interaction.connect.take() {
                if let Some(target) = connect.target {
                    self.link(connect.from, target);
                }
            }
            self.interaction.grabbed = None;
        }

        // ---- 左键单击：先看卡片上的工具按钮，再看选中 ----------------
        if resp.clicked_by(PointerButton::Primary) {
            let pointer = resp.interact_pointer_pos();
            let mut handled = false;
            if let Some(p) = pointer {
                for i in (0..self.nodes.len()).rev() {
                    let (play, del) = self.node_tool_rects(kinds, i);
                    if del.contains(p) {
                        self.delete_node(i);
                        handled = true;
                        break;
                    }
                    if play.contains(p) && self.can_run_node(kinds, i) {
                        // 要模型 / 外部程序但本地还没有：这个节点还不能跑 —— 点运行不生效。
                        if self.nodes[i].requirements.model_missing
                            || self.nodes[i].requirements.tool_missing
                            || self.nodes[i].requirements.tool_checking
                        {
                            handled = true;
                            break;
                        }
                        self.pending_run = Some(self.nodes[i].id.clone());
                        self.selected = Some(i);
                        handled = true;
                        break;
                    }
                    // 产物操作行：在文件夹中显示 / 另存为。
                    let file = self.nodes[i].run.as_ref().and_then(|run| run.file.clone());
                    if let Some(file) = file {
                        let (reveal, save) = self.node_action_rects(kinds, i);
                        if reveal.contains(p) {
                            if let Some(dir) = std::path::Path::new(&file).parent() {
                                let _ = open::that_detached(dir);
                            }
                            handled = true;
                            break;
                        }
                        if save.contains(p) {
                            // 另存为也走后台选择器，不阻塞界面。
                            self.picker.open(
                                dialog::Action::SaveAs {
                                    source: std::path::PathBuf::from(&file),
                                },
                                dialog::Spec {
                                    title: "另存为".into(),
                                    extensions: Vec::new(),
                                    mode: dialog::Mode::Save {
                                        file_name: file_name(&file),
                                    },
                                },
                            );
                            handled = true;
                            break;
                        }
                    }
                }
            }
            if !handled {
                let hit = pointer.and_then(|p| self.hit(kinds, self.view.to_flow(p)));
                if hit.is_none() {
                    // 点空白处：把上一次运行的高亮收起来（下一次运行再亮回来）。
                    if !self.run_marks_hidden {
                        self.run_marks_hidden = true;
                        ui.ctx().request_repaint();
                    }
                }
                self.selected = hit.map(|index| self.bring_to_front(index));
                // 点一下就收掉节点菜单 —— 选中变了，菜单里的下标就靠不住了。
                self.interaction.menu = None;
            }
        }

        // ---- 右键：划一刀 ----
        // 右键单击（没拖过阈值）不划刀 —— 那是要在节点上开菜单。
        if resp.secondary_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let hit = self.hit(kinds, self.view.to_flow(p));
                self.interaction.menu = hit.map(|index| (index, p));
                if let Some(index) = hit {
                    self.selected = Some(index);
                }
            } else {
                self.interaction.menu = None;
            }
        }
        if resp.drag_started_by(PointerButton::Secondary) {
            // 和左键同理：用按下的那一点起步，刀口才不会一开就偏几像素。
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(p) = origin {
                // 落在节点上的右键不划刀 —— 那该留给节点的右键菜单。
                if self.hit(kinds, self.view.to_flow(p)).is_none() {
                    self.interaction.slash = Some(Slash {
                        from: p,
                        to: p,
                        doomed: Vec::new(),
                        release: None,
                    });
                }
            }
        }
        if resp.dragged_by(PointerButton::Secondary) {
            if let (Some(p), Some(slash)) =
                (resp.interact_pointer_pos(), self.interaction.slash.as_mut())
            {
                slash.to = p;
            }
            self.refresh_doomed(kinds);
        }
        if resp.drag_stopped_by(PointerButton::Secondary) {
            self.finish_slash(kinds, now);
        }

        // ---- 键盘：Delete 删节点，Ctrl+C/V 复制粘贴，Ctrl+S 保存 ----
        // 正在输入框里打字时一概不抢键。
        if ui.ctx().memory(|memory| memory.focused()).is_none() {
            let (delete, copy, paste, save) = ui.input(|input| {
                let command = input.modifiers.command;
                (
                    input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
                    command && input.key_pressed(egui::Key::C),
                    command && input.key_pressed(egui::Key::V),
                    command && input.key_pressed(egui::Key::S),
                )
            });
            if delete {
                if let Some(index) = self.selected {
                    self.delete_node(index);
                }
            }
            if copy {
                self.clipboard = self
                    .selected
                    .and_then(|index| self.nodes.get(index).cloned());
            }
            if paste {
                if let Some(node) = self.clipboard.clone() {
                    // 粘到指针处；指针不在画布上就退回原件的位置。
                    let at = ui
                        .input(|input| input.pointer.hover_pos())
                        .map(|p| self.view.to_flow(p))
                        .unwrap_or(node.pos);
                    self.paste_node(node, at);
                }
            }
            if save {
                self.save_requested = true;
            }
        }
    }

    /// 拖动过程中重算「将被删除」的连线。
    pub(super) fn refresh_doomed(&mut self, kinds: &[NodeKindInfo]) {
        let Some(slash) = &self.interaction.slash else {
            return;
        };
        let (from, to) = (slash.from, slash.to);
        // 太短的一划不当刀 —— 免得右键单击就切掉一堆线。
        if (to - from).length() < 2.0 {
            return;
        }

        let a = self.view.to_flow(from);
        let b = self.view.to_flow(to);

        let doomed = (0..self.wires.len())
            .filter_map(|i| {
                let curve = self.wire_points(kinds, &self.wires[i]);
                // 先用控制点包围盒预筛：贝塞尔落在控制点凸包里，包围盒都不碰的线
                // 不可能被切到，采样与相交测试都省了。
                if !curve_maybe_cut(&curve, a, b) {
                    return None;
                }
                let poly = geometry::sample(&curve, geometry::steps_for(&curve));
                geometry::first_hit(&poly, a, b).map(|(t, _)| (i, t))
            })
            .collect();

        if let Some(slash) = self.interaction.slash.as_mut() {
            slash.doomed = doomed;
        }
    }

    /// 收刀：把切中的连线摘掉、起断裂动画，并让刀光跟出后消散。
    pub(super) fn finish_slash(&mut self, kinds: &[NodeKindInfo], now: f64) {
        let Some(mut slash) = self.interaction.slash.take() else {
            return;
        };

        let a = self.view.to_flow(slash.from);
        let b = self.view.to_flow(slash.to);

        let mut cuts = Vec::new();
        for i in 0..self.wires.len() {
            let curve = self.wire_points(kinds, &self.wires[i]);
            if !curve_maybe_cut(&curve, a, b) {
                continue;
            }
            let poly = geometry::sample(&curve, geometry::steps_for(&curve));
            if let Some((t, _)) = geometry::first_hit(&poly, a, b) {
                cuts.push((i, curve, t));
            }
        }

        // 从后往前删，免得下标挪位。
        let cut_count = cuts.len();
        for (i, curve, t) in cuts.into_iter().rev() {
            self.anim.severing.push(Severing {
                curve,
                t,
                started: now,
            });
            self.wires.remove(i);
        }
        if cut_count > 0 {
            self.touch_content();
        }

        slash.doomed.clear();
        slash.release = Some(now);
        self.interaction.slash = Some(slash);
    }

    /// 这两个端口能不能接上。
    ///
    /// 用**端口自己的真实类型**判断（`PortType::accepts`），不看徽标字符串 ——
    /// 编辑期那条规则本来就是宽松的（一头是通配就放行、格式未知的图像哪儿都能接），
    /// 和 core 的静态检查是同一套。
    pub(super) fn can_link(&self, from: PortRef, to: PortRef) -> bool {
        // 必须一头输入一头输出，而且不给自己接线。
        if from.input == to.input || from.node == to.node {
            return false;
        }
        let (out, input) = if from.input { (to, from) } else { (from, to) };

        let (Some(source), Some(target)) = (
            self.nodes
                .get(out.node)
                .and_then(|node| node.outputs.get(out.port)),
            self.nodes
                .get(input.node)
                .and_then(|node| node.inputs.get(input.port)),
        ) else {
            return false;
        };

        // 同一个上游接两次没有意义。
        if self.wires.iter().any(|wire| {
            wire.from == out.node
                && wire.from_port == source.id
                && wire.to == input.node
                && wire.to_port == target.id
        }) {
            return false;
        }

        if source.exec != target.exec {
            return false;
        }
        if source.exec {
            // Exec 输出只允许一条边，分支使用 Sequence。
            return self.wire_at_port(out.node, &source.id, false).is_none();
        }
        let occupied = self.wire_at_port(input.node, &target.id, true).is_some();
        (target.multi || !occupied) && target.ty.accepts(source.ty)
    }

    /// 把两个端口接上。
    pub(super) fn link(&mut self, from: PortRef, to: PortRef) {
        if !self.can_link(from, to) {
            return;
        }
        let (out, input) = if from.input { (to, from) } else { (from, to) };
        let wire = Wire {
            from: out.node,
            from_port: self.nodes[out.node].outputs[out.port].id.clone(),
            to: input.node,
            to_port: self.nodes[input.node].inputs[input.port].id.clone(),
            id: Wire::make_id(
                &self.nodes[out.node].id,
                &self.nodes[out.node].outputs[out.port].id,
                &self.nodes[input.node].id,
                &self.nodes[input.node].inputs[input.port].id,
            ),
        };
        self.wires.push(wire);
        self.selected = Some(input.node);
        self.touch_content();
    }

    /// 节点上的右键菜单。
    pub(super) fn draw_menu(&mut self, ui: &egui::Ui) {
        let Some((index, at)) = self.interaction.menu else {
            return;
        };
        // 先把要显示的字取出来，免得闭包里再借 `self`。
        let Some(title) = self.nodes.get(index).map(|node| node.title.clone()) else {
            self.interaction.menu = None;
            return;
        };

        let id = egui::Id::new("starrytools-node-menu");
        // `open` 由弹层自己管：点到外面它会把这位置回 false。
        let mut open = true;
        let mut action: Option<MenuAction> = None;

        egui::Popup::new(
            id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(at),
            egui::LayerId::new(egui::Order::Foreground, id),
        )
        .kind(egui::PopupKind::Menu)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(136.0);
            ui.label(egui::RichText::new(title).color(theme::INK_3).size(10.0));
            ui.separator();
            if ui.button("复制节点").clicked() {
                action = Some(MenuAction::Duplicate(index));
            }
            if ui.button("删除节点").clicked() {
                action = Some(MenuAction::Delete(index));
            }
        });

        let acted = action.is_some();
        match action {
            Some(MenuAction::Duplicate(i)) => self.duplicate_node(i),
            Some(MenuAction::Delete(i)) => self.delete_node(i),
            None => {}
        }
        if !open || acted {
            self.interaction.menu = None;
        }
    }

    /// 端口现在的放大倍数。
    ///
    /// **拉线的起点、它可能落下的落点、以及指针悬着的那个**都算「被强调」—— 两端
    /// （起点 / 落点）因此永远一样大。只看 `hovered_port` 是不够的：落点靠的是
    /// `connect.target`，两者偶尔会错开一帧，端点就会一大一小。
    pub(super) fn port_scale(&self, port: PortRef) -> f32 {
        let emphasized = self.interaction.hovered_port == Some(port)
            || self
                .interaction
                .connect
                .as_ref()
                .is_some_and(|connect| connect.from == port || connect.target == Some(port));
        if emphasized {
            1.8
        } else {
            1.0
        }
    }

    /// 这个端口是不是当前这根线可以落下的地方。
    pub(super) fn is_valid_target(&self, port: PortRef) -> bool {
        self.interaction
            .connect
            .as_ref()
            .is_some_and(|connect| connect.target == Some(port))
    }

    /// 这个端口上接的线（有的话）。输入 / 输出两种端口共用它。
    pub(super) fn wire_at_port(
        &self,
        node_index: usize,
        port_id: &str,
        input: bool,
    ) -> Option<&Wire> {
        self.wires.iter().find(|wire| {
            if input {
                wire.to == node_index && wire.to_port == port_id
            } else {
                wire.from == node_index && wire.from_port == port_id
            }
        })
    }

    /// 这个端口是不是接了条被静态检查判为非法的线。
    pub(super) fn port_invalid(
        &self,
        marks: &Marks,
        node_index: usize,
        port_id: &str,
        input: bool,
    ) -> bool {
        self.wires.iter().any(|wire| {
            let at_port = if input {
                wire.to == node_index && wire.to_port == port_id
            } else {
                wire.from == node_index && wire.from_port == port_id
            };
            at_port && marks.invalid_edges.contains(&wire.id)
        })
    }

    /// 这个端口接没接线。
    pub(super) fn port_linked(&self, node_index: usize, port_id: &str, input: bool) -> bool {
        self.wire_at_port(node_index, port_id, input).is_some()
    }
}

impl Interaction {
    /// 画布上删掉了一只节点（下标 `removed`）之后，把记着的节点下标跟着挪一挪。
    ///
    /// 拖拽中的节点刚好被删掉时就松手（而不是拖到错的对象上、或直接越界）。
    pub(super) fn fixup_after_removal(&mut self, removed: usize) {
        let shift = |node: &mut usize| {
            if *node > removed {
                *node -= 1;
            }
        };
        match self.grabbed {
            Some(grab) if grab == removed => self.grabbed = None,
            Some(grab) if grab > removed => self.grabbed = Some(grab - 1),
            _ => {}
        }
        if let Some(connect) = &mut self.connect {
            if connect.from.node == removed
                || connect.target.is_some_and(|target| target.node == removed)
            {
                self.connect = None;
            } else {
                shift(&mut connect.from.node);
                if let Some(target) = &mut connect.target {
                    shift(&mut target.node);
                }
            }
        }
        if let Some((index, _)) = &mut self.menu {
            if *index == removed {
                self.menu = None;
            } else {
                shift(index);
            }
        }
    }
}
