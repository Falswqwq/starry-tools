//! 直接消费 core 的节点、参数和端口契约；这里只保留 UI 专属行为。

use serde_json::Value;
use starrytools_core::bg_model;
use starrytools_core::model::node_kind::{NodeKindInfo, ParamDef, ParamSpec, PortDef};
use starrytools_core::model::params::Params;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use starrytools_core::registry::registry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    Pending,
    Available,
    Missing,
}

type ToolProbe = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
struct ToolChecks {
    states: Mutex<HashMap<String, ToolStatus>>,
    revision: AtomicU64,
}

impl ToolChecks {
    fn request(
        self: &Arc<Self>,
        id: &str,
        probe: Option<ToolProbe>,
        ctx: Option<eframe::egui::Context>,
    ) -> ToolStatus {
        let mut states = self.states.lock().expect("tool snapshots");
        if let Some(status) = states.get(id) {
            return *status;
        }
        let Some(probe) = probe else {
            states.insert(id.to_owned(), ToolStatus::Missing);
            return ToolStatus::Missing;
        };
        states.insert(id.to_owned(), ToolStatus::Pending);
        drop(states);
        let checks = self.clone();
        let id = id.to_owned();
        std::thread::spawn(move || {
            // Never hold the snapshot lock while a process/OnceLock is running.
            let available =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe())).unwrap_or(false);
            let status = if available {
                ToolStatus::Available
            } else {
                ToolStatus::Missing
            };
            checks
                .states
                .lock()
                .expect("tool snapshots")
                .insert(id, status);
            checks.revision.fetch_add(1, Ordering::Release);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        ToolStatus::Pending
    }
}

fn tool_checks() -> &'static Arc<ToolChecks> {
    static CHECKS: OnceLock<Arc<ToolChecks>> = OnceLock::new();
    CHECKS.get_or_init(|| Arc::new(ToolChecks::default()))
}

pub(crate) fn tool_status(id: &str, ctx: Option<eframe::egui::Context>) -> ToolStatus {
    tool_checks().request(id, registry().tool(id).map(|tool| tool.probe.clone()), ctx)
}

pub(crate) fn tool_revision() -> u64 {
    tool_checks().revision.load(Ordering::Acquire)
}

pub(crate) fn refresh_tool_requirement(
    kind_id: &str,
    requirement: &mut crate::canvas::node::Requirements,
) {
    let status = registry()
        .get(kind_id)
        .and_then(|spec| spec.requires_tool.as_deref())
        .map(|tool| tool_status(tool, crate::state::ui_context()));
    requirement.tool_missing = status == Some(ToolStatus::Missing);
    requirement.tool_checking = status == Some(ToolStatus::Pending);
}

/// 节点库展开卡片的控件摘要，不属于可序列化的参数契约。
pub trait ParamSummary {
    fn summary(&self) -> String;
}

