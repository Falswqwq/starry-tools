//! StarryTools —— 节点式小工具集合。
//!
//! 每个小功能是一个节点，节点有「输入 / 参数 / 输出」三部分。工作流从输入
//! 节点开始，沿着连线把数据喂给后面的工具。整个类型系统和执行引擎都在 Rust
//! 这边，前端只负责渲染和拖拽。

pub mod commands;
pub mod engine;
pub mod error;
pub mod image_io;
pub mod model;
pub mod nodes;
pub mod png_opt;
pub mod png_quant;
pub mod registry;
pub mod storage;

pub use error::AppError;

/// Linux 上让 WebKitGTK 真的走硬件加速合成。
///
/// **为什么**：WebKitGTK 的合成策略默认是「按需」（`OnDemand`）—— 只有页面确实需要
/// （视频、3D、canvas 之类）时才开加速合成。我们这个应用是「一大片 DOM 卡片 +
/// 2D transform」的画布，可能就一直待在软件光栅化那条路上：平移 / 拖动时整屏重新光栅化，
/// 而成本按屏幕像素面积算 —— 所以缩得越小反而越顺。
///
/// **怎么设**：用 WebKit 自己的环境变量（在这台机器的 `libwebkit2gtk-4.1.so.0` 里
/// 确认过它认这个变量）。关键是必须**在建 webview 之前**设好：WebKit 是在构造页面时
/// 读它的，建完之后再调 `set_hardware_acceleration_policy` 已经晚了。
///
/// **前提**：机器得真有一个能用的 GPU。`glxinfo -B` 里如果是 `llvmpipe` / `swrast`，
/// 那就是没硬件可用，强制开也不会变快，只能从「让卡片少画点」入手。
///
/// 环境里已经设过这两个变量的话这里就不插手，所以对照实验还做得成：
///
/// ```sh
/// WEBKIT_DISABLE_COMPOSITING_MODE=1 npm run tauri dev   # 强制关，当对照组
/// ```
#[cfg(target_os = "linux")]
fn prefer_accelerated_compositing() {
    let already_set = std::env::var_os("WEBKIT_FORCE_COMPOSITING_MODE").is_some()
        || std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_some();
    if !already_set {
        std::env::set_var("WEBKIT_FORCE_COMPOSITING_MODE", "1");
    }
}

use tauri::Manager;

/// 全局状态：工作流存在哪、运行产物写在哪。
pub struct AppState {
    pub storage: storage::Storage,
    pub output_root: std::path::PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 必须在 Builder 之前：WebKit 是在建 webview 的时候读这些环境变量的。
    #[cfg(target_os = "linux")]
    prefer_accelerated_compositing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let storage = storage::Storage::new(data_dir.join("workflows"));
            storage.ensure()?;
            let output_root = data_dir.join("outputs");
            std::fs::create_dir_all(&output_root)?;
            app.manage(AppState {
                storage,
                output_root,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::node_kinds,
            commands::create_workflow,
            commands::resolve_workflow,
            commands::run_workflow,
            commands::list_workflows,
            commands::load_workflow,
            commands::save_workflow,
            commands::delete_workflow,
            commands::inspect_image,
            commands::export_file,
            commands::reveal_path,
            commands::app_info,
        ])
        .run(tauri::generate_context!())
        .expect("StarryTools 启动失败");
}
