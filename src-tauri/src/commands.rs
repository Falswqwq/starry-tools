//! 暴露给前端的命令。前端能做的事，这里都能找到对应的一条。

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::engine::{self, ResolvedWorkflow, RunReport};
use crate::error::AppError;
use crate::image_io::ImageInfo;
use crate::model::node_kind::NodeKindInfo;
use crate::model::workflow::{NodeInstance, Position, Workflow, WorkflowSummary};
use crate::{nodes, registry, AppState};

/// 传给前端的缩略图长边上限。
const PREVIEW_MAX_DIM: u32 = 256;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub workflow_dir: String,
    pub output_dir: String,
}

#[tauri::command]
pub fn node_kinds() -> Vec<NodeKindInfo> {
    registry::registry().kinds()
}

/// 新建一个工作流。默认自带一个输入节点 —— 所有工作流都从输入开始。
#[tauri::command]
pub fn create_workflow(name: Option<String>) -> Workflow {
    let mut workflow = Workflow::new(name.unwrap_or_else(|| "未命名工作流".to_string()));
    if let Some(spec) = registry::registry().get(nodes::input::KIND) {
        workflow.nodes.push(NodeInstance {
            id: uuid::Uuid::new_v4().to_string(),
            kind: nodes::input::KIND.to_string(),
            position: Position { x: 96.0, y: 168.0 },
            params: spec.kind.default_params(),
        });
    }
    workflow
}

/// 静态检查。编辑器每改一下都调它，用来画端口、判连线、标红。
#[tauri::command]
pub fn resolve_workflow(workflow: Workflow) -> ResolvedWorkflow {
    engine::resolve(&workflow)
}

/// 执行工作流。CPU 活比较重，放到阻塞线程池里，别占着异步运行时。
#[tauri::command]
pub async fn run_workflow(
    state: State<'_, AppState>,
    workflow: Workflow,
    only_node: Option<String>,
) -> Result<RunReport, AppError> {
    let output_root = state.output_root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine::run(&workflow, only_node.as_deref(), &output_root)
    })
    .await
    .map_err(|err| AppError::msg(format!("运行线程异常退出：{err}")))?
}

#[tauri::command]
pub async fn list_workflows(state: State<'_, AppState>) -> Result<Vec<WorkflowSummary>, AppError> {
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || storage.list())
        .await
        .map_err(|err| AppError::msg(format!("读取失败：{err}")))?
}

#[tauri::command]
pub async fn load_workflow(state: State<'_, AppState>, id: String) -> Result<Workflow, AppError> {
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || storage.load(&id))
        .await
        .map_err(|err| AppError::msg(format!("读取失败：{err}")))?
}

#[tauri::command]
pub async fn save_workflow(
    state: State<'_, AppState>,
    workflow: Workflow,
) -> Result<Workflow, AppError> {
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || storage.save(&workflow))
        .await
        .map_err(|err| AppError::msg(format!("保存失败：{err}")))?
}

#[tauri::command]
pub async fn delete_workflow(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || storage.delete(&id))
        .await
        .map_err(|err| AppError::msg(format!("删除失败：{err}")))?
}

/// 看一眼某个文件是什么图像。用户选完文件后调用，用来显示缩略图和尺寸。
#[tauri::command]
pub async fn inspect_image(path: String) -> Result<ImageInfo, AppError> {
    tauri::async_runtime::spawn_blocking(move || ImageInfo::inspect(Path::new(&path), PREVIEW_MAX_DIM))
        .await
        .map_err(|err| AppError::msg(format!("读取图像失败：{err}")))?
}

/// 把运行产物复制到用户挑的位置。
#[tauri::command]
pub async fn export_file(source: String, destination: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = PathBuf::from(&source);
        if !source.exists() {
            return Err(AppError::msg(format!(
                "找不到源文件：{}（可能上一次运行已经把它覆盖了）",
                source.display()
            )));
        }
        std::fs::copy(&source, &destination)?;
        Ok(destination)
    })
    .await
    .map_err(|err| AppError::msg(format!("导出失败：{err}")))?
}

/// 在系统文件管理器里定位一个文件，或打开一个目录。
#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: String) -> Result<(), AppError> {
    use tauri_plugin_opener::OpenerExt;

    let target = PathBuf::from(&path);
    if !target.exists() {
        return Err(AppError::msg(format!("路径不存在：{path}")));
    }
    let opener = app.opener();
    if target.is_dir() {
        opener
            .open_path(target.to_string_lossy().to_string(), None::<String>)
            .map_err(map_opener_err)?;
    } else {
        opener
            .reveal_item_in_dir(&target)
            .map_err(map_opener_err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "StarryTools".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        workflow_dir: state.storage.root().to_string_lossy().to_string(),
        output_dir: state.output_root.to_string_lossy().to_string(),
    }
}

fn map_opener_err(err: tauri_plugin_opener::Error) -> AppError {
    AppError::msg(format!("打不开：{err}"))
}
