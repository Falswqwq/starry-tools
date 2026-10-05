//! 图标 —— 直接用 vendored 的 **lucide 原始 SVG**（`assets/icons/*.svg`）。
//!
//! 画法不是「描边 + 圆头」那套缝缝补补，而是从原理上做对：
//!
//! 1. 把 SVG 里的路径折线化成**中心线**；
//! 2. 对每个像素求「到中心线的最短距离」；
//! 3. `距离 < 线宽/2` 就是实心，边界一个像素内做过渡。
//!
//! 这一步天然给出 lucide 要的**圆头端点、圆角连接和抗锯齿** —— 因为「描边」的定义
//! 本来就是这个距离场。也不受 epaint 固定像素羽化的影响。
//!
//! 结果按**实际显示尺寸**光栅化成一张贴图（缓存起来），画的时候直接贴 —— 每个尺寸
//! 都是按那个尺寸算的，不会糊也不会抖。着色用贴图 tint，所以一张白模可以染成任意颜色。

use eframe::egui::{self, Color32, Pos2, Rect};

use crate::ui::svgpath;

/// lucide 的视口边长。
const VIEW: f32 = 24.0;
/// lucide 的 `stroke-width="2"`，半宽就是 1。
const HALF_WIDTH: f32 = 1.0;
/// 贴图最大边长（按物理像素）—— 够覆盖节点缩放到 2× 的情况。
const MAX_PX: usize = 96;
/// 抗锯齿过渡带的宽度（单位：像素）。
///
/// 取 1.0 相当于「真实覆盖」的近似，但 lucide 的 2/24 线宽在 14px 的图标里只有 1.2px，
/// 过渡带比线本身还宽，于是没有一格是实心的 —— 看着就发虚。收窄到 0.75 折中一下：
/// 细线能有一点实心芯，又不会明显见锯齿。
const EDGE: f32 = 0.75;

/// 一个图标绘制函数的签名：往 `rect` 里用 `color` 画。
pub type IconFn = fn(&egui::Painter, Rect, Color32);

/// SVG 里的基本形状。
enum El {
    Path(String),
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

// ---------------------------------------------------------------------------
// SVG 解析（只认 lucide 会用到的那几种形状）
// ---------------------------------------------------------------------------

/// 取 `<tag>` 里名为 `key` 的属性值。属性名要独立（`x` 不会匹配到 `x1`）。
fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while let Some(found) = tag[i..].find(key) {
        let start = i + found;
        let after = start + key.len();
        let name_ok = start == 0 || matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r');
        if name_ok && bytes.get(after) == Some(&b'=') {
            let quote = *bytes.get(after + 1)?;
            let vstart = after + 2;
            if let Some(len) = tag[vstart..].find(quote as char) {
                return Some(&tag[vstart..vstart + len]);
            }
        }
        i = start + 1;
        if i >= tag.len() {
            break;
        }
    }
    None
}

fn num(tag: &str, key: &str, fallback: f32) -> f32 {
    attr(tag, key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(fallback)
}

fn parse(svg: &str) -> Vec<El> {
    let mut els = Vec::new();
    let mut rest = svg;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        let name = tag.split([' ', '\t', '\n', '\r']).next().unwrap_or("");
        match name {
            "path" => {
                if let Some(d) = attr(tag, "d") {
                    els.push(El::Path(d.to_string()));
                }
            }
            "circle" => els.push(El::Circle {
                cx: num(tag, "cx", 0.0),
                cy: num(tag, "cy", 0.0),
                r: num(tag, "r", 0.0),
            }),
            "line" => els.push(El::Line {
                x1: num(tag, "x1", 0.0),
                y1: num(tag, "y1", 0.0),
                x2: num(tag, "x2", 0.0),
                y2: num(tag, "y2", 0.0),
            }),
            "rect" => els.push(El::Rect {
                x: num(tag, "x", 0.0),
                y: num(tag, "y", 0.0),
                w: num(tag, "width", 0.0),
                h: num(tag, "height", 0.0),
                rx: num(tag, "rx", 0.0),
            }),
            _ => {}
        }
    }
    els
}

// ---------------------------------------------------------------------------
// 中心线：形状 → 线段 / 圆
// ---------------------------------------------------------------------------

