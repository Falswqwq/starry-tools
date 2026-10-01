//! 外壳：画布 + 几组浮动控件。
//!
//! 屏幕上除了画布只有浮动控件 —— 没有左右边栏，一切都浮在画布上。

mod catalog;
mod chrome;
mod geometry;
mod graph;
mod icons;
mod library;
mod models;
mod prompt;
mod report;
mod run;
mod settings;
mod svgpath;
mod theme;
mod widgets;
mod workspace;

use eframe::egui;
use egui::CornerRadius;

use catalog::Kind;
use graph::Graph;
use library::Library;
use workspace::Workspace;

/// 动画期间的重绘上限（帧/秒）。空闲时不重绘，所以这只是「动起来时」的上限。
///
/// 整帧的硬上限在 `settings::Settings::max_fps` 里（设置面板可调）—— 它按最小间隔
/// 在帧开头节流，连鼠标事件带来的帧也算在内。
const ANIM_FPS: f32 = 60.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("StarryTools")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([1040.0, 660.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "StarryTools",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// 窗口 / 任务栏图标。和原始应用用的是同一张图，打包时打进二进制。
fn app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(bytes)
        .expect("assets/icon.png 应当是有效 PNG")
        .to_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

struct App {
    /// 全部内置工具，启动时从 core 的注册表读一次。
    kinds: Vec<Kind>,
    graph: Graph,
    workspace: Workspace,
    library: Library,
    chrome: chrome::Chrome,
    /// 静态检查的结果，按画布版本号缓存。
    check: run::Check,
    /// 跑工作流 —— 在后台线程上。
    runner: run::Runner,
    /// 底部状态药丸与运行记录。
    report: report::Report,
    /// 紫色节点停下时的那个交互浮层。
    prompt: prompt::Prompt,
    /// 应用级设置（帧率上限、是否显示 fps）。
    settings: settings::Settings,
    /// 上一帧开始的时刻，用来做帧率节流。
    last_frame: Option<std::time::Instant>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        // 界面不认识任何具体工具：下面这一行就是它了解全部节点的全部途径。
        let kinds = catalog::all();
        let mut workspace = Workspace::open(&kinds);
        let graph = Graph::from_workflow(workspace.workflow(), &kinds);
        workspace.mark_saved(&graph);

        Self {
            library: Library::new(kinds.clone()),
            kinds,
            graph,
            workspace,
            chrome: chrome::Chrome::default(),
            check: run::Check::default(),
            runner: run::Runner::default(),
            report: report::Report::default(),
            prompt: prompt::Prompt::default(),
            settings: settings::Settings::load(),
            last_frame: None,
        }
    }

    /// 帧率节流：距离上一帧不足 `1 / max_fps` 就睡一会儿。
    ///
    /// 这么做的代价是给输入带来最多约一帧的延迟，换的是帧率不再被鼠标事件拉满。
    fn limit_frame_rate(&mut self) {
        let target = std::time::Duration::from_secs_f32(1.0 / self.settings.clamped_fps());
        let now = std::time::Instant::now();
        if let Some(last) = self.last_frame {
            let elapsed = now.duration_since(last);
            if elapsed < target {
                std::thread::sleep(target - elapsed);
            }
        }
        self.last_frame = Some(std::time::Instant::now());
    }

    /// 新建 / 打开之后把画布换成工作区当前那一份。
    fn adopt(&mut self) {
        let graph = Graph::from_workflow(self.workspace.workflow(), &self.kinds);
        self.workspace.mark_saved(&graph);
        self.graph = graph;
    }

    fn apply(&mut self, action: chrome::Action) {
        match action {
            chrome::Action::Save => {
                self.workspace.save(&self.graph);
            }
            chrome::Action::New => {
                self.workspace.new_workflow(&self.kinds);
                self.adopt();
            }
            chrome::Action::Load(id) => {
                if self.workspace.load(&id).is_some() {
                    self.adopt();
                }
            }
            chrome::Action::Run => self.start_run(None),
            chrome::Action::Select(id) => self.select(&id),
        }
    }

    fn select(&mut self, node_id: &str) {
        self.graph.selected = self.graph.nodes.iter().position(|node| node.id == node_id);
    }

    /// 开跑。`only` 是「运行至此」的那个节点，`None` 就是跑整张图。
    fn start_run(&mut self, only: Option<String>) {
        // 新的一次运行：先把上一次留下的缩略图 / 色板 / 状态清掉，
        // 接下来缩略图会随着每个节点跑完一张张出现。
        self.graph.reset_run_marks();
        let workflow = self.graph.to_workflow(self.workspace.workflow());
        let output_root = self.workspace.output_root.clone();
        self.runner.start(workflow, only, output_root);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.limit_frame_rate();
        let ctx = ui.ctx().clone();

        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, CornerRadius::same(0), theme::CANVAS);

        // 静态检查按画布版本号缓存 —— 它会去读输入节点选中的文件（要读文件头），
        // 不能每帧跑。借用在块里结束，免得和下面画布的 `&mut self` 撞上。
        let workflow = self.graph.to_workflow(self.workspace.workflow());
        let (mut marks, runnable, errors, first_bad) = {
            let resolved = self.check.get(self.graph.revision, &workflow);
            let bad = resolved
                .issues
                .iter()
                .find(|issue| {
                    matches!(issue.severity, starrytools_core::engine::Severity::Error)
                        && issue.node_id.is_some()
                })
                .and_then(|issue| issue.node_id.clone());
            (
                run::Marks::build(resolved, self.runner.report()),
                resolved.runnable,
                self.check.errors(),
                bad,
            )
        };
        // 点过空白处之后，把「跑过了」的高亮收起来。
        if self.graph.run_marks_hidden() {
            marks.dismiss_ok_highlight();
        }
        // 正在等用户操作的紫色节点，画布要把它高亮。
        marks.waiting = self.runner.waiting().map(|request| request.node_id.clone());
        // 运行进行中的实时进度：跑到哪儿、哪些已经跑完（带各自的结果）。
        if let Some(live) = self.runner.live() {
            marks.running = live.running.clone();
            for (id, result) in &live.done {
                match result.status {
                    starrytools_core::engine::NodeStatus::Ok => {
                        marks.ok_nodes.insert(id.clone());
                    }
                    starrytools_core::engine::NodeStatus::Failed => {
                        marks.failed_nodes.insert(id.clone());
                    }
                    starrytools_core::engine::NodeStatus::Skipped => {}
                }
                marks.node_ms.entry(id.clone()).or_insert(result.elapsed_ms);
            }
        }

        let view = graph::RunView {
            report: self.runner.report(),
            live: self.runner.live(),
            show_fps: self.settings.show_fps,
        };
        if let Some(node_id) = self.graph.ui(ui, rect, &self.kinds, &marks, view) {
            self.start_run(Some(node_id));
        }
        // Ctrl+S：画布把请求挂出来，这里执行落盘。
        if self.graph.take_save_request() {
            self.workspace.save(&self.graph);
        }

        // ---- 左上 / 右上：浮动控件 ----
        let dirty = self.workspace.is_dirty(&self.graph);
        let running = self.runner.is_running();
        let nodes = self.graph.nodes.len();
        let edges = self.graph.wires.len();
        if let Some(action) = self.chrome.ui(
            &ctx,
            &mut self.workspace,
            &mut self.library,
            &mut self.settings,
            dirty,
            running,
            errors,
            first_bad.as_deref(),
            nodes,
            edges,
        ) {
            self.apply(action);
        }

        // ---- 节点库浮层 ----
        if let Some(drop) = self.library.ui(&ctx) {
            let now = ctx.input(|input| input.time);
            self.graph
                .add_node_at(drop.screen_pos, &self.kinds[drop.kind], now);
        }

        // ---- 紫色节点的交互浮层 ----
        self.prompt.ui(&ctx, &mut self.runner);

        // ---- 底部：状态药丸 + 运行记录 ----
        self.runner.poll();
        let pick = self.report.ui(&ctx, &mut self.runner, runnable, errors);
        if let Some(report::Action::Select(id)) = pick {
            self.select(&id);
        }
        // 运行记录被清空了：画布上的运行痕迹也一并抹掉。
        if self.runner.take_reset() {
            self.graph.reset_run_marks();
        }

        // ---- 需要时继续重绘 ----
        // （动画本身会自己请求重绘；这里只补上「时间在走」的那种：运行中、刀光未散。）
        //
        // 用 `request_repaint_after` 而不是 `request_repaint`：空闲时根本不会重绘，
        // 而动画期间也只钉在 ANIM_FPS 这一档，不至于在高刷屏上把 CPU 拉满。
        if running || self.graph.is_animating() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.0 / ANIM_FPS));
        }
    }
}
