//! 画布状态与每帧协调入口。
//!
//! 文档编辑、坐标定位、交互、绘制、参数、资源与运行结果由职责明确的子模块实现。
//! 子模块共享这里的私有状态；卡片尺寸和端口位置始终共用 `layout` 的排版结果。
//! 节点契约直接消费 core 注册表，不镜像节点、参数或端口类型。

use crate::canvas::layout::TextBlock;
use crate::canvas::node::{Connect, Node, PortRef, Severing, Slash, Wire};
use crate::canvas::view::Viewport;
use crate::state::run::{LiveRun, Marks};
use crate::state::{dialog, models::Downloads};
use eframe::egui::{self, Pos2, Rect, Sense};
use starrytools_core::engine::RunReport;
use starrytools_core::model::node_kind::NodeKindInfo;
use std::collections::HashMap;

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
/// 参数区下方一行「设定值」输入的高度（流坐标）。
pub(crate) const PANEL_INPUT_H: f32 = 22.0;
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

/// 按节点 id 索引的贴图缓存。
#[derive(Default)]
struct Textures {
    /// 卡片缩略图。开始新的一次运行 / 清空运行记录时整批丢掉重建。
    card: HashMap<String, egui::TextureHandle>,
    /// 上一次解码失败的 URL；新运行清空，新 URL 允许重试。
    failed: HashMap<String, String>,
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
    /// 后台快照版本，不参与文档 dirty/语义检查。
    tool_revision: Option<u64>,
    ports_revision: Option<u64>,
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

#[path = "document.rs"]
mod document;
#[path = "execution.rs"]
mod execution;
#[path = "interaction.rs"]
mod interaction;
#[path = "parameters.rs"]
mod parameters;
#[path = "placement.rs"]
mod placement;
#[path = "render.rs"]
mod render;
#[path = "resources.rs"]
mod resources;

impl Graph {
    /// 记一笔「画布变过了」—— 保存之后用它判断有没有未保存的改动。
    ///
    /// 几何变化（挪动节点）也走它：位置要落盘，所以仍是「改动」。
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// 记一笔**语义**改动：增删节点、改参数、改连线。除了 dirty，还让静态检查失效 ——
    /// 那个检查会读输入文件头、跑全图拓扑，不能因为挪一下节点就重算。
    pub(crate) fn touch_content(&mut self) {
        self.revision += 1;
        self.check_revision += 1;
    }

    /// 静态检查的缓存键（只有语义改动会变）。
    pub fn check_revision(&self) -> u64 {
        self.check_revision
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        view: RunView<'_>,
    ) -> Option<String> {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let now = ui.input(|input| input.time);
        self.measure_fps(now);
        self.sync_tool_requirements();
        // 收一收后台下载的进度 —— 下完了这项就消失，节点随之自动恢复。
        // 刚下完的节点重算一下「缺不缺模型」缓存（文件刚落到磁盘）。
        for finished in self.downloads.poll() {
            if let Some(node) = self.nodes.iter_mut().find(|node| node.id == finished) {
                node.requirements = crate::catalog::requirements(&node.kind, &node.params);
            }
        }
        // 收一收后台文件选择的结果 —— 选好了就应用到节点上。
        self.apply_pick(ui.ctx());

        // 先把这次的缩略图准备好 —— 卡片高度、预览绘制都要用到。
        self.sync_previews(ui.ctx(), view.report, view.live);
        // 正在跑的那个节点报的进度贴到它自己的卡片上（卡片底部那段信息区）。
        self.sync_step(view.live);
        // 参数说明与运行提示先折好行、量好高 —— 卡片高度靠它，折行也靠它。
        self.sync_notes(ui, kinds);

        self.handle_input(ui, &resp, kinds, now);
        self.advance(now);

        self.draw_canvas(ui, rect, kinds, marks, now, view.show_fps);

        self.pending_run.take()
    }

    /// 有没有还在跑的动画，决定要不要继续申请重绘。
    pub fn is_animating(&self) -> bool {
        self.interaction.slash.is_some()
            || !self.anim.severing.is_empty()
            || self.interaction.connect.is_some()
            // 悬停光晕在呼吸；不持续重绘就会停在一帧上。
            || self.interaction.hovered_port.is_some()
            || !self.anim.entering.is_empty()
            || self.downloads.any()
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