/// 要算距离的对象（都是**中心线** —— 描边就是「离它不超过半线宽」的那片区域）。
enum Geo {
    Segment((f32, f32), (f32, f32)),
    /// 描边的圆：中心线是半径 `r` 的那个圆。
    Ring {
        cx: f32,
        cy: f32,
        r: f32,
    },
}

type P = (f32, f32);

fn dist_point_segment(p: P, a: P, b: P) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= 1e-12 {
        0.0
    } else {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

impl Geo {
    fn distance(&self, p: P) -> f32 {
        match self {
            Geo::Segment(a, b) => dist_point_segment(p, *a, *b),
            Geo::Ring { cx, cy, r } => {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                ((dx * dx + dy * dy).sqrt() - r).abs()
            }
        }
    }

    /// `(min_x, min_y, max_x, max_y)`，外扩 `margin`。
    fn bounds(&self, margin: f32) -> (f32, f32, f32, f32) {
        match self {
            Geo::Segment((ax, ay), (bx, by)) => (
                ax.min(*bx) - margin,
                ay.min(*by) - margin,
                ax.max(*bx) + margin,
                ay.max(*by) + margin,
            ),
            Geo::Ring { cx, cy, r } => (
                cx - r - margin,
                cy - r - margin,
                cx + r + margin,
                cy + r + margin,
            ),
        }
    }
}

/// 一段圆弧折成折线（角点补点用）。
fn push_arc(out: &mut Vec<P>, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
    let steps = (((to - from).abs() / std::f32::consts::FRAC_PI_2) * 4.0).ceil() as usize;
    for k in 1..=steps.max(1) {
        let t = from + (to - from) * (k as f32 / steps.max(1) as f32);
        out.push((cx + r * t.cos(), cy + r * t.sin()));
    }
}

/// 折线 → 线段。
fn segments(out: &mut Vec<Geo>, points: &[P]) {
    for pair in points.windows(2) {
        out.push(Geo::Segment(pair[0], pair[1]));
    }
}

fn geometry(els: &[El]) -> Vec<Geo> {
    let mut out = Vec::new();
    for el in els {
        match el {
            El::Path(d) => {
                for sub in svgpath::flatten(d) {
                    segments(&mut out, &sub);
                }
            }
            El::Line { x1, y1, x2, y2 } => {
                out.push(Geo::Segment((*x1, *y1), (*x2, *y2)));
            }
            El::Circle { cx, cy, r } => out.push(Geo::Ring {
                cx: *cx,
                cy: *cy,
                r: *r,
            }),
            El::Rect { x, y, w, h, rx } => {
                // 圆角矩形：四条直边 + 四个角。
                let (x0, y0, x1, y1) = (*x, *y, x + w, y + h);
                let r = rx.min(w / 2.0).min(h / 2.0).max(0.0);
                let mut pts: Vec<P> = vec![(x0 + r, y0)];
                pts.push((x1 - r, y0));
                push_arc(
                    &mut pts,
                    x1 - r,
                    y0 + r,
                    r,
                    -std::f32::consts::FRAC_PI_2,
                    0.0,
                );
                pts.push((x1, y1 - r));
                push_arc(
                    &mut pts,
                    x1 - r,
                    y1 - r,
                    r,
                    0.0,
                    std::f32::consts::FRAC_PI_2,
                );
                pts.push((x0 + r, y1));
                push_arc(
                    &mut pts,
                    x0 + r,
                    y1 - r,
                    r,
                    std::f32::consts::FRAC_PI_2,
                    std::f32::consts::PI,
                );
                pts.push((x0, y0 + r));
                push_arc(
                    &mut pts,
                    x0 + r,
                    y0 + r,
                    r,
                    std::f32::consts::PI,
                    std::f32::consts::PI * 1.5,
                );
                pts.push((x0 + r, y0));
                segments(&mut out, &pts);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 光栅化
// ---------------------------------------------------------------------------

/// 把图标按 `px × px` 光栅化成一张白色（可染色）的遮盖贴图。
fn rasterize(els: &[El], px: usize) -> egui::ColorImage {
    let geo = geometry(els);
    let texel = VIEW / px as f32; // 一个像素等于多少 viewBox 单位
    let mut alpha = vec![0.0f32; px * px];

    for shape in &geo {
        let (min_x, min_y, max_x, max_y) = shape.bounds(HALF_WIDTH + texel);
        let x0 = (min_x / texel).floor().clamp(0.0, (px - 1) as f32) as usize;
        let x1 = (max_x / texel).ceil().clamp(0.0, (px - 1) as f32) as usize;
        let y0 = (min_y / texel).floor().clamp(0.0, (px - 1) as f32) as usize;
        let y1 = (max_y / texel).ceil().clamp(0.0, (px - 1) as f32) as usize;
        for py in y0..=y1 {
            for x in x0..=x1 {
                let p = ((x as f32 + 0.5) * texel, (py as f32 + 0.5) * texel);
                let d = shape.distance(p);
                // 边界一个像素内线性过渡 —— 这就是抗锯齿。
                let band = EDGE * texel;
                let coverage = ((HALF_WIDTH + 0.5 * band - d) / band).clamp(0.0, 1.0);
                let slot = &mut alpha[py * px + x];
                *slot = slot.max(coverage);
            }
        }
    }

    let pixels = alpha
        .into_iter()
        .map(|a| Color32::from_white_alpha((a * 255.0).round() as u8))
        .collect();
    egui::ColorImage::new([px, px], pixels)
}

/// 取（或建）某个图标在某个像素尺寸下的贴图。按 `(图标, 尺寸)` 缓存。
fn texture(
    ctx: &egui::Context,
    name: &'static str,
    svg: &'static str,
    px: usize,
) -> egui::TextureHandle {
    let id = egui::Id::new(("starrytools-icon", name, px));
    if let Some(handle) = ctx.data_mut(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let image = rasterize(&parse(svg), px);
    let handle = ctx.load_texture(
        format!("icon:{name}:{px}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
    handle
}

/// 把一个图标贴到 `rect` 里。`angle != 0` 时绕中心旋转（loader 转圈、胶囊箭头的翻转）。
fn draw(
    painter: &egui::Painter,
    rect: Rect,
    name: &'static str,
    svg: &'static str,
    color: Color32,
    angle: f32,
) {
    let ctx = painter.ctx();
    let side = rect.width().min(rect.height());
    if side <= 0.5 {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let square = Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    // 把位置**贴到物理像素网格**上：贴图是按物理像素 1:1 光栅化的，
    // 哪怕只差半个像素，双线性采样也会把整张图抹糊。
    let snap = |p: Pos2| Pos2::new((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp);
    let square = Rect::from_min_max(snap(square.min), snap(square.max));
    let px = ((square.width() * ppp).round() as usize).clamp(8, MAX_PX);
    let handle = texture(ctx, name, svg, px);
    let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));

    if angle == 0.0 {
        painter.image(handle.id(), square, uv, color);
        return;
    }

    // 需要旋转：自己拼一个四边形。
    let rot = egui::emath::Rot2::from_angle(angle);
    let center = square.center();
    let corners = [
        (square.left_top(), uv.left_top()),
        (square.right_top(), uv.right_top()),
        (square.right_bottom(), uv.right_bottom()),
        (square.left_bottom(), uv.left_bottom()),
    ];
    let mut mesh = egui::epaint::Mesh::with_texture(handle.id());
    for (pos, tex) in corners {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + rot * (pos - center),
            uv: tex,
            color,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// 定义一批图标：`include_str!` 把 vendored 的 SVG 打进来，用时才解析 / 光栅化。
macro_rules! icon {
    ($(#[$meta:meta])* $name:ident, $file:literal) => {
        $(#[$meta])*
        pub fn $name(painter: &egui::Painter, rect: Rect, color: Color32) {
            draw(
                painter,
                rect,
                stringify!($name),
                include_str!(concat!("../../assets/icons/", $file)),
                color,
                0.0,
            );
        }
    };
    ($(#[$meta:meta])* $name:ident, $file:literal, turned) => {
        $(#[$meta])*
        pub fn $name(painter: &egui::Painter, rect: Rect, color: Color32, angle: f32) {
            draw(
                painter,
                rect,
                stringify!($name),
                include_str!(concat!("../../assets/icons/", $file)),
                color,
                angle,
            );
        }
    };
}

icon!(
    /// 节点库：四宫格。
    blocks, "blocks.svg"
);
icon!(
    /// 说明：一页纸。
    file_text, "file-text.svg"
);
icon!(
    /// 加载：打开的文件夹。
    folder_open, "folder-open.svg"
);
icon!(
    /// 保存：软盘。
    save, "save.svg"
);
icon!(
    /// 运行：空心三角（lucide 是描边不是填充）。
    play, "play.svg"
);
icon!(
    /// 收起：上折角（状态药丸右侧的小箭头）。
    chevron_up, "chevron-up.svg"
);
icon!(
    /// 展开：下折角（下拉框右侧的小箭头）。
    chevron_down, "chevron-down.svg"
);
icon!(
    /// 展开：下折角，绕自身中心转过 `angle` 弧度（用于「翻转」入场）。
    chevron_down_turned, "chevron-down.svg", turned
);
icon!(
    /// 下拉里选中的那一项前面的对勾。
    check, "check.svg"
);
icon!(
    /// 提示：一个圈里的 i（运行后的「提示」用它）。
    info, "info.svg"
);
icon!(
    /// 删除：垃圾桶。
    trash, "trash-2.svg"
);
icon!(
    /// 取消：向左回退的箭头。
    undo, "undo-2.svg"
);
icon!(
    /// 新建：加号。
    plus, "plus.svg"
);
icon!(
    /// 卡片上的那个小箭头。
    arrow_right, "arrow-right.svg"
);
icon!(
    /// 关闭 / 删除节点：叉。
    x, "x.svg"
);
icon!(
    /// 在文件夹中显示。
    folder_search, "folder-search.svg"
);
icon!(
    /// 另存为。
    download, "download.svg"
);
icon!(
    /// 放大（画布控件）。
    zoom_in, "zoom-in.svg"
);
icon!(
    /// 缩小（画布控件）。
    zoom_out, "zoom-out.svg"
);
icon!(
    /// 适应视图（画布控件）。
    fit_view, "maximize.svg"
);
icon!(
    /// 设置：齿轮。
    settings, "settings.svg"
);

/// 运行中：转圈的弧。`phase` 是 0..1 的相位。
pub fn loader(painter: &egui::Painter, rect: Rect, color: Color32, phase: f32) {
    draw(
        painter,
        rect,
        "loader",
        include_str!("../../assets/icons/loader-circle.svg"),
        color,
        phase * std::f32::consts::TAU,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg_of(file: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/icons")
            .join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {file}：{e}"))
    }

    /// 每份 SVG 都要解析出至少一个形状 —— 免得文件取错或解析器退化时静默画不出东西。
    #[test]
    fn every_vendored_icon_parses() {
        let files = [
            "blocks.svg",
            "file-text.svg",
            "folder-open.svg",
            "save.svg",
            "play.svg",
            "loader-circle.svg",
            "chevron-up.svg",
            "chevron-down.svg",
            "check.svg",
            "info.svg",
            "trash-2.svg",
            "undo-2.svg",
            "plus.svg",
            "arrow-right.svg",
            "x.svg",
            "folder-search.svg",
            "download.svg",
            "zoom-in.svg",
            "zoom-out.svg",
            "maximize.svg",
            "settings.svg",
        ];
        for file in files {
            let els = parse(&svg_of(file));
            assert!(!els.is_empty(), "{file} 没解析出任何形状");
            assert!(!geometry(&els).is_empty(), "{file} 没折出任何中心线");
        }
    }

    /// 属性名要独立：`x` 不能命中 `x1` / `x2`（zoom-in 的 `<line>` 就是这么写的）。
    #[test]
    fn attribute_lookup_is_not_confused_by_prefixes() {
        let tag = r#"line x1="21" x2="16.65" y1="21" y2="16.65""#;
        assert_eq!(attr(tag, "x1"), Some("21"));
        assert_eq!(attr(tag, "x2"), Some("16.65"));
        assert_eq!(attr(tag, "y1"), Some("21"));
        assert_eq!(attr(tag, "y2"), Some("16.65"));
    }

    /// `<svg>` 这类外壳标签不该被当成形状。
    #[test]
    fn container_tags_are_ignored() {
        let svg = r#"<svg width="24" height="24"><circle cx="12" cy="12" r="10" /></svg>"#;
        assert_eq!(parse(svg).len(), 1);
    }

    /// 距离场必须真的落在中心线上，且端点是**圆头**：中线实心、垂直两单位外空、
    /// 端点正外侧约一单位内还被圆头盖着，再远就没了。
    #[test]
    fn the_distance_field_follows_the_centre_line() {
        let els = parse(r#"<path d="M6 12h12" />"#);
        let img = rasterize(&els, 24);
        let at = |x: usize, y: usize| img.pixels[y * 24 + x].a();
        assert!(at(12, 12) > 200, "中线上应当实心");
        assert!(at(12, 9) < 20, "离中线两单位就不该有了");
        // 圆头：端点 (6,12) 往左一格还有，再一格就没了 —— 方头的话第一格就该是空的。
        assert!(at(5, 12) > 100, "端点外侧应当被圆头盖住");
        assert!(at(4, 12) < 20, "再往外应当空出来");
    }

    /// `info` 的 `i` 头上那一点（`M12 8h.01`）是零长线段，距离场会把它变成圆点。
    #[test]
    fn the_info_dot_survives() {
        let img = rasterize(&parse(&svg_of("info.svg")), 24);
        // 点落在像素角上，最近的像素中心离它 0.7 单位，覆盖约 0.8。
        let dot = img.pixels[8 * 24 + 12].a();
        let blank = img.pixels[8 * 24 + 8].a();
        assert!(dot > 150, "`i` 头上的点应当被画出来，实际 alpha={dot}");
        assert!(blank < 20, "同一行远处应当是空的，实际 alpha={blank}");
    }

    /// 描边不是填充：图标内部应当是空的（`play` 那个空心三角形。
    #[test]
    fn icons_are_stroked_not_filled() {
        let img = rasterize(&parse(&svg_of("play.svg")), 48);
        let at = |x: usize, y: usize| img.pixels[y * 48 + x].a();
        // 三角形内部约在 viewBox (11, 12) → 48px 下的像素 (22, 24)。
        assert!(at(22, 24) < 20, "三角形内部被填实了");
        // 左侧那条竖边在 viewBox x=5 → 48px 下的像素 x=10。
        assert!(at(10, 24) > 100, "左边那条竖线上应当是实心");
    }

    /// 细线也要有实心芯，不能整张都是灰的 —— 那样看着就发虚。
    #[test]
    fn icons_have_a_solid_core_at_button_size() {
        for file in [
            "blocks.svg",
            "file-text.svg",
            "folder-open.svg",
            "save.svg",
            "play.svg",
            "info.svg",
            "x.svg",
            "check.svg",
            "trash-2.svg",
        ] {
            let img = rasterize(&parse(&svg_of(file)), 14);
            let full = img.pixels.iter().filter(|p| p.a() >= 250).count();
            assert!(
                full >= 4,
                "{file} 在 14px 下只有 {full} 个不透明像素，会发虚"
            );
        }
    }

    /// 沿一列数「深色带」的条数：细线并成一块时条数会变少。
    fn bands(img: &egui::ColorImage, x: usize) -> usize {
        let w = img.size[0];
        let mut bands = 0;
        let mut inside = false;
        for y in 0..img.size[1] {
            let dark = img.pixels[y * w + x].a() > 127;
            if dark && !inside {
                bands += 1;
            }
            inside = dark;
        }
        bands
    }

    /// `file-text` / `save` 都是细密图标：在按钮里那个尺寸（14px）下也必须还能数出好几笔，
    /// 不能糊成一块。这条测试守着「按显示尺寸做距离场光栅化」这套机制不被改退化。
    #[test]
    fn dense_icons_stay_legible_at_button_size() {
        for file in ["file-text.svg", "save.svg"] {
            let img = rasterize(&parse(&svg_of(file)), 14);
            let bands = bands(&img, 7);
            assert!(
                bands >= 3,
                "{file} 在 14px 下糊在一起了：只数出 {bands} 条笔画"
            );
        }
    }
}
