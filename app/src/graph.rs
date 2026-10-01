//! 手绘的节点画布，外加那把刀。
//!
//! 节点的端口、标题、参数控件全部来自 [`crate::catalog`]（core 的注册表）——
//! 这里同样不认识任何具体工具。
//!
//! 刀光完全是自己算的：连线的控制点本来就在手上，采样和判交都不依赖任何外部渲染。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
};
use egui::epaint::CubicBezierShape;
use serde_json::Value;
use starrytools_core::engine::{NodeStatus, RunReport};
use starrytools_core::model::params::Params;
use starrytools_core::model::workflow::{Edge, NodeInstance, Position, Workflow};
use std::collections::HashMap;

use crate::catalog::{Control, Kind, Param, Port};
use crate::geometry::{self, Cubic};
use crate::icons::{self, IconFn};
use crate::run::Marks;
use crate::theme;
use crate::widgets;

/// 节点的显示尺寸（流坐标，不含缩放）。
const NODE_W: f32 = 216.0;
const HEADER_H: f32 = 30.0;
const PORT_ROW_H: f32 = 20.0;
const BODY_PAD: f32 = 8.0;
/// 参数块：标签一行、控件一行（上下排列，和原设计一致）。
const PARAM_LABEL_H: f32 = 15.0;
const PARAM_LABEL_GAP: f32 = 4.0;
const PARAM_CONTROL_H: f32 = 24.0;
const PARAM_NOTE_GAP: f32 = 4.0;
/// 两个参数之间留的缝。
const PARAM_GAP: f32 = 8.0;
/// 开关是「标签 + 开关」一行放。
const PARAM_BOOL_H: f32 = 20.0;
/// 多行文本框的最小高度，以及每行估高。
const PARAM_TEXT_ROW_H: f32 = 52.0;
const PARAM_TEXT_LINE: f32 = 16.0;
/// 端口和参数之间留的那条缝。
const PARAMS_GAP: f32 = 9.0;
/// 参数区底部留的那点白，免得最后一个控件贴着圆角。
const PARAMS_BOTTOM: f32 = 8.0;
/// 缩得比这个还小就不画控件了 —— 控件挤不下，只留标题和端口。
const PARAM_LOD_ZOOM: f32 = 0.5;
/// 端口的点击判定半径（**屏幕**像素）。比画出来的圆大不少 —— 小圆点不好点，
/// 而且拖动要等指针移过几像素才触发，判定太紧就点不中了。
const PORT_HIT: f32 = 13.0;
/// 背景小点的间距。
const DOT_GAP: f32 = 22.0;

/// 卡片底部的预览条高度（流坐标）。跑完有缩略图时卡片才多出这么高。
const PREVIEW_H: f32 = 84.0;
/// 预览的棋盘格边长（流坐标）。
const CHECKER: f32 = 8.0;

/// 运行痕迹各段的高度（流坐标）。
const RUN_LINE_H: f32 = 14.0;
const RUN_WARN_H: f32 = 14.0;
const RUN_ERROR_H: f32 = 30.0;
const RUN_ACTIONS_H: f32 = 30.0;

/// 节点落到画布上时那段入场动画的时长。
const ENTER_SECS: f64 = 0.34;

/// 断口回缩并消散的时长。
const SEVER_SECS: f64 = 0.46;
/// 收刀「跟出」的时长。
const OVERTAKE_SECS: f64 = 0.28;
/// 收刀时刀尖再往前送多远（屏幕像素）。
const OVERTAKE_REACH: f32 = 160.0;

#[derive(Clone)]
pub struct Node {
    /// 节点实例 id，落盘时用它（存档里的 `source` / `target` 认的就是它）。
    pub id: String,
    pub pos: Pos2,
    /// 节点类型 id。参数声明按这个 id 到注册表里查。
    pub kind: String,
    pub title: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// 这个节点实例自己的参数值。
    pub params: Params,
    /// 跑完之后有没有缩略图可看。有的活卡片底下多留一条预览区（**不落盘**，是视图状态）。
    pub preview: bool,
    /// 上一次运行的结果（状态 / 耗时 / 警告 / 错误 / 产物）。**不落盘**。
    pub run: Option<NodeRun>,
}

/// 一个节点上一次运行时留下的痕迹，画在卡片上。
#[derive(Clone)]
pub struct NodeRun {
    pub status: NodeStatus,
    pub ms: u64,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// 写下去的产物路径（有的话卡片底部会出现一行操作）。
    pub file: Option<String>,
}

#[derive(Clone)]
pub struct Wire {
    pub from: usize,
    /// 起点端口 id（**不是下标**）。端口会随参数增减，下标靠不住。
    pub from_port: String,
    pub to: usize,
    pub to_port: String,
}

/// 卡片右上角工具按钮的边长（屏幕像素，随缩放走）。
const TOOL_SIZE: f32 = 20.0;

/// 一个端口的位置引用。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PortRef {
    pub node: usize,
    pub port: usize,
    /// 是输入端口吗？否则是输出端口。
    pub input: bool,
}

/// 正在拉的那根线。
struct Connect {
    from: PortRef,
    /// 当前指针位置（屏幕坐标）。
    to: Pos2,
    /// 落点候选 —— 只在悬停到一个**合法**端口上时才有值。
    target: Option<PortRef>,
}

impl Wire {
    /// 连线的稳定 id。落盘和静态检查的结果都认它 —— 两边必须是同一个拼法。
    pub fn id(&self, nodes: &[Node]) -> String {
        format!(
            "{}:{}->{}:{}",
            nodes[self.from].id, self.from_port, nodes[self.to].id, self.to_port
        )
    }
}

/// 一道刀光。
struct Slash {
    /// 屏幕坐标。刀口要始终锐利，所以不跟画布缩放位移。
    from: Pos2,
    to: Pos2,
    /// 这一刀会切到的连线（`wires` 的下标），拖动过程中实时更新。
    doomed: Vec<usize>,
    /// 收刀跟出的开始时刻。`None` 表示还按着。
    release: Option<f64>,
}

/// 正在断裂的连线。
///
/// 切下去的那一刻就把这条线从 `wires` 里摘掉，动画自己带着曲线走 ——
/// 这样就不会出现「动画还在放、下标已经挪位」的问题。
struct Severing {
    curve: Cubic,
    /// 断口在曲线上的参数。
    t: f32,
    started: f64,
}

/// 节点右键菜单里选中的动作。
#[derive(Clone, Copy)]
enum MenuAction {
    Delete(usize),
    Duplicate(usize),
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    pub pan: Vec2,
    pub zoom: f32,
    /// 画布内容的版本号。改一下就加一，用来判断有没有未保存的改动。
    /// 平移 / 缩放不算 —— 那是视图状态，不落盘。
    pub revision: u64,
    /// 当前选中的节点（`运行至此` 认它）。
    pub selected: Option<usize>,
    /// 这一帧被抓住的节点。`None` 表示抓的是空白处（= 平移画布）。
    grabbed: Option<usize>,
    /// 指针底下的端口，用来放大高亮。
    hovered_port: Option<PortRef>,
    /// 正在从某个端口往外拉线。
    connect: Option<Connect>,
    /// 打开着的节点右键菜单：哪个节点、在屏幕哪儿弹出。
    menu: Option<(usize, Pos2)>,
    slash: Option<Slash>,
    severing: Vec<Severing>,
    /// 节点缩略图（按节点 id）。报告一换就整批丢掉重建。
    textures: HashMap<String, egui::TextureHandle>,
    /// 当前纹理对应哪次运行（`RunReport::finished_at`）。
    report_stamp: Option<i64>,
    /// 点了卡片上的「运行至此」：这个节点 id 要交给调用方去跑。
    pending_run: Option<String>,
    /// 刚落到画布上的节点（id → 落下的时刻），用来放入场动画。
    entering: HashMap<String, f64>,
    /// Ctrl+C 复制的节点。
    clipboard: Option<Node>,
    /// Ctrl+S：交给外壳去落盘。
    save_requested: bool,
}

/// 按端口 id 找下标。认不出来就退回第一个 —— 老存档里可能存着已经删掉的端口。
fn port_index(ports: &[Port], id: &str) -> usize {
    ports.iter().position(|port| port.id == id).unwrap_or(0)
}

/// 新的节点实例 id。
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 按当前参数重算一个节点的端口。
///
/// 参数一改就调 —— 端口类型可能跟着参数走（「图像格式转换」的目标格式、
/// 「图像压缩」的模式、「输入」选的文件）。
fn refresh_ports(node: &mut Node) {
    let (inputs, outputs) = crate::catalog::ports_for(&node.kind, &node.params);
    node.inputs = inputs;
    node.outputs = outputs;
}

/// 按 id 找节点类型。
///
/// 故意写成不挂在 `self` 上的自由函数：这样「读类型声明」和「改节点参数」
/// 是两个互不相干的借用，不用为了避开借用检查去克隆参数。
fn kind_of<'a>(kinds: &'a [Kind], node: &Node) -> Option<&'a Kind> {
    kinds.iter().find(|kind| kind.id == node.kind)
}

