//! 手绘的节点画布，外加那把刀。
//!
//! 节点的端口、标题、参数控件全部来自 [`crate::catalog`]（core 的注册表）——
//! 这里同样不认识任何具体工具。
//!
//! 刀光完全是自己算的：连线的控制点本来就在手上，采样和判交都不依赖任何外部渲染。
//!
//! 背景点阵是一大票小圆，逐颗 `circle_filled` 会让 epaint 每帧重新三角化几千个
//! 形状（平移时位置全变，缓存也帮不上忙）。所以改成：一张小圆贴图 + 一个 `Mesh`
//! 把所有点拼起来，一次画出去 —— 圆的数量照样，形状数却只剩一个。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
};
use egui::epaint::CubicBezierShape;
use serde_json::Value;
use starrytools_core::engine::{NodeRunResult, NodeStatus, RunReport};
use starrytools_core::model::params::Params;
use starrytools_core::model::workflow::{Edge, NodeInstance, Position, Workflow};
use std::collections::{HashMap, HashSet};

use crate::catalog::{Control, Kind, Param, Port};
use crate::geometry::{self, Cubic};
use crate::icons::{self, IconFn};
use crate::models::Downloads;
use crate::run::{LiveRun, Marks};
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
/// 「输入框」那块大输入区的高度。
const DROP_H: f32 = 132.0;
/// 「输入框」下面那行「清空」按钮（含它上面的缝）。
const DROP_ACTION_H: f32 = 30.0;
/// 模型还没下载时，卡片上那块「下载模型」面板的高度（流坐标）。
const MODEL_PANEL_H: f32 = 114.0;
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

/// 画布要展示的运行状态：正式报告 / 实时进度 / 要不要画帧率读数。
///
/// 报告要等整张图跑完才回来；在那之前靠 `live` 把「正在跑的那个」和已经完成的画出来。
pub struct RunView<'a> {
    pub report: Option<&'a RunReport>,
    pub live: Option<&'a LiveRun>,
    pub show_fps: bool,
}

/// 卡片底部的预览条高度（流坐标）。跑完有缩略图时卡片才多出这么高。
const PREVIEW_H: f32 = 84.0;

/// 卡片底部的色板条高度（流坐标）。输出是色板时卡片才多出这么高。
const PALETTE_H: f32 = 30.0;

/// 预览的棋盘格边长（流坐标）。
const CHECKER: f32 = 8.0;

/// 运行痕迹各段的高度（流坐标）。
const RUN_ACTIONS_H: f32 = 30.0;
/// 运行「提示」那一块：上下留白、图标的大小、图标与文字之间的缝、两条提示之间的缝。
const NOTICE_PAD: f32 = 8.0;
const NOTICE_ICON: f32 = 12.0;
const NOTICE_GAP: f32 = 6.0;
const NOTICE_STACK: f32 = 5.0;

/// 提示文字能用的宽度（流坐标）—— 减掉图标那一截。
fn notice_text_width() -> f32 {
    NODE_W - 20.0 - NOTICE_ICON - NOTICE_GAP
}

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
    /// 跑出来是个色板（「色彩分析」）时，卡片底下摆一排小色块。**不落盘**。
    pub palette: Option<Vec<[u8; 4]>>,
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
    /// 节点缩略图（按节点 id）。开始新的一次运行 / 清空运行记录时整批丢掉重建。
    textures: HashMap<String, egui::TextureHandle>,
    /// 点了卡片上的「运行至此」：这个节点 id 要交给调用方去跑。
    pending_run: Option<String>,
    /// 刚落到画布上的节点（id → 落下的时刻），用来放入场动画。
    entering: HashMap<String, f64>,
    /// Ctrl+C 复制的节点。
    clipboard: Option<Node>,
    /// Ctrl+S：交给外壳去落盘。
    save_requested: bool,
    /// 参数说明 / 运行提示的**折行结果**（流坐标），键是原文。
    ///
    /// 折行在缩放前就定死，画的时候只按 `zoom` 缩放 —— 否则每换一个字号就要
    /// 重新折行，某些缩放下会多出一行，文字于是一跳一跳的。同时卡片高度也照着
    /// 这个高度算，长说明不会把卡片撑出下边框。
    notes: HashMap<String, TextBlock>,
    /// 「输入框」节点里那张图（按节点 id）。存的路径没变就不重新解码。
    drop_textures: HashMap<String, (String, egui::TextureHandle)>,
    /// 正在「打字」的「输入框」节点（按 id）。空框点一下才进这个状态。
    drop_editing: HashSet<String>,
    /// 点过空白处：把上一次运行的高亮收起来（下一次运行再亮回来）。
    run_marks_hidden: bool,
    /// 右下角帧率读数（指数平滑后的 fps）。
    fps: f32,
    /// 上一帧的时刻，用来算帧间隔。
    last_frame: Option<f64>,
    /// 需要模型的节点正在下的那些模型（按节点 id）。**不落盘**。
    downloads: Downloads,
}

/// 一段折好行的文字：每行文本、每行高度、总高。都在流坐标下（未乘缩放）。
#[derive(Clone)]
struct TextBlock {
    lines: Vec<String>,
    rows: Vec<f32>,
    height: f32,
}

