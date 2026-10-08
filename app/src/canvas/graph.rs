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
    StrokeKind,
};
use egui::epaint::CubicBezierShape;
use starrytools_core::engine::{NodeRunResult, NodeStatus, RunReport};
use starrytools_core::model::workflow::{Edge, NodeInstance, Position, Workflow};
use std::collections::HashMap;

use crate::canvas::geometry::{self, Cubic};
use crate::canvas::layout::*;
use crate::canvas::node::*;
use crate::canvas::view::Viewport;
use crate::catalog::{Control, Kind, Param, Port};
use crate::state::dialog;
use crate::state::models::Downloads;
use crate::state::run::{LiveRun, Marks};
use crate::ui::controls::*;
use crate::ui::easing;
use crate::ui::icons::{self, IconFn};
use crate::ui::theme;
use crate::ui::widgets;

/// 节点的显示尺寸（流坐标，不含缩放）。
pub(crate) const NODE_W: f32 = 216.0;
pub(crate) const HEADER_H: f32 = 30.0;
pub(crate) const PORT_ROW_H: f32 = 20.0;
pub(crate) const BODY_PAD: f32 = 8.0;
/// 参数块：标签一行、控件一行（上下排列，和原设计一致）。
pub(crate) const PARAM_LABEL_H: f32 = 15.0;
pub(crate) const PARAM_LABEL_GAP: f32 = 4.0;
/// 模型还没下载时，卡片上那块「下载模型」面板。各段高度**一处定义**，
/// 面板总高由它们相加得出 —— 改其中任何一段，卡片高度和里面控件的位置都自动跟上。
const MODEL_PAD: f32 = 10.0;
const MODEL_TITLE_H: f32 = 16.0;
const MODEL_NOTE_H: f32 = 18.0;
const MODEL_SELECT_H: f32 = 24.0;
const MODEL_ACTION_GAP: f32 = 8.0;
const MODEL_ACTION_H: f32 = 28.0;
pub(crate) const MODEL_PANEL_H: f32 = MODEL_PAD * 2.0
    + MODEL_TITLE_H
    + MODEL_NOTE_H
    + MODEL_SELECT_H
    + MODEL_ACTION_GAP
    + MODEL_ACTION_H;
/// 「缺外部程序」面板的高度（流坐标）：一行带图标的提示。
pub(crate) const TOOL_PANEL_H: f32 = 34.0;
pub(crate) const PARAM_NOTE_GAP: f32 = 4.0;
/// 两个参数之间留的缝。
pub(crate) const PARAM_GAP: f32 = 8.0;
/// 开关是「标签 + 开关」一行放。
pub(crate) const PARAM_BOOL_H: f32 = 20.0;
/// 多行文本框每行的估高。
pub(crate) const PARAM_TEXT_LINE: f32 = 16.0;
/// 端口和参数之间留的那条缝。
pub(crate) const PARAMS_GAP: f32 = 9.0;
/// 参数区底部留的那点白，免得最后一个控件贴着圆角。
pub(crate) const PARAMS_BOTTOM: f32 = 8.0;
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
pub(crate) const PREVIEW_H: f32 = 84.0;

/// 卡片底部的色板条高度（流坐标）。输出是色板时卡片才多出这么高。
pub(crate) const PALETTE_H: f32 = 30.0;

/// 进度块的高度（流坐标）。运行中的长任务（视频压缩）才多出这么高：
/// 一行「帧 12/300 · 35%」下面一条会流动的进度条。
pub(crate) const PROGRESS_H: f32 = 42.0;

/// 预览的棋盘格边长（流坐标）。
const CHECKER: f32 = 8.0;

/// 运行痕迹各段的高度（流坐标）。
pub(crate) const RUN_ACTIONS_H: f32 = 30.0;
/// 运行「提示」那一块：上下留白、图标的大小、图标与文字之间的缝、两条提示之间的缝。
pub(crate) const NOTICE_PAD: f32 = 8.0;
pub(crate) const NOTICE_ICON: f32 = 12.0;
pub(crate) const NOTICE_GAP: f32 = 6.0;
pub(crate) const NOTICE_STACK: f32 = 5.0;

/// 节点落到画布上时那段入场动画的时长。
const ENTER_SECS: f64 = 0.34;

/// 断口回缩并消散的时长。
const SEVER_SECS: f64 = 0.46;
/// 收刀「跟出」的时长。
const OVERTAKE_SECS: f64 = 0.28;
/// 收刀时刀尖再往前送多远（屏幕像素）。
const OVERTAKE_REACH: f32 = 160.0;

/// 交互瞬态：这一帧正被抓住的东西、正在拉的线、右键菜单、刀光。
#[derive(Default)]
struct Interaction {
    /// 这一帧被抓住的节点。`None` 表示抓的是空白处（= 平移画布）。
    grabbed: Option<usize>,
    /// 指针底下的端口，用来放大高亮。
    hovered_port: Option<PortRef>,
    /// 正在从某个端口往外拉线。
    connect: Option<Connect>,
    /// 打开着的节点右键菜单：哪个节点、在屏幕哪儿弹出。
    menu: Option<(usize, Pos2)>,
    /// 一道刀光。
    slash: Option<Slash>,
}