impl Graph {
    /// 用真实节点类型铺一张 `count` 个节点的网格图，连线连成一张网。
    ///
    /// 只在测试里用（曾经是压力测试图的来源，那个已经拆掉了）。
    #[cfg(test)]
    pub fn demo(count: usize, kinds: &[Kind]) -> Self {
        const COLS: usize = 6;

        let mut nodes = Vec::with_capacity(count);
        for i in 0..count {
            let (cx, cy) = ((i % COLS) as f32, (i / COLS) as f32);
            let kind = &kinds[i % kinds.len().max(1)];
            let mut node = Node {
                id: new_id(),
                pos: egui::pos2(cx * 340.0, cy * 260.0),
                kind: kind.id.clone(),
                title: kind.name.clone(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                params: kind.defaults.clone(),
                preview: false,
                run: None,
            };
            refresh_ports(&mut node);
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
            pan: egui::vec2(40.0, 30.0),
            zoom: 0.8,
            revision: 0,
            selected: None,
            grabbed: None,
            hovered_port: None,
            connect: None,
            menu: None,
            slash: None,
            severing: Vec::new(),
            textures: HashMap::new(),
            report_stamp: None,
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
        }
    }

    /// 从节点库拖出来的节点落在哪儿：`screen` 是松手时的屏幕位置。`now` 用来做入场动画。
    pub fn add_node_at(&mut self, screen: Pos2, kind: &Kind, now: f64) {
        let height = height_of(kind, &kind.defaults);
        let pos = self.to_flow(screen) - egui::vec2(NODE_W, height) * 0.5;
        let mut node = Node {
            id: new_id(),
            pos,
            kind: kind.id.clone(),
            title: kind.name.clone(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: kind.defaults.clone(),
            preview: false,
            run: None,
        };
        refresh_ports(&mut node);
        self.entering.insert(node.id.clone(), now);
        self.nodes.push(node);
        self.grabbed = Some(self.nodes.len() - 1);
        self.touch();
    }

    /// 记一笔「画布变过了」—— 保存之后用它判断有没有未保存的改动。
    pub fn touch(&mut self) {
        self.revision += 1;
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
                    id: wire.id(&self.nodes),
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
    pub fn from_workflow(workflow: &Workflow, kinds: &[Kind]) -> Self {
        let mut nodes: Vec<Node> = Vec::with_capacity(workflow.nodes.len());
        for instance in &workflow.nodes {
            let title = kinds
                .iter()
                .find(|kind| kind.id == instance.kind)
                .map(|kind| kind.name.clone())
                .unwrap_or_else(|| instance.kind.clone());
            let mut node = Node {
                id: instance.id.clone(),
                pos: egui::pos2(instance.position.x as f32, instance.position.y as f32),
                kind: instance.kind.clone(),
                title,
                inputs: Vec::new(),
                outputs: Vec::new(),
                params: instance.params.clone(),
                preview: false,
                run: None,
            };
            refresh_ports(&mut node);
            nodes.push(node);
        }

        // 存档里连线存的是节点 id，画布上按下标跑，所以换算一次。
        let wires = {
            let index_of = |id: &str| nodes.iter().position(|node| node.id == id);
            workflow
                .edges
                .iter()
                .filter_map(|edge| {
                    Some(Wire {
                        from: index_of(&edge.source)?,
                        from_port: edge.source_port.clone(),
                        to: index_of(&edge.target)?,
                        to_port: edge.target_port.clone(),
                    })
                })
                .collect()
        };

        Self {
            nodes,
            wires,
            pan: egui::vec2(40.0, 30.0),
            zoom: 0.8,
            // 刚打开的工作流没有未保存的改动。
            revision: 0,
            selected: None,
            grabbed: None,
            hovered_port: None,
            connect: None,
            menu: None,
            slash: None,
            severing: Vec::new(),
            textures: HashMap::new(),
            report_stamp: None,
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
        }
    }

    // ---- 坐标换算：流坐标 <-> 屏幕坐标 ----

    fn to_screen(&self, p: Pos2) -> Pos2 {
        (p.to_vec2() * self.zoom + self.pan).to_pos2()
    }

    fn to_flow(&self, p: Pos2) -> Pos2 {
        ((p.to_vec2() - self.pan) / self.zoom).to_pos2()
    }

    fn port_rows(&self, i: usize) -> usize {
        self.nodes[i]
            .inputs
            .len()
            .max(self.nodes[i].outputs.len())
            .max(1)
    }

    fn flow_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        Rect::from_min_size(
            self.nodes[i].pos,
            egui::vec2(NODE_W, height_of_node(kinds, &self.nodes[i])),
        )
    }

    fn screen_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        let r = self.flow_rect(kinds, i);
        Rect::from_min_max(self.to_screen(r.min), self.to_screen(r.max))
    }

    /// 第 `k` 个输入端口（流坐标）。
    fn in_port(&self, kinds: &[Kind], i: usize, k: usize) -> Pos2 {
        let r = self.flow_rect(kinds, i);
        egui::pos2(r.left(), r.top() + HEADER_H + PORT_ROW_H * (k as f32 + 0.5))
    }

    fn out_port(&self, kinds: &[Kind], i: usize, k: usize) -> Pos2 {
        let r = self.flow_rect(kinds, i);
        egui::pos2(
            r.right(),
            r.top() + HEADER_H + PORT_ROW_H * (k as f32 + 0.5),
        )
    }

    /// 参数区左上角相对卡片顶部的高度（流坐标）。
    fn params_top(&self, i: usize) -> f32 {
        HEADER_H + self.port_rows(i) as f32 * PORT_ROW_H + PARAMS_GAP
    }

    /// 连线的四个控制点（**流坐标**）。
    ///
    /// 一定要在缩放之前算：否则 `reach` 的那个最小值会被缩放影响，
    /// 同一张图在不同缩放下的形状会不一样。
    fn wire_points(&self, kinds: &[Kind], wire: &Wire) -> Cubic {
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
    fn hit(&self, kinds: &[Kind], flow: Pos2) -> Option<usize> {
        (0..self.nodes.len())
            .rev()
            .find(|&i| self.flow_rect(kinds, i).contains(flow))
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        kinds: &[Kind],
        marks: &Marks,
        report: Option<&RunReport>,
    ) -> Option<String> {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let now = ui.input(|input| input.time);

        // 先把这次的缩略图准备好 —— 卡片高度、预览绘制都要用到。
        self.sync_previews(ui.ctx(), report);

        self.handle_input(ui, &resp, kinds, now);
        self.advance(now);

        // 十字光标只在**还按着右键**的时候。收刀之后那段跟出的光效不该拖着光标 ——
        // 松手就该变回普通光标。
        let holding = self
            .slash
            .as_ref()
            .is_some_and(|slash| slash.release.is_none());
        if holding {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if self.hovered_port.is_some() || self.connect.is_some() {
            // 端口上给个「能点」的手型。
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let painter = ui.painter_at(rect);

        // ---- 背景小点 ----
        // 只在点距够大（没缩太远）时才画，省得小到看不清还白花功夫。
        let step = DOT_GAP * self.zoom;
        if step > 5.0 {
            let ox = self.pan.x.rem_euclid(step);
            let oy = self.pan.y.rem_euclid(step);
            let mut y = rect.top() - step + oy;
            while y < rect.bottom() + step {
                let mut x = rect.left() - step + ox;
                while x < rect.right() + step {
                    painter.circle_filled(egui::pos2(x, y), 1.0, theme::CANVAS_DOT);
                    x += step;
                }
                y += step;
            }
        }

        // ---- 连线 ----
        for i in 0..self.wires.len() {
            self.draw_wire(&painter, kinds, marks, i, now);
        }

        // ---- 正在断开的连线（它们已经不在 `wires` 里了） ----
        self.draw_severing(&painter, now);

        // ---- 正在拉的那根线：画在节点**下面**，这样线尾会被端点盖住 ----
        self.draw_connecting(&painter, kinds, now);

        // ---- 节点 ----
        let pointer = ui.input(|input| input.pointer.hover_pos());
        self.entering
            .retain(|_, started| now - *started < ENTER_SECS);
        let enter: Vec<f32> = self
            .nodes
            .iter()
            .map(|node| self.enter_factor(&node.id, now))
            .collect();
        // 先把所有阴影铺一遍，再把卡片本体盖上去 —— 两张卡叠在一起时，
        // 上面那张的阴影不会落到下面那张身上，看着才像真的盖住了。
        for (i, &factor) in enter.iter().enumerate() {
            if factor >= 1.0 {
                self.draw_node_shadow(&painter, kinds, i);
            } else {
                let mut faded = ui.painter_at(rect);
                faded.set_opacity(factor);
                self.draw_node_shadow(&faded, kinds, i);
            }
        }
        // 卡片 + 它自己的参数控件，**逐个节点**画 ——
        // 控件必须夹在卡片之间，否则下面那张卡的控件会盖到上面那张卡上。
        let mut controls_changed = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            {
                let style = ui.style_mut();
                for (text_style, size) in [
                    (egui::TextStyle::Body, 11.0),
                    (egui::TextStyle::Button, 11.0),
                    (egui::TextStyle::Monospace, 11.0),
                    (egui::TextStyle::Small, 9.5),
                ] {
                    style
                        .text_styles
                        .insert(text_style, FontId::monospace(size * self.zoom));
                }
                style.spacing.button_padding = egui::vec2(5.0, 1.0) * self.zoom;
                style.spacing.interact_size.y = 18.0 * self.zoom;
                style.spacing.icon_width = 9.0 * self.zoom;
                style.spacing.slider_width = 96.0 * self.zoom;
                style.spacing.item_spacing = egui::vec2(4.0, 3.0) * self.zoom;
            }
            for (i, &factor) in enter.iter().enumerate() {
                let mut faded = ui.painter_at(rect);
                if factor < 1.0 {
                    faded.set_opacity(factor);
                }
                self.draw_node(&faded, kinds, marks, i, pointer, now);
                // 参数控件紧跟在这张卡后面注册 —— 这就是原版的 `nodrag`，
                // 同时也让叠放顺序和卡片一致。
                if self.draw_node_controls(ui, kinds, i) {
                    controls_changed = true;
                }
            }
        });
        if controls_changed {
            self.touch();
        }

        // ---- 刀光在最上面 ----
        self.draw_slash(&painter, now);

        // ---- 节点右键菜单（是另一层浮层，和 painter 的先后无关） ----
        self.draw_menu(ui);

        // ---- 左下角：缩放控件 ----
        self.zoom_controls(ui, rect, kinds);

        self.pending_run.take()
    }

    /// 左下角一竖条缩放控件（放大 / 缩小 / 适应画面）。
    fn zoom_controls(&mut self, ui: &mut egui::Ui, canvas: Rect, kinds: &[Kind]) {
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("zoom-controls"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                let size = egui::vec2(29.0, 29.0 * 3.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                widgets::card_shadow(ui.painter(), rect, theme::R_CARD);
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(theme::R_CARD), theme::HAIRLINE);
                let inner = rect.shrink(1.0);
                let h = inner.height() / 3.0;
                let icons: [IconFn; 3] = [icons::zoom_in, icons::zoom_out, icons::fit_view];
                for (k, icon) in icons.into_iter().enumerate() {
                    let b = Rect::from_min_size(
                        egui::pos2(inner.left(), inner.top() + k as f32 * h),
                        egui::vec2(inner.width(), h - if k < 2 { 1.0 } else { 0.0 }),
                    );
                    let resp = ui.interact(b, ui.id().with(k), Sense::click());
                    let hot = resp.hovered();
                    let radius = CornerRadius {
                        nw: if k == 0 { theme::R_CARD - 1 } else { 0 },
                        ne: if k == 0 { theme::R_CARD - 1 } else { 0 },
                        sw: if k == 2 { theme::R_CARD - 1 } else { 0 },
                        se: if k == 2 { theme::R_CARD - 1 } else { 0 },
                    };
                    ui.painter().rect_filled(
                        b,
                        radius,
                        if hot {
                            theme::SURFACE_2
                        } else {
                            theme::SURFACE
                        },
                    );
                    icon(
                        ui.painter(),
                        widgets::icon_rect(b, 14.0),
                        if hot { theme::ACCENT } else { theme::INK_3 },
                    );
                    if resp.clicked() {
                        match k {
                            0 => self.zoom_by(1.2, canvas),
                            1 => self.zoom_by(1.0 / 1.2, canvas),
                            _ => self.fit_view(canvas, kinds),
                        }
                    }
                }
            });
    }

    /// 以画布中央为锚点缩放。
    fn zoom_by(&mut self, factor: f32, canvas: Rect) {
        let anchor = self.to_flow(canvas.center());
        self.zoom = (self.zoom * factor).clamp(0.15, 2.0);
        self.pan = canvas.center().to_vec2() - anchor.to_vec2() * self.zoom;
    }

    /// 把所有节点框进视野。
    fn fit_view(&mut self, canvas: Rect, kinds: &[Kind]) {
        if self.nodes.is_empty() {
            self.zoom = 1.0;
            self.pan = egui::vec2(40.0, 30.0);
            return;
        }
        let mut bounds: Option<Rect> = None;
        for node in &self.nodes {
            let rect =
                Rect::from_min_size(node.pos, egui::vec2(NODE_W, height_of_node(kinds, node)));
            bounds = Some(match bounds {
                Some(b) => b.union(rect),
                None => rect,
            });
        }
        let Some(bounds) = bounds else { return };
        let scale = (canvas.width() / bounds.width().max(1.0))
            .min(canvas.height() / bounds.height().max(1.0));
        self.zoom = (scale * 0.75).clamp(0.2, 1.05);
        self.pan = canvas.center().to_vec2() - bounds.center().to_vec2() * self.zoom;
    }

    /// 有没有还在跑的动画，决定要不要继续申请重绘。
    pub fn is_animating(&self) -> bool {
        self.slash.is_some()
            || !self.severing.is_empty()
            || self.connect.is_some()
            || !self.entering.is_empty()
    }

    /// 入场动画的进度（0→1，缓出）。不在 `entering` 里就是 1。
    fn enter_factor(&self, id: &str, now: f64) -> f32 {
        match self.entering.get(id) {
            Some(started) => {
                let t = ((now - started) / ENTER_SECS).clamp(0.0, 1.0) as f32;
                1.0 - (1.0 - t) * (1.0 - t)
            }
            None => 1.0,
        }
    }

    /// 推进两个定时动画：断口回缩、收刀跟出。
    fn advance(&mut self, now: f64) {
        self.severing
            .retain(|entry| now - entry.started < SEVER_SECS);

        if let Some(slash) = &self.slash {
            if let Some(start) = slash.release {
                if now - start >= OVERTAKE_SECS {
                    self.slash = None;
                }
            }
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, resp: &egui::Response, kinds: &[Kind], now: f64) {
        // ---- 滚轮缩放（以指针为锚点） ----
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 && resp.contains_pointer() {
            if let Some(p) = ui.input(|input| input.pointer.hover_pos()) {
                let anchor = self.to_flow(p);
                self.zoom = (self.zoom * (1.0 + scroll * 0.0015)).clamp(0.15, 2.0);
                self.pan = p.to_vec2() - anchor.to_vec2() * self.zoom;
            }
        }

        // ---- 端口悬停 ----
        self.hovered_port = ui
            .input(|input| input.pointer.hover_pos())
            .and_then(|p| self.port_at(kinds, self.to_flow(p)));

        // ---- 左键：从端口拉线 / 拖节点 / 拖空白平移画布 ----
        if resp.drag_started_by(PointerButton::Primary) {
            // 用**按下时**的位置判定端口，而不是拖过阈值之后的当前位置 ——
            // 等 `drag_started` 触发时指针已经移开好几像素，小小的端口就点不中了，
            // 拖动于是掉给背景去平移画布。`press_origin` 一直停在按下的那一点。
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(p) = origin {
                let flow = self.to_flow(p);
                // 落在端口上就是拉线，否则才是拖节点 —— 端口优先。
                match self.port_at(kinds, flow) {
                    Some(from) => {
                        self.connect = Some(Connect {
                            from,
                            to: p,
                            target: None,
                        });
                        self.grabbed = None;
                    }
                    None => {
                        // 摇起来的节点提到最上面（最后碰过的在最上层），顺便让它聚焦。
                        let hit = self
                            .hit(kinds, flow)
                            .map(|index| self.bring_to_front(index));
                        if let Some(index) = hit {
                            self.selected = Some(index);
                        }
                        self.grabbed = hit;
                    }
                }
            }
        }

        if resp.dragged_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some(from) = self.connect.as_ref().map(|connect| connect.from) {
                    let target = self
                        .port_at(kinds, self.to_flow(p))
                        .filter(|candidate| self.can_link(from, *candidate));
                    if let Some(connect) = self.connect.as_mut() {
                        connect.to = p;
                        connect.target = target;
                    }
                }
            }

            // 正在拉线就别同时拖节点了。
            if self.connect.is_none() {
                let delta = resp.drag_delta();
                match self.grabbed {
                    Some(i) => {
                        self.nodes[i].pos += delta / self.zoom;
                        self.touch();
                    }
                    None => self.pan += delta,
                }
            }
        }

        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Some(connect) = self.connect.take() {
                if let Some(target) = connect.target {
                    self.link(connect.from, target);
                }
            }
            self.grabbed = None;
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
                    if play.contains(p) {
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
                                let _ = open::that(dir);
                            }
                            handled = true;
                            break;
                        }
                        if save.contains(p) {
                            self.export_output(&file);
                            handled = true;
                            break;
                        }
                    }
                }
            }
            if !handled {
                self.selected = pointer
                    .and_then(|p| self.hit(kinds, self.to_flow(p)))
                    .map(|index| self.bring_to_front(index));
                // 点一下就收掉节点菜单 —— 选中变了，菜单里的下标就靠不住了。
                self.menu = None;
            }
        }

        // ---- 右键：划一刀 ----
        // 右键单击（没拖过阈值）不划刀 —— 那是要在节点上开菜单。
        if resp.secondary_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let hit = self.hit(kinds, self.to_flow(p));
                self.menu = hit.map(|index| (index, p));
                if let Some(index) = hit {
                    self.selected = Some(index);
                }
            } else {
                self.menu = None;
            }
        }
        if resp.drag_started_by(PointerButton::Secondary) {
            // 和左键同理：用按下的那一点起步，刀口才不会一开就偏几像素。
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(p) = origin {
                // 落在节点上的右键不划刀 —— 那该留给节点的右键菜单。
                if self.hit(kinds, self.to_flow(p)).is_none() {
                    self.slash = Some(Slash {
                        from: p,
                        to: p,
                        doomed: Vec::new(),
                        release: None,
                    });
                }
            }
        }
        if resp.dragged_by(PointerButton::Secondary) {
            if let (Some(p), Some(slash)) = (resp.interact_pointer_pos(), self.slash.as_mut()) {
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
                        .map(|p| self.to_flow(p))
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
    fn refresh_doomed(&mut self, kinds: &[Kind]) {
        let Some(slash) = &self.slash else {
            return;
        };
        let (from, to) = (slash.from, slash.to);
        // 太短的一划不当刀 —— 免得右键单击就切掉一堆线。
        if (to - from).length() < 2.0 {
            return;
        }

        let a = self.to_flow(from);
        let b = self.to_flow(to);

        let doomed = (0..self.wires.len())
            .filter(|&i| {
                let curve = self.wire_points(kinds, &self.wires[i]);
                let poly = geometry::sample(&curve, geometry::steps_for(&curve));
                geometry::first_hit(&poly, a, b).is_some()
            })
            .collect();

        if let Some(slash) = self.slash.as_mut() {
            slash.doomed = doomed;
        }
    }

    /// 收刀：把切中的连线摘掉、起断裂动画，并让刀光跟出后消散。
    fn finish_slash(&mut self, kinds: &[Kind], now: f64) {
        let Some(mut slash) = self.slash.take() else {
            return;
        };

        let a = self.to_flow(slash.from);
        let b = self.to_flow(slash.to);

        let mut cuts = Vec::new();
        for i in 0..self.wires.len() {
            let curve = self.wire_points(kinds, &self.wires[i]);
            let poly = geometry::sample(&curve, geometry::steps_for(&curve));
            if let Some((t, _)) = geometry::first_hit(&poly, a, b) {
                cuts.push((i, curve, t));
            }
        }

        // 从后往前删，免得下标挪位。
        let cut_count = cuts.len();
        for (i, curve, t) in cuts.into_iter().rev() {
            self.severing.push(Severing {
                curve,
                t,
                started: now,
            });
            self.wires.remove(i);
        }
        if cut_count > 0 {
            self.touch();
        }

        slash.doomed.clear();
        slash.release = Some(now);
        self.slash = Some(slash);
    }

    fn draw_wire(
        &self,
        painter: &egui::Painter,
        kinds: &[Kind],
        marks: &Marks,
        i: usize,
        now: f64,
    ) {
        let wire = &self.wires[i];
        let curve = self.wire_points(kinds, wire);
        let screen: Cubic = curve.map(|p| self.to_screen(p));

        // 被刀光扫到：先铺一层更宽的发光，再画本体，并轻轻搏动。
        if let Some(slash) = &self.slash {
            if slash.doomed.contains(&i) {
                let pulse = 0.55 + 0.45 * (now * 6.0).sin() as f32;
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(9.0 * self.zoom, theme::danger_alpha(0.16 * pulse)),
                ));
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.4 * self.zoom, theme::danger_alpha(pulse)),
                ));
                return;
            }
        }

        // 静态检查说这条线接不上 → 红色虚线。
        if marks.invalid_edges.contains(&wire.id(&self.nodes)) {
            self.dashed_curve(
                painter,
                &curve,
                Stroke::new((1.6 * self.zoom).max(1.0), theme::DANGER),
                5.0 * self.zoom,
                4.0 * self.zoom,
            );
            return;
        }

        // 两端都跑成功了 → 点亮成强调色，一眼能看出信号流到哪儿了。
        let ran = marks.ok_nodes.contains(&self.nodes[wire.from].id)
            && marks.ok_nodes.contains(&self.nodes[wire.to].id);

        painter.add(CubicBezierShape::from_points_stroke(
            screen,
            false,
            Color32::TRANSPARENT,
            Stroke::new(
                (1.5 * self.zoom).max(1.0),
                if ran { theme::ACCENT } else { theme::WIRE },
            ),
        ));
    }

    /// 虚线贝塞尔。egui 没有虚线描边，就沿采样点按累计长度切段画 ——
    /// 按累计长度（而不是每小段各自重头算）才能让相位连续。
    fn dashed_curve(
        &self,
        painter: &egui::Painter,
        curve: &Cubic,
        stroke: Stroke,
        dash: f32,
        gap: f32,
    ) {
        let period = dash + gap;
        if period <= 0.0 {
            return;
        }

        let points: Vec<Pos2> = geometry::sample(curve, geometry::steps_for(curve))
            .into_iter()
            .map(|p| self.to_screen(p))
            .collect();

        let mut travelled = 0.0;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = (b - a).length();
            if length <= 0.001 {
                continue;
            }
            let dir = (b - a) / length;
            let mut at = 0.0;
            while at < length {
                let phase = (travelled + at) % period;
                let step = if phase < dash {
                    (dash - phase).min(length - at)
                } else {
                    (period - phase).min(length - at)
                };
                if phase < dash {
                    painter.line_segment([a + dir * at, a + dir * (at + step)], stroke);
                }
                at += step;
            }
            travelled += length;
        }
    }

    fn draw_severing(&self, painter: &egui::Painter, now: f64) {
        for entry in &self.severing {
            let progress = ((now - entry.started) / SEVER_SECS).clamp(0.0, 1.0) as f32;
            let eased = 1.0 - (1.0 - progress) * (1.0 - progress);
            let fade = 1.0 - ((progress - 0.5) / 0.5).max(0.0);
            let stroke = Stroke::new(2.4 * self.zoom, theme::danger_alpha(fade));

            let (left, right) = geometry::split(&entry.curve, entry.t);

            // 两截各自从断口往回缩：左半留前 `keep`，右半留后 `keep`。
            let keep = 1.0 - eased;
            if keep > 0.01 {
                self.stroke_curve(painter, &geometry::split(&left, keep).0, stroke);
                self.stroke_curve(painter, &geometry::split(&right, 1.0 - keep).1, stroke);
            }

            // 断口的那一下光。
            let point = self.to_screen(geometry::at(&entry.curve, entry.t));
            let radius = (3.0 + eased * 20.0) * self.zoom;
            let alpha = (1.0 - progress * 2.4).max(0.0);
            painter.circle_filled(point, radius, theme::danger_alpha(alpha * 0.9));
        }
    }

    fn stroke_curve(&self, painter: &egui::Painter, curve: &Cubic, stroke: Stroke) {
        let points: Vec<Pos2> = geometry::sample(curve, geometry::steps_for(curve))
            .into_iter()
            .map(|p| self.to_screen(p))
            .collect();
        painter.line(points, stroke);
    }

    fn draw_slash(&self, painter: &egui::Painter, now: f64) {
        let Some(slash) = &self.slash else {
            return;
        };

        // 收刀之后：刀尖往前送一段，同时变淡变细，像力道用完了。
        let (tip, fade, scale) = match slash.release {
            None => (slash.to, 1.0, 1.0),
            Some(start) => {
                let t = ((now - start) / OVERTAKE_SECS).clamp(0.0, 1.0) as f32;
                let eased = 1.0 - (1.0 - t).powi(3);
                (
                    geometry::extend(slash.from, slash.to, OVERTAKE_REACH * eased),
                    1.0 - eased.powf(1.6),
                    1.0 - eased * 0.45,
                )
            }
        };
        if fade <= 0.01 {
            return;
        }

        // 尾巴透明、刀尖最亮。egui 没有描边渐变，就切成小段自己调透明度。
        const SEGMENTS: usize = 14;
        for i in 0..SEGMENTS {
            let t0 = i as f32 / SEGMENTS as f32;
            let t1 = (i + 1) as f32 / SEGMENTS as f32;
            let a = slash.from.lerp(tip, t0);
            let b = slash.from.lerp(tip, t1);
            let bright = (t0 + t1) * 0.5;

            painter.line_segment(
                [a, b],
                Stroke::new(16.0 * scale, theme::accent_alpha(bright * 0.16 * fade)),
            );
            painter.line_segment(
                [a, b],
                Stroke::new(3.5 * scale, theme::accent_alpha(bright * fade)),
            );
        }

        // 刀尖。命中的时候亮一点、大一点。
        let radius = if slash.doomed.is_empty() { 3.0 } else { 4.5 };
        painter.circle_filled(tip, radius * scale, theme::accent_alpha(fade));
    }

    /// 卡片右上角两个工具按钮的矩形（屏幕坐标）：返回 `(运行至此, 删除)`。
    fn node_tool_rects(&self, kinds: &[Kind], i: usize) -> (Rect, Rect) {
        let r = self.screen_rect(kinds, i);
        let z = self.zoom;
        let s = TOOL_SIZE * z;
        let pad = 5.0 * z;
        let gap = 1.0 * z;
        let y = r.top() + (HEADER_H * z - s) * 0.5;
        let del = Rect::from_min_size(egui::pos2(r.right() - pad - s, y), egui::vec2(s, s));
        let play = Rect::from_min_size(egui::pos2(del.left() - gap - s, y), egui::vec2(s, s));
        (play, del)
    }

    /// 指针现在悬在哪个节点上（决定要不要露出工具按钮）。
    fn hovered_node(&self, kinds: &[Kind], pointer: Option<Pos2>) -> Option<usize> {
        pointer.and_then(|p| self.hit(kinds, self.to_flow(p)))
    }

    /// 某个端口在流坐标里的位置。
    fn port_pos(&self, kinds: &[Kind], port: PortRef) -> Pos2 {
        if port.input {
            self.in_port(kinds, port.node, port.port)
        } else {
            self.out_port(kinds, port.node, port.port)
        }
    }

    /// 指针底下有没有端口。判定半径是**屏幕**像素 —— 缩放不该让端口更难或更好点。
    fn port_at(&self, kinds: &[Kind], flow: Pos2) -> Option<PortRef> {
        let reach = PORT_HIT / self.zoom;
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

    /// 端口现在的放大倍数：指针悬在上面、或者正是这根线的起点时，放大一点。
    fn port_scale(&self, port: PortRef) -> f32 {
        let hot = self.hovered_port == Some(port)
            || self.connect.as_ref().is_some_and(|c| c.from == port);
        if hot {
            1.8
        } else {
            1.0
        }
    }

    /// 这个端口是不是当前这根线可以落下的地方。
    fn is_valid_target(&self, port: PortRef) -> bool {
        self.connect
            .as_ref()
            .is_some_and(|connect| connect.target == Some(port))
    }

    /// 这两个端口能不能接上。
    ///
    /// 用**端口自己的真实类型**判断（`PortType::accepts`），不看徽标字符串 ——
    /// 编辑期那条规则本来就是宽松的（一头是通配就放行、格式未知的图像哪儿都能接），
    /// 和 core 的静态检查是同一套。
    fn can_link(&self, from: PortRef, to: PortRef) -> bool {
        // 必须一头输入一头输出，而且不给自己接线。
        if from.input == to.input || from.node == to.node {
            return false;
        }
        let (out, input) = if from.input { (to, from) } else { (from, to) };

        let (Some(source), Some(target)) = (
            self.nodes[out.node].outputs.get(out.port),
            self.nodes[input.node].inputs.get(input.port),
        ) else {
            return false;
        };

        // 一个输入端口只接一条线。想换就先划断那一条。
        let occupied = self
            .wires
            .iter()
            .any(|wire| wire.to == input.node && wire.to_port == target.id);

        !occupied && target.ty.accepts(source.ty)
    }

    /// 把两个端口接上。
    fn link(&mut self, from: PortRef, to: PortRef) {
        let (out, input) = if from.input { (to, from) } else { (from, to) };
        let wire = Wire {
            from: out.node,
            from_port: self.nodes[out.node].outputs[out.port].id.clone(),
            to: input.node,
            to_port: self.nodes[input.node].inputs[input.port].id.clone(),
        };
        self.wires.push(wire);
        self.selected = Some(input.node);
        self.touch();
    }

    /// 节点上的右键菜单。
    fn draw_menu(&mut self, ui: &egui::Ui) {
        let Some((index, at)) = self.menu else {
            return;
        };
        // 先把要显示的字取出来，免得闭包里再借 `self`。
        let Some(title) = self.nodes.get(index).map(|node| node.title.clone()) else {
            self.menu = None;
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
            self.menu = None;
        }
    }

    /// 把一个节点提到最上层：把它挪到 `nodes` 末尾，并把连线 / 选中里的下标跟着换算。
    ///
    /// 返回它挪到的下标 —— 调用方接着要用。
    fn bring_to_front(&mut self, index: usize) -> usize {
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
    fn delete_node(&mut self, index: usize) {
        if index >= self.nodes.len() {
            return;
        }
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
        self.menu = None;
        self.touch();
    }

    /// 复制一个节点，错开一点免得盖住原件。
    fn duplicate_node(&mut self, index: usize) {
        let Some(source) = self.nodes.get(index) else {
            return;
        };
        let mut copy = source.clone();
        copy.id = new_id();
        copy.pos += egui::vec2(28.0, 28.0);
        refresh_ports(&mut copy);
        self.nodes.push(copy);
        self.selected = Some(self.nodes.len() - 1);
        self.menu = None;
        self.touch();
    }

    /// 粘贴一个节点：换新 id、清掉上次运行的痕迹，落在 `at`（流坐标）。
    fn paste_node(&mut self, mut node: Node, at: Pos2) {
        node.id = new_id();
        node.pos = at;
        node.run = None;
        node.preview = false;
        refresh_ports(&mut node);
        self.nodes.push(node);
        self.selected = Some(self.nodes.len() - 1);
        self.menu = None;
        self.touch();
    }

    /// 外壳读一下有没有 Ctrl+S 的请求，读到就清零。
    pub fn take_save_request(&mut self) -> bool {
        std::mem::take(&mut self.save_requested)
    }

    /// 把这次运行的缩略图传成纹理、并把运行痕迹贴到各节点上。
    ///
    /// 报告一换（`finished_at` 变了）就把旧纹理全丢掉重建 —— `TextureHandle`
    /// 一 drop，纹理由 egui 回收，不会越攒越多。
    fn sync_previews(&mut self, ctx: &egui::Context, report: Option<&RunReport>) {
        let stamp = report.map(|report| report.finished_at);
        if self.report_stamp != stamp {
            self.textures.clear();
            for node in &mut self.nodes {
                node.preview = false;
                node.run = None;
            }
            self.report_stamp = stamp;
        }

        let Some(report) = report else {
            return;
        };

        // 先把运行痕迹（状态 / 耗时 / 警告 / 错误 / 产物）贴到各节点上。
        for result in &report.nodes {
            let Some(node) = self.nodes.iter_mut().find(|node| node.id == result.node_id) else {
                continue;
            };
            let file = result.outputs.iter().find_map(|output| output.path.clone());
            node.run = Some(NodeRun {
                status: result.status,
                ms: result.elapsed_ms,
                error: result.error.clone(),
                warnings: result.warnings.clone(),
                file,
            });
        }

        // 再把缩略图传成纹理（每个节点只传一次）。
        for result in &report.nodes {
            if self.textures.contains_key(&result.node_id) {
                continue;
            }
            // 一个节点只取第一张能预览的图 —— 卡上只放一条。
            let Some(url) = result
                .outputs
                .iter()
                .find_map(|output| output.preview.as_deref())
            else {
                continue;
            };
            let Some((width, height, rgba)) =
                starrytools_core::image_io::decode_preview_data_url(url)
            else {
                continue;
            };
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
            let texture = ctx.load_texture(
                format!("preview:{}", result.node_id),
                image,
                egui::TextureOptions::NEAREST,
            );
            self.textures.insert(result.node_id.clone(), texture);
            if let Some(node) = self.nodes.iter_mut().find(|node| node.id == result.node_id) {
                node.preview = true;
            }
        }
    }

    /// 卡片底部的预览条：先铺棋盘格（透出底下的透明像素），再把图像按比例放进去。
    fn draw_preview(&self, painter: &egui::Painter, inner: Rect, texture: &egui::TextureHandle) {
        let cr = CornerRadius::same(theme::R_CTL);
        painter.rect_filled(inner, cr, theme::SURFACE_3);

        // 棋盘格：只在格子上画第二种颜色，比每格画两个矩形省一半。
        let cell = CHECKER * self.zoom;
        if cell > 2.0 {
            let mut row = 0usize;
            let mut y = inner.top();
            while y < inner.bottom() {
                let mut col = 0usize;
                let mut x = inner.left();
                while x < inner.right() {
                    if (row + col).is_multiple_of(2) {
                        let rect = Rect::from_min_max(
                            egui::pos2(x, y),
                            egui::pos2(
                                (x + cell).min(inner.right()),
                                (y + cell).min(inner.bottom()),
                            ),
                        );
                        painter.rect_filled(rect, CornerRadius::ZERO, theme::SURFACE_2);
                    }
                    x += cell;
                    col += 1;
                }
                y += cell;
                row += 1;
            }
        }

        // 图像按比例居中放进去 —— 像素画用最近邻，贴图里已经设好了。
        let size = texture.size_vec2();
        if size.x > 0.0 && size.y > 0.0 {
            let scale = (inner.width() / size.x).min(inner.height() / size.y);
            let fitted = Rect::from_center_size(inner.center(), size * scale);
            painter.image(
                texture.id(),
                fitted,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        painter.rect_stroke(
            inner,
            cr,
            Stroke::new(1.0, theme::HAIRLINE),
            StrokeKind::Inside,
        );
    }

    /// 正在拉的那根线。靠近合法端口就吸附到端口中心。
    fn draw_connecting(&self, painter: &egui::Painter, kinds: &[Kind], now: f64) {
        let Some(connect) = &self.connect else {
            return;
        };

        let a = self.to_screen(self.port_pos(kinds, connect.from));
        // 有落点候选就直接吸到那个端口上 —— 手感上就是「啪」地贴过去。
        let b = match connect.target {
            Some(target) => self.to_screen(self.port_pos(kinds, target)),
            None => connect.to,
        };
        let snapped = connect.target.is_some();
        // 从输出端往外拉，控制点朝右；从输入端往外拉就朝左 —— 看起来才像「拽出一根线」。
        let reach = ((b.x - a.x).abs() * 0.55).max(48.0);
        let (c1, c2) = if connect.from.input {
            (egui::pos2(a.x - reach, a.y), egui::pos2(b.x + reach, b.y))
        } else {
            (egui::pos2(a.x + reach, a.y), egui::pos2(b.x - reach, b.y))
        };

        let color = if snapped { theme::ACCENT } else { theme::INK_3 };
        let width = (1.8 * self.zoom).max(1.2);

        // 还没接到端口上：整根线一呼一吸地泛蓝光，提示「尚未连接」。
        let pulse = if snapped {
            0.0
        } else {
            let wave = 0.5 + 0.5 * (now * 5.0).sin() as f32;
            // 外圈光晕
            painter.add(CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(
                    width + 7.0 * self.zoom,
                    theme::accent_alpha(0.04 + 0.10 * wave),
                ),
            ));
            wave
        };

        painter.add(CubicBezierShape::from_points_stroke(
            [a, c1, c2, b],
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        ));
        // 叠一层会呼吸的强调色，让「还没接上」这件事一眼看得出来。
        if !snapped {
            painter.add(CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, theme::accent_alpha(0.14 + 0.40 * pulse)),
            ));
        }
        if snapped {
            painter.circle_filled(b, 4.5 * self.zoom, Color32::WHITE);
            painter.circle_filled(b, 3.0 * self.zoom, color);
        } else {
            // 自由端也跟着一明一灭。
            painter.circle_filled(
                b,
                (5.0 + pulse * 2.5) * self.zoom,
                theme::accent_alpha(0.16 + 0.18 * pulse),
            );
            painter.circle_filled(b, 3.0 * self.zoom, color);
        }
    }

    /// 只画卡片的投影。单独一遍，为了不和别的卡片的填色打架。
    fn draw_node_shadow(&self, painter: &egui::Painter, kinds: &[Kind], i: usize) {
        widgets::card_shadow(painter, self.screen_rect(kinds, i), theme::R_CARD);
    }

    fn draw_node(
        &self,
        painter: &egui::Painter,
        kinds: &[Kind],
        marks: &Marks,
        i: usize,
        pointer: Option<Pos2>,
        now: f64,
    ) {
        let node = &self.nodes[i];
        let r = self.screen_rect(kinds, i);
        let cr = CornerRadius::same(theme::R_CARD);
        let z = self.zoom;
        // 卡片里面的字一律剪到卡片内 —— 长警告 / 长错误、甚至文件名也不会溢出去。
        let body = painter.with_clip_rect(r);

        // 白底（投影已经在上一遍里铺好了）。
        painter.rect_filled(r, cr, theme::SURFACE);

        // 边线颜色说三件事：跑失败了、跑成功了、还是平常。
        // **边线放到最后画** —— 里面的各段底色是铺满整宽的，先画会被它们盖住。
        let border = if marks.failed_nodes.contains(&node.id) {
            theme::DANGER
        } else if marks.ok_nodes.contains(&node.id) {
            theme::ACCENT
        } else {
            theme::HAIRLINE
        };

        // ---- 头部 ----
        let head_h = HEADER_H * z;
        let cy = r.top() + head_h * 0.5;
        painter.line_segment(
            [
                egui::pos2(r.left(), r.top() + head_h),
                egui::pos2(r.right(), r.top() + head_h),
            ],
            Stroke::new(1.0, theme::HAIRLINE),
        );

        let title =
            painter.layout_no_wrap(node.title.clone(), FontId::monospace(12.0 * z), theme::INK);
        let mut x = r.left() + 10.0 * z;
        let title_w = title.size().x;
        painter.galley(egui::pos2(x, cy - title.size().y * 0.5), title, theme::INK);
        x += title_w + 6.0 * z;

        // 「起点」标签：输入端口的节点才没有输入。
        if node.inputs.is_empty() {
            let tag = painter.layout_no_wrap(
                "起点".to_string(),
                FontId::monospace(9.0 * z),
                theme::INK_3,
            );
            let pad = 4.0 * z;
            let box_rect = Rect::from_min_size(
                egui::pos2(x, cy - (tag.size().y + 2.0 * z) * 0.5),
                egui::vec2(tag.size().x + pad * 2.0, tag.size().y + 2.0 * z),
            );
            painter.rect_stroke(
                box_rect,
                CornerRadius::same((3.0 * z) as u8),
                Stroke::new(1.0, theme::HAIRLINE),
                StrokeKind::Inside,
            );
            painter.galley(
                egui::pos2(
                    box_rect.center().x - tag.size().x * 0.5,
                    box_rect.center().y - tag.size().y * 0.5,
                ),
                tag,
                theme::INK_3,
            );
        }

        // 状态 + 工具（鼠标悬停或选中才露出工具）。
        let hovered =
            pointer.is_some_and(|p| r.contains(p)) && self.hovered_node(kinds, pointer) == Some(i);
        let show_tools = hovered || self.selected == Some(i);
        let (play_rect, del_rect) = self.node_tool_rects(kinds, i);
        let right_edge = if show_tools {
            play_rect.left() - 6.0 * z
        } else {
            r.right() - 8.0 * z
        };

        if let Some(run) = &node.run {
            let (text, color) = match run.status {
                NodeStatus::Ok => ("完成", theme::OK),
                NodeStatus::Failed => ("出错", theme::DANGER),
                NodeStatus::Skipped => ("跳过", theme::INK_3),
            };
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                format!("{text} {}ms", run.ms),
                FontId::monospace(9.5 * z),
                color,
            );
        }

        if show_tools {
            for (rect, icon) in [
                (play_rect, icons::play as IconFn),
                (del_rect, icons::x as IconFn),
            ] {
                let hot = pointer.is_some_and(|p| rect.contains(p));
                if hot {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same((theme::R_CTL as f32 * z) as u8),
                        theme::SURFACE_3,
                    );
                }
                icon(
                    painter,
                    widgets::icon_rect(rect, 12.0 * z),
                    if hot { theme::INK } else { theme::INK_2 },
                );
            }
        }

        // ---- 端口（输入在左、输出在右，同一行对齐） ----
        // 注意这几行放到函数**最后**去画：端口要盖在卡片边线上，不能被它切一刀。

        // ---- 参数区底色（浅灰，圆底） ----
        let (warn_h, error_h, actions_h) = run_extra(node);
        // 参数区是不是最后一段 —— 是的话底角才收圆（且贴到卡片底部）。
        let params_last = !params_following(node);
        if let Some(kind) = kind_of(kinds, node) {
            let ph = params_height(kind, &node.params);
            if ph > 0.0 {
                let top = r.top() + node_body_top(node) * z;
                let rect =
                    Rect::from_min_size(egui::pos2(r.left(), top), egui::vec2(r.width(), ph * z));
                painter.rect_filled(
                    rect,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: if params_last { theme::R_CARD } else { 0 },
                        se: if params_last { theme::R_CARD } else { 0 },
                    },
                    theme::SURFACE_2,
                );
                painter.line_segment(
                    [egui::pos2(rect.left(), top), egui::pos2(rect.right(), top)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
            }
        }

        // ---- 参数区以下的几段 ----
        let body_pad = if params_following(node) {
            BODY_PAD
        } else {
            0.0
        };
        let mut y = r.top() + (node_body_top(node) + params_height_of(kinds, node) + body_pad) * z;

        if node.preview {
            if let Some(texture) = self.textures.get(&node.id) {
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
                let inner = Rect::from_min_max(
                    egui::pos2(r.left() + 10.0 * z, y + 8.0 * z),
                    egui::pos2(r.right() - 10.0 * z, y + PREVIEW_H * z - 9.0 * z),
                );
                self.draw_preview(painter, inner, texture);
            }
            y += PREVIEW_H * z;
        }

        let (warn_h, error_h, actions_h) = (warn_h, error_h, actions_h);
        if warn_h > 0.0 {
            if let Some(run) = &node.run {
                for (n, warning) in run.warnings.iter().enumerate() {
                    body.text(
                        egui::pos2(
                            r.left() + 10.0 * z,
                            y + (RUN_WARN_H * 0.5 + n as f32 * RUN_LINE_H) * z,
                        ),
                        Align2::LEFT_CENTER,
                        format!("! {warning}"),
                        FontId::monospace(9.5 * z),
                        theme::WARN,
                    );
                }
            }
            y += warn_h * z;
        }

        if error_h > 0.0 {
            if let Some(run) = &node.run {
                if let Some(error) = &run.error {
                    let last = actions_h == 0.0;
                    let rect = Rect::from_min_size(
                        egui::pos2(r.left(), y),
                        egui::vec2(r.width(), error_h * z),
                    );
                    painter.rect_filled(
                        rect,
                        CornerRadius {
                            nw: 0,
                            ne: 0,
                            sw: if last { theme::R_CARD } else { 0 },
                            se: if last { theme::R_CARD } else { 0 },
                        },
                        theme::DANGER_SOFT,
                    );
                    body.text(
                        egui::pos2(r.left() + 10.0 * z, y + error_h * 0.5 * z),
                        Align2::LEFT_CENTER,
                        error,
                        FontId::monospace(9.5 * z),
                        theme::DANGER,
                    );
                }
            }
            y += error_h * z;
        }

        if actions_h > 0.0 {
            if let Some(run) = &node.run {
                if let Some(file) = &run.file {
                    painter.line_segment(
                        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                        Stroke::new(1.0, theme::HAIRLINE),
                    );
                    let (reveal, save) = self.node_action_rects(kinds, i);
                    for (rect, icon) in [
                        (reveal, icons::folder_search as IconFn),
                        (save, icons::download as IconFn),
                    ] {
                        let hot = pointer.is_some_and(|p| rect.contains(p));
                        if hot {
                            painter.rect_filled(
                                rect,
                                CornerRadius::same((theme::R_CTL as f32 * z) as u8),
                                theme::SURFACE_3,
                            );
                        }
                        icon(
                            painter,
                            widgets::icon_rect(rect, 12.0 * z),
                            if hot { theme::INK } else { theme::INK_3 },
                        );
                    }
                    let name = std::path::Path::new(file)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| file.clone());
                    body.text(
                        egui::pos2(save.right() + 6.0 * z, save.center().y),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::monospace(9.0 * z),
                        theme::INK_3,
                    );
                }
            }
            let _ = actions_h;
        }

        // 卡片边线最后画 —— 否则会被参数区那层铺满整宽的灰底盖住（失败时尤其明显）。
        painter.rect_stroke(r, cr, Stroke::new(1.0, border), StrokeKind::Inside);
        if self.selected == Some(i) {
            painter.rect_stroke(
                r.expand(2.5),
                CornerRadius::same(theme::R_CARD + 2),
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
        }

        // ---- 端口最后画：压在边线上面 ----
        let rows = node.inputs.len().max(node.outputs.len());
        for k in 0..rows {
            if let Some(port) = node.inputs.get(k) {
                self.draw_port(painter, kinds, marks, i, k, port, true, r, z, pointer, now);
            }
            if let Some(port) = node.outputs.get(k) {
                self.draw_port(painter, kinds, marks, i, k, port, false, r, z, pointer, now);
            }
        }
    }

    /// 画一个端口：卡片边线上的圆点 + 徽标 + 标签。
    #[allow(clippy::too_many_arguments)]
    fn draw_port(
        &self,
        painter: &egui::Painter,
        kinds: &[Kind],
        marks: &Marks,
        i: usize,
        k: usize,
        port: &Port,
        input: bool,
        r: Rect,
        z: f32,
        _pointer: Option<Pos2>,
        now: f64,
    ) {
        let here = PortRef {
            node: i,
            port: k,
            input,
        };
        let p = if input {
            self.to_screen(self.in_port(kinds, i, k))
        } else {
            self.to_screen(self.out_port(kinds, i, k))
        };
        let linked = self.port_linked(i, &port.id, input);
        // 出错的（非法连线 / 必填未接）→ 红；其余端点一律主题蓝。
        let bad = self.port_invalid(marks, i, &port.id, input) || (port.required && !linked);
        let dot = if bad { theme::DANGER } else { theme::ACCENT };
        let type_ink = if bad {
            theme::DANGER
        } else {
            theme::port_color(&port.family)
        };

        let hot = self.hovered_port == Some(here)
            || self
                .connect
                .as_ref()
                .is_some_and(|connect| connect.from == here);
        let valid = self.is_valid_target(here);

        // 可落点：外面一圈搏动的柔光，越靠近越亮。
        if valid {
            let pulse = 0.5 + 0.5 * (now * 6.0).sin() as f32;
            painter.circle_filled(p, (8.0 + pulse * 2.5) * z, theme::accent_alpha(0.18));
        }
        // 悬停 / 正在拉的那一端：一圈更静的柔光。
        if hot {
            painter.circle_filled(p, 8.5 * z, theme::accent_alpha(0.20));
        }

        // 圆点：白圈里一个蓝点（对应 Handle 的 2px surface 描边）。
        let scale = self.port_scale(here);
        painter.circle_filled(p, 5.0 * z * scale, Color32::WHITE);
        painter.circle_filled(p, 3.0 * z * scale, dot);
        if bad {
            painter.circle_stroke(p, 6.0 * z * scale, Stroke::new(1.0, theme::DANGER));
        }
        if valid {
            painter.circle_stroke(p, 7.5 * z, Stroke::new(1.5, theme::ACCENT));
        }

        // 徽标（chip）：带色的小框。
        let badge =
            painter.layout_no_wrap(port.badge.clone(), FontId::monospace(8.0 * z), type_ink);
        let label = painter.layout_no_wrap(
            port.label.clone(),
            FontId::monospace(10.0 * z),
            if bad { theme::DANGER } else { theme::INK_3 },
        );
        let pad = 4.0 * z;
        let chip_w = badge.size().x + pad * 2.0;
        let chip_h = badge.size().y + 2.0 * z;
        let gap = 4.0 * z;

        let chip = if input {
            Rect::from_min_size(
                egui::pos2(r.left() + 10.0 * z, p.y - chip_h * 0.5),
                egui::vec2(chip_w, chip_h),
            )
        } else {
            // 输出这一侧：[徽标][标签][圆点]，整体往右靠。
            let right = r.right() - 10.0 * z;
            Rect::from_min_size(
                egui::pos2(right - label.size().x - gap - chip_w, p.y - chip_h * 0.5),
                egui::vec2(chip_w, chip_h),
            )
        };

        painter.rect_stroke(
            chip,
            CornerRadius::same((3.0 * z) as u8),
            Stroke::new(1.0, type_ink),
            StrokeKind::Inside,
        );
        // 徽标与标签都按**墨迹中心**对齐到胶囊中线 ——
        // 汉字墨迹在框里偏下、拉丁徽标偏上，各自按框高校正就会显得「下端对齐」。
        painter.galley(
            egui::pos2(
                chip.center().x - badge.size().x * 0.5,
                crate::widgets::ink_top(&badge, chip.center().y),
            ),
            badge,
            type_ink,
        );

        let label_color = if bad { theme::DANGER } else { theme::INK_3 };
        painter.galley(
            egui::pos2(
                chip.right() + gap,
                crate::widgets::ink_top(&label, chip.center().y),
            ),
            label,
            label_color,
        );
    }

    /// 这个端口是不是接了条被静态检查判为非法的线。
    fn port_invalid(&self, marks: &Marks, node_index: usize, port_id: &str, input: bool) -> bool {
        self.wires.iter().any(|wire| {
            let touches = if input {
                wire.to == node_index && wire.to_port == port_id
            } else {
                wire.from == node_index && wire.from_port == port_id
            };
            touches && marks.invalid_edges.contains(&wire.id(&self.nodes))
        })
    }

    /// 这个端口接没接线。
    fn port_linked(&self, node_index: usize, port_id: &str, input: bool) -> bool {
        self.wires.iter().any(|wire| {
            if input {
                wire.to == node_index && wire.to_port == port_id
            } else {
                wire.from == node_index && wire.from_port == port_id
            }
        })
    }

    /// 「另存为」：把产物拷到用户挑的位置。
    fn export_output(&self, file: &str) {
        let name = std::path::Path::new(file)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "output".to_string());
        if let Some(target) = rfd::FileDialog::new().set_file_name(name).save_file() {
            let _ = std::fs::copy(file, target);
        }
    }

    /// 卡片底部「产物操作行」两个按钮的矩形：`(在文件夹中显示, 另存为)`。
    fn node_action_rects(&self, kinds: &[Kind], i: usize) -> (Rect, Rect) {
        let r = self.screen_rect(kinds, i);
        let z = self.zoom;
        let s = TOOL_SIZE * z;
        let pad = 7.0 * z;
        let y = r.bottom() - RUN_ACTIONS_H * z + (RUN_ACTIONS_H * z - s) * 0.5;
        let reveal = Rect::from_min_size(egui::pos2(r.left() + pad, y), egui::vec2(s, s));
        let save = Rect::from_min_size(egui::pos2(reveal.right() + 2.0 * z, y), egui::vec2(s, s));
        (reveal, save)
    }

    /// 画一个节点的参数控件。**在它的卡片刚画完之后**调用 ——
    /// 这样叠放顺序才和卡片一致（下面那张卡的控件不会盖到上面那张卡上）。
    fn draw_node_controls(&mut self, ui: &mut egui::Ui, kinds: &[Kind], i: usize) -> bool {
        let zoom = self.zoom;
        // 缩得太小时控件会挤成一团，索性只留标题和端口（LOD）。
        if zoom < PARAM_LOD_ZOOM {
            return false;
        }
        let Some(kind) = kind_of(kinds, &self.nodes[i]).cloned() else {
            return false;
        };
        let showing: Vec<usize> = (0..kind.params.len())
            .filter(|&k| kind.params[k].visible(&self.nodes[i].params))
            .collect();
        if showing.is_empty() {
            return false;
        }

        let card = self.screen_rect(kinds, i);
        let mut y = card.top() + self.params_top(i) * zoom;
        let mut changed = false;

        for k in showing {
            let param = &kind.params[k];
            let left = card.left() + 10.0 * zoom;
            let width = card.width() - 20.0 * zoom;

            // 开关：标签和开关同一行。
            if matches!(param.control, Control::Bool) {
                let row = Rect::from_min_size(
                    egui::pos2(left, y),
                    egui::vec2(width, PARAM_BOOL_H * zoom),
                );
                ui.painter().text(
                    egui::pos2(row.left(), row.center().y),
                    Align2::LEFT_CENTER,
                    &param.label,
                    FontId::monospace(10.0 * zoom),
                    theme::INK_3,
                );
                let switch = Rect::from_min_size(
                    egui::pos2(row.right() - 30.0 * zoom, row.top()),
                    egui::vec2(30.0 * zoom, row.height()),
                );
                if let Some(value) = control(ui, i, param, switch, &self.nodes[i].params, zoom) {
                    self.nodes[i].params.insert(param.id.clone(), value);
                    refresh_ports(&mut self.nodes[i]);
                    changed = true;
                }
                y += (PARAM_BOOL_H + PARAM_GAP) * zoom;
                continue;
            }

            // 标签一行。
            ui.painter().text(
                egui::pos2(left, y),
                Align2::LEFT_TOP,
                &param.label,
                FontId::monospace(10.0 * zoom),
                theme::INK_3,
            );
            y += (PARAM_LABEL_H + PARAM_LABEL_GAP) * zoom;

            // 控件一行。
            let height = control_height(param, &self.nodes[i].params) * zoom;
            let control_rect = Rect::from_min_size(egui::pos2(left, y), egui::vec2(width, height));
            if let Some(value) = control(ui, i, param, control_rect, &self.nodes[i].params, zoom) {
                self.nodes[i].params.insert(param.id.clone(), value);
                refresh_ports(&mut self.nodes[i]);
                changed = true;
            }
            y += height;

            // 说明一行（自动折行）。
            if let Some(note) = &param.description {
                y += PARAM_NOTE_GAP * zoom;
                let galley = ui.painter().layout(
                    note.clone(),
                    FontId::monospace(9.5 * zoom),
                    theme::INK_3,
                    width,
                );
                ui.painter()
                    .galley(egui::pos2(left, y), galley, theme::INK_3);
                y += note_height(note, NODE_W - 20.0) * zoom;
            }

            y += PARAM_GAP * zoom;
        }
        changed
    }
}

