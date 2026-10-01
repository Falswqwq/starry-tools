//! 外壳：画布 + 几组浮动控件。
//!
//! 屏幕上除了画布只有浮动控件 —— 没有左右边栏，一切都浮在画布上。

mod catalog;
mod chrome;
mod geometry;
mod graph;
mod icons;
mod library;
mod report;
mod run;
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
        }
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
        let workflow = self.graph.to_workflow(self.workspace.workflow());
        let output_root = self.workspace.output_root.clone();
        self.runner.start(workflow, only, output_root);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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

        if let Some(node_id) = self
            .graph
            .ui(ui, rect, &self.kinds, &marks, self.runner.report())
        {
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

        // ---- 底部：状态药丸 + 运行记录 ----
        self.runner.poll();
        let pick = self.report.ui(&ctx, &mut self.runner, runnable, errors);
        if let Some(report::Action::Select(id)) = pick {
            self.select(&id);
        }

        // ---- 需要时继续重绘 ----
        // （动画本身会自己请求重绘；这里只补上「时间在走」的那种：运行中、刀光未散。）
        if running || self.graph.is_animating() {
            ctx.request_repaint();
        }
    }
}