impl Interaction {
    /// 画布上删掉了一只节点（下标 `removed`）之后，把记着的节点下标跟着挪一挪。
    ///
    /// 拖拽中的节点刚好被删掉时就松手（而不是拖到错的对象上、或直接越界）。
    fn fixup_after_removal(&mut self, removed: usize) {
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

/// 按节点 id 索引的贴图缓存。
#[derive(Default)]
struct Textures {
    /// 卡片缩略图。开始新的一次运行 / 清空运行记录时整批丢掉重建。
    card: HashMap<String, egui::TextureHandle>,
}

/// 动画与帧率读数。
#[derive(Default)]
struct Anim {
    /// 正在断裂的连线。
    severing: Vec<Severing>,
    /// 刚落到画布上的节点（id → 落下的时刻），用来放入场动画。
    entering: HashMap<String, f64>,
    /// 右下角帧率读数（指数平滑后的 fps）。
    fps: f32,
    /// 上一帧的时刻，用来算帧间隔。
    last_frame: Option<f64>,
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    pub view: Viewport,
    /// 画布内容的版本号。改一下就加一，用来判断有没有未保存的改动。
    /// 平移 / 缩放不算 —— 那是视图状态，不落盘。
    pub revision: u64,
    /// **语义**版本号：只有会改变静态检查结果的东西（增删节点、改参数、改连线）
    /// 才加一。挪动节点只动几何、不动它 —— 静态检查（会读输入文件头）据此缓存，
    /// 否则拖动时每帧都会重跑一遍检查、每帧读一次磁盘。
    check_revision: u64,
    /// 当前选中的节点（`运行至此` 认它）。
    pub selected: Option<usize>,
    /// 交互瞬态（抓住的节点 / 端口 / 菜单 / 刀光）。
    interaction: Interaction,
    /// 按节点 id 索引的贴图缓存。
    textures: Textures,
    /// 动画与帧率。
    anim: Anim,
    /// 点了卡片上的「运行至此」：这个节点 id 要交给调用方去跑。
    pending_run: Option<String>,
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
    /// 点过空白处：把上一次运行的高亮收起来（下一次运行再亮回来）。
    run_marks_hidden: bool,
    /// 需要模型的节点正在下的那些模型（按节点 id）。**不落盘**。
    downloads: Downloads,
    /// 正在后台开的文件 / 目录选择框（结果回来了再应用到节点上）。**不落盘**。
    picker: dialog::Picker,
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
                step: None,
                requirements: Requirements::default(),
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
    pub fn add_node_at(&mut self, screen: Pos2, kind: &Kind, now: f64) {
        let mut node = Node {
            id: new_id(),
            pos: Pos2::ZERO,
            kind: kind.id.clone(),
            title: kind.name.clone(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: kind.defaults.clone(),
            preview: false,
            palette: None,
            run: None,
            step: None,
            requirements: Requirements::default(),
        };
        refresh_ports(&mut node);
        // 高度用**画卡片那一套**算（同一份代码），落点才不会偏。
        let height = height_of_node(std::slice::from_ref(kind), &node, &self.notes);
        node.pos = self.view.to_flow(screen) - egui::vec2(NODE_W, height) * 0.5;
        self.anim.entering.insert(node.id.clone(), now);
        self.nodes.push(node);
        self.interaction.grabbed = Some(self.nodes.len() - 1);
        self.touch_content();
    }

    /// 记一笔「画布变过了」—— 保存之后用它判断有没有未保存的改动。
    ///
    /// 几何变化（挪动节点）也走它：位置要落盘，所以仍是「改动」。
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// 记一笔**语义**改动：增删节点、改参数、改连线。除了 dirty，还让静态检查失效 ——
    /// 那个检查会读输入文件头、跑全图拓扑，不能因为挪一下节点就重算。
    fn touch_content(&mut self) {
        self.revision += 1;
        self.check_revision += 1;
    }

    /// 静态检查的缓存键（只有语义改动会变）。
    pub fn check_revision(&self) -> u64 {
        self.check_revision
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
                step: None,
                requirements: Requirements::default(),
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

    // ---- 坐标换算：流坐标 <-> 屏幕坐标 ----

    fn flow_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        Rect::from_min_size(
            self.nodes[i].pos,
            egui::vec2(NODE_W, height_of_node(kinds, &self.nodes[i], &self.notes)),
        )
    }

    fn screen_rect(&self, kinds: &[Kind], i: usize) -> Rect {
        let r = self.flow_rect(kinds, i);
        Rect::from_min_max(self.view.to_screen(r.min), self.view.to_screen(r.max))
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

    /// 某个参数行上那个小圆点相对卡片顶部的高度（流坐标）。
    ///
    /// 参数不可见、或者这个参数根本没有端口时就返回 `None`。位置直接读
    /// [`param_slots`] —— 和绘制控件用的是同一份排版。
    fn param_port_offset(&self, kinds: &[Kind], i: usize, param_id: &str) -> Option<f32> {
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

    /// 把后台选好的文件 / 目录贴到它该去的地方。
    ///
    /// 选择框是在后台线程上开的（见 [`crate::state::dialog`]），结果用通道送回来；
    /// 这里每帧收一次。
    fn apply_pick(&mut self) {
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
                    std::thread::spawn(move || {
                        let _ = std::fs::copy(source, &path);
                    });
                }
            }
        }
        if changed {
            self.touch_content();
        }
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
        // 刚下完的节点重算一下「缺不缺模型」缓存（文件刚落到磁盘）。
        for finished in self.downloads.poll() {
            if let Some(node) = self.nodes.iter_mut().find(|node| node.id == finished) {
                refresh_ports(node);
            }
        }
        // 收一收后台文件选择的结果 —— 选好了就应用到节点上。
        self.apply_pick();

        // 先把这次的缩略图准备好 —— 卡片高度、预览绘制都要用到。
        self.sync_previews(ui.ctx(), view.report, view.live);
        // 正在跑的那个节点报的进度贴到它自己的卡片上（卡片底部那段信息区）。
        self.sync_step(view.live);
        // 参数说明与运行提示先折好行、量好高 —— 卡片高度靠它，折行也靠它。
        self.sync_notes(ui, kinds);

        self.handle_input(ui, &resp, kinds, now);
        self.advance(now);

        let painter = ui.painter_at(rect);

        // ---- 背景小点 ----
        // 只在点距够大（没缩太远）时才画，省得小到看不清还白花功夫。
        // 全部拼进**一个 Mesh**：几千个 `circle_filled` 会让 epaint 每帧重新三角化
        // 几千个形状，平移时位置又全变、缓存也命中不了 —— 用一个网格一次画出去，
        // 图形一模一样，每帧的开销则从「几千个形状」降到「一个」+ 拼顶点的循环。
        let step = DOT_GAP * self.view.zoom;
        if step > 5.0 {
            let texture = dot_texture(ui.ctx());
            let mut mesh = egui::Mesh::with_texture(texture.id());
            let ox = self.view.pan.x.rem_euclid(step);
            let oy = self.view.pan.y.rem_euclid(step);
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
        self.anim
            .entering
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
                        .insert(text_style, FontId::monospace(size * self.view.zoom));
                }
                style.spacing.button_padding = egui::vec2(5.0, 1.0) * self.view.zoom;
                style.spacing.interact_size.y = 18.0 * self.view.zoom;
                style.spacing.icon_width = 9.0 * self.view.zoom;
                style.spacing.item_spacing = egui::vec2(4.0, 3.0) * self.view.zoom;
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
            self.touch_content();
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

    /// 记一帧的时间，算出平滑后的 fps。
    ///
    /// 空闲一段时间后的第一帧间隔会很大（甚至几秒），那种不算 —— 否则一恢复
    /// 交互，读数会被那一帧拖到个位数。
    fn measure_fps(&mut self, now: f64) {
        if let Some(last) = self.anim.last_frame {
            let delta = now - last;
            if delta > 0.0 && delta <= 0.5 {
                let instant = (1.0 / delta) as f32;
                self.anim.fps = if self.anim.fps <= 0.0 {
                    instant
                } else {
                    self.anim.fps * 0.9 + instant * 0.1
                };
            }
        }
        self.anim.last_frame = Some(now);
    }

    /// 右下角一个灰色的帧率读数，保留到整数位。
    ///
    /// 数字和 `fps` 都自己排版：数字在一个固定宽度里**右对齐**，紧跟着单位 ——
    /// 位数变化（60 → 100）时数字从右边长出去，`fps` 一动不动，才不会左右抖。
    fn draw_fps(&self, ui: &egui::Ui) {
        if self.anim.fps <= 0.0 {
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
                    format!("{}", self.anim.fps.round() as i64),
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
        let anchor = self.view.to_flow(canvas.center());
        self.view.zoom = (self.view.zoom * factor).clamp(0.15, 2.0);
        self.view.pan = canvas.center().to_vec2() - anchor.to_vec2() * self.view.zoom;
    }

    /// 把所有节点框进视野。
    fn fit_view(&mut self, canvas: Rect, kinds: &[Kind]) {
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

    /// 有没有还在跑的动画，决定要不要继续申请重绘。
    pub fn is_animating(&self) -> bool {
        self.interaction.slash.is_some()
            || !self.anim.severing.is_empty()
            || self.interaction.connect.is_some()
            || !self.anim.entering.is_empty()
            || self.downloads.any()
    }

    /// 入场动画的进度（0→1，缓出）。不在 `entering` 里就是 1。
    fn enter_factor(&self, id: &str, now: f64) -> f32 {
        match self.anim.entering.get(id) {
            Some(started) => {
                let t = ((now - started) / ENTER_SECS).clamp(0.0, 1.0) as f32;
                easing::ease_out_quad(t)
            }
            None => 1.0,
        }
    }

    /// 推进两个定时动画：断口回缩、收刀跟出。
    fn advance(&mut self, now: f64) {
        self.anim
            .severing
            .retain(|entry| now - entry.started < SEVER_SECS);

        if let Some(slash) = &self.interaction.slash {
            if let Some(start) = slash.release {
                if now - start >= OVERTAKE_SECS {
                    self.interaction.slash = None;
                }
            }
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, resp: &egui::Response, kinds: &[Kind], now: f64) {
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
                    if play.contains(p) {
                        // 要模型 / 外部程序但本地还没有：这个节点还不能跑 —— 点运行不生效。
                        if self.nodes[i].requirements.model_missing
                            || self.nodes[i].requirements.tool_missing
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
    fn refresh_doomed(&mut self, kinds: &[Kind]) {
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
    fn finish_slash(&mut self, kinds: &[Kind], now: f64) {
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
        let screen: Cubic = curve.map(|p| self.view.to_screen(p));

        // 被刀光扫到：先铺一层更宽的发光，再画本体，并轻轻搏动；
        // 刀口切到的那一点再闪一下白光 —— 像真的被切开。
        if let Some(slash) = &self.interaction.slash {
            if let Some((_, cut_t)) = slash.doomed.iter().find(|(index, _)| *index == i) {
                let pulse = 0.55 + 0.45 * (now * easing::BREATH).sin() as f32;
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(9.0 * self.view.zoom, theme::danger_alpha(0.16 * pulse)),
                ));
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.4 * self.view.zoom, theme::danger_alpha(pulse)),
                ));
                // 刀口那一点：白光亮一下。
                let cut = geometry::at(&screen, *cut_t);
                let flash = (3.0 + 2.0 * pulse) * self.view.zoom;
                painter.circle_filled(
                    cut,
                    flash,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                );
                painter.circle_filled(
                    cut,
                    flash * 2.4,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 40),
                );
                return;
            }
        }

        // 静态检查说这条线接不上 → 红色虚线。
        if marks.invalid_edges.contains(&wire.id) {
            self.dashed_curve(
                painter,
                &curve,
                Stroke::new((1.6 * self.view.zoom).max(1.0), theme::DANGER),
                5.0 * self.view.zoom,
                4.0 * self.view.zoom,
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
                (1.5 * self.view.zoom).max(1.0),
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
            .map(|p| self.view.to_screen(p))
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
        for entry in &self.anim.severing {
            let progress = ((now - entry.started) / SEVER_SECS).clamp(0.0, 1.0) as f32;
            let eased = easing::ease_out_quad(progress);
            let fade = 1.0 - ((progress - 0.5) / 0.5).max(0.0);
            let stroke = Stroke::new(2.4 * self.view.zoom, theme::danger_alpha(fade));

            let (left, right) = geometry::split(&entry.curve, entry.t);

            // 两截各自从断口往回缩：左半留前 `keep`，右半留后 `keep`。
            let keep = 1.0 - eased;
            if keep > 0.01 {
                self.stroke_curve(painter, &geometry::split(&left, keep).0, stroke);
                self.stroke_curve(painter, &geometry::split(&right, 1.0 - keep).1, stroke);
            }

            // 断口的那一下光。
            let point = self.view.to_screen(geometry::at(&entry.curve, entry.t));
            let radius = (3.0 + eased * 20.0) * self.view.zoom;
            let alpha = (1.0 - progress * 2.4).max(0.0);
            painter.circle_filled(point, radius, theme::danger_alpha(alpha * 0.9));
        }
    }

    fn stroke_curve(&self, painter: &egui::Painter, curve: &Cubic, stroke: Stroke) {
        let points: Vec<Pos2> = geometry::sample(curve, geometry::steps_for(curve))
            .into_iter()
            .map(|p| self.view.to_screen(p))
            .collect();
        painter.line(points, stroke);
    }

    fn draw_slash(&self, painter: &egui::Painter, now: f64) {
        let Some(slash) = &self.interaction.slash else {
            return;
        };

        // 收刀之后：刀尖往前送一段，同时变淡变细，像力道用完了。
        let (tip, fade, scale) = match slash.release {
            None => (slash.to, 1.0, 1.0),
            Some(start) => {
                let t = ((now - start) / OVERTAKE_SECS).clamp(0.0, 1.0) as f32;
                let eased = easing::ease_out_cubic(t);
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
    fn hovered_node(&self, kinds: &[Kind], pointer: Option<Pos2>) -> Option<usize> {
        pointer.and_then(|p| self.hit(kinds, self.view.to_flow(p)))
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

    /// 端口现在的放大倍数。
    ///
    /// **拉线的起点、它可能落下的落点、以及指针悬着的那个**都算「被强调」—— 两端
    /// （起点 / 落点）因此永远一样大。只看 `hovered_port` 是不够的：落点靠的是
    /// `connect.target`，两者偶尔会错开一帧，端点就会一大一小。
    fn port_scale(&self, port: PortRef) -> f32 {
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
    fn is_valid_target(&self, port: PortRef) -> bool {
        self.interaction
            .connect
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
        let occupied = self.wire_at_port(input.node, &target.id, true).is_some();

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
    fn draw_menu(&mut self, ui: &egui::Ui) {
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
        self.interaction.menu = None;
        // 交互里记着的下标也要跟着挪 —— 否则拖拽中删掉节点会拖到错的对象，
        // 删的恰好是最后一个还会越界 panic。
        self.interaction.fixup_after_removal(index);
        self.touch_content();
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
        self.interaction.menu = None;
        self.touch_content();
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
        self.interaction.menu = None;
        self.touch_content();
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

    /// 把「正在跑的那个节点报的进度」贴到它自己的卡片上，其余节点清掉。
    ///
    /// 卡片底部的那段信息区靠它决定要不要长出来（见 [`crate::canvas::layout`]）。
    fn sync_step(&mut self, live: Option<&LiveRun>) {
        let step = live.and_then(|live| live.step.clone());
        for node in &mut self.nodes {
            node.step = match &step {
                Some((id, step)) if *id == node.id => Some(step.clone()),
                _ => None,
            };
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
        let palette: Option<Vec<[u8; 4]>> = result
            .outputs
            .iter()
            .find_map(|output| output.palette.as_ref())
            .map(|colors| colors.iter().map(|color| color_rgba(color)).collect());

        // 幂等短路：内容没变就不重写 —— 否则每帧都要 clone 一遍运行文本。
        let node = &self.nodes[index];
        let unchanged = node.palette == palette
            && node.run.as_ref().is_some_and(|run| {
                run.status == result.status
                    && run.ms == result.elapsed_ms
                    && run.error == result.error
                    && run.warnings == result.warnings
                    && run.file == file
            });
        if !unchanged {
            self.nodes[index].palette = palette;
            self.nodes[index].run = Some(NodeRun {
                status: result.status,
                ms: result.elapsed_ms,
                error: result.error.clone(),
                warnings: result.warnings.clone(),
                file,
            });
        }

        // 缩略图一个节点只传一次。
        if self.textures.card.contains_key(&result.node_id) {
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
        self.textures.card.insert(result.node_id.clone(), texture);
        self.nodes[index].preview = true;
    }

    /// 清掉上一次运行留在画布上的痕迹：缩略图 / 色板 / 状态 / 高亮。
    ///
    /// 开始新的一次运行、或用户清空运行记录时调。
    pub fn reset_run_marks(&mut self) {
        self.textures.card.clear();
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
        let cell = CHECKER * self.view.zoom;
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
        let Some(connect) = &self.interaction.connect else {
            return;
        };

        let a = self.view.to_screen(self.port_pos(kinds, connect.from));
        // 有落点候选就直接吸到那个端口上 —— 手感上就是「啪」地贴过去。
        let b = match connect.target {
            Some(target) => self.view.to_screen(self.port_pos(kinds, target)),
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
        let width = (1.8 * self.view.zoom).max(1.2);

        // 还没接到端口上：整根线一呼一吸地泛蓝光，提示「尚未连接」。
        let pulse = if snapped {
            0.0
        } else {
            let wave = 0.5 + 0.5 * (now * easing::BREATH).sin() as f32;
            // 外圈光晕
            painter.add(CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(
                    width + 7.0 * self.view.zoom,
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
                (5.0 + pulse * 2.5) * self.view.zoom,
                theme::accent_alpha(0.16 + 0.18 * pulse),
            );
            painter.circle_filled(b, 3.0 * self.view.zoom, color);
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
        let z = self.view.zoom;

        // 卡片里面的字一律剪到卡片内 —— 长警告 / 长错误、甚至文件名也不会溢出去。
        let body = painter.with_clip_rect(r);

        // 边线颜色说几件事：跑失败了、跑成功了、还是平常；紫色则说明这是一个
        // 「会拦住运行等你动手」的节点（运行到它时会额外搏动高亮）。
        // **边线放到最后画** —— 里面的各段底色是铺满整宽的，先画会被它们盖住。
        let interactive = crate::catalog::is_interactive(&node.kind, &node.params);
        let waiting = marks.waiting.as_deref() == Some(node.id.as_str());
        // 「正在跑的那个」—— 运行中牌子的蓝色边框。
        let running = marks.running.as_deref() == Some(node.id.as_str());
        // 这个节点自己报上来的实时进度（视频压缩这类长任务才有）。
        let step = marks
            .step
            .as_ref()
            .filter(|(id, _)| id == &node.id)
            .map(|(_, step)| step);
        // 要模型但本地还没有：整个节点禁用，卡片上挂一个下载面板（见 `draw_node_controls`）。
        // 要一个外部程序（如 ffmpeg）但本地没有：一样禁用，卡片上挂一句提示。
        // 这两项都在 `refresh_ports` 里缓存好了 —— 绘制路径上不再碰磁盘。
        let model_missing = node.requirements.model_missing;
        let tool_missing = node.requirements.tool_missing;
        // 是不是流程的起点 —— 只看元数据，不靠「有没有输入端口」猜。
        let is_source = kind_of(kinds, node).is_some_and(|kind| kind.is_source);

        // 白底（投影已经在上一遍里铺好了）。
        painter.rect_filled(r, cr, theme::SURFACE);

        let border = if marks.failed_nodes.contains(&node.id) {
            theme::DANGER
        } else if running {
            theme::ACCENT
        } else if model_missing || tool_missing {
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
        } else if running || interactive || model_missing || tool_missing {
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

        // 「起点」标签：流程从它开始。
        if is_source {
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
            // 进度（帧数 / 百分比 / 进度条）画在卡片下方的信息区，不挤在这里。
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
        } else if tool_missing {
            // 要外部程序但本地没有：把工具名也报出来，好知道去装什么。
            let label = match kind_of(kinds, node).and_then(|kind| kind.requires_tool.as_deref()) {
                Some(tool) => format!("需 {tool}"),
                None => "缺工具".to_string(),
            };
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                label,
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
        // 参数区以下各段的高度只算一次（`card_sections`），下面按顺序往下画。
        let sections = card_sections(kinds, node, &self.notes);
        let params_last = sections.total() <= 0.0;
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

        // ---- 「缺东西」面板的底色（紧贴参数区，控件由 `draw_node_controls` 画） ----
        if sections.requirement > 0.0 {
            let rect = Rect::from_min_size(
                egui::pos2(r.left(), params_bottom),
                egui::vec2(r.width(), sections.requirement * z),
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
        let body_pad = if sections.requirement > 0.0 {
            0.0
        } else if sections.total() > 0.0 {
            BODY_PAD
        } else {
            0.0
        };
        let mut y = params_bottom + (sections.requirement + body_pad) * z;

        // ---- 运行中的进度（长任务才有这一段）：帧计数 + 百分比 + 会波的进度条 ----
        if sections.progress > 0.0 {
            if let Some(step) = step {
                let last = sections.warn == 0.0
                    && sections.error == 0.0
                    && sections.actions == 0.0
                    && !node.preview
                    && sections.palette == 0.0;
                let panel = Rect::from_min_size(
                    egui::pos2(r.left(), y),
                    egui::vec2(r.width(), sections.progress * z),
                );
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
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );

                let inner = Rect::from_min_max(
                    egui::pos2(r.left() + 10.0 * z, y + 7.0 * z),
                    egui::pos2(r.right() - 10.0 * z, y + PROGRESS_H * z - 7.0 * z),
                );
                let percent = match step.fraction {
                    Some(fraction) => format!("{}%", (fraction * 100.0).round() as i32),
                    None => "…".to_string(),
                };
                let left_text = match (step.frame, step.total_frames) {
                    (Some(frame), Some(total)) => format!("帧 {frame}/{total}"),
                    (Some(frame), None) => format!("帧 {frame}"),
                    _ => "处理中".to_string(),
                };
                let right_text = match &step.text {
                    Some(text) => format!("{percent} · {text}"),
                    None => percent,
                };
                let font = FontId::monospace(9.5 * z);
                painter.text(
                    egui::pos2(inner.left(), inner.top()),
                    Align2::LEFT_TOP,
                    left_text,
                    font.clone(),
                    theme::INK_2,
                );
                painter.text(
                    egui::pos2(inner.right(), inner.top()),
                    Align2::RIGHT_TOP,
                    right_text,
                    font,
                    theme::INK_2,
                );

                let bar = Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - 12.0 * z),
                    egui::pos2(inner.right(), inner.bottom()),
                );
                draw_progress_bar(&body, bar, step.fraction, now);
            }
            y += sections.progress * z;
        }

        if node.preview {
            if let Some(texture) = self.textures.card.get(&node.id) {
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
            y += sections.preview * z;
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
            y += sections.palette * z;
        }

        // ---- 运行提示 ----
        // 不再是一排橙色的「!」，而是一块淡蓝底 + 信息图标 + 会折行的正文：
        // 读完是「知道发生了什么」而不是「出事了」。
        if sections.warn > 0.0 {
            if let Some(run) = &node.run {
                let last = sections.error == 0.0 && sections.actions == 0.0;
                let panel = Rect::from_min_size(
                    egui::pos2(r.left(), y),
                    egui::vec2(r.width(), sections.warn * z),
                );
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
            y += sections.warn * z;
        }

        if sections.error > 0.0 {
            if let Some(run) = &node.run {
                if let Some(error) = &run.error {
                    let last = sections.actions == 0.0;
                    let rect = Rect::from_min_size(
                        egui::pos2(r.left(), y),
                        egui::vec2(r.width(), sections.error * z),
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
            y += sections.error * z;
        }

        if sections.actions > 0.0 {
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
            self.view.to_screen(self.in_port(kinds, i, k))
        } else {
            self.view.to_screen(self.out_port(kinds, i, k))
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
            .interaction
            .connect
            .as_ref()
            .is_some_and(|connect| connect.from == here);
        let hovering = self.interaction.hovered_port == Some(here);
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
                crate::ui::widgets::ink_top(&badge, chip.center().y),
            ),
            badge,
            type_ink,
        );

        let label_color = if bad { theme::DANGER } else { theme::INK_3 };
        painter.galley(
            egui::pos2(
                chip.right() + gap,
                crate::ui::widgets::ink_top(&label, chip.center().y),
            ),
            label,
            label_color,
        );
    }

    /// 这个端口上接的线（有的话）。输入 / 输出两种端口共用它。
    fn wire_at_port(&self, node_index: usize, port_id: &str, input: bool) -> Option<&Wire> {
        self.wires.iter().find(|wire| {
            if input {
                wire.to == node_index && wire.to_port == port_id
            } else {
                wire.from == node_index && wire.from_port == port_id
            }
        })
    }

    /// 这个端口是不是接了条被静态检查判为非法的线。
    fn port_invalid(&self, marks: &Marks, node_index: usize, port_id: &str, input: bool) -> bool {
        self.wire_at_port(node_index, port_id, input)
            .is_some_and(|wire| marks.invalid_edges.contains(&wire.id))
    }

    /// 这个端口接没接线。
    fn port_linked(&self, node_index: usize, port_id: &str, input: bool) -> bool {
        self.wire_at_port(node_index, port_id, input).is_some()
    }

    /// 「另存为」：把产物拷到用户挑的位置。
    /// 指针现在悬在哪个节点上（决定要不要露出工具按钮）。
    /// 卡片底部「产物操作行」两个按钮的矩形：`(在文件夹中显示, 另存为)`。
    fn node_action_rects(&self, kinds: &[Kind], i: usize) -> (Rect, Rect) {
        let r = self.screen_rect(kinds, i);
        let z = self.view.zoom;
        let s = TOOL_SIZE * z;
        let pad = 7.0 * z;
        let y = r.bottom() - RUN_ACTIONS_H * z + (RUN_ACTIONS_H * z - s) * 0.5;
        let reveal = Rect::from_min_size(egui::pos2(r.left() + pad, y), egui::vec2(s, s));
        let save = Rect::from_min_size(egui::pos2(reveal.right() + 2.0 * z, y), egui::vec2(s, s));
        (reveal, save)
    }

    /// 一个参数控件这一帧发生了什么的统一处理（画布上所有控件都走它）。
    ///
    /// 返回「参数是否被改了」。文件框不在控件里弹对话框，而是把请求交给
    /// 后台选择器 [`dialog::Picker`]。
    fn handle_control(&mut self, i: usize, param: &Param, event: Option<ControlEvent>) -> bool {
        match event {
            Some(ControlEvent::Changed(value)) => {
                self.nodes[i].params.insert(param.id.clone(), value);
                refresh_ports(&mut self.nodes[i]);
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
    fn draw_node_controls(
        &mut self,
        ui: &mut egui::Ui,
        kinds: &[Kind],
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
        let showing: Vec<usize> = (0..kind.params.len())
            .filter(|&k| kind.params[k].visible(&self.nodes[i].params))
            .collect();
        // 要模型但本地还没下载：整个节点禁用，参数全部灰下去 —— **只有那个「模型」
        // 下拉框还留着**，因为用户正是靠它挑要下哪个模型。缺外部程序（如 ffmpeg）时
        // 也整个禁用，但没有任何参数需要留着。
        let model_missing = self.nodes[i].requirements.model_missing;
        let tool_missing = self.nodes[i].requirements.tool_missing;
        if showing.is_empty() && !model_missing && !tool_missing {
            return false;
        }

        let card = self.screen_rect(kinds, i);
        // 每个可见参数块占哪几行 —— 绘制读它，端口圆点也读它（`param_slots`）。
        let slots = param_slots(&self.nodes[i], &kind, &self.notes);
        let mut changed = false;

        for (k, slot) in showing.iter().copied().zip(slots.iter()) {
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
            // 缺模型时：除「模型」参数外全部禁用。缺外部程序时全部禁用。
            let is_model_param = kind.model_param.as_deref() == Some(param.id.as_str());
            let disabled = linked || tool_missing || (model_missing && !is_model_param);

            // 开关：标签和开关同一行。
            if matches!(param.control, Control::Bool) {
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
                    egui::pos2(row.right() - 30.0 * zoom, row.top()),
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

            // 控件一行。
            let height = slot.control_h * zoom;
            let control_rect = Rect::from_min_size(
                egui::pos2(left, card.top() + slot.control_y * zoom),
                egui::vec2(width, height),
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

    /// 画「缺外部程序」那块面板：一个警示图标 + 一句提示（底色已由 `draw_node` 铺好）。
    fn draw_tool_panel(&self, ui: &egui::Ui, kind: &Kind, rect: Rect, zoom: f32) {
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
                    crate::ui::widgets::ink_top(&galley, action.center().y),
                ),
                galley,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.downloads.start(&node_id, model, source);
            }
        }
    }
}

/// 一条连线是否**可能**被从 `a` 到 `b` 的刀光切到（快速预筛）。
///
/// 贝塞尔整条落在它的控制点凸包里，所以控制点的包围盒就是一个上界：
/// 包围盒都不和刀光相交的线，不用采样、不用做线段相交测试。
fn curve_maybe_cut(curve: &Cubic, a: Pos2, b: Pos2) -> bool {
    let mut min = curve[0];
    let mut max = curve[0];
    for p in &curve[1..] {
        min = egui::pos2(min.x.min(p.x), min.y.min(p.y));
        max = egui::pos2(max.x.max(p.x), max.y.max(p.y));
    }
    geometry::segment_hits_aabb(a, b, min, max)
}

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

/// 区别只在「正要连上」（`pending`）时多一圈搏动的描边。要改这份动画，只改这里一处。
fn port_halo(painter: &egui::Painter, p: Pos2, z: f32, now: f64, pending: bool) {
    if pending {
        pending_halo(painter, p, 8.5 * z, now);
    } else {
        painter.circle_filled(p, 8.5 * z, theme::accent_alpha(0.20));
    }
}

/// 「活性」光晕：一圈会**伸缩**的柔光 + 一道定住的描边。看点、落点、悬停的端点都走它。
///
/// 半径在 `base` 到 `base * 1.24` 之间呼吸，看着就是「活的」。
fn pending_halo(painter: &egui::Painter, center: Pos2, base: f32, now: f64) {
    let pulse = 0.5 + 0.5 * (now * easing::BREATH).sin() as f32;
    painter.circle_filled(
        center,
        base * (1.0 + pulse * 0.24),
        theme::accent_alpha(0.18),
    );
    painter.circle_stroke(center, base * 0.88, Stroke::new(1.5, theme::ACCENT));
}

/// 一条「胶片」进度条：一条实心蓝条，里面一条条细白纹随进度向右流动，
/// 前沿一道细亮线。全是纯色细线，**不用渐变、也没有圆点**。
///
/// 动画落在**条身**上（细纹在走），所以哪怕进度一时不动，它看着也是活的。
/// `fraction` 是 `None` 时画**不确定态**：整块胶片在轨道上来回扫。
/// 时间从 `now` 来，与帧率无关；跑着的时候外壳会持续申请重绘。
fn draw_progress_bar(painter: &egui::Painter, rect: Rect, fraction: Option<f32>, now: f64) {
    if rect.width() <= 6.0 || rect.height() <= 3.0 {
        return;
    }
    let r = (rect.height() * 0.28).round() as u8;
    let radius = CornerRadius::same(r);
    // 轨道：浅灰底 + 一道发丝边框。略方的角，比胶囊更像一条胶片。
    painter.rect_filled(rect, radius, theme::SURFACE_3);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );

    // 胶片上的细纹：间距固定，整体随 `now` 向右匀速流动（与帧率无关）。
    const SPACING: f32 = 9.0;
    let offset = (now as f32 * 26.0).rem_euclid(SPACING);

    // 画一段胶片：实心蓝底 + 流动细纹 + 前沿细亮线。
    let draw_strip = |painter: &egui::Painter, strip: Rect| {
        if strip.width() <= 1.5 {
            return;
        }
        // 结束那头贴着轨道右端时（进度跑满）才收圆角，否则前沿是切平的。
        let flush = strip.right() >= rect.right() - 1.0;
        let corners = CornerRadius {
            nw: r,
            sw: r,
            ne: if flush { r } else { 0 },
            se: if flush { r } else { 0 },
        };
        let clip = painter.with_clip_rect(strip);
        clip.rect_filled(strip, corners, theme::ACCENT);

        let mut x = strip.left() - SPACING + offset;
        while x < strip.right() {
            if x > strip.left() + 1.0 {
                clip.line_segment(
                    [
                        egui::pos2(x, strip.top() + 2.5),
                        egui::pos2(x, strip.bottom() - 2.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(56)),
                );
            }
            x += SPACING;
        }

        if strip.width() > 2.5 {
            clip.line_segment(
                [
                    egui::pos2(strip.right() - 1.0, strip.top()),
                    egui::pos2(strip.right() - 1.0, strip.bottom()),
                ],
                Stroke::new(2.0, Color32::WHITE),
            );
        }
    };

    match fraction {
        Some(fraction) => {
            let width = rect.width() * fraction.clamp(0.0, 1.0);
            if width <= 1.5 {
                return;
            }
            let strip = Rect::from_min_size(rect.min, egui::vec2(width, rect.height()));
            draw_strip(painter, strip);
        }
        None => {
            // 不确定态：整块胶片在轨道上来回扫，细纹照旧在它自己身上流。
            let block_w = rect.width() * 0.3;
            let sweep = (now * 0.5).rem_euclid(1.0) as f32;
            let x = rect.left() - block_w + sweep * (rect.width() + block_w);
            let strip = Rect::from_min_size(
                egui::pos2(x, rect.top()),
                egui::vec2(block_w, rect.height()),
            );
            draw_strip(painter, strip);
        }
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
            crate::ui::widgets::ink_top(&g, r.center().y),
        ),
        g,
        color,
    );
    w + CHIP_TRAIL * zoom
}

#[cfg(test)]
mod tests {
    use super::*;
    use starrytools_core::model::params::Params;

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
            step: None,
            requirements: Requirements::default(),
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

    /// 拖动节点只动几何：`touch` 不阭让静态检查失效，否则每帧都会重跑检查、读一次文件头。
    #[test]
    fn moving_a_node_does_not_invalidate_the_static_check() {
        let kinds = crate::catalog::all();
        let mut graph = Graph::demo(3, &kinds);
        let content = graph.check_revision();
        graph.touch(); // 拖动节点走的就是它
        assert_eq!(graph.revision, 1);
        assert_eq!(graph.check_revision(), content, "几何变化不该让检查失效");
    }

    /// 删节点是语义变化：静态检查该失效。
    #[test]
    fn deleting_a_node_invalidates_the_static_check() {
        let kinds = crate::catalog::all();
        let mut graph = Graph::demo(3, &kinds);
        let before = graph.check_revision();
        graph.delete_node(0);
        assert!(graph.check_revision() > before, "删节点该让检查失效");
        assert!(graph.revision > 0);
    }

    /// 拖拽中把那个节点删掉：不能留下越界的 `grabbed` 下标（否则下一帧 panic）。
    #[test]
    fn deleting_the_grabbed_node_releases_the_grab() {
        let kinds = crate::catalog::all();
        let mut graph = Graph::demo(2, &kinds);
        let last = graph.nodes.len() - 1;
        graph.interaction.grabbed = Some(last);

        graph.delete_node(last);

        assert_eq!(graph.interaction.grabbed, None, "抓着的节点没了就该松手");
        assert_eq!(graph.nodes.len(), 1);
    }

    /// 连线的稳定 id 在创建时就该算好，且和落盘用的拼法一致。
    #[test]
    fn a_wires_id_is_stable_and_matches_its_ports() {
        let mut graph = canvas(&["read", "convert_image"]);
        graph.link(
            PortRef {
                node: 0,
                port: 0,
                input: false,
            },
            PortRef {
                node: 1,
                port: 0,
                input: true,
            },
        );
        let wire = &graph.wires[0];
        assert_eq!(
            wire.id,
            Wire::make_id(
                &graph.nodes[0].id,
                &wire.from_port,
                &graph.nodes[1].id,
                &wire.to_port
            )
        );
    }

    /// 刀光预筛：离得远的连线不该被判定为「可能切到」。
    #[test]
    fn the_slash_prefilter_rejects_far_away_wires() {
        let curve = [
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::pos2(20.0, 0.0),
            egui::pos2(30.0, 0.0),
        ];
        assert!(curve_maybe_cut(
            &curve,
            egui::pos2(15.0, -5.0),
            egui::pos2(15.0, 5.0)
        ));
        assert!(!curve_maybe_cut(
            &curve,
            egui::pos2(100.0, -5.0),
            egui::pos2(100.0, 5.0)
        ));
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
                step: None,
                requirements: Requirements::default(),
            };
            refresh_ports(&mut node);
            nodes.push(node);
        }
        Graph {
            nodes,
            wires: Vec::new(),
            view: Viewport {
                pan: egui::Vec2::ZERO,
                zoom: 1.0,
            },
            revision: 0,
            check_revision: 0,
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
            node_body_top(&graph.nodes[0]),
            HEADER_H + PORT_ROW_H,
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
        let params_top = node_body_top(&graph.nodes[0]) + PARAMS_GAP;

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
        let kinds = crate::catalog::all();
        let graph = canvas(&["upscale"]);
        let kind = kinds.iter().find(|kind| kind.id == "upscale").unwrap();

        // 灰底参数框的上沿（相对卡片顶部）。
        let panel_top = node_body_top(&graph.nodes[0]);
        // 实际绘制用的第一个参数块的顶部 —— 应当正好在参数框上沿下方 PARAMS_GAP。
        let slots = param_slots(&graph.nodes[0], kind, &graph.notes);
        let first_top = slots[0].label_y.unwrap_or(slots[0].control_y);
        assert_eq!(
            first_top - panel_top,
            PARAMS_GAP,
            "参数框上沿到第一个参数块之间应当正好是 PARAMS_GAP"
        );
        // upscale 只声明了一个图像输入；参数端口 `param:percent` 不该算进端口列。
        assert_eq!(
            panel_top,
            HEADER_H + PORT_ROW_H,
            "端口区高度只按声明端口算，参数端口不占一席"
        );
    }

    /// 刚拖出来的节点要**以指针为中心**落下来 —— 落点用的高度和画卡片用的高度必须是同一个
    /// （`add_node_at` 现在直接复用 `height_of_node`，所以这条是结构上的保证，不再是两条求和路径）。
    #[test]
    fn a_dropped_node_is_centred_on_the_pointer() {
        let kinds = crate::catalog::all();
        for id in [
            "read",
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
            let mut graph = canvas(&[]);
            let kind = kinds.iter().find(|kind| kind.id == id).unwrap();
            // `canvas` 的 pan 是 0、zoom 是 1，所以流坐标就是屏幕坐标。
            let screen = egui::pos2(300.0, 200.0);
            graph.add_node_at(screen, kind, 0.0);
            let node = graph.nodes.last().unwrap();
            let height = height_of_node(&kinds, node, &graph.notes);
            let centre = egui::pos2(node.pos.x + NODE_W * 0.5, node.pos.y + height * 0.5);
            assert!(
                (centre - screen).length() < 0.001,
                "{id}：落点 {centre:?} 偏离指针 {screen:?}"
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
