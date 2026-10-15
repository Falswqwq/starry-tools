//! 原生文件 / 目录选择 —— 在**后台线程**上开，界面不阻塞。
//!
//! 直接把 `rfd::FileDialog`（同步版）放在界面线程上会卡住整帧：对话框开着的时候窗口
//! 完全不再重绘，合成器 / 窗口管理器就会认为应用「未响应」（尤其是 Wayland）。
//! 沿用 rfd 的异步接口在工作线程等待，不推断其他平台内部的主线程派发行为；
//! 选完用一条通道把结果送回来，界面每帧收一次。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// 选完之后要做什么。
pub enum Action {
    /// 把选到的路径塞进某个节点的某个参数。
    SetParam { node_id: String, param_id: String },
    /// 把某个产物文件另存到选到的位置。
    SaveAs { source: PathBuf },
}

/// 对话框要哪种模式。
pub enum Mode {
    File,
    Folder,
    /// 保存对话框，带一个默认文件名。
    Save {
        file_name: String,
    },
}

/// 对话框长什么样 / 要什么。
pub struct Spec {
    pub title: String,
    pub extensions: Vec<String>,
    pub mode: Mode,
}

/// 一次只开一个选择框；结果回来了就交给界面处理。
#[derive(Default)]
pub struct Picker {
    pending: Option<(Action, Receiver<Option<PathBuf>>)>,
}

impl Picker {
    /// 开一个选择框；已经有一个开着就忽略（一次只开一个）。
    pub fn open(&mut self, action: Action, spec: Spec) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = super::ui_context();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| run_dialog(spec)).ok().flatten();
            let _ = tx.send(result);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        self.pending = Some((action, rx));
    }

    /// 收一次结果：选好了就返回「要做什么 + 选到的路径」。
    pub fn poll(&mut self) -> Option<(Action, PathBuf)> {
        let (_, rx) = self.pending.as_ref()?;
        match rx.try_recv() {
            Ok(Some(path)) => {
                let (action, _) = self.pending.take().expect("刚看过它在");
                Some((action, path))
            }
            // 用户取消了，或者线程出了岔子：两种情况都当没选。
            Ok(None) | Err(TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }
}

fn run_dialog(spec: Spec) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(spec.title);
    if !spec.extensions.is_empty() {
        dialog = dialog.add_filter("支持的格式", &spec.extensions);
    }
    match spec.mode {
        Mode::Save { file_name } => {
            dialog = dialog.set_file_name(file_name);
            pollster::block_on(dialog.save_file()).map(|handle| handle.path().to_path_buf())
        }
        Mode::Folder => {
            pollster::block_on(dialog.pick_folder()).map(|handle| handle.path().to_path_buf())
        }
        Mode::File => {
            pollster::block_on(dialog.pick_file()).map(|handle| handle.path().to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action() -> Action {
        Action::SetParam {
            node_id: "node".into(),
            param_id: "file".into(),
        }
    }

    #[test]
    fn completed_dialog_delivers_once() {
        let (tx, rx) = mpsc::channel();
        let mut picker = Picker {
            pending: Some((action(), rx)),
        };
        tx.send(Some(PathBuf::from("picked.png"))).unwrap();
        assert_eq!(picker.poll().unwrap().1, PathBuf::from("picked.png"));
        assert!(picker.poll().is_none());
        assert!(picker.pending.is_none());
    }

    #[test]
    fn cancelled_or_disconnected_dialog_releases_pending() {
        let (tx, rx) = mpsc::channel();
        let mut picker = Picker {
            pending: Some((action(), rx)),
        };
        tx.send(None).unwrap();
        assert!(picker.poll().is_none());
        assert!(picker.pending.is_none());
        let (tx, rx) = mpsc::channel();
        picker.pending = Some((action(), rx));
        drop(tx);
        assert!(picker.poll().is_none());
        assert!(picker.pending.is_none());
    }
}
