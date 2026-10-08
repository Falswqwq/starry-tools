//! 节点的尺寸与纵向排版。
//!
//! 「一张卡片多高、每段占哪几行、参数控件落在哪」的算法都在这里 —— 绘制、命中、
//! 端口定位、卡片高度全部读它，所以只有一处说了算。这些函数不碰画布状态（平移 / 缩放 /
//! 交互），只按节点当前的状态算尺寸。

use std::collections::HashMap;

use eframe::egui::{self, Color32, FontId};
use serde_json::Value;
use starrytools_core::model::params::Params;

use crate::canvas::graph::{
    BODY_PAD, HEADER_H, MODEL_PANEL_H, NODE_W, NOTICE_GAP, NOTICE_ICON, NOTICE_PAD, NOTICE_STACK,
    PALETTE_H, PARAMS_BOTTOM, PARAMS_GAP, PARAM_BOOL_H, PARAM_GAP, PARAM_LABEL_GAP, PARAM_LABEL_H,
    PARAM_NOTE_GAP, PARAM_TEXT_LINE, PORT_ROW_H, PREVIEW_H, PROGRESS_H, RUN_ACTIONS_H,
    TOOL_PANEL_H,
};
use crate::canvas::node::{kind_of, Node};
use crate::catalog::{Control, Kind, Param};
use crate::ui::controls::{color_height, PARAM_CONTROL_H};

/// 一段折好行的文字：每行文本、每行高度、总高。都在流坐标下（未乘缩放）。
#[derive(Clone)]
pub(crate) struct TextBlock {
    pub(crate) lines: Vec<String>,
    pub(crate) rows: Vec<f32>,
    pub(crate) height: f32,
}

/// 说明 / 提示用的字号（流坐标）。
const NOTE_FONT: f32 = 9.5;

/// 提示文字能用的宽度（流坐标）—— 减掉图标那一截。
pub(crate) fn notice_text_width() -> f32 {
    NODE_W - 20.0 - NOTICE_ICON - NOTICE_GAP
}

/// 卡片在参数区**下面**那几段各自的高度（流坐标）。
///
/// 一次算好，算总高（[`height_of_node`]）、决定参数区要不要收圆角（[`params_following`]）、
/// 以及 [`Graph::draw_node`] 往下画，全读这一份 —— 段数、顺序、高度只有一处说了算，
/// 不会再出现「高度算了一套、画的是另一套」。
#[derive(Clone, Copy, Default)]
pub(crate) struct CardSections {
    /// 「缺东西」面板（缺模型 / 缺外部程序），紧贴参数区。
    pub(crate) requirement: f32,
    /// 运行中的进度块（帧计数 + 进度条）。
    pub(crate) progress: f32,
    pub(crate) preview: f32,
    pub(crate) palette: f32,
    pub(crate) warn: f32,
    pub(crate) error: f32,
    pub(crate) actions: f32,
}

impl CardSections {
    pub(crate) fn total(&self) -> f32 {
        self.requirement
            + self.progress
            + self.preview
            + self.palette
            + self.warn
            + self.error
            + self.actions
    }
}

/// 按当前状态把卡片参数区以下各段的高度算出来。
pub(crate) fn card_sections(
    kinds: &[Kind],
    node: &Node,
    notes: &HashMap<String, TextBlock>,
) -> CardSections {
    let (warn, error, actions) = run_extra(node, notes);
    CardSections {
        requirement: requirement_panel_height(kinds, node),
        progress: if node.step.is_some() { PROGRESS_H } else { 0.0 },
        preview: if node.preview { PREVIEW_H } else { 0.0 },
        palette: if node
            .palette
            .as_ref()
            .is_some_and(|colors| !colors.is_empty())
        {
            PALETTE_H
        } else {
            0.0
        },
        warn,
        error,
        actions,
    }
}

/// 一个节点的完整高度（流坐标）。
pub(crate) fn height_of_node(
    kinds: &[Kind],
    node: &Node,
    notes: &HashMap<String, TextBlock>,
) -> f32 {
    let sections = card_sections(kinds, node, notes);
    // 「缺东西」面板紧贴参数区，中间不留 `BODY_PAD` 那条缝；其余段与参数区之间才留。
    let gap = if sections.requirement > 0.0 {
        0.0
    } else if sections.total() > 0.0 {
        BODY_PAD
    } else {
        0.0
    };
    node_body_top(node) + params_height_of(kinds, node, notes) + sections.total() + gap
}

