//! 图标 —— 直接用 lucide 图标集的**原始路径数据**，不是手描的近似。
//!
//! 每个图标就是 lucide 的 24×24 视口里那几条 `path` / `circle` / `line` / `rect`，
//! 由 [`crate::svgpath`] 折线化后描边画出（线宽 2、圆头，和 lucide 一致）。
//! 这样界面上的图形和原版一模一样，只是换成 egui 来画。

use eframe::egui::{self, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind};

use crate::svgpath;

/// 图标的构成元素，对应 lucide 的 `<path>` / `<circle>` / `<line>` / `<rect>`。
enum El {
    Path(&'static str),
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
    },
}

/// 一个图标绘制函数的签名：往 `rect` 里用 `color` 画。
pub type IconFn = fn(&Painter, Rect, Color32);

struct Pen<'a> {
    painter: &'a Painter,
    rect: Rect,
    scale: f32,
    color: Color32,
    width: f32,
    rotation: f32,
}

impl<'a> Pen<'a> {
    fn new(painter: &'a Painter, rect: Rect, color: Color32, rotation: f32) -> Self {
        let side = rect.width().min(rect.height());
        let scale = side / 24.0;
        Self {
            painter,
            rect: Rect::from_center_size(rect.center(), egui::Vec2::splat(side)),
            scale,
            color,
            width: (2.0 * scale).max(1.0),
            rotation,
        }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        let (mut px, mut py) = (x, y);
        if self.rotation != 0.0 {
            let (s, c) = self.rotation.sin_cos();
            let dx = px - 12.0;
            let dy = py - 12.0;
            px = 12.0 + dx * c - dy * s;
            py = 12.0 + dx * s + dy * c;
        }
        egui::pos2(
            self.rect.left() + px * self.scale,
            self.rect.top() + py * self.scale,
        )
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(self.width, self.color)
    }

    fn polyline(&self, pts: &[(f32, f32)]) {
        if pts.len() < 2 {
            return;
        }
        let path: Vec<Pos2> = pts.iter().map(|p| self.at(p.0, p.1)).collect();
        self.painter.add(egui::Shape::line(path, self.stroke()));
    }

    fn circle(&self, cx: f32, cy: f32, r: f32) {
        self.painter
            .circle_stroke(self.at(cx, cy), r * self.scale, self.stroke());
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, rx: f32) {
        let rect = Rect::from_min_max(self.at(x, y), self.at(x + w, y + h));
        self.painter.rect_stroke(
            rect,
            CornerRadius::same((rx * self.scale).round() as u8),
            self.stroke(),
            StrokeKind::Middle,
        );
    }
}

fn render(painter: &Painter, rect: Rect, color: Color32, els: &[El], rotation: f32) {
    let pen = Pen::new(painter, rect, color, rotation);
    for el in els {
        match el {
            El::Path(d) => {
                for sub in svgpath::flatten(d) {
                    pen.polyline(&sub);
                }
            }
            El::Circle { cx, cy, r } => pen.circle(*cx, *cy, *r),
            El::Line { x1, y1, x2, y2 } => pen.polyline(&[(*x1, *y1), (*x2, *y2)]),
            El::Rect { x, y, w, h, rx } => pen.rect(*x, *y, *w, *h, *rx),
        }
    }
}

// 下面每一条 `d` / 坐标都照抄自 lucide 图标集（v1.49）对应的模块。

const BLOCKS: &[El] = &[
    El::Path("M10 22V7a1 1 0 0 0-1-1H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5a1 1 0 0 0-1-1H2"),
    El::Rect { x: 14.0, y: 2.0, w: 8.0, h: 8.0, rx: 1.0 },
];

const FILE_TEXT: &[El] = &[
    El::Path("M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"),
    El::Path("M14 2v5a1 1 0 0 0 1 1h5"),
    El::Path("M10 9H8"),
    El::Path("M16 13H8"),
    El::Path("M16 17H8"),
];

const FOLDER_OPEN: &[El] = &[El::Path(
    "m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2",
)];

const SAVE: &[El] = &[
    El::Path("M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"),
    El::Path("M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"),
    El::Path("M7 3v4a1 1 0 0 0 1 1h7"),
];

const PLAY: &[El] = &[El::Path(
    "M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z",
)];

const LOADER: &[El] = &[El::Path("M21 12a9 9 0 1 1-6.219-8.56")];

const CHEVRON_UP: &[El] = &[El::Path("m18 15-6-6-6 6")];

const CHEVRON_DOWN: &[El] = &[El::Path("m6 9 6 6 6-6")];

const CHECK: &[El] = &[El::Path("M20 6 9 17l-5-5")];