/// 一个节点的完整高度（流坐标）。
fn height_of_node(kinds: &[Kind], node: &Node) -> f32 {
    let (warn, error, actions) = run_extra(node);
    node_body_top(node)
        + params_height_of(kinds, node)
        + if params_following(node) {
            BODY_PAD
        } else {
            0.0
        }
        + if node.preview { PREVIEW_H } else { 0.0 }
        + warn
        + error
        + actions
}

/// 参数区后面还有没有别的段（缩略图 / 警告 / 错误 / 产物行）。
/// 没有的话，参数区的圆底就贴到卡片底部，和卡片自己的圆角对齐。
fn params_following(node: &Node) -> bool {
    let (warn, error, actions) = run_extra(node);
    node.preview || warn > 0.0 || error > 0.0 || actions > 0.0
}

/// 端口区下沿（相对卡片顶部，流坐标）—— 也就是参数区的上沿。
fn node_body_top(node: &Node) -> f32 {
    let rows = node.inputs.len().max(node.outputs.len()).max(1) as f32;
    HEADER_H + rows * PORT_ROW_H
}

/// 运行痕迹那几段各占多高：`(警告, 错误, 产物操作行)`。
fn run_extra(node: &Node) -> (f32, f32, f32) {
    let Some(run) = &node.run else {
        return (0.0, 0.0, 0.0);
    };
    let warn = if run.warnings.is_empty() {
        0.0
    } else {
        RUN_WARN_H + run.warnings.len() as f32 * RUN_LINE_H
    };
    let error = if run.error.is_some() {
        RUN_ERROR_H
    } else {
        0.0
    };
    let actions = if run.file.is_some() {
        RUN_ACTIONS_H
    } else {
        0.0
    };
    (warn, error, actions)
}