/// 说明 / 提示用的字号（流坐标）。
const NOTE_FONT: f32 = 9.5;

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
/// 参数一改就调 —— 端口类型可能跟着参数走（「输入」选的文件、「图像格式转换」的目标格式、
/// 「图像压缩」的模式）。
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
                palette: None,
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
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
            notes: HashMap::new(),
            drop_textures: HashMap::new(),
            drop_editing: HashSet::new(),
            run_marks_hidden: false,
            fps: 0.0,
            last_frame: None,
            downloads: Downloads::default(),
        }
    }

    /// 从节点库拖出来的节点落在哪儿：`screen` 是松手时的屏幕位置。`now` 用来做入场动画。
    pub fn add_node_at(&mut self, screen: Pos2, kind: &Kind, now: f64) {
        let height = height_of(kind, &kind.defaults, &self.notes);
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
            palette: None,
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
                palette: None,
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
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
            notes: HashMap::new(),
            drop_textures: HashMap::new(),
            drop_editing: HashSet::new(),
            run_marks_hidden: false,
            fps: 0.0,
            last_frame: None,
            downloads: Downloads::default(),
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
        let node = &self.nodes[i];
        // 只数**节点自己的**输入：参数端口画在参数那一行，不占端口列。
        let declared = node.inputs.iter().filter(|port| !port.is_param()).count();
        declared.max(node.outputs.len()).max(1)
    }

    fn flow_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        Rect::from_min_size(
            self.nodes[i].pos,
            egui::vec2(NODE_W, height_of_node(kinds, &self.nodes[i], &self.notes)),
        )
    }

    fn screen_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        let r = self.flow_rect(kinds, i);
        Rect::from_min_max(self.to_screen(r.min), self.to_screen(r.max))
    }

    /// 第 `k` 个输入端口（流坐标）。
    ///
    /// 参数端口不在端口列里 —— 它跟着那个参数的行走（浅蓝的小圆点）。
    fn in_port(&self, kinds: &[Kind], i: usize, k: usize) -> Pos2 {
        let r = self.flow_rect(kinds, i);
        let param_id = self.nodes[i]
            .inputs
            .get(k)
            .and_then(|port| port.param.as_deref());
        match param_id.and_then(|id| self.param_port_offset(kinds, i, id)) {
            Some(offset) => egui::pos2(r.left(), r.top() + offset),
            None => egui::pos2(r.left(), r.top() + HEADER_H + PORT_ROW_H * (k as f32 + 0.5)),
        }
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

    /// 某个参数行上那个小圆点相对卡片顶部的高度（流坐标）。
    ///
    /// 参数不可见、或者这个参数根本没有端口时就返回 `None`。走法必须和
    /// [`Self::draw_node_controls`] 的排版完全一致，否则圆点会飘到别的行上。
    fn param_port_offset(&self, kinds: &[Kind], i: usize, param_id: &str) -> Option<f32> {
        let node = &self.nodes[i];
        let kind = kind_of(kinds, node)?;
        let mut y = self.params_top(i);
        for param in &kind.params {
            if !param.visible(&node.params) {
                continue;
            }
            // 开关是「标签 + 开关」一行，圆点就对在这一行正中。
            if matches!(param.control, crate::catalog::Control::Bool) {
                if param.id == param_id {
                    return Some(y + PARAM_BOOL_H * 0.5);
                }
                y += PARAM_BOOL_H + PARAM_GAP;
                continue;
            }
            // 其余是「标签一行 + 控件一行」—— 圆点对在标签那一行（没标签就对控件）。
            if param.id == param_id {
                let line = if has_label(param) {
                    PARAM_LABEL_H * 0.5
                } else {
                    control_height(param, &node.params) * 0.5
                };
                return Some(y + line);
            }
            y += param_block_height(param, &node.params, &self.notes) + PARAM_GAP;
        }
        None
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
        view: RunView<'_>,
    ) -> Option<String> {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let now = ui.input(|input| input.time);
        self.measure_fps(now);
        // 收一收后台下载的进度 —— 下完了这项就消失，节点随之自动恢复。
        self.downloads.poll();

        // 先把这次的缩略图准备好 —— 卡片高度、预览绘制都要用到。
        self.sync_previews(ui.ctx(), view.report, view.live);
        // 参数说明与运行提示先折好行、量好高 —— 卡片高度靠它，折行也靠它。
        self.sync_notes(ui, kinds);

        // 文件拖入 / Ctrl+V 粘贴：先收下来，这一帧就能看到结果。
        if self.handle_drop_input(ui.ctx(), kinds) {
            self.touch();
        }

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
        // 全部拼进**一个 Mesh**：几千个 `circle_filled` 会让 epaint 每帧重新三角化
        // 几千个形状，平移时位置又全变、缓存也命中不了 —— 用一个网格一次画出去，
        // 图形一模一样，每帧的开销则从「几千个形状」降到「一个」+ 拼顶点的循环。
        let step = DOT_GAP * self.zoom;
        if step > 5.0 {
            let texture = dot_texture(ui.ctx());
            let mut mesh = egui::Mesh::with_texture(texture.id());
            let ox = self.pan.x.rem_euclid(step);
            let oy = self.pan.y.rem_euclid(step);
            let mut y = rect.top() - step + oy;
            while y < rect.bottom() + step {
                let mut x = rect.left() - step + ox;
                while x < rect.right() + step {
                    push_dot(&mut mesh, egui::pos2(x, y), 1.0, theme::CANVAS_DOT);
                    x += step;
                }
                y += step;
            }
            painter.add(egui::Shape::mesh(mesh));
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
                if self.draw_node_controls(ui, kinds, i, now) {
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

        // ---- 右下角：帧率读数 ----
        if view.show_fps {
            self.draw_fps(ui);
        }

        self.pending_run.take()
    }

    /// 记一帧的时间，算出平滑后的 fps。
    ///
    /// 空闲一段时间后的第一帧间隔会很大（甚至几秒），那种不算 —— 否则一恢复
    /// 交互，读数会被那一帧拖到个位数。
    fn measure_fps(&mut self, now: f64) {
        if let Some(last) = self.last_frame {
            let delta = now - last;
            if delta > 0.0 && delta <= 0.5 {
                let instant = (1.0 / delta) as f32;
                self.fps = if self.fps <= 0.0 {
                    instant
                } else {
                    self.fps * 0.9 + instant * 0.1
                };
            }
        }
        self.last_frame = Some(now);
    }

    /// 右下角一个灰色的帧率读数，保留到整数位。
    ///
    /// 数字和 `fps` 都自己排版：数字在一个固定宽度里**右对齐**，紧跟着单位 ——
    /// 位数变化（60 → 100）时数字从右边长出去，`fps` 一动不动，才不会左右抖。
    fn draw_fps(&self, ui: &egui::Ui) {
        if self.fps <= 0.0 {
            return;
        }
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("fps"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -8.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(&ctx, |ui| {
                let font = FontId::monospace(11.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 15.0), Sense::hover());
                let painter = ui.painter();
                let unit = painter.layout_no_wrap("fps".to_string(), font.clone(), theme::INK_3);
                let unit_x = rect.right() - unit.size().x;
                painter.galley(
                    egui::pos2(unit_x, rect.center().y - unit.size().y * 0.5),
                    unit.clone(),
                    theme::INK_3,
                );
                let number = painter.layout_no_wrap(
                    format!("{}", self.fps.round() as i64),
                    font,
                    theme::INK_3,
                );
                painter.galley(
                    egui::pos2(
                        unit_x - 5.0 - number.size().x,
                        rect.center().y - number.size().y * 0.5,
                    ),
                    number,
                    theme::INK_3,
                );
            });
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
        self.zoom = (scale * 0.75).clamp(0.2, 1.05);
        self.pan = canvas.center().to_vec2() - bounds.center().to_vec2() * self.zoom;
    }

    /// 有没有还在跑的动画，决定要不要继续申请重绘。
    pub fn is_animating(&self) -> bool {
        self.slash.is_some()
            || !self.severing.is_empty()
            || self.connect.is_some()
            || !self.entering.is_empty()
            || self.downloads.any()
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
                        // 要模型但本地还没有：这个节点还不能跑 —— 点运行不生效。
                        if kind_of(kinds, &self.nodes[i])
                            .is_some_and(|kind| kind.model_missing(&self.nodes[i].params))
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
                let hit = pointer.and_then(|p| self.hit(kinds, self.to_flow(p)));
                if hit.is_none() {
                    // 点空白处：把上一次运行的高亮收起来（下一次运行再亮回来）。
                    if !self.run_marks_hidden {
                        self.run_marks_hidden = true;
                        ui.ctx().request_repaint();
                    }
                }
                self.selected = hit.map(|index| self.bring_to_front(index));
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

    /// 端口现在的放大倍数。
    ///
    /// **拉线的起点、它可能落下的落点、以及指针悬着的那个**都算「被强调」—— 两端
    /// （起点 / 落点）因此永远一样大。只看 `hovered_port` 是不够的：落点靠的是
    /// `connect.target`，两者偶尔会错开一帧，端点就会一大一小。
    fn port_scale(&self, port: PortRef) -> f32 {
        let emphasized = self.hovered_port == Some(port)
            || self
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

    /// 点过空白处之后，上一次运行的高亮该收起来了。外壳据此把 `Marks` 里那几个
    /// 「跑过了」的集合清掉。
    pub fn run_marks_hidden(&self) -> bool {
        self.run_marks_hidden
    }

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
    fn sync_notes(&mut self, ui: &egui::Ui, kinds: &[Kind]) {
        let painter = ui.painter();
        for kind in kinds {
            for param in &kind.params {
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

    /// 把这次运行的缩略图传成纹理、并把运行痕迹贴到各节点上。
    ///
    /// 报告一换（`finished_at` 变了）就把旧纹理全丢掉重建 —— `TextureHandle`
    /// 一 drop，纹理由 egui 回收，不会越攒越多。
    fn sync_previews(
        &mut self,
        ctx: &egui::Context,
        report: Option<&RunReport>,
        live: Option<&LiveRun>,
    ) {
        // 运行过程中：每跑完一个节点就把它的结果贴上卡片（状态 / 缩略图 / 色板 / 产物）——
        // 图片于是跟着流程一张张出现，而不是等到最后一并显示。
        if let Some(live) = live {
            for result in live.done.values() {
                self.apply_result(ctx, result);
            }
        }
        // 报告到了：再整体对一遗（幂等 —— 缩略图只传一次，其余值一样）。
        if let Some(report) = report {
            for result in &report.nodes {
                self.apply_result(ctx, result);
            }
        }
    }

    /// 把一个节点的结果贴到它的卡片上：运行痕迹 + 色板 + 缩略图。
    ///
    /// **幂等**，所以运行中（实时结果）和跑完（正式报告）可以都调它一遍 ——
    /// 缩略图只传一次（`textures` 里有了就跳过）。
    fn apply_result(&mut self, ctx: &egui::Context, result: &NodeRunResult) {
        let Some(index) = self.nodes.iter().position(|node| node.id == result.node_id) else {
            return;
        };

        let file = result.outputs.iter().find_map(|output| output.path.clone());
        // 输出是色板（hex 一行一个）的话，把颜色拆出来，卡片底下画一排小色块。
        let palette = result
            .outputs
            .iter()
            .find_map(|output| output.palette.as_ref())
            .map(|colors| colors.iter().map(|color| color_rgba(color)).collect());
        self.nodes[index].palette = palette;
        self.nodes[index].run = Some(NodeRun {
            status: result.status,
            ms: result.elapsed_ms,
            error: result.error.clone(),
            warnings: result.warnings.clone(),
            file,
        });

        // 缩略图一个节点只传一次；「输入框」自己就把值摆出来了，不用再摆一条。
        if self.textures.contains_key(&result.node_id)
            || crate::catalog::has_drop_zone(&self.nodes[index].kind)
        {
            return;
        }
        let Some(url) = result
            .outputs
            .iter()
            .find_map(|output| output.preview.as_deref())
        else {
            return;
        };
        let Some((width, height, rgba)) = starrytools_core::image_io::decode_preview_data_url(url)
        else {
            return;
        };
        let image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let texture = ctx.load_texture(
            format!("preview:{}", result.node_id),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.textures.insert(result.node_id.clone(), texture);
        self.nodes[index].preview = true;
    }

    /// 清掉上一次运行留在画布上的痕迹：缩略图 / 色板 / 状态 / 高亮。
    ///
    /// 开始新的一次运行、或用户清空运行记录时调。
    pub fn reset_run_marks(&mut self) {
        self.textures.clear();
        for node in &mut self.nodes {
            node.preview = false;
            node.palette = None;
            node.run = None;
        }
        // 高亮亮回来（点过空白处那下作废）。
        self.run_marks_hidden = false;
    }

    /// 卡片底部的预览条：先铺棋盘格（透出底下的透明像素），再把图像按比例放进去。
    ///
    /// `bg` 是这块预览**周围**的背景色 —— 圆角外面要补回它，见 [`mask_corners`]。
    fn draw_preview(
        &self,
        painter: &egui::Painter,
        inner: Rect,
        texture: &egui::TextureHandle,
        bg: Color32,
    ) {
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

        // 棋盘格和图片都是直角矩形，会盖到圆角外面 —— 不补的话四角会露出灰角。
        mask_corners(painter, inner, theme::R_CTL as f32, bg);

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
        // 自由端（还没吸附到端口）自己一明一灭。吸附上之后不另画点 ——
        // 落点那一端由端口自己的强调动画负责（`port_halo`），两端才完全一致。
        if !snapped {
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

        // 边线颜色说几件事：跑失败了、跑成功了、还是平常；紫色则说明这是一个
        // 「会拦住运行等你动手」的节点（运行到它时会额外搏动高亮）。
        // **边线放到最后画** —— 里面的各段底色是铺满整宽的，先画会被它们盖住。
        let interactive = crate::catalog::is_interactive(&node.kind, &node.params);
        let waiting = marks.waiting.as_deref() == Some(node.id.as_str());
        // 「正在跑的那个」—— 运行中牌子的蓝色边框。
        let running = marks.running.as_deref() == Some(node.id.as_str());
        // 要模型但本地还没有：整个节点禁用，卡片上挂一个下载面板（见 `draw_node_controls`）。
        let model_missing =
            kind_of(kinds, node).is_some_and(|kind| kind.model_missing(&node.params));

        // 白底（投影已经在上一遍里铺好了）。
        painter.rect_filled(r, cr, theme::SURFACE);

        let border = if marks.failed_nodes.contains(&node.id) {
            theme::DANGER
        } else if running {
            theme::ACCENT
        } else if model_missing {
            theme::WARN
        } else if interactive {
            theme::PURPLE
        } else if marks.ok_nodes.contains(&node.id) {
            theme::ACCENT
        } else {
            theme::HAIRLINE
        };
        // 正在等用户操作 / 正在跑：把边框加粗一档。
        let border_width = if waiting {
            3.0
        } else if running || interactive || model_missing {
            1.5
        } else {
            1.0
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

        if running {
            // 跑到它了：右边一个转圈的箭头 + 「运行中」，一眼看出光标在哪儿。
            let spinner = Rect::from_center_size(
                egui::pos2(right_edge - 6.0 * z, cy),
                egui::Vec2::splat(10.0 * z),
            );
            icons::loader(painter, spinner, theme::ACCENT, (now * 1.2) as f32 % 1.0);
            painter.text(
                egui::pos2(right_edge - 13.0 * z, cy),
                Align2::RIGHT_CENTER,
                "运行中",
                FontId::monospace(9.5 * z),
                theme::ACCENT,
            );
        } else if model_missing {
            // 要模型但本地没有：右边一个「需模型」，提示这个节点还不能跑。
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                "需模型",
                FontId::monospace(9.5 * z),
                theme::WARN,
            );
        } else if let Some(run) = &node.run {
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
        let (warn_h, error_h, actions_h) = run_extra(node, &self.notes);
        // 这一段是不是最后一段（模型面板 / 缩略图 / 提示……）—— 不是的话底角不收圆。
        let panel_h = model_panel_height(kinds, node);
        let params_last = !params_following(kinds, node, &self.notes);
        let params_bottom =
            r.top() + (node_body_top(node) + params_height_of(kinds, node, &self.notes)) * z;
        if let Some(kind) = kind_of(kinds, node) {
            let ph = params_height(kind, &node.params, &self.notes);
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

        // ---- 模型下载面板的底色（紧贴参数区，控件由 `draw_node_controls` 画） ----
        if panel_h > 0.0 {
            let rect = Rect::from_min_size(
                egui::pos2(r.left(), params_bottom),
                egui::vec2(r.width(), panel_h * z),
            );
            painter.rect_filled(
                rect,
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: theme::R_CARD,
                    se: theme::R_CARD,
                },
                theme::WARN_SOFT,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left(), params_bottom),
                    egui::pos2(rect.right(), params_bottom),
                ],
                Stroke::new(1.0, theme::WARN_LINE),
            );
        }

        // ---- 参数区以下的几段 ----
        let body_pad = if panel_h > 0.0 {
            0.0
        } else if params_following(kinds, node, &self.notes) {
            BODY_PAD
        } else {
            0.0
        };
        let mut y = params_bottom + (panel_h + body_pad) * z;

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
                self.draw_preview(painter, inner, texture, theme::SURFACE);
            }
            y += PREVIEW_H * z;
        }

        // ---- 色板：一排小色块 ----
        if let Some(palette) = &node.palette {
            if !palette.is_empty() {
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
                draw_palette(
                    painter,
                    Rect::from_min_size(
                        egui::pos2(r.left() + 10.0 * z, y + 8.0 * z),
                        egui::vec2(r.width() - 20.0 * z, PALETTE_H * z - 16.0 * z),
                    ),
                    palette,
                    z,
                );
            }
            y += PALETTE_H * z;
        }

        // ---- 运行提示 ----
        // 不再是一排橙色的「!」，而是一块淡蓝底 + 信息图标 + 会折行的正文：
        // 读完是「知道发生了什么」而不是「出事了」。
        if warn_h > 0.0 {
            if let Some(run) = &node.run {
                let last = error_h == 0.0 && actions_h == 0.0;
                let panel =
                    Rect::from_min_size(egui::pos2(r.left(), y), egui::vec2(r.width(), warn_h * z));
                painter.rect_filled(
                    panel,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: if last { theme::R_CARD } else { 0 },
                        se: if last { theme::R_CARD } else { 0 },
                    },
                    theme::ACCENT_SOFT,
                );
                let mut ny = y + NOTICE_PAD * z;
                for warning in &run.warnings {
                    let icon = Rect::from_min_size(
                        egui::pos2(r.left() + 10.0 * z, ny),
                        egui::Vec2::splat(NOTICE_ICON * z),
                    );
                    icons::info(&body, icon, theme::ACCENT);
                    if let Some(block) = self.notes.get(warning) {
                        draw_block(
                            &body,
                            block,
                            z,
                            icon.right() + NOTICE_GAP * z,
                            ny,
                            theme::INK_2,
                        );
                    }
                    ny += (notice_height(warning, &self.notes) + NOTICE_STACK) * z;
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
                    // 错误也是会折行的 —— 同样按定死的折行画，不靠定值。
                    if let Some(block) = self.notes.get(error) {
                        draw_block(
                            &body,
                            block,
                            z,
                            r.left() + 10.0 * z,
                            y + NOTICE_PAD * z,
                            theme::DANGER,
                        );
                    }
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
        painter.rect_stroke(r, cr, Stroke::new(border_width, border), StrokeKind::Inside);
        if self.selected == Some(i) {
            painter.rect_stroke(
                r.expand(2.5),
                CornerRadius::same(theme::R_CARD + 2),
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
        }

        // ---- 端口最后画：压在边线上面 ----
        self.draw_ports(painter, kinds, marks, i, r, z, pointer, now);
    }

    /// 画一个节点所有端口。**最后画** —— 端口要压住卡片边线，不能被它切一刀。
    #[allow(clippy::too_many_arguments)]
    fn draw_ports(
        &self,
        painter: &egui::Painter,
        kinds: &[Kind],
        marks: &Marks,
        i: usize,
        r: Rect,
        z: f32,
        pointer: Option<Pos2>,
        now: f64,
    ) {
        let node = &self.nodes[i];
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
        // 出错的（非法连线 / 必填未接）→ 红；参数端口是浅蓝（可选）；
        // 其余端点一律主题蓝。
        let bad = self.port_invalid(marks, i, &port.id, input) || (port.required && !linked);
        let dot = if bad {
            theme::DANGER
        } else if port.is_param() {
            theme::ACCENT_LIGHT
        } else {
            theme::ACCENT
        };
        let type_ink = theme::badge_color(port.ty);

        // 拉线的**起点**和它可能落下的**落点** —— 两端都用同一套「正要连上」的动画
        // （同一个 `port_pending`），看起来才是一件东西。
        let is_source = self
            .connect
            .as_ref()
            .is_some_and(|connect| connect.from == here);
        let hovering = self.hovered_port == Some(here);
        let valid = self.is_valid_target(here);

        // 端点「被强调」时统一的一套观感 —— 拉线的**起点**、它可能落下的**落点**、
        // 以及只是被悬停的端点，圈的大小都一样，只有「正要连上」多一圈搏动的描边。
        let pending = is_source || valid;
        if pending || hovering {
            port_halo(painter, p, z, now, pending);
        }

        // 圆点：白圈里一个蓝点（对应 Handle 的 2px surface 描边）。
        let scale = self.port_scale(here);
        painter.circle_filled(p, 5.0 * z * scale, Color32::WHITE);
        painter.circle_filled(p, 3.0 * z * scale, dot);
        if bad {
            painter.circle_stroke(p, 6.0 * z * scale, Stroke::new(1.0, theme::DANGER));
        }

        // 参数端口就到这里：它的徽标和标签由参数那一行自己画（见
        // `draw_node_controls`），端口列里只出一个小圆点。
        if port.is_param() {
            return;
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
    fn draw_node_controls(
        &mut self,
        ui: &mut egui::Ui,
        kinds: &[Kind],
        i: usize,
        now: f64,
    ) -> bool {
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
        // 要模型但本地还没下载：整个节点禁用，参数全部灰下去 —— **只有那个「模型」
        // 下拉框还留着**，因为用户正是靠它挑要下哪个模型。
        let model_missing = kind.model_missing(&self.nodes[i].params);
        if showing.is_empty() && !model_missing {
            return false;
        }

        let card = self.screen_rect(kinds, i);
        let mut y = card.top() + self.params_top(i) * zoom;
        let mut changed = false;

        for k in showing {
            let param = &kind.params[k];
            let left = card.left() + 10.0 * zoom;
            let width = card.width() - 20.0 * zoom;
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
            // 缺模型时：除「模型」参数外全部禁用。
            let is_model_param = kind.model_param.as_deref() == Some(param.id.as_str());
            let disabled = linked || (model_missing && !is_model_param);

            // 开关：标签和开关同一行。
            if matches!(param.control, Control::Bool) {
                let row = Rect::from_min_size(
                    egui::pos2(left, y),
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
                    egui::pos2(row.right() - 30.0 * zoom, row.top()),
                    egui::vec2(30.0 * zoom, row.height()),
                );
                if let Some(value) =
                    control(ui, i, param, switch, &self.nodes[i].params, zoom, disabled)
                {
                    self.nodes[i].params.insert(param.id.clone(), value);
                    refresh_ports(&mut self.nodes[i]);
                    changed = true;
                }
                y += (PARAM_BOOL_H + PARAM_GAP) * zoom;
                continue;
            }

            // 标签一行（没有标签就整行省掉）。
            if has_label(param) {
                let line = PARAM_LABEL_H * zoom;
                let mut label_x = left;
                if let Some(port) = &port {
                    label_x += type_chip(ui.painter(), port, left, y + line * 0.5, zoom);
                }
                ui.painter().text(
                    egui::pos2(label_x, y),
                    Align2::LEFT_TOP,
                    &param.label,
                    FontId::monospace(10.0 * zoom),
                    theme::INK_3,
                );
                y += (PARAM_LABEL_H + PARAM_LABEL_GAP) * zoom;
            }

            // 控件一行。
            let height = control_height(param, &self.nodes[i].params) * zoom;
            let control_rect = Rect::from_min_size(egui::pos2(left, y), egui::vec2(width, height));
            if matches!(param.control, Control::DropZone) {
                // 「输入框」是一块自定义的大输入区（拖文件 / 粘贴 / 打字）。
                if self.draw_drop_zone(ui, i, &param.id, control_rect, zoom) {
                    changed = true;
                }
            } else if let Some(value) = control(
                ui,
                i,
                param,
                control_rect,
                &self.nodes[i].params,
                zoom,
                disabled,
            ) {
                self.nodes[i].params.insert(param.id.clone(), value);
                refresh_ports(&mut self.nodes[i]);
                changed = true;
            }
            y += height;

            // 说明一行（自动折行）。
            if let Some(note) = &param.description {
                y += PARAM_NOTE_GAP * zoom;
                if let Some(block) = self.notes.get(note) {
                    draw_block(ui.painter(), block, zoom, left, y, theme::INK_3);
                }
                y += note_height(note, &self.notes) * zoom;
            }

            y += PARAM_GAP * zoom;
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
        }

        changed
    }

    /// 画「模型还没下载」那块面板：模型名 + 大小 + 一句说明 + 源下拉 + 下载 / 进度 / 重试。
    ///
    /// 面板底色由 [`Self::draw_node`] 铺在参数区下面，这里只画内容。
    fn draw_model_panel(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        kind: &Kind,
        rect: Rect,
        zoom: f32,
        now: f64,
    ) {
        let node_id = self.nodes[i].id.clone();
        let Some(model) = kind.model(&self.nodes[i].params) else {
            return;
        };

        let pad = 10.0 * zoom;
        let left = rect.left() + pad;
        let width = rect.width() - pad * 2.0;
        let top = rect.top() + pad;
        let note_y = top + 16.0 * zoom;
        let select_y = top + 34.0 * zoom;
        let select_h = 24.0 * zoom;
        let action_h = 28.0 * zoom;

        // 当前状态：正在下 / 失败 / 待下载。
        let busy = self.downloads.busy(&node_id);
        let failed = self
            .downloads
            .get(&node_id)
            .and_then(|task| task.failed.clone());
        let fraction = self
            .downloads
            .get(&node_id)
            .and_then(crate::models::Download::fraction);
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
        let options: Vec<crate::catalog::Choice> = model
            .sources
            .iter()
            .enumerate()
            .map(|(k, source)| crate::catalog::Choice {
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
                ui.id().with(("model-source", i)),
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
        let action = Rect::from_min_size(
            egui::pos2(left, rect.bottom() - pad - action_h),
            egui::vec2(width, action_h),
        );
        let resp = ui.interact(action, ui.id().with(("model-action", i)), Sense::click());
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
                Color32::WHITE,
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
                Color32::WHITE,
            );
            painter.galley(
                egui::pos2(
                    x + icon_box + gap,
                    crate::widgets::ink_top(&galley, action.center().y),
                ),
                galley,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.downloads.start(&node_id, model, source);
            }
        }
    }

    /// 文件拖入 / `Ctrl+V` 粘贴：统一收下来，交给一个「输入区」节点。返回有没有变。
    ///
    /// **不能按指针位置挑**：系统在拖放过程中根本不上报光标位置（X11 的 `XdndPosition`
    /// 不转成 `CursorMoved`，winit 的 Wayland 后端干脆没有文件拖放）。所以顺序是：
    /// 光标（如果恰好有效）落在哪个框里 → 选中的那个 → 唯一的那个。
    fn handle_drop_input(&mut self, ctx: &egui::Context, kinds: &[Kind]) -> bool {
        // 有东西聚焦（用户在打字）时，粘贴归那个文本框。
        let typing = ctx.memory(|memory| memory.focused()).is_some();

        let dropped: Option<String> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_string_lossy().to_string())
        });
        let pasted: Option<String> = if typing {
            None
        } else {
            ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            })
        };
        let Some(payload) = dropped.or(pasted) else {
            return false;
        };

        // 所有带「输入区」的节点：`(节点下标, 参数 id)`。
        let targets: Vec<(usize, String)> = (0..self.nodes.len())
            .filter_map(|i| {
                let param = kind_of(kinds, &self.nodes[i])?
                    .params
                    .iter()
                    .find(|param| matches!(param.control, Control::DropZone))?;
                Some((i, param.id.clone()))
            })
            .collect();
        if targets.is_empty() {
            return false;
        }

        let pos = ctx.input(|input| input.pointer.latest_pos());
        let chosen = pos
            .and_then(|p| {
                targets
                    .iter()
                    .find(|(i, _)| self.screen_rect(kinds, *i).contains(p))
                    .cloned()
            })
            .or_else(|| {
                targets
                    .iter()
                    .find(|(i, _)| self.selected == Some(*i))
                    .cloned()
            })
            .unwrap_or_else(|| targets[0].clone());
        let (i, param_id) = chosen;

        // 粘 / 拖进来的如果是一个真实存在的文件，就当文件；否则当文本。
        let single = payload.trim();
        if !single.contains('\n') && std::path::Path::new(single).is_file() {
            self.set_drop_file(i, &param_id, single);
        } else {
            self.set_drop_text(i, &param_id, &payload);
        }
        true
    }

    /// 「输入框」节点那块大输入区：把文件拖进来、`Ctrl+V` 粘贴，或者点一下直接打字。
    /// 返回内容有没有变。
    fn draw_drop_zone(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        param_id: &str,
        rect: Rect,
        zoom: f32,
    ) -> bool {
        let ctx = ui.ctx().clone();
        let node_id = self.nodes[i].id.clone();
        let interact_id = ui.id().with(("drop", i, param_id));
        let edit_id = interact_id.with("edit");
        let cr = CornerRadius::same(theme::R_CTL);

        // 控件区 = 上面一块框（固定 `DROP_H` 高）+ 下面一行「清空」按钮。
        let box_rect = Rect::from_min_size(rect.min, egui::vec2(rect.width(), DROP_H * zoom));
        let action_rect = Rect::from_min_max(egui::pos2(rect.left(), box_rect.bottom()), rect.max);
        let content = drop_content(&self.nodes[i].params, param_id);

        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let over = pointer.is_some_and(|p| box_rect.contains(p));
        let dragging_files = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let editing = self.drop_editing.contains(&node_id);
        // 悬停 / 拖文件：平滑过渡，而不是一下变色。拖文件时这个高亮不依赖指针位置
        // （拖放过程里系统根本不上报光标位置），见 `handle_drop_input`。
        let hot = ctx.animate_bool_with_time(interact_id.with("hot"), over || dragging_files, 0.16);
        let drag = ctx.animate_bool_with_time(interact_id.with("drag"), dragging_files, 0.14);
        // 内容出现时淡入。
        let appear = ctx.animate_bool_with_time(
            interact_id.with("appear"),
            !matches!(content, Drop::Empty) || editing,
            0.22,
        );

        let mut changed = false;
        // ---- 下方：清空按钮（只有有东西可清时才出现）----
        if !matches!(content, Drop::Empty) {
            let button = Rect::from_min_size(
                egui::pos2(action_rect.left(), action_rect.top() + 6.0 * zoom),
                egui::vec2(74.0 * zoom, 24.0 * zoom),
            );
            if clear_button(ui, interact_id.with("clear"), button, zoom) {
                self.set_drop_text(i, param_id, "");
                self.drop_editing.remove(&node_id);
                changed = true;
            }
        }

        let rect = box_rect;
        match &content {
            // ---- 装着一张图：直接显示 ----
            Drop::File(path) => {
                let texture = self.drop_texture(&ctx, &node_id, path);
                let mut painter = ui.painter_at(rect.expand(1.0));
                painter.set_opacity(appear);
                match &texture {
                    Some(texture) => self.draw_preview(&painter, rect, texture, theme::SURFACE_2),
                    None => {
                        painter.rect_filled(rect, cr, theme::SURFACE_2);
                        painter.rect_stroke(
                            rect,
                            cr,
                            Stroke::new(1.0, theme::HAIRLINE),
                            StrokeKind::Inside,
                        );
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "读不出这个文件",
                            FontId::monospace(10.0 * zoom),
                            theme::DANGER,
                        );
                    }
                }
            }
            // ---- 文本（或空框刚点进打字）：一块文本框 ----
            _ if matches!(&content, Drop::Text(_)) || editing => {
                let mut draft = match &content {
                    Drop::Text(text) => text.clone(),
                    _ => String::new(),
                };
                let inner = rect.shrink(9.0 * zoom);
                // **先把框画出来，再放文本框** —— 反了的话白底会把字盖住。
                {
                    let mut painter = ui.painter_at(rect.expand(1.0));
                    painter.set_opacity(appear);
                    input_shell(&painter, rect, theme::R_CTL);
                }
                let resp = ui.put(
                    inner,
                    egui::TextEdit::multiline(&mut draft)
                        .id(edit_id)
                        .frame(egui::Frame::NONE)
                        .font(FontId::monospace(11.5 * zoom))
                        .desired_width(inner.width())
                        .hint_text("在这里打字…"),
                );
                input_shell_state(
                    ui.painter(),
                    rect,
                    theme::R_CTL,
                    resp.hovered(),
                    resp.has_focus(),
                );
                if resp.changed() {
                    self.set_drop_text(i, param_id, &draft);
                    changed = true;
                } else if resp.lost_focus() {
                    // 没打字就点开了别处 —— 退回虚线框。
                    self.drop_editing.remove(&node_id);
                }
            }
            // ---- 空着：虚线大框 + 输入图标 + 提示 ----
            _ => {
                {
                    let mut painter = ui.painter_at(rect.expand(1.0));
                    painter.set_opacity(1.0 - appear);
                    // 悬停 / 拖上去都平滑变色。
                    let border = mix(theme::HAIRLINE_STRONG, theme::ACCENT, hot.max(drag));
                    let fill = mix(theme::SURFACE_2, theme::ACCENT_SOFT, drag);
                    painter.rect_filled(rect, cr, fill);
                    dashed_rect(&painter, rect, zoom, border);
                    let icon = Rect::from_center_size(
                        egui::pos2(rect.center().x, rect.center().y - 12.0 * zoom),
                        egui::Vec2::splat(24.0 * zoom),
                    );
                    icons::plus(&painter, icon, mix(theme::INK_3, theme::ACCENT, hot));
                    let hint = if can_drop_files() {
                        "拖入文件 · 点击输入 · Ctrl+V"
                    } else {
                        "点击输入 · Ctrl+V"
                    };
                    painter.text(
                        egui::pos2(rect.center().x, rect.center().y + 18.0 * zoom),
                        Align2::CENTER_CENTER,
                        hint,
                        FontId::monospace(9.5 * zoom),
                        mix(theme::INK_3, theme::ACCENT, hot),
                    );
                }
                let resp = ui.interact(rect, interact_id, Sense::click());
                if resp.clicked() {
                    self.drop_editing.insert(node_id.clone());
                    ctx.memory_mut(|memory| memory.request_focus(edit_id));
                }
            }
        }

        changed
    }

    /// 把一份文本写进输入区。
    fn set_drop_text(&mut self, i: usize, param_id: &str, text: &str) {
        self.nodes[i]
            .params
            .insert(param_id.to_string(), serde_json::json!(text));
        refresh_ports(&mut self.nodes[i]);
    }

    /// 把一个文件路径写进输入区。
    fn set_drop_file(&mut self, i: usize, param_id: &str, path: &str) {
        self.nodes[i]
            .params
            .insert(param_id.to_string(), serde_json::json!({ "file": path }));
        refresh_ports(&mut self.nodes[i]);
    }

    /// 输入区里那张图的纹理（按节点 id 缓存，路径没变就不重新解码）。
    fn drop_texture(
        &mut self,
        ctx: &egui::Context,
        node_id: &str,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some((cached, texture)) = self.drop_textures.get(node_id) {
            if cached == path {
                return Some(texture.clone());
            }
        }
        let bytes = std::fs::read(path).ok()?;
        let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (width, height) = image.dimensions();
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            image.as_raw(),
        );
        let texture = ctx.load_texture(
            format!("drop:{node_id}"),
            color,
            egui::TextureOptions::NEAREST,
        );
        self.drop_textures
            .insert(node_id.to_string(), (path.to_string(), texture.clone()));
        Some(texture)
    }
}

/// 两个颜色之间按 `t` 线性混合（0 = 全 `a`，1 = 全 `b`）。渐变用。
/// 背景点阵用的小圆贴图：一张 `SIDE×SIDE` 的白色圆，带一点抗锯齿。
///
/// 按 `(图标, 尺寸)` 的思路缓存进 `ctx.data`，只建一次。
fn dot_texture(ctx: &egui::Context) -> egui::TextureHandle {
    const SIDE: usize = 16;
    let id = egui::Id::new("starrytools-canvas-dot");
    if let Some(handle) = ctx.data_mut(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let center = (SIDE as f32 - 1.0) / 2.0;
    // 圆几乎顶满贴图，这样贴到屏幕上的实际半径才和原来 `circle_filled(1.0)` 接近。
    let radius = SIDE as f32 / 2.0 - 0.5;
    let mut pixels = Vec::with_capacity(SIDE * SIDE);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            pixels.push(Color32::from_white_alpha((coverage * 255.0) as u8));
        }
    }
    let handle = ctx.load_texture(
        "canvas-dot",
        egui::ColorImage::new([SIDE, SIDE], pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
    handle
}

/// 往网格里塞一颗小圆（用一个采样小圆贴图的四边形）。
fn push_dot(mesh: &mut egui::Mesh, center: Pos2, radius: f32, color: Color32) {
    let base = mesh.vertices.len() as u32;
    let half = egui::vec2(radius, radius);
    let (min, max) = (center - half, center + half);
    let vertex = |pos: Pos2, uv: Pos2| egui::epaint::Vertex { pos, uv, color };
    mesh.vertices.push(vertex(min, egui::pos2(0.0, 0.0)));
    mesh.vertices
        .push(vertex(egui::pos2(max.x, min.y), egui::pos2(1.0, 0.0)));
    mesh.vertices.push(vertex(max, egui::pos2(1.0, 1.0)));
    mesh.vertices
        .push(vertex(egui::pos2(min.x, max.y), egui::pos2(0.0, 1.0)));
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// 一排小色块，展示一个色板。色块太多就只画能放下的那些。
fn draw_palette(painter: &egui::Painter, band: Rect, palette: &[[u8; 4]], z: f32) {
    if palette.is_empty() || band.width() <= 0.0 || band.height() <= 0.0 {
        return;
    }
    let gap = 3.0 * z;
    let min_w = 4.0 * z;
    let max_w = 22.0 * z;
    let radius = CornerRadius::same((2.0 * z).round().max(1.0) as u8);
    let count = palette.len();

    // 放得下就一个不落；放不下就按能放的数量截断，别挤成一团。
    let width = (band.width() - gap * (count as f32 - 1.0)) / count as f32;
    let (shown, width) = if width >= min_w {
        (count, width.min(max_w))
    } else {
        let fits = (((band.width() + gap) / (min_w + gap)).floor() as usize).clamp(1, count);
        (
            fits,
            (band.width() - gap * (fits as f32 - 1.0)) / fits as f32,
        )
    };

    let mut x = band.left();
    for color in palette.iter().take(shown) {
        let cell = Rect::from_min_size(egui::pos2(x, band.top()), egui::vec2(width, band.height()));
        painter.rect_filled(
            cell,
            radius,
            Color32::from_rgb(color[0], color[1], color[2]),
        );
        painter.rect_stroke(
            cell,
            radius,
            Stroke::new(1.0, theme::HAIRLINE),
            StrokeKind::Inside,
        );
        x += width + gap;
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// 这个会话能不能收文件拖放。
///
/// winit 在设置了 `WAYLAND_DISPLAY` 时会选 Wayland 后端，而它的 **Wayland 后端不实现
/// 文件拖放** —— 拖文件过来不会有任何事件。这种会话里就不提「拖入」了。
fn can_drop_files() -> bool {
    static CAN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CAN.get_or_init(|| std::env::var("WAYLAND_DISPLAY").map_or(true, |value| value.is_empty()))
}

/// 「输入区」下面那个「清空」按钮。返回是否被点了。
fn clear_button(ui: &mut egui::Ui, id: egui::Id, rect: Rect, zoom: f32) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let cr = CornerRadius::same(theme::R_CTL);
    let hot = resp.hovered();
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        cr,
        if hot {
            theme::SURFACE_3
        } else {
            theme::SURFACE
        },
    );
    painter.rect_stroke(
        rect,
        cr,
        Stroke::new(
            1.0,
            if hot {
                theme::HAIRLINE_STRONG
            } else {
                theme::HAIRLINE
            },
        ),
        StrokeKind::Inside,
    );
    let ink = if hot { theme::INK } else { theme::INK_2 };
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + 16.0 * zoom, rect.center().y),
        egui::Vec2::splat(12.0 * zoom),
    );
    icons::trash(painter, icon, ink);
    painter.text(
        egui::pos2(icon.right() + 6.0 * zoom, rect.center().y),
        Align2::LEFT_CENTER,
        "清空",
        FontId::monospace(10.5 * zoom),
        ink,
    );
    resp.clicked()
}

/// 「输入框」里现在装的是什么。
#[derive(Clone, PartialEq)]
enum Drop {
    Empty,
    Text(String),
    File(String),
}

fn drop_content(params: &Params, param_id: &str) -> Drop {
    match params.get(param_id) {
        Some(Value::String(text)) if !text.is_empty() => Drop::Text(text.clone()),
        Some(Value::Object(map)) => map
            .get("file")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| Drop::File(path.to_string()))
            .unwrap_or(Drop::Empty),
        _ => Drop::Empty,
    }
}

/// 一个节点的完整高度（流坐标）。
fn height_of_node(kinds: &[Kind], node: &Node, notes: &HashMap<String, TextBlock>) -> f32 {
    let (warn, error, actions) = run_extra(node, notes);
    let panel = model_panel_height(kinds, node);
    // 模型面板紧贴参数区，中间不留 `BODY_PAD` 那条缝。
    let gap = if panel > 0.0 {
        0.0
    } else if params_following(kinds, node, notes) {
        BODY_PAD
    } else {
        0.0
    };
    node_body_top(node)
        + params_height_of(kinds, node, notes)
        + panel
        + gap
        + if node.preview { PREVIEW_H } else { 0.0 }
        + if node
            .palette
            .as_ref()
            .is_some_and(|colors| !colors.is_empty())
        {
            PALETTE_H
        } else {
            0.0
        }
        + warn
        + error
        + actions
}

/// 模型「下载面板」占多高（不缺模型时是 0）。面板的控件在 `draw_node_controls` 里画。
fn model_panel_height(kinds: &[Kind], node: &Node) -> f32 {
    match kind_of(kinds, node) {
        Some(kind) if kind.model_missing(&node.params) => MODEL_PANEL_H,
        _ => 0.0,
    }
}

/// 参数区后面还有没有别的段（模型面板 / 缩略图 / 色板 / 提示 / 错误 / 产物行）。
/// 没有的话，参数区的圆底就贴到卡片底部，和卡片自己的圆角对齐。
fn params_following(kinds: &[Kind], node: &Node, notes: &HashMap<String, TextBlock>) -> bool {
    let (warn, error, actions) = run_extra(node, notes);
    model_panel_height(kinds, node) > 0.0
        || node.preview
        || node
            .palette
            .as_ref()
            .is_some_and(|colors| !colors.is_empty())
        || warn > 0.0
        || error > 0.0
        || actions > 0.0
}

/// 端口区下沿（相对卡片顶部，流坐标）—— 也就是参数区的上沿。
fn node_body_top(node: &Node) -> f32 {
    // 只数**声明**的输入端口：参数端口画在参数那一行，不占端口列，也就
    // 不撑高端口区 —— 否则灰底参数框会比里面的控件低一整行。
    let declared = node.inputs.iter().filter(|port| !port.is_param()).count();
    let rows = declared.max(node.outputs.len()).max(1) as f32;
    HEADER_H + rows * PORT_ROW_H
}

/// 运行痕迹那几段各占多高：`(提示, 错误, 产物操作行)`。
///
/// 提示 / 错误都会折行，所以高度按实测的折行高度算 —— 否则长提示会被卡片的
/// 下边框截掉（“图像压缩”运行后那行小结就是这么溢出的）。
fn run_extra(node: &Node, notes: &HashMap<String, TextBlock>) -> (f32, f32, f32) {
    let Some(run) = &node.run else {
        return (0.0, 0.0, 0.0);
    };
    let warn = if run.warnings.is_empty() {
        0.0
    } else {
        NOTICE_PAD * 2.0
            + run
                .warnings
                .iter()
                .map(|warning| notice_height(warning, notes))
                .sum::<f32>()
            + (run.warnings.len() - 1) as f32 * NOTICE_STACK
    };
    let error = match &run.error {
        Some(error) => NOTICE_PAD * 2.0 + note_height(error, notes),
        None => 0.0,
    };
    let actions = if run.file.is_some() {
        RUN_ACTIONS_H
    } else {
        0.0
    };
    (warn, error, actions)
}

/// 节点在**默认参数、还没跑过**时的高度（流坐标）—— 只用来算刚拖出来的节点落在哪。
/// 必须和 [`height_of_node`] 对得上：那边只有「参数区后面还有内容」时才多加 `BODY_PAD`，
/// 刚拖出来的节点没有运行痕迹，所以这里也不加。模型面板要算进去，否则落点会偏高。
fn height_of(kind: &Kind, params: &Params, notes: &HashMap<String, TextBlock>) -> f32 {
    let rows = kind.inputs.len().max(kind.outputs.len()).max(1) as f32;
    HEADER_H
        + rows * PORT_ROW_H
        + params_height(kind, params, notes)
        + if kind.model_missing(params) {
            MODEL_PANEL_H
        } else {
            0.0
        }
}

/// 参数区占多高。被 `visible_when` 藏掉的不占地方。
fn params_height(kind: &Kind, params: &Params, notes: &HashMap<String, TextBlock>) -> f32 {
    let showing: Vec<&Param> = kind.params.iter().filter(|p| p.visible(params)).collect();
    if showing.is_empty() {
        return 0.0;
    }
    let blocks: f32 = showing
        .iter()
        .map(|param| param_block_height(param, params, notes))
        .sum();
    let gaps = (showing.len() - 1) as f32 * PARAM_GAP;
    PARAMS_GAP + blocks + gaps + PARAMS_BOTTOM
}

/// 多行文本框要占多高：行数越多越高，长文本不会溢出下边框。
fn multiline_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1) as f32;
    (lines * PARAM_TEXT_LINE + 10.0).max(PARAM_TEXT_ROW_H)
}

/// 一段会折行的文字的**总高**（流坐标）。优先用实测值（`sync_notes` 量的）；
/// 还没量到（比如刚从节点库拖出来的那一帧）就退回一个粗略估计。
fn wrapped_height(text: &str, width: f32, notes: &HashMap<String, TextBlock>) -> f32 {
    notes
        .get(text)
        .map(|block| block.height)
        .unwrap_or_else(|| estimate_wrapped_height(text, width))
}

/// 一句参数说明占多高（整宽）。
fn note_height(note: &str, notes: &HashMap<String, TextBlock>) -> f32 {
    wrapped_height(note, NODE_W - 20.0, notes)
}

/// 一条运行提示占多高（要减掉前面那个图标）。
fn notice_height(note: &str, notes: &HashMap<String, TextBlock>) -> f32 {
    wrapped_height(note, notice_text_width(), notes)
}

/// 按固定字号、给定宽度折好行，量出行高与总高。
fn measure_block(painter: &egui::Painter, text: &str, width: f32, color: Color32) -> TextBlock {
    let galley = painter.layout(text.to_owned(), FontId::monospace(NOTE_FONT), color, width);
    TextBlock {
        lines: galley.rows.iter().map(|row| row.text()).collect(),
        rows: galley.rows.iter().map(|row| row.rect().height()).collect(),
        height: galley.size().y,
    }
}

/// 画一段折好的文字：逐行按 `zoom` 缩放，不再重新折行（位置与预留高度完全一致）。
fn draw_block(
    painter: &egui::Painter,
    block: &TextBlock,
    zoom: f32,
    x: f32,
    y: f32,
    color: Color32,
) {
    let mut yy = y;
    for (line, row) in block.lines.iter().zip(&block.rows) {
        let galley =
            painter.layout_no_wrap(line.clone(), FontId::monospace(NOTE_FONT * zoom), color);
        painter.galley(egui::pos2(x, yy), galley, color);
        yy += row * zoom;
    }
}

/// 粗略估计：汉字按约 9.6px、半角按约 5.7px 估宽，再按可用宽度折行。
fn estimate_wrapped_height(text: &str, width: f32) -> f32 {
    let mut lines = 1.0f32;
    let mut used = 0.0f32;
    for ch in text.chars() {
        let w = if ch.is_ascii() { 5.7 } else { 9.6 };
        if used > 0.0 && used + w > width {
            lines += 1.0;
            used = 0.0;
        }
        used += w;
    }
    // 行高取彮：与实测（9.5px 等宽约 13px 一行）对齐。
    lines * 13.0
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
        Control::Color => color_height(),
        Control::DropZone => {
            // 有内容时下面多一行「清空」按钮。
            if matches!(drop_content(params, &param.id), Drop::Empty) {
                DROP_H
            } else {
                DROP_H + DROP_ACTION_H
            }
        }
        _ => PARAM_CONTROL_H,
    }
}