impl ParamSummary for ParamDef {
    fn summary(&self) -> String {
        match &self.spec {
            ParamSpec::Number {
                min,
                max,
                unit,
                integer,
                ..
            }
            | ParamSpec::Slider {
                min,
                max,
                unit,
                integer,
                ..
            } => {
                let kind = if *integer { "整数" } else { "数值" };
                format!("{kind} {min}–{max}{}", unit.as_deref().unwrap_or(""))
            }
            ParamSpec::Text { multiline, .. } => {
                if *multiline { "多行文本" } else { "文本" }.to_string()
            }
            ParamSpec::Select { options, .. } => format!(
                "下拉：{}",
                options
                    .iter()
                    .map(|choice| choice.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
            ParamSpec::Color { .. } => "颜色".to_string(),
            ParamSpec::Bool { .. } => "开关".to_string(),
            ParamSpec::Ports { .. } => "动态输出（可增删）".to_string(),
            ParamSpec::File {
                directory,
                extensions,
                ..
            } => {
                if *directory {
                    "选择目录".to_string()
                } else if extensions.is_empty() {
                    "选择文件".to_string()
                } else {
                    format!("选择文件（{}）", extensions.join(" / "))
                }
            }
        }
    }
}

pub trait KindRequirements {
    fn model(&self, params: &Params) -> Option<&'static bg_model::Model>;
}

impl KindRequirements for NodeKindInfo {
    fn model(&self, params: &Params) -> Option<&'static bg_model::Model> {
        let param = self.model_param.as_deref()?;
        let id = params
            .get(param)
            .and_then(Value::as_str)
            .unwrap_or_else(|| bg_model::default_model().id);
        bg_model::find(id)
    }
}

pub fn all() -> Vec<NodeKindInfo> {
    registry().kinds()
}

/// UI 仅计算廉价的声明端口；动态输出由后台 Check 的 ResolvedNode 快照覆盖。
pub fn ports_for(kind_id: &str, params: &Params) -> (Vec<PortDef>, Vec<PortDef>) {
    match registry().get(kind_id) {
        Some(spec) => {
            let outputs = spec.outputs_for(params);
            (spec.inputs_for(params), outputs)
        }
        None => (Vec::new(), Vec::new()),
    }
}

/// 只在参数变化或下载完成后重算，不在绘制路径重复探测磁盘/工具。
pub fn requirements(kind_id: &str, params: &Params) -> crate::canvas::node::Requirements {
    let mut missing = crate::canvas::node::Requirements::default();
    let Some(spec) = registry().get(kind_id) else {
        return missing;
    };
    if let Some(param) = spec.model_param.as_deref() {
        let id = params
            .get(param)
            .and_then(Value::as_str)
            .unwrap_or_else(|| bg_model::default_model().id);
        missing.model_missing =
            bg_model::find(id).is_none_or(|model| !bg_model::is_downloaded(model.id));
    }
    refresh_tool_requirement(kind_id, &mut missing);
    missing
}

pub fn is_interactive(kind_id: &str, params: &Params) -> bool {
    registry().is_interactive(kind_id, params)
}

/// 返回类别 ID；显示名称由 `registry().category(id).label` 获取。
pub fn categories(kinds: &[NodeKindInfo]) -> Vec<String> {
    let mut out = vec!["全部".to_string()];
    for info in kinds {
        if !out.contains(&info.kind.category) {
            out.push(info.kind.category.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn slow_probe_does_not_block_snapshots_or_spawn_twice_and_wakes_ui() {
        let checks = Arc::new(ToolChecks::default());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let calls = Arc::new(AtomicU64::new(0));
        let probe_calls = calls.clone();
        let probe: ToolProbe = Arc::new(move || {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            started_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            true
        });
        let ctx = eframe::egui::Context::default();
        let input = eframe::egui::RawInput::default();
        for _ in 0..2 {
            let _ = ctx.run_logic(&input, |_| {});
        }
        let (wake_tx, wake_rx) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = wake_tx.send(());
        });
        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let ui_checks = checks.clone();
        std::thread::spawn(move || {
            let first = ui_checks.request("slow", Some(probe.clone()), Some(ctx.clone()));
            let repeat = ui_checks.request("slow", Some(probe), Some(ctx));
            let unknown = ui_checks.request("unknown", None, None);
            snapshot_tx.send((first, repeat, unknown)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let snapshots = snapshot_rx.recv_timeout(Duration::from_secs(2));
        // Release even on failure, so a regression cannot strand the worker.
        release_tx.send(()).unwrap();
        assert_eq!(
            snapshots.unwrap(),
            (
                ToolStatus::Pending,
                ToolStatus::Pending,
                ToolStatus::Missing
            )
        );
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(checks.revision.load(Ordering::Acquire), 1);
        assert_eq!(checks.request("slow", None, None), ToolStatus::Available);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn failed_probe_finishes_as_missing_without_retry() {
        let checks = Arc::new(ToolChecks::default());
        assert_eq!(
            checks.request("missing", Some(Arc::new(|| false)), None),
            ToolStatus::Pending
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while checks.revision.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(checks.revision.load(Ordering::Acquire), 1);
        assert_eq!(
            checks.request(
                "missing",
                Some(Arc::new(|| panic!("no second probe"))),
                None
            ),
            ToolStatus::Missing
        );
        assert_eq!(
            tool_status("not-a-registered-tool", None),
            ToolStatus::Missing
        );
    }
}