const TRASH: &[El] = &[
    El::Path("M10 11v6"),
    El::Path("M14 11v6"),
    El::Path("M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"),
    El::Path("M3 6h18"),
    El::Path("M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"),
];

const UNDO: &[El] = &[
    El::Path("M9 14 4 9l5-5"),
    El::Path("M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"),
];

const PLUS: &[El] = &[El::Path("M5 12h14"), El::Path("M12 5v14")];

const ARROW_RIGHT: &[El] = &[El::Path("M5 12h14"), El::Path("m12 5 7 7-7 7")];

const X: &[El] = &[El::Path("M18 6 6 18"), El::Path("m6 6 12 12")];

const FOLDER_SEARCH: &[El] = &[
    El::Path("M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1"),
    El::Path("m21 21-1.9-1.9"),
    El::Circle { cx: 17.0, cy: 17.0, r: 3.0 },
];

const DOWNLOAD: &[El] = &[
    El::Path("M12 15V3"),
    El::Path("M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"),
    El::Path("m7 10 5 5 5-5"),
];

const ZOOM_IN: &[El] = &[
    El::Circle {
        cx: 11.0,
        cy: 11.0,
        r: 8.0,
    },
    El::Line {
        x1: 21.0,
        y1: 21.0,
        x2: 16.65,
        y2: 16.65,
    },
    El::Line {
        x1: 11.0,
        y1: 8.0,
        x2: 11.0,
        y2: 14.0,
    },
    El::Line {
        x1: 8.0,
        y1: 11.0,
        x2: 14.0,
        y2: 11.0,
    },
];

const ZOOM_OUT: &[El] = &[
    El::Circle {
        cx: 11.0,
        cy: 11.0,
        r: 8.0,
    },
    El::Line {
        x1: 21.0,
        y1: 21.0,
        x2: 16.65,
        y2: 16.65,
    },
    El::Line {
        x1: 8.0,
        y1: 11.0,
        x2: 14.0,
        y2: 11.0,
    },
];

const FIT_VIEW: &[El] = &[
    El::Path("M8 3H5a2 2 0 0 0-2 2v3"),
    El::Path("M21 8V5a2 2 0 0 0-2-2h-3"),
    El::Path("M3 16v3a2 2 0 0 0 2 2h3"),
    El::Path("M16 21h3a2 2 0 0 0 2-2v-3"),
];

/// 节点库：四宫格。
pub fn blocks(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, BLOCKS, 0.0);
}

/// 说明：一页纸。
pub fn file_text(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, FILE_TEXT, 0.0);
}

/// 加载：打开的文件夹。
pub fn folder_open(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, FOLDER_OPEN, 0.0);
}

/// 保存：软盘。
pub fn save(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, SAVE, 0.0);
}

/// 运行：空心三角（lucide 是描边不是填充）。
pub fn play(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, PLAY, 0.0);
}

/// 运行中：转圈的弧。`phase` 是 0..1 的相位。
pub fn loader(p: &Painter, r: Rect, c: Color32, phase: f32) {
    render(p, r, c, LOADER, phase * std::f32::consts::TAU);
}

/// 收起：上折角（状态药丸右侧的小箭头）。
pub fn chevron_up(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, CHEVRON_UP, 0.0);
}

/// 展开：下折角（下拉框右侧的小箭头）。
pub fn chevron_down(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, CHEVRON_DOWN, 0.0);
}

/// 展开：下折角，绕自身中心转过 `angle` 弧度（用于「翻转」入场）。
pub fn chevron_down_turned(p: &Painter, r: Rect, c: Color32, angle: f32) {
    render(p, r, c, CHEVRON_DOWN, angle);
}

/// 删除：垃圾桶。
pub fn trash(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, TRASH, 0.0);
}

/// 取消：向左回退的箭头。
pub fn undo(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, UNDO, 0.0);
}

/// 新建：加号。
pub fn plus(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, PLUS, 0.0);
}

/// 卡片上的那个小箭头。
pub fn arrow_right(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, ARROW_RIGHT, 0.0);
}

/// 关闭 / 删除节点：叉。
pub fn x(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, X, 0.0);
}

/// 下拉里选中的那一项前面的对勾。
pub fn check(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, CHECK, 0.0);
}

/// 在文件夹中显示。
pub fn folder_search(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, FOLDER_SEARCH, 0.0);
}

/// 另存为。
pub fn download(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, DOWNLOAD, 0.0);
}

/// 放大（画布控件）。
pub fn zoom_in(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, ZOOM_IN, 0.0);
}

/// 缩小（画布控件）。
pub fn zoom_out(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, ZOOM_OUT, 0.0);
}

/// 适应视图（画布控件）。
pub fn fit_view(p: &Painter, r: Rect, c: Color32) {
    render(p, r, c, FIT_VIEW, 0.0);
}
