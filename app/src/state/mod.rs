//! 应用状态与服务：工作区、设置、运行、模型下载、文件选择。

pub(crate) mod dialog;
pub(crate) mod models;
pub(crate) mod pacing;
pub(crate) mod run;
pub(crate) mod settings;
pub(crate) mod workspace;

// Graph 内部按需创建 Picker/Downloads；构造边界注入 UI 上下文，工作线程只持有克隆。
// 线程局部避免跨窗口/测试共享可变状态，不在后台查找当前 UI。
thread_local! {
    static UI_CONTEXT: std::cell::RefCell<Option<eframe::egui::Context>> = const { std::cell::RefCell::new(None) };
}

pub fn bind_ui(ctx: &eframe::egui::Context) {
    UI_CONTEXT.with(|slot| *slot.borrow_mut() = Some(ctx.clone()));
}

pub(super) fn ui_context() -> Option<eframe::egui::Context> {
    UI_CONTEXT.with(|slot| slot.borrow().clone())
}
