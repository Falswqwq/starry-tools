//! 节点文档的构造、存档往返和编辑；保持实例 ID、参数及端口 ID 的存档语义。
use super::{Anim, Graph, Interaction, Textures, NODE_W};
use crate::canvas::layout::height_of_node;
use crate::canvas::node::{new_id, refresh_ports, Node, Wire};
use crate::canvas::view::Viewport;
use crate::state::{dialog, models::Downloads};
use eframe::egui::{self, Pos2};
use starrytools_core::model::node_kind::NodeKindInfo;
use starrytools_core::model::workflow::{Edge, NodeInstance, Position, Workflow};
use std::collections::HashMap;

impl Graph {
    /// 复用 Check 的后台结果；旧语义版本和重复结果均不应用。
    pub(crate) fn sync_resolved_ports(
        &mut self,
        revision: u64,
        resolved: &starrytools_core::engine::ResolvedWorkflow,
    ) {
        if revision != self.check_revision || self.ports_revision == Some(revision) {
            return;
        }
        let ports: HashMap<_, _> = resolved
            .nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node))
            .collect();
        let mut changed = false;
        for node in &mut self.nodes {
            if let Some(snapshot) = ports
                .get(node.id.as_str())
                .filter(|snapshot| snapshot.kind == node.kind)
            {
                changed |= node
                    .inputs
                    .iter()
                    .map(|port| {
                        (
                            &port.id,
                            port.ty,
                            port.exec,
                            &port.label,
                            port.required,
                            &port.hint,
                            &port.param,
                            port.panel_row,
                            port.panel_input_row,
                            port.multi,
                        )
                    })
                    .ne(snapshot.inputs.iter().map(|port| {
                        (
                            &port.id,
                            port.ty,
                            port.exec,
                            &port.label,
                            port.required,
                            &port.hint,
                            &port.param,
                            port.panel_row,
                            port.panel_input_row,
                            port.multi,
                        )
                    }))
                    || node
                        .outputs
                        .iter()
                        .map(|port| {
                            (
                                &port.id,
                                port.ty,
                                port.exec,
                                &port.label,
                                port.required,
                                &port.hint,
                                &port.param,
                                port.panel_row,
                                port.panel_input_row,
                                port.multi,
                            )
                        })
                        .ne(snapshot.outputs.iter().map(|port| {
                            (
                                &port.id,
                                port.ty,
                                port.exec,
                                &port.label,
                                port.required,
                                &port.hint,
                                &port.param,
                                port.panel_row,
                                port.panel_input_row,
                                port.multi,
                            )
                        }));
                node.inputs.clone_from(&snapshot.inputs);
                node.outputs.clone_from(&snapshot.outputs);
            }
        }
        // 端口下标可能改变，不能继续使用旧的拖线/悬停引用。
        if changed {
            self.interaction.connect = None;
            self.interaction.hovered_port = None;
        }
        self.ports_revision = Some(revision);
    }
    /// 用真实节点类型铺一张 `count` 个节点的网格图，连线连成一张网。
    ///
    /// 只在测试里用（曾经是压力测试图的来源，那个已经拆掉了）。
    #[cfg(test)]
    pub fn demo(count: usize, kinds: &[NodeKindInfo]) -> Self {
        const COLS: usize = 6;

        let mut nodes = Vec::with_capacity(count);
        for i in 0..count {
            let (cx, cy) = ((i % COLS) as f32, (i / COLS) as f32);
            let kind = &kinds[i % kinds.len().max(1)];
            let node = Node::new(
                new_id(),
                egui::pos2(cx * 340.0, cy * 260.0),
                kind.kind.id.clone(),
                kind.kind.name.clone(),
                kind.defaults.clone(),
            );
            nodes.push(node);
        }

        // 只往有输入端口的节点上连 —— 「输入」节点没有输入，连上去没有意义。
        let mut wires = Vec::new();
        let link = |wires: &mut Vec<Wire>, from: usize, to: usize| {
            let (Some(out), Some(input)) = (nodes[from].outputs.first(), nodes[to].inputs.first())
            else {
                return;
            };
            wires.push(Wire {
                from,
                from_port: out.id.clone(),
                to,
                to_port: input.id.clone(),
                id: Wire::make_id(&nodes[from].id, &out.id, &nodes[to].id, &input.id),
            });
        };
        for i in 0..count {
            if i + 1 < count && i % COLS != COLS - 1 {
                link(&mut wires, i, i + 1);
            }
            if i + COLS < count {
                link(&mut wires, i, i + COLS);
            }
        }

        Self {
            nodes,
            wires,
            view: Viewport {
                pan: egui::vec2(40.0, 30.0),
                zoom: 0.8,
            },
            revision: 0,
            check_revision: 0,
            tool_revision: None,
            ports_revision: None,
            selected: None,
            interaction: Interaction::default(),
            textures: Textures::default(),
            anim: Anim::default(),
            pending_run: None,
            clipboard: None,
            save_requested: false,
            notes: HashMap::new(),
            run_marks_hidden: false,
            downloads: Downloads::default(),
            picker: dialog::Picker::default(),
        }
    }

    /// 从节点库拖出来的节点落在哪儿：`screen` 是松手时的屏幕位置。`now` 用来做入场动画。
    pub fn add_node_at(&mut self, screen: Pos2, kind: &NodeKindInfo, now: f64) {
        let mut node = Node::new(
            new_id(),
            Pos2::ZERO,
            kind.kind.id.clone(),
            kind.kind.name.clone(),
            kind.defaults.clone(),
        );
        // 高度用**画卡片那一套**算（同一份代码），落点才不会偏。
        let height = height_of_node(std::slice::from_ref(kind), &node, &self.notes);
        node.pos = self.view.to_flow(screen) - egui::vec2(NODE_W, height) * 0.5;
        self.anim.entering.insert(node.id.clone(), now);
        self.nodes.push(node);
        self.interaction.grabbed = Some(self.nodes.len() - 1);
        self.touch_content();
    }

    /// 把画布上的东西写进一份工作流。`base` 提供 id / 名字 / 描述 / 创建时间。
    pub fn to_workflow(&self, base: &Workflow) -> Workflow {
        let mut workflow = base.clone();
        workflow.nodes = self
            .nodes
            .iter()
            .map(|node| NodeInstance {
                id: node.id.clone(),
                kind: node.kind.clone(),
                position: Position {
                    x: f64::from(node.pos.x),
                    y: f64::from(node.pos.y),
                },
                params: node.params.clone(),
            })
            .collect();
        workflow.edges = self
            .wires
            .iter()
            .map(|wire| {
                let source = &self.nodes[wire.from];
                let target = &self.nodes[wire.to];
                Edge {
                    id: wire.id.clone(),
                    source: source.id.clone(),
                    source_port: wire.from_port.clone(),
                    target: target.id.clone(),
                    target_port: wire.to_port.clone(),
                }
            })
            .collect();
        workflow
    }

    /// 从一份工作流把画布重建出来。
    ///
    /// 认不出的类型会**保留下来**（`refresh_ports` 会给它空端口，画成一个空壳），
    /// 不丢数据 —— 老存档里可能有现在的版本已经不认识的节点。
    pub fn from_workflow(workflow: &Workflow, kinds: &[NodeKindInfo]) -> Self {
        let mut nodes: Vec<Node> = Vec::with_capacity(workflow.nodes.len());
        for instance in &workflow.nodes {
            let title = kinds
                .iter()
                .find(|kind| kind.kind.id == instance.kind)
                .map(|kind| kind.kind.name.clone())
                .unwrap_or_else(|| instance.kind.clone());
            let node = Node::new(
                instance.id.clone(),
                egui::pos2(instance.position.x as f32, instance.position.y as f32),
                instance.kind.clone(),
                title,
                instance.params.clone(),
            );
            nodes.push(node);
        }

        // 存档里连线存的是节点 id，画布上按下标跑，所以换算一次。
        let wires = {
            let index_of = |id: &str| nodes.iter().position(|node| node.id == id);
            workflow
                .edges
                .iter()
                .filter_map(|edge| {
                    let from = index_of(&edge.source)?;
                    let to = index_of(&edge.target)?;
                    Some(Wire {
                        from,
                        from_port: edge.source_port.clone(),
                        to,
                        to_port: edge.target_port.clone(),
                        id: Wire::make_id(
                            &nodes[from].id,
                            &edge.source_port,
                            &nodes[to].id,
                            &edge.target_port,
                        ),
                    })
                })
                .collect()
        };

        Self {
            nodes,
            wires,
            view: Viewport {
                pan: egui::vec2(40.0, 30.0),
                zoom: 0.8,
            },
            // 刚打开的工作流没有未保存的改动。
            revision: 0,
            check_revision: 0,
            tool_revision: None,
            ports_revision: None,
            selected: None,
            interaction: Interaction::default(),
            textures: Textures::default(),
            anim: Anim::default(),
            pending_run: None,
            clipboard: None,
            save_requested: false,
            notes: HashMap::new(),
            run_marks_hidden: false,
            downloads: Downloads::default(),
            picker: dialog::Picker::default(),
        }
    }

    /// 把一个节点提到最上层：把它挪到 `nodes` 末尾，并把连线 / 选中里的下标跟着换算。
    ///
    /// 返回它挪到的下标 —— 调用方接着要用。
    pub(super) fn bring_to_front(&mut self, index: usize) -> usize {
        if index + 1 == self.nodes.len() {
            return index;
        }
        let node = self.nodes.remove(index);
        self.nodes.push(node);
        let last = self.nodes.len() - 1;
        for wire in &mut self.wires {
            if wire.from == index {
                wire.from = last;
            } else if wire.from > index {
                wire.from -= 1;
            }
            if wire.to == index {
                wire.to = last;
            } else if wire.to > index {
                wire.to -= 1;
            }
        }
        if let Some(selected) = self.selected {
            self.selected = Some(if selected == index {
                last
            } else if selected > index {
                selected - 1
            } else {
                selected
            });
        }
        last
    }

    /// 删掉一个节点，顺带把它两端的连线都断掉。
    pub(super) fn delete_node(&mut self, index: usize) {
        if index >= self.nodes.len() {
            return;
        }
        // 节点没了，它留下的下载状态（正在下的任务 / 选中的源）也一并清掉。
        self.downloads.forget(&self.nodes[index].id);
        self.nodes.remove(index);
        // 先按老下标滤掉断掉的线，再把剩下连线的下标往前挪一格。
        self.wires
            .retain(|wire| wire.from != index && wire.to != index);
        for wire in &mut self.wires {
            if wire.from > index {
                wire.from -= 1;
            }
            if wire.to > index {
                wire.to -= 1;
            }
        }
        self.selected = None;
        self.interaction.menu = None;
        // 交互里记着的下标也要跟着挪 —— 否则拖拽中删掉节点会拖到错的对象，
        // 删的恰好是最后一个还会越界 panic。
        self.interaction.fixup_after_removal(index);
        self.touch_content();
    }

    /// 复制一个节点，错开一点免得盖住原件。
    pub(super) fn duplicate_node(&mut self, index: usize) {
        let Some(source) = self.nodes.get(index) else {
            return;
        };
        let mut copy = source.clone();
        copy.id = new_id();
        copy.pos += egui::vec2(28.0, 28.0);
        refresh_ports(&mut copy);
        self.nodes.push(copy);
        self.selected = Some(self.nodes.len() - 1);
        self.interaction.menu = None;
        self.touch_content();
    }

    /// 粘贴一个节点：换新 id、清掉上次运行的痕迹，落在 `at`（流坐标）。
    pub(super) fn paste_node(&mut self, mut node: Node, at: Pos2) {
        node.id = new_id();
        node.pos = at;
        node.run = None;
        node.preview = false;
        refresh_ports(&mut node);
        self.nodes.push(node);
        self.selected = Some(self.nodes.len() - 1);
        self.interaction.menu = None;
        self.touch_content();
    }

    /// 外壳读一下有没有 Ctrl+S 的请求，读到就清零。
    pub fn take_save_request(&mut self) -> bool {
        std::mem::take(&mut self.save_requested)
    }
}