fn height_of(kind: &Kind, params: &Params) -> f32 {
    let rows = kind.inputs.len().max(kind.outputs.len()).max(1) as f32;
    HEADER_H + rows * PORT_ROW_H + BODY_PAD + params_height(kind, params)
}

/// 参数区占多高。被 `visible_when` 藏掉的不占地方。
fn params_height(kind: &Kind, params: &Params) -> f32 {
    let width = NODE_W - 20.0;
    let showing: Vec<&Param> = kind.params.iter().filter(|p| p.visible(params)).collect();
    if showing.is_empty() {
        return 0.0;
    }
    let blocks: f32 = showing
        .iter()
        .map(|param| param_block_height(param, params, width))
        .sum();
    let gaps = (showing.len() - 1) as f32 * PARAM_GAP;
    PARAMS_GAP + blocks + gaps + PARAMS_BOTTOM
}

/// 多行文本框要占多高：行数越多越高，长文本不会溢出下边框。
fn multiline_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1) as f32;
    (lines * PARAM_TEXT_LINE + 10.0).max(PARAM_TEXT_ROW_H)
}

/// 一句参数说明大概占多高（稍微往高了估，宁可多留白也不重叠）。
fn note_height(note: &str, width: f32) -> f32 {
    let per_line = (width / 8.0).max(6.0);
    let lines = (note.chars().count() as f32 / per_line).ceil().max(1.0);
    lines * 14.0
}