/// 一个参数（标签 + 控件 + 说明）一共占多高。
fn param_block_height(param: &Param, params: &Params, notes: &HashMap<String, TextBlock>) -> f32 {
    let control = control_height(param, params);
    if matches!(param.control, Control::Bool) {
        return control.max(PARAM_LABEL_H);
    }
    // 没有标签就不占标签那一行（「来源」里的节点只有一个值，不写名字更简洁）。
    let mut height = control;
    if has_label(param) {
        height += PARAM_LABEL_H + PARAM_LABEL_GAP;
    }
    if let Some(note) = &param.description {
        height += PARAM_NOTE_GAP + note_height(note, notes);
    }
    height
}

/// 参数有没有名字。空名字表示这条参数不画标签行 —— 「来源」里那些「只有一个值」的参数
/// （字面量的值、输入框）就用它，省掉一行「值」。
fn has_label(param: &Param) -> bool {
    !param.label.is_empty()
}

fn params_height_of(kinds: &[Kind], node: &Node, notes: &HashMap<String, TextBlock>) -> f32 {
    match kind_of(kinds, node) {
        Some(kind) => params_height(kind, &node.params, notes),
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
    disabled: bool,
) -> Option<Value> {
    let id = ui.id().with(("param", node, param.id.as_str()));
    let current = params.get(&param.id);

    match &param.control {
        Control::Bool => {
            let mut value = current.and_then(Value::as_bool).unwrap_or(false);
            // 禁用态：开关还是开关，只是灰下去、不再响应 —— 而不是换成一个灰块。
            if disabled {
                toggle_disabled(ui.painter(), rect, value);
                return None;
            }
            widgets::switch(ui, id, rect, &mut value).then(|| serde_json::json!(value))
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
        ),

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
            .map(|next| serde_json::json!(next as f64))
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
        ),

        Control::Select { options } => select_field(ui, id, rect, current, options, zoom, disabled),

        Control::Color => color_field(ui, id, rect, current, zoom, disabled),

        Control::File { .. } => file_field(ui, id, rect, current, &param.control, zoom, disabled),

        // 输入区不走这里 —— 它由 `Graph::draw_drop_zone` 自己画。
        Control::DropZone => None,
    }
}