/// 「缺东西」面板占多高（什么都不缺时是 0）：缺模型给下载面板，缺外部程序给一句提示。
///
/// 判定读的是节点上缓存的 [`Node::requirements`]（见 `refresh_ports`），
/// 绘制路径上不碰磁盘。
pub(crate) fn requirement_panel_height(_kinds: &[Kind], node: &Node) -> f32 {
    if node.requirements.model_missing {
        MODEL_PANEL_H
    } else if node.requirements.tool_missing {
        TOOL_PANEL_H
    } else {
        0.0
    }
}

/// 端口区下沿（相对卡片顶部，流坐标）—— 也就是参数区的上沿。
pub(crate) fn node_body_top(node: &Node) -> f32 {
    // 只数**声明**的输入端口：参数端口画在参数那一行，不占端口列，也就
    // 不撑高端口区 —— 否则灰底参数框会比里面的控件低一整行。
    let declared = node.inputs.iter().filter(|port| !port.is_param()).count();
    let rows = declared.max(node.outputs.len()).max(1) as f32;
    HEADER_H + rows * PORT_ROW_H
}

/// 一个可见参数块在卡片里的纵向位置（**流坐标，相对卡片顶部**）。
#[derive(Clone)]
pub(crate) struct ParamSlot {
    pub(crate) id: String,
    /// 标签行顶部（没标签时为 `None`）。
    pub(crate) label_y: Option<f32>,
    /// 控件顶部与高度。
    pub(crate) control_y: f32,
    pub(crate) control_h: f32,
    /// 说明行顶部（没说明时为 `None`）。
    pub(crate) note_y: Option<f32>,
    /// 参数端口那个小圆点的中心。
    pub(crate) dot_y: f32,
}

/// 可见参数的纵向排版：每个参数块占哪几行。
///
/// **绘制控件**（[`Graph::draw_node_controls`]）和**参数端口圆点**（[`Graph::param_port_offset`]）
/// 都读这一份 —— 圆点于是永远落在它那个参数的控件旁边，不会再出现两套并行排版对不上的问题。
pub(crate) fn param_slots(
    node: &Node,
    kind: &Kind,
    notes: &HashMap<String, TextBlock>,
) -> Vec<ParamSlot> {
    let mut y = node_body_top(node) + PARAMS_GAP;
    let mut slots = Vec::new();
    for param in &kind.params {
        if !param.visible(&node.params) {
            continue;
        }
        // 开关：标签和开关同一行，圆点对在这一行正中。
        if matches!(param.control, Control::Bool) {
            slots.push(ParamSlot {
                id: param.id.clone(),
                label_y: None,
                control_y: y,
                control_h: PARAM_BOOL_H,
                note_y: None,
                dot_y: y + PARAM_BOOL_H * 0.5,
            });
            y += PARAM_BOOL_H + PARAM_GAP;
            continue;
        }

        // 其余是「标签一行 + 控件一行」。
        let label_y = has_label(param).then_some(y);
        if label_y.is_some() {
            y += PARAM_LABEL_H + PARAM_LABEL_GAP;
        }
        let control_y = y;
        let control_h = control_height(param, &node.params);
        // 圆点对在标签那一行的正中；没标签就对控件正中。
        let dot_y = match label_y {
            Some(ly) => ly + PARAM_LABEL_H * 0.5,
            None => control_y + control_h * 0.5,
        };
        y += control_h;

        let note_y = if let Some(note) = &param.description {
            y += PARAM_NOTE_GAP;
            let top = y;
            y += note_height(note, notes);
            Some(top)
        } else {
            None
        };
        slots.push(ParamSlot {
            id: param.id.clone(),
            label_y,
            control_y,
            control_h,
            note_y,
            dot_y,
        });
        y += PARAM_GAP;
    }
    slots
}