/// 这个参数的控件占多高（流坐标）。
fn control_height(param: &Param, params: &Params) -> f32 {
    match &param.control {
        Control::Text {
            multiline: true, ..
        } => {
            let text = params.get(&param.id).and_then(Value::as_str).unwrap_or("");
            multiline_height(text)
        }
        Control::Bool => PARAM_BOOL_H,
        _ => PARAM_CONTROL_H,
    }
}

/// 一个参数（标签 + 控件 + 说明）一共占多高。
fn param_block_height(param: &Param, params: &Params, width: f32) -> f32 {
    let control = control_height(param, params);
    if matches!(param.control, Control::Bool) {
        return control.max(PARAM_LABEL_H);
    }
    let mut height = PARAM_LABEL_H + PARAM_LABEL_GAP + control;
    if let Some(note) = &param.description {
        height += PARAM_NOTE_GAP + note_height(note, width);
    }
    height
}

fn params_height_of(kinds: &[Kind], node: &Node) -> f32 {
    match kind_of(kinds, node) {
        Some(kind) => params_height(kind, &node.params),
        None => 0.0,
    }
}

/// 画一个参数的控件；返回值表示这个参数被改了。
fn control(
    ui: &mut egui::Ui,
    node: usize,
    param: &Param,
    rect: Rect,
    params: &Params,
    zoom: f32,
) -> Option<Value> {
    let id = ui.id().with(("param", node, param.id.as_str()));
    let current = params.get(&param.id);

    match &param.control {
        Control::Bool => {
            let mut value = current.and_then(Value::as_bool).unwrap_or(false);
            toggle(ui, id, rect, &mut value).then(|| serde_json::json!(value))
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
        ),

        Control::Slider {
            min,
            max,
            step,
            integer,
            unit,
        } => {
            let mut value = current.and_then(Value::as_f64).unwrap_or(*min);
            let widget = egui::Slider::new(&mut value, *min..=*max)
                .step_by(*step)
                .suffix(unit.as_deref().unwrap_or(""));
            if ui.put(rect, widget).changed() {
                if *integer {
                    value = value.round();
                }
                Some(serde_json::json!(value))
            } else {
                None
            }
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
        ),

        Control::Select { options } => select_field(ui, id, rect, current, options, zoom),

        Control::File { .. } => file_field(ui, id, rect, current, &param.control, zoom),
    }
}