/// 端点被强调时的统一外观。拉线的起点、落点、被悬停的端点都调它 ——
/// **外圈半径始终一样**，所以从「悬停」到「起点 / 落点」不会有大小的跳变；
/// 区别只在「正要连上」（`pending`）时多一圈搏动的描边。要改这份动画，只改这里一处。
fn port_halo(painter: &egui::Painter, p: Pos2, z: f32, now: f64, pending: bool) {
    let (radius, alpha) = if pending {
        let pulse = 0.5 + 0.5 * (now * 6.0).sin() as f32;
        ((8.5 + pulse * 2.0) * z, 0.18)
    } else {
        (8.5 * z, 0.20)
    };
    painter.circle_filled(p, radius, theme::accent_alpha(alpha));
    if pending {
        painter.circle_stroke(p, 7.5 * z, Stroke::new(1.5, theme::ACCENT));
    }
}

/// 类型徽标的字号、左右内边距、后面那段间隔。
const CHIP_FONT: f32 = 8.5;
const CHIP_PAD_X: f32 = 4.0;
const CHIP_TRAIL: f32 = 5.0;

/// 类型徽标（chip）：标在参数名前，标出这个参数 / 这个端口的类型。
/// 颜色走 [`theme::badge_color`]，和端口列上的徽标同色。返回它连同后面那段间隔的宽度。
fn type_chip(painter: &egui::Painter, port: &Port, x: f32, center_y: f32, zoom: f32) -> f32 {
    let color = theme::badge_color(port.ty);
    let g = painter.layout_no_wrap(
        port.badge.clone(),
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
            crate::widgets::ink_top(&g, r.center().y),
        ),
        g,
        color,
    );
    w + CHIP_TRAIL * zoom
}