/// 运行痕迹那几段各占多高：`(提示, 错误, 产物操作行)`。
///
/// 提示 / 错误都会折行，所以高度按实测的折行高度算 —— 否则长提示会被卡片的
/// 下边框截掉（“图像压缩”运行后那行小结就是这么溢出的）。
pub(crate) fn run_extra(node: &Node, notes: &HashMap<String, TextBlock>) -> (f32, f32, f32) {
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

/// 参数区占多高。被 `visible_when` 藏掉的不占地方。
pub(crate) fn params_height(
    kind: &Kind,
    params: &Params,
    notes: &HashMap<String, TextBlock>,
) -> f32 {
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

/// 多行文本框内可用的宽度（流坐标）：卡片宽 - 卡片内边距 - 文本框自身的内缩。
const TEXT_AREA_WIDTH: f32 = NODE_W - 20.0 - 16.0;
/// 多行文本框的最少 / 最多行数。超过最多行数就不再长高，改成框内滚动。
const MULTILINE_MIN_ROWS: usize = 3;
const MULTILINE_MAX_ROWS: usize = 20;

/// 一段文本在多行文本框里折成几行（粗略估计，汉字按整字宽、半角按约 0.6 字宽）。
/// **不封顶** —— 卡不卡在上下限由调用方决定。
fn wrapped_rows(text: &str) -> usize {
    const FONT: f32 = 11.5;
    let mut rows = 0usize;
    for line in text.split('\n') {
        let mut used = 0.0f32;
        let mut line_rows = 1usize;
        for ch in line.chars() {
            let w = if ch.is_ascii() { FONT * 0.6 } else { FONT };
            if used > 0.0 && used + w > TEXT_AREA_WIDTH {
                line_rows += 1;
                used = 0.0;
            }
            used += w;
        }
        rows += line_rows;
    }
    rows
}

/// 一段文本在多行文本框里占几行（估计），夹在 3– 20 行之间。
///
/// 绘制（[`text_field`] 的 `desired_rows`）和估高（[`multiline_height`]）都用这一个数 ——
/// 框才会跟着内容长，也不会超出预留的位置。
pub(crate) fn multiline_rows(text: &str) -> usize {
    wrapped_rows(text).clamp(MULTILINE_MIN_ROWS, MULTILINE_MAX_ROWS)
}

/// 内容已经超过最多行数了吗 —— 超过之后框不再长高，改成框内滚动。
pub(crate) fn multiline_scrolls(text: &str) -> bool {
    wrapped_rows(text) > MULTILINE_MAX_ROWS
}

/// 多行文本框要占多高（跟随内容）。
///
/// 按**折行后的可视行数**算：长文本换行会把框撑高，到 20 行为止；再长就在框内滚动。
pub(crate) fn multiline_height(text: &str) -> f32 {
    multiline_rows(text) as f32 * PARAM_TEXT_LINE + 10.0
}

/// 一段会折行的文字的**总高**（流坐标）。优先用实测值（`sync_notes` 量的）；
/// 还没量到（比如刚从节点库拖出来的那一帧）就退回一个粗略估计。
pub(crate) fn wrapped_height(text: &str, width: f32, notes: &HashMap<String, TextBlock>) -> f32 {
    notes
        .get(text)
        .map(|block| block.height)
        .unwrap_or_else(|| estimate_wrapped_height(text, width))
}

/// 一句参数说明占多高（整宽）。
pub(crate) fn note_height(note: &str, notes: &HashMap<String, TextBlock>) -> f32 {
    wrapped_height(note, NODE_W - 20.0, notes)
}

/// 一条运行提示占多高（要减掉前面那个图标）。
pub(crate) fn notice_height(note: &str, notes: &HashMap<String, TextBlock>) -> f32 {
    wrapped_height(note, notice_text_width(), notes)
}

/// 按固定字号、给定宽度折好行，量出行高与总高。
pub(crate) fn measure_block(
    painter: &egui::Painter,
    text: &str,
    width: f32,
    color: Color32,
) -> TextBlock {
    let galley = painter.layout(text.to_owned(), FontId::monospace(NOTE_FONT), color, width);
    TextBlock {
        lines: galley.rows.iter().map(|row| row.text()).collect(),
        rows: galley.rows.iter().map(|row| row.rect().height()).collect(),
        height: galley.size().y,
    }
}

/// 画一段折好的文字：逐行按 `zoom` 缩放，不再重新折行（位置与预留高度完全一致）。
pub(crate) fn draw_block(
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
pub(crate) fn estimate_wrapped_height(text: &str, width: f32) -> f32 {
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
pub(crate) fn control_height(param: &Param, params: &Params) -> f32 {
    match &param.control {
        Control::Text {
            multiline: true, ..
        } => {
            let text = params.get(&param.id).and_then(Value::as_str).unwrap_or("");
            multiline_height(text)
        }
        Control::Bool => PARAM_BOOL_H,
        Control::Color => color_height(),
        _ => PARAM_CONTROL_H,
    }
}

/// 一个参数（标签 + 控件 + 说明）一共占多高。
pub(crate) fn param_block_height(
    param: &Param,
    params: &Params,
    notes: &HashMap<String, TextBlock>,
) -> f32 {
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
pub(crate) fn has_label(param: &Param) -> bool {
    !param.label.is_empty()
}

pub(crate) fn params_height_of(
    kinds: &[Kind],
    node: &Node,
    notes: &HashMap<String, TextBlock>,
) -> f32 {
    match kind_of(kinds, node) {
        Some(kind) => params_height(kind, &node.params, notes),
        None => 0.0,
    }
}