/// 输入框外壳：白底 + 发丝边 + 卡片圆角。
fn input_shell(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.rect_filled(rect, CornerRadius::same(radius), theme::SURFACE);
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );
}

/// 悬停 / 聚焦时叠上去的那圈边（画在内容之上，不会盖住字）。
fn input_shell_state(
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
fn format_number(value: f64, integer: bool) -> String {
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
fn number_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    value: f64,
    min: f64,
    max: f64,
    integer: bool,
    unit: Option<&str>,
    zoom: f32,
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
fn text_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    multiline: bool,
    placeholder: Option<&str>,
    zoom: f32,
) -> Option<Value> {
    let radius = theme::R_CTL;
    input_shell(ui.painter(), rect, radius);

    let mut value = current.and_then(Value::as_str).unwrap_or("").to_string();
    let inner = rect.shrink2(egui::vec2(8.0 * zoom, 3.0 * zoom));
    let mut widget = if multiline {
        // 行数跟着内容走 —— 长文本会把框撑高，不会溢出下边框。
        let rows = value.lines().count().clamp(3, 20);
        egui::TextEdit::multiline(&mut value).desired_rows(rows)
    } else {
        egui::TextEdit::singleline(&mut value)
    };
    if let Some(hint) = placeholder {
        widget = widget.hint_text(hint);
    }
    let resp = ui.put(
        inner,
        widget
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::monospace(11.5 * zoom))
            .desired_width(inner.width()),
    );
    input_shell_state(ui.painter(), rect, radius, resp.hovered(), resp.has_focus());

    if resp.changed() {
        Some(serde_json::json!(value))
    } else {
        None
    }
}