/// 禁用态的外壳：灰底 + 发丝边。参数被上游接管（或控件本身不可用时）用它，
/// 控件还是控件的样子，只是不再响应、颜色弱下去 —— 不再用统一灰块代替。
fn disabled_shell(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.rect_filled(rect, CornerRadius::same(radius), theme::SURFACE_2);
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );
}

/// 禁用态里那行读不切的字：左对齐，和可编辑时的内边距一致，剪在框里。
fn muted_value(painter: &egui::Painter, rect: Rect, text: &str, zoom: f32) {
    let inner = rect.shrink2(egui::vec2(8.0 * zoom, 0.0));
    painter.with_clip_rect(inner).text(
        egui::pos2(inner.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(11.5 * zoom),
        theme::INK_3,
    );
}

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
fn text_field(
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

/// 颜色控件（收起时）那条色条的高度（流坐标）。
const COLOR_BAR_H: f32 = PARAM_CONTROL_H;
/// 弹出的取色器宽度与内部各段高度（屏幕像素，不跟缩放走 —— 取色要看得清）。
const PICKER_W: f32 = 236.0;
const PICKER_SV_H: f32 = 132.0;
const PICKER_HUE_H: f32 = 16.0;

/// 颜色控件占多高（流坐标）—— 收起时就是一条色条。
fn color_height() -> f32 {
    COLOR_BAR_H
}

/// 颜色控件：收起时是一条显示当前颜色的色条，点开是一个**通用取色器**
/// —— 取色区（x = 饱和度，y = 亮度）、色相条、R/G/B、Hex、不透明度，双向同步。
///
/// 通用可复用：哪个节点声明一个 `Color` 参数就能用上它。
fn color_field(
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
fn color_picker(ui: &mut egui::Ui, id: egui::Id, bar: Rect, rgba: [u8; 4]) -> Option<Value> {
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
fn picker_row(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui, Rect)) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
        // 占下整行的位置，让父级按这一行的尺寸推进（而不是按内容拼出来的那块）。
        let _ = ui.allocate_rect(row, Sense::hover());
        add(ui, row);
    });
}