/// 下拉框：带边框的按钮 + 一个弹出的列表（对勾 + 短说明）。
fn select_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    options: &[crate::catalog::Choice],
    zoom: f32,
) -> Option<Value> {
    let radius = theme::R_CTL;
    let chosen = current.and_then(Value::as_str).unwrap_or("");
    let label = options
        .iter()
        .find(|choice| choice.value == chosen)
        .map(|choice| choice.label.clone())
        .unwrap_or_else(|| "—".to_string());

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
fn menu_item(
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
        egui::pos2(label_x, crate::widgets::ink_top(&galley, rect.center().y)),
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

/// 自己画的开关。
///
/// egui 只有 `checkbox`，而这个设计里开关是有滑动动画的（原版为它专门改过一次），
/// 所以轨道和圆点都自己画。
fn toggle(ui: &mut egui::Ui, id: egui::Id, rect: Rect, value: &mut bool) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let changed = resp.clicked();
    if changed {
        *value = !*value;
    }
    let t = ui.ctx().animate_bool_with_time(id, *value, 0.16);

    let height = (rect.height() * 0.6).clamp(11.0, 18.0);
    let width = (height * 1.8).min(rect.width());
    let track = Rect::from_center_size(
        egui::pos2(rect.left() + width * 0.5, rect.center().y),
        egui::vec2(width, height),
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

    let dot = height * 0.5 - 2.0;
    let travel = (track.width() - 2.0 * (dot + 2.0)).max(0.0);
    let x = track.left() + dot + 2.0 + travel * t;
    painter.circle_filled(egui::pos2(x, track.center().y), dot, theme::SURFACE);

    changed
}

/// 文件 / 目录选择。
///
/// 设计上：空着时是个虚线框的按钮；选完之后文件名顶在原来的位置，
/// 鼠标移上去才重新变回「换一个」。
fn file_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    control: &Control,
    zoom: f32,
) -> Option<Value> {
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

    let mut dialog = rfd::FileDialog::new().set_title(dialog_title);
    if !extensions.is_empty() {
        dialog = dialog.add_filter("支持的格式", extensions);
    }
    let picked = if *directory {
        dialog.pick_folder()
    } else {
        dialog.pick_file()
    };
    picked.map(|path| serde_json::json!(path.to_string_lossy()))
}

/// 路径最后一段。目录选完也是显示最后一段，和文件一样。
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 虚线矩形。egui 没有内置的虚线描边，就自己按段画。
fn dashed_rect(painter: &egui::Painter, rect: Rect, zoom: f32, color: Color32) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 换目标格式之后，节点的输出端口必须跟着变。
    ///
    /// 这条是实际踩过的坑：端口原本是在建节点时从 catalog 抄一份，之后再也
    /// 不管了 —— 于是「图像格式转换」选成 JPEG，卡片上还写着 PNG。
    #[test]
    fn changing_a_param_refreshes_the_ports() {
        let kinds = crate::catalog::all();
        let convert = kinds
            .iter()
            .find(|kind| kind.id == "convert_image")
            .expect("应当有「图像格式转换」");

        // 参数 id 和选项值都从元数据里读，不硬编码 —— 那边改了这里不用跟着改。
        let (param_id, jpeg) = convert
            .params
            .iter()
            .find_map(|param| match &param.control {
                Control::Select { options } => options
                    .iter()
                    .find(|option| option.value == "jpeg")
                    .map(|option| (param.id.clone(), option.value.clone())),
                _ => None,
            })
            .expect("「图像格式转换」应当有一个能选到 jpeg 的下拉");

        let mut node = Node {
            id: new_id(),
            pos: Pos2::ZERO,
            kind: convert.id.clone(),
            title: convert.name.clone(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: convert.defaults.clone(),
            preview: false,
            run: None,
        };
        refresh_ports(&mut node);
        let before = node.outputs[0].badge.clone();

        node.params.insert(param_id, serde_json::json!(jpeg));
        refresh_ports(&mut node);
        let after = node.outputs[0].badge.clone();

        assert_ne!(
            before, after,
            "换了目标格式，输出端口的类型应当跟着变（换前 {before}、换后 {after}）"
        );
    }

    /// 不认识的类型（老存档里的未知节点）不该把界面弄崩。
    #[test]
    fn an_unknown_kind_has_no_ports() {
        let (inputs, outputs) = crate::catalog::ports_for("没有这个节点", &Params::new());
        assert!(inputs.is_empty());
        assert!(outputs.is_empty());
    }

    /// 画布 → 工作流 → 画布：节点、位置、参数、连线都得原样回来。
    #[test]
    fn a_graph_survives_a_round_trip() {
        let kinds = crate::catalog::all();
        let graph = Graph::demo(12, &kinds);
        let workflow = graph.to_workflow(&Workflow::new("往返"));
        let back = Graph::from_workflow(&workflow, &kinds);

        assert_eq!(back.nodes.len(), graph.nodes.len());
        assert_eq!(back.wires.len(), graph.wires.len());

        for (before, after) in graph.nodes.iter().zip(&back.nodes) {
            assert_eq!(before.id, after.id, "节点 id 要原样保留");
            assert_eq!(before.kind, after.kind);
            assert!((before.pos - after.pos).length() < 0.001, "位置要原样保留");
            assert_eq!(before.params, after.params, "参数要原样保留");
            assert_eq!(before.outputs.len(), after.outputs.len(), "端口要重建出来");
        }

        // 连线的端口 id 也要对得上 —— 对不上就会接到别的端口上去。
        for (before, after) in graph.wires.iter().zip(&back.wires) {
            assert_eq!(before.from_port, after.from_port);
            assert_eq!(before.to_port, after.to_port);
            assert_eq!(graph.nodes[before.from].id, back.nodes[after.from].id);
            assert_eq!(graph.nodes[before.to].id, back.nodes[after.to].id);
        }

        // 刚打开的图不算「有未保存的改动」。
        assert_eq!(back.revision, 0);
    }

    /// 改一下画布，版本号要动 —— 未保存的小蓝点就靠它。
    #[test]
    fn editing_bumps_the_revision() {
        let kinds = crate::catalog::all();
        let mut graph = Graph::demo(3, &kinds);
        assert_eq!(graph.revision, 0);
        graph.touch();
        assert_eq!(graph.revision, 1);
    }

    // ---- 连线规则 ----

    /// 造一张只有若干节点、没有连线的空画布，节点的端口按默认参数算好。
    fn canvas(kind_ids: &[&str]) -> Graph {
        let kinds = crate::catalog::all();
        let mut nodes = Vec::new();
        for (i, id) in kind_ids.iter().enumerate() {
            let kind = kinds
                .iter()
                .find(|kind| kind.id == *id)
                .unwrap_or_else(|| panic!("没有「{id}」这个节点"));
            let mut node = Node {
                id: new_id(),
                pos: egui::pos2(i as f32 * 400.0, 0.0),
                kind: kind.id.clone(),
                title: kind.name.clone(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                params: kind.defaults.clone(),
                preview: false,
                run: None,
            };
            refresh_ports(&mut node);
            nodes.push(node);
        }
        Graph {
            nodes,
            wires: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            revision: 0,
            selected: None,
            grabbed: None,
            hovered_port: None,
            connect: None,
            menu: None,
            slash: None,
            severing: Vec::new(),
            textures: HashMap::new(),
            report_stamp: None,
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
        }
    }

    fn out(node: usize, port: usize) -> PortRef {
        PortRef {
            node,
            port,
            input: false,
        }
    }

    fn input(node: usize, port: usize) -> PortRef {
        PortRef {
            node,
            port,
            input: true,
        }
    }

    /// 从输出端口拉到输入端口，应当接上 —— 并在两端记下正确的端口 id。
    #[test]
    fn an_output_links_to_an_input() {
        let mut graph = canvas(&["input", "crop_image"]);
        let from = out(0, 0);
        let to = input(1, 0);

        assert!(graph.can_link(from, to), "输出应当能接到同类输入上");
        graph.link(from, to);

        assert_eq!(graph.wires.len(), 1);
        assert_eq!(graph.wires[0].from, 0);
        assert_eq!(graph.wires[0].to, 1);
        // 记的是**端口 id**，不是下标 —— 端口会随参数增减。
        assert_eq!(graph.wires[0].from_port, graph.nodes[0].outputs[0].id);
        assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[0].id);
        assert_eq!(graph.revision, 1, "接上一条线要算改动");
        assert_eq!(graph.selected, Some(1), "接完顺手选上落点那个节点");
    }

    /// 一个节点不能给自己接线。
    #[test]
    fn a_node_cannot_link_to_itself() {
        let graph = canvas(&["crop_image"]);
        assert!(!graph.can_link(out(0, 0), input(0, 0)), "不能自己接自己");
    }

    /// 必须一头输入一头输出。
    #[test]
    fn a_link_needs_one_input_and_one_output() {
        let graph = canvas(&["input", "crop_image"]);
        assert!(!graph.can_link(out(0, 0), out(1, 0)), "两个输出接不上");
        assert!(!graph.can_link(input(1, 0), input(1, 0)), "两个输入接不上");
        assert!(
            !graph.can_link(out(0, 0), input(1, 9)),
            "不存在的端口接不上（不能越界崩掉）"
        );
    }

    /// 一个输入端口只接一条线，第二条要被拒。
    #[test]
    fn an_input_port_takes_only_one_wire() {
        let mut graph = canvas(&["input", "input", "crop_image"]);
        graph.link(out(0, 0), input(2, 0));
        assert_eq!(graph.wires.len(), 1);

        assert!(
            !graph.can_link(out(1, 0), input(2, 0)),
            "输入端口已经被占了，第二条该被拒"
        );
    }

    /// 类型对不上就接不上：文本输出进不了图像输入。
    #[test]
    fn a_type_mismatch_is_rejected() {
        let mut graph = canvas(&["input", "crop_image"]);
        graph.nodes[0]
            .params
            .insert("valueType".into(), serde_json::json!("text"));
        refresh_ports(&mut graph.nodes[0]);

        assert_eq!(
            graph.nodes[0].outputs[0].badge, "TXT",
            "输入节点的输出该变成文本"
        );
        assert!(
            !graph.can_link(out(0, 0), input(1, 0)),
            "文本不该接到图像输入上"
        );
    }

    // ---- 节点右键菜单的两件事 ----

    /// 删掉一个节点，接在它身上的连线要一起断，剩下连线的下标要跟着往前挪。
    #[test]
    fn deleting_a_node_drops_its_wires_and_reindexes_the_rest() {
        let mut graph = canvas(&["input", "input", "crop_image"]);
        graph.link(out(0, 0), input(2, 0));
        graph.link(out(1, 0), input(2, 0));
        assert_eq!(graph.wires.len(), 2);

        graph.delete_node(0);

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.wires.len(), 1, "接在被删节点上的连线要一起断掉");
        // 原 1→2 的那条，两个下标都该减一。
        assert_eq!(graph.wires[0].from, 0, "剩下的连线下标要往前挪");
        assert_eq!(graph.wires[0].to, 1);
        assert_eq!(graph.selected, None);
        // 挪完还得指得对端口 —— 指错就会接到别的端口上去。
        assert_eq!(graph.wires[0].from_port, graph.nodes[0].outputs[0].id);
        assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[0].id);
    }

    /// 复制一个节点：新 id、参数照搬、端口重算。
    #[test]
    fn duplicating_a_node_gives_it_a_new_id_and_keeps_its_params() {
        let mut graph = canvas(&["crop_image"]);
        graph.nodes[0]
            .params
            .insert("mode".into(), serde_json::json!("size"));
        refresh_ports(&mut graph.nodes[0]);
        let want = graph.nodes[0].params.clone();

        graph.duplicate_node(0);

        assert_eq!(graph.nodes.len(), 2);
        assert_ne!(graph.nodes[0].id, graph.nodes[1].id, "复制体要有自己的 id");
        assert_eq!(graph.nodes[1].params, want, "参数要一起复制过去");
        assert_eq!(graph.nodes[1].outputs.len(), graph.nodes[0].outputs.len());
        assert_eq!(graph.selected, Some(1), "复制完选中新的那个");
    }
}