/// 取色器一行的左侧小标签（在行内坐标里，`offset` 是相对行左边的偏移）。
fn picker_label(ui: &egui::Ui, row: Rect, offset: f32, text: &str) {
    ui.painter().text(
        egui::pos2(row.left() + offset, row.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(10.0),
        theme::INK_3,
    );
}

/// 取色 / 拖动时指针落在控件上的那一点。
fn dragged_point(resp: &egui::Response) -> Option<Pos2> {
    (resp.dragged() || resp.clicked())
        .then(|| resp.interact_pointer_pos())
        .flatten()
}

/// 取色区：白色→色相 的横向渐变，叠上 透明→黑 的纵向渐变。
/// x 是饱和度、y 是亮度，所以左上角是白、右上角是纯色、底部是黑。
fn draw_sv_square(painter: &egui::Painter, rect: Rect, hue: f32) {
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
fn draw_hue_bar(painter: &egui::Painter, rect: Rect) {
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
fn push_quad(mesh: &mut egui::Mesh, rect: Rect, colors: [Color32; 4]) {
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

fn solid_vertex(pos: Pos2, color: Color32) -> egui::epaint::Vertex {
    egui::epaint::Vertex {
        pos,
        uv: egui::epaint::WHITE_UV,
        color,
    }
}

/// 把 `rect` 的四个圆角补成 `bg`：直角矩形（棋盘格、图片）会盖到圆角外面，
/// 四角于是露出灰角。这里在四个角各补一小块「方角减四分之一圆」的月亮形，
/// 把溢出圆角的那部分盖回背景色。用一小片顶点色网格拼出来（`radius` 是圆角半径）。
fn mask_corners(painter: &egui::Painter, rect: Rect, radius: f32, bg: Color32) {
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
fn paint_color_chip(
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
fn readable_ink(rgba: [u8; 4]) -> Color32 {
    let luma = 0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32;
    // 半透明时底下透出的是浅灰底，亮色或透明都用深色字。
    if rgba[3] < 128 || luma > 150.0 {
        theme::INK
    } else {
        Color32::WHITE
    }
}

/// 一个小的十六进制输入框（类似 `number_field`，但它收的是文本）。
fn hex_field(ui: &mut egui::Ui, id: egui::Id, rect: Rect, current: &str) -> Option<String> {
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

fn temp_bool(ctx: &egui::Context, id: egui::Id) -> Option<bool> {
    ctx.data_mut(|data| data.get_temp::<bool>(id))
}

fn temp_f32(ctx: &egui::Context, id: egui::Id) -> Option<f32> {
    ctx.data_mut(|data| data.get_temp::<f32>(id))
}

fn temp_rect(ctx: &egui::Context, id: egui::Id) -> Option<Rect> {
    ctx.data_mut(|data| data.get_temp::<Rect>(id))
}

/// RGB → HSV。h 是 0–360，s / v 是 0–1。
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
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
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
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
fn color_rgba(value: &str) -> [u8; 4] {
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
fn color_string(rgba: [u8; 4]) -> String {
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
fn select_field(
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

/// 禁用态的开关：同一个轨道和圆点，只是灰下去、不响应。
fn toggle_disabled(painter: &egui::Painter, rect: Rect, value: bool) {
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
fn file_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    current: Option<&Value>,
    control: &Control,
    zoom: f32,
    disabled: bool,
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

    /// 汉字说明不能按「字符数 ÷ 固定值」估 —— 那样一条 20 多字的说明会被估成一行，
    /// 卡片就不够高、把说明截出下边框（“图像压缩”的“元数据”就是这么溢出的）。
    #[test]
    fn a_cjk_note_is_estimated_as_multiple_lines() {
        let note = "ICC 这类影响颜色显示的默认留着，不当垃圾删。";
        assert!(
            estimate_wrapped_height(note, NODE_W - 20.0) >= 26.0,
            "汉字说明该被估成两行以上，实际 {}（{:?}）",
            estimate_wrapped_height(note, NODE_W - 20.0),
            note,
        );
        // 短的半角说明仍是一行。
        assert!(estimate_wrapped_height("越大越慢越小。", NODE_W - 20.0) < 26.0);
    }

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
            palette: None,
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
                palette: None,
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
            pending_run: None,
            entering: HashMap::new(),
            clipboard: None,
            save_requested: false,
            notes: HashMap::new(),
            drop_textures: HashMap::new(),
            drop_editing: HashSet::new(),
            run_marks_hidden: false,
            fps: 0.0,
            last_frame: None,
            downloads: Downloads::default(),
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
        let mut graph = canvas(&["read", "crop_image"]);
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
        let graph = canvas(&["read", "crop_image"]);
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
        let mut graph = canvas(&["read", "read", "crop_image"]);
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
        let graph = canvas(&["literal_text", "crop_image"]);
        assert_eq!(graph.nodes[0].outputs[0].badge, "TXT", "文本节点输出文本");
        assert!(
            !graph.can_link(out(0, 0), input(1, 0)),
            "文本不该接到图像输入上"
        );
    }

    // ---- 节点右键菜单的两件事 ----

    /// 删掉一个节点，接在它身上的连线要一起断，剩下连线的下标要跟着往前挪。
    #[test]
    fn deleting_a_node_drops_its_wires_and_reindexes_the_rest() {
        let mut graph = canvas(&["read", "read", "crop_image"]);
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

    // ---- 字面量节点与参数端口 ----

    /// 字面量节点是起点：只有输出，没有输入，也没有参数端口
    /// （它自己就是喂参数的那一端）。名字要能一眼分清。
    #[test]
    fn literal_nodes_are_sources_without_param_ports() {
        // 名字要能一眼分清 —— 「文本」和「数字」不能都叫一个含糊的名。
        for (id, name) in [
            ("literal_text", "文本"),
            ("literal_number", "数字"),
            ("literal_bool", "布尔"),
        ] {
            let kind = crate::catalog::all()
                .into_iter()
                .find(|kind| kind.id == id)
                .unwrap_or_else(|| panic!("该有「{id}」这个节点"));
            assert_eq!(kind.name, name, "{id} 的名字该是「{name}」");

            let (inputs, outputs) = crate::catalog::ports_for(id, &kind.defaults);
            assert!(
                inputs.is_empty(),
                "{id} 不该有输入端口（{} 个）",
                inputs.len()
            );
            assert_eq!(outputs.len(), 1, "{id} 该只有一个输出");
            // 输出端口的类型徽标让三种字面量一眼分得开。
            assert!(
                matches!(outputs[0].badge.as_str(), "TXT" | "NUM" | "BOOL"),
                "{id} 的输出该带一个类型徽标，实际是「{}」",
                outputs[0].badge
            );
        }
    }

    /// 普通节点里「能被上游喂」的参数自动多一个参数端口，并标好它属于哪个参数。
    /// 【关键】它不算进端口列 —— 端口列只数非参数端口，否则圆点会和端口挤在一起。
    #[test]
    fn param_ports_are_derived_marked_and_kept_out_of_the_port_column() {
        let kind = crate::catalog::all()
            .into_iter()
            .find(|kind| kind.id == "upscale")
            .expect("该有「缩放图像」");
        let (inputs, _) = crate::catalog::ports_for("upscale", &kind.defaults);

        let percent = inputs
            .iter()
            .find(|port| port.id == "param:percent")
            .expect("数字参数该多一个端口");
        assert!(percent.is_param());
        assert_eq!(percent.param.as_deref(), Some("percent"));
        assert!(!percent.required, "参数端口永远是可选的");
        // Select 的合法值是一张固定表，不能让上游乱喂 —— 不该有端口。
        assert!(
            !inputs.iter().any(|port| port.id == "param:filter"),
            "下拉参数不该有端口"
        );

        let graph = canvas(&["upscale"]);
        assert_eq!(
            graph.port_rows(0),
            1,
            "端口列只数非参数端口（这里只有一个图像输入）"
        );
    }

    /// 参数端口要落在它那一行参数的标签上（浅蓝小圆点），而不是掉进端口列。
    /// percent 是「缩放图像」的第一个参数，圆点该贴左缘、落在参数区最上面那行。
    #[test]
    fn a_param_port_sits_on_its_param_row() {
        let kinds = crate::catalog::all();
        let graph = canvas(&["upscale"]);
        let k = graph.nodes[0]
            .inputs
            .iter()
            .position(|port| port.param.as_deref() == Some("percent"))
            .expect("缩放图像该有 percent 的参数端口");

        let card = graph.flow_rect(&kinds, 0);
        let port = graph.in_port(&kinds, 0, k);
        let offset = graph
            .param_port_offset(&kinds, 0, "percent")
            .expect("percent 该有定位偏移");
        let params_top = graph.params_top(0);

        assert_eq!(port.x, card.left(), "参数端口该贴卡片左缘");
        assert!(
            offset >= params_top,
            "参数端口该在参数区里（端口列下方）：{offset} < {params_top}"
        );
        assert!(
            offset - params_top <= PARAM_LABEL_H,
            "percent 是第一个参数，圆点该在最上面那行：偏移 {}",
            offset - params_top
        );
        assert!(port.y < card.bottom(), "参数端口不能掉出卡片");
    }

    /// 灰底参数框的上沿要和里面第一个控件对得上 —— 参数端口不能撑高端口区。
    /// 曾经就错在这里：`node_body_top` 把参数端口也算进了行数，于是每多一个
    /// 参数端口，灰框就比控件低一整行。
    #[test]
    fn the_params_panel_lines_up_with_where_the_controls_are_drawn() {
        let graph = canvas(&["upscale"]);

        // 灰底参数框的上沿（相对卡片顶部）。
        let panel_top = node_body_top(&graph.nodes[0]);
        // 控件开始画的位置（= 上沿 + PARAMS_GAP）。
        let controls_top = graph.params_top(0);
        assert_eq!(
            controls_top - panel_top,
            PARAMS_GAP,
            "参数框上沿到第一个控件之间应当正好是 PARAMS_GAP"
        );
        // upscale 只声明了一个图像输入；参数端口 `param:percent` 不该算进端口列。
        assert_eq!(
            panel_top,
            HEADER_H + PORT_ROW_H,
            "端口区高度只按声明端口算，参数端口不占一席"
        );
    }

    /// 刚拖出来的节点落在哪儿用的是 `height_of`，画出来用的是 `height_of_node` ——
    /// 两者必须一致，否则落点会比实际位置偏一点。
    #[test]
    fn a_dropped_node_is_centred_by_its_real_height() {
        let kinds = crate::catalog::all();
        for id in [
            "read",
            "input_box",
            "convert_image",
            "compress_image",
            "crop_image",
            "upscale",
            "rename",
            "save_output",
            "literal_text",
            // 本地没模型时卡片会多出一块下载面板 —— 落点也得把它算进去。
            "background_removal",
        ] {
            let graph = canvas(&[id]);
            let kind = kinds.iter().find(|kind| kind.id == id).unwrap();
            let projected = height_of(kind, &kind.defaults, &graph.notes);
            let actual = height_of_node(&kinds, &graph.nodes[0], &graph.notes);
            assert!(
                (projected - actual).abs() < 0.001,
                "{id}：height_of={projected} 与实际高度 {actual} 对不上"
            );
        }
    }

    /// 字面量的输出能接到参数端口上 —— 这是「参数改从上游取值」的入口。
    #[test]
    fn a_literal_links_into_a_param_port() {
        let mut graph = canvas(&["literal_text", "rename"]);
        let to = graph.nodes[1]
            .inputs
            .iter()
            .position(|port| port.param.as_deref() == Some("name"))
            .expect("重命名该有文件名参数端口");

        assert!(
            graph.can_link(out(0, 0), input(1, to)),
            "文本该能接到文本参数上"
        );
        graph.link(out(0, 0), input(1, to));
        assert_eq!(graph.wires.len(), 1);
        assert_eq!(graph.wires[0].to_port, graph.nodes[1].inputs[to].id);
    }

    /// 取色器的两个颜色空间互为逆运算 —— 不然拖一下取色区颜色就偏了。
    #[test]
    fn hsv_round_trips_through_its_inverse() {
        for rgba in [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [18, 52, 86, 255],
            [200, 200, 200, 255],
            [0, 0, 0, 255],
            [255, 255, 255, 255],
        ] {
            let (h, s, v) = rgb_to_hsv(rgba[0], rgba[1], rgba[2]);
            let (r, g, b) = hsv_to_rgb(h, s, v);
            assert_eq!([r, g, b], [rgba[0], rgba[1], rgba[2]], "{rgba:?}");
        }
    }

    /// 颜色字符串和 RGBA 互为逆运算；坏值不 panic。
    #[test]
    fn color_strings_round_trip() {
        for value in ["#000000", "#ffffff", "#123456", "#12345678"] {
            assert_eq!(color_string(color_rgba(value)), value);
        }
        assert_eq!(color_rgba("transparent"), [0, 0, 0, 0]);
        assert_eq!(color_string(color_rgba("transparent")), "#00000000");
        assert_eq!(color_rgba("不是颜色"), [0, 0, 0, 255]);
    }
}
