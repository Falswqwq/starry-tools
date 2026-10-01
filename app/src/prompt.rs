//! 紫色节点停下时弹出来的交互浮层。
//!
//! 目前只有一种：`CropShape` —— 在下方把图片摆出来，让人拖一个遮罩框定要裁的范围，
//! 按「确定」才继续。运行的后台线程正阻塞在这里等着这份答案。
//!
//! 遮罩用**图像像素**存（不是屏幕、也不是归一化坐标），这样「Shift → 正方形 / 正圆」
//! 的约束就是干脆的 `w == h`，不必考虑显示区的缩放。

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use starrytools_core::interaction::{InteractionKind, InteractionResponse, MaskShape};

use crate::run::Runner;
use crate::theme;
use crate::widgets::{self, Size, Variant};

const PANEL_W: f32 = 720.0;
const PAD: f32 = 20.0;
/// 图片显示区的最大高度。
const IMAGE_MAX_H: f32 = 460.0;
/// 角上手柄的边长（屏幕像素）。
const HANDLE: f32 = 14.0;
/// 角上手柄的抓取半径（屏幕像素）—— 比画出来的再大一圈，好点。
const HANDLE_HIT: f32 = 22.0;
/// 交互区比图片向外多出的那一圈（屏幕像素）：角把柄有一半落在图片外时也能抓到。
const EDGE: f32 = HANDLE_HIT;
/// 遮罩最小边长（图像像素）。
const MIN_PX: f32 = 4.0;

#[derive(Default)]
pub struct Prompt {
    /// 当前请求的指纹。换了请求就把遮罩和贴图重置。
    key: Option<String>,
    /// 遮罩，图像像素坐标。
    mask: Mask,
    /// 预览贴图（按请求缓存）。
    texture: Option<egui::TextureHandle>,
    /// 正在拖的是哪一块。
    grab: Option<Grab>,
}

/// 遮罩：图像像素坐标。
#[derive(Clone, Copy, PartialEq)]
struct Mask {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

/// 拖动的是遮罩的哪一部分。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grab {
    Move,
    Nw,
    Ne,
    Sw,
    Se,
}

impl Prompt {
    pub fn ui(&mut self, ctx: &egui::Context, runner: &mut Runner) {
        let Some(request) = runner.waiting().cloned() else {
            self.clear();
            return;
        };
        let node_name = request.node_name.clone();
        let InteractionKind::CropShape(crop) = request.kind;

        let key = format!(
            "{}:{}x{}:{}",
            request.node_id,
            crop.width,
            crop.height,
            crop.preview.len()
        );
        if self.key.as_deref() != Some(key.as_str()) {
            self.key = Some(key);
            self.mask = Mask {
                x: 0.0,
                y: 0.0,
                w: crop.width.max(1) as f32,
                h: crop.height.max(1) as f32,
            };
            self.grab = None;
            self.texture = load_texture(ctx, &request.node_id, &crop.preview);
        }

        // 背幕：把画布压暗，顺带吃掉点击 —— 这是一次模态操作。
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("prompt-scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .movable(false)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                // 轻轻压暗即可 —— 别把人正等着看的那圈紫色高亮也盖没了。
                ui.painter()
                    .rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(48));
            });

        let mut answer: Option<InteractionResponse> = None;

        egui::Area::new(egui::Id::new("prompt-panel"))
            .order(egui::Order::Tooltip)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                widgets::panel_frame().show(ui, |ui| {
                    ui.set_width(PANEL_W);
                    // ---- 顶栏 ----
                    egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: PAD as i8,
                            right: PAD as i8,
                            top: 12,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (dot, _) =
                                    ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 4.0, theme::PURPLE);
                                ui.label(
                                    egui::RichText::new(&node_name)
                                        .color(theme::INK)
                                        .size(12.0)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new("等待你框定范围")
                                        .color(theme::PURPLE)
                                        .size(11.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new("按住 Shift 框出正方形 / 正圆")
                                                .color(theme::INK_3)
                                                .size(10.0),
                                        );
                                    },
                                );
                            });
                        });

                    // ---- 图片 + 遮罩 ----
                    padded(ui, PAD, |ui| {
                        // 交互区比图片再向外扩一圈 `EDGE`，这样角把柄有一半落在图片外时
                        // 也能抓到（遮罩铺满整张图时四个角正落在图片边缘上）。
                        let avail_w = PANEL_W - PAD * 2.0;
                        let aspect = crop.width.max(1) as f32 / crop.height.max(1) as f32;
                        let max_w = (avail_w - EDGE * 2.0).max(32.0);
                        let max_h = (IMAGE_MAX_H - EDGE * 2.0).max(32.0);
                        let mut fit_w = max_w;
                        let mut fit_h = fit_w / aspect;
                        if fit_h > max_h {
                            fit_h = max_h;
                            fit_w = fit_h * aspect;
                        }
                        let (img_rect, resp) = ui.allocate_exact_size(
                            Vec2::new(fit_w + EDGE * 2.0, fit_h + EDGE * 2.0),
                            Sense::click_and_drag(),
                        );
                        // 图片居中在交互区里。
                        let img =
                            Rect::from_center_size(img_rect.center(), Vec2::new(fit_w, fit_h));

                        if let Some(texture) = &self.texture {
                            ui.painter().image(
                                texture.id(),
                                img,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().rect_filled(
                                img,
                                CornerRadius::same(theme::R_CTL),
                                theme::SURFACE_2,
                            );
                        }

                        let scale = img.width() / crop.width.max(1) as f32;
                        self.handle_drag(&resp, ui, img, scale, crop.width, crop.height);

                        // 遮罩：外圈压暗 + 边框 + 角手柄。
                        let mask = mask_screen(self.mask, img, scale);
                        let cr = CornerRadius::same(2);
                        if crop.shape == MaskShape::Ellipse {
                            // 椭圆：整块图压暗，再把椭圆内的图像重画回去 ——
                            // 这样四周被压暗的是真正的椭圆外，而不是矩形的四条边。
                            ui.painter().rect_filled(img, CornerRadius::ZERO, dim());
                            if let Some(texture) = &self.texture {
                                ellipse_image(ui.painter(), texture.id(), img, mask);
                            }
                            let mut points = Vec::with_capacity(65);
                            for k in 0..=64 {
                                let t = k as f32 / 64.0 * std::f32::consts::TAU;
                                points.push(egui::pos2(
                                    mask.center().x + mask.width() * 0.5 * t.cos(),
                                    mask.center().y + mask.height() * 0.5 * t.sin(),
                                ));
                            }
                            ui.painter()
                                .add(egui::Shape::line(points, Stroke::new(2.0, theme::PURPLE)));
                        } else {
                            draw_scrim(ui.painter(), img, mask);
                            ui.painter().rect_stroke(
                                mask,
                                cr,
                                Stroke::new(2.0, theme::PURPLE),
                                StrokeKind::Inside,
                            );
                        }
                        // 四个角的把柄：实心紫方块（不带白边），鼠标移到哪个上面
                        // 就把那个放大一点 —— 一眼看出「能拽」。
                        let hover = resp.hover_pos().and_then(|p| corner_at(mask, p));
                        for (corner, which) in [
                            (mask.left_top(), Grab::Nw),
                            (mask.right_top(), Grab::Ne),
                            (mask.left_bottom(), Grab::Sw),
                            (mask.right_bottom(), Grab::Se),
                        ] {
                            let active = hover == Some(which) || self.grab == Some(which);
                            let side = if active { HANDLE * 1.35 } else { HANDLE };
                            let handle = Rect::from_center_size(corner, Vec2::splat(side));
                            ui.painter()
                                .rect_filled(handle, CornerRadius::same(3), theme::PURPLE);
                        }

                        // 尺寸标签贴在遮罩上边。
                        let label = format!(
                            "{} × {} px",
                            self.mask.w.round().max(0.0) as i64,
                            self.mask.h.round().max(0.0) as i64
                        );
                        let galley = ui.painter().layout_no_wrap(
                            label,
                            FontId::monospace(10.5),
                            Color32::WHITE,
                        );
                        let tag = Rect::from_center_size(
                            egui::pos2(mask.center().x, (mask.top() - 12.0).max(img.top() + 10.0)),
                            galley.size() + Vec2::new(12.0, 5.0),
                        );
                        ui.painter()
                            .rect_filled(tag, CornerRadius::same(4), theme::PURPLE);
                        ui.painter().galley(
                            tag.center() - galley.size() * 0.5,
                            galley,
                            Color32::WHITE,
                        );
                    });

                    // ---- 底部按钮 ----
                    padded(ui, PAD, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(
                                ui,
                                "确定裁切",
                                None,
                                Variant::Primary,
                                Size::Md,
                                true,
                            )
                            .clicked()
                            {
                                answer = Some(InteractionResponse::Crop {
                                    x: self.mask.x.round().max(0.0) as u32,
                                    y: self.mask.y.round().max(0.0) as u32,
                                    width: self.mask.w.round().max(1.0) as u32,
                                    height: self.mask.h.round().max(1.0) as u32,
                                });
                            }
                            if widgets::button(ui, "取消", None, Variant::Solid, Size::Md, true)
                                .clicked()
                            {
                                answer = Some(InteractionResponse::Cancel);
                            }
                        });
                    });
                });
            });

        if let Some(response) = answer {
            runner.respond(response);
        }
    }

    /// 处理遮罩的拖动：角手柄改大小、中间拖动整体移动，Shift 锁定为正方形 / 正圆。
    fn handle_drag(
        &mut self,
        resp: &egui::Response,
        ui: &egui::Ui,
        img: Rect,
        scale: f32,
        img_w: u32,
        img_h: u32,
    ) {
        if scale <= 0.0 || img_w == 0 || img_h == 0 {
            return;
        }
        let clamp_min = MIN_PX.min(img_w as f32).min(img_h as f32);
        let mask = mask_screen(self.mask, img, scale);

        if resp.drag_started() {
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(point) = origin {
                // 靠近哪个角就是拖哪个角，否则整体移动。
                self.grab = Some(corner_at(mask, point).unwrap_or(Grab::Move));
            }
        }

        // 鼠标反馈：拖的时候给「拖 / 抓」的光标，只是悬停也要让把柄像能点。
        if let Some(grab) = self.grab {
            ui.ctx().set_cursor_icon(match grab {
                Grab::Move => CursorIcon::Grabbing,
                Grab::Nw | Grab::Se => CursorIcon::ResizeNwSe,
                Grab::Ne | Grab::Sw => CursorIcon::ResizeNeSw,
            });
        } else if resp.hovered() {
            if let Some(point) = resp.hover_pos() {
                let icon = match corner_at(mask, point) {
                    Some(Grab::Nw | Grab::Se) => CursorIcon::ResizeNwSe,
                    Some(Grab::Ne | Grab::Sw) => CursorIcon::ResizeNeSw,
                    _ => CursorIcon::Grab,
                };
                ui.ctx().set_cursor_icon(icon);
            }
        }

        if resp.dragged() {
            if let (Some(grab), Some(point)) = (self.grab, resp.interact_pointer_pos()) {
                let px = ((point.x - img.left()) / scale).clamp(0.0, img_w as f32);
                let py = ((point.y - img.top()) / scale).clamp(0.0, img_h as f32);
                let want_square = ui.input(|input| input.modifiers.shift);
                self.mask = resize(
                    self.mask,
                    grab,
                    egui::pos2(px, py),
                    clamp_min,
                    Vec2::new(img_w as f32, img_h as f32),
                    want_square,
                );
            }
        }

        if resp.drag_stopped() {
            self.grab = None;
        }
    }

    fn clear(&mut self) {
        if self.key.is_some() {
            self.key = None;
            self.texture = None;
            self.mask = Mask::default();
            self.grab = None;
        }
    }
}

/// 一个只负责留白的小包装。
fn padded(ui: &mut egui::Ui, pad: f32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .inner_margin(egui::Margin {
            left: pad as i8,
            right: pad as i8,
            top: 0,
            bottom: if pad > 0.0 { 8 } else { 0 },
        })
        .show(ui, |ui| add(ui));
}

/// 指针靠哪个角就把那个角的拖拽把柄交出去；都不靠就返回 `None`。
fn corner_at(mask: Rect, point: Pos2) -> Option<Grab> {
    let near = |corner: Pos2| corner.distance(point) <= HANDLE_HIT;
    if near(mask.left_top()) {
        Some(Grab::Nw)
    } else if near(mask.right_top()) {
        Some(Grab::Ne)
    } else if near(mask.left_bottom()) {
        Some(Grab::Sw)
    } else if near(mask.right_bottom()) {
        Some(Grab::Se)
    } else {
        None
    }
}

/// 把遮罩换算成屏幕矩形。
fn mask_screen(mask: Mask, img: Rect, scale: f32) -> Rect {
    Rect::from_min_size(
        egui::pos2(img.left() + mask.x * scale, img.top() + mask.y * scale),
        Vec2::new(mask.w * scale, mask.h * scale),
    )
}

/// 遮罩压暗用的颜色。
fn dim() -> Color32 {
    Color32::from_black_alpha(120)
}

/// 遮罩外边压暗（矩形：四块边框）。
fn draw_scrim(painter: &egui::Painter, img: Rect, mask: Rect) {
    let dim = dim();
    let cr = CornerRadius::ZERO;
    let top = Rect::from_min_max(img.min, egui::pos2(img.right(), mask.top()));
    let bottom = Rect::from_min_max(egui::pos2(img.left(), mask.bottom()), img.max);
    let left = Rect::from_min_max(
        egui::pos2(img.left(), mask.top()),
        egui::pos2(mask.left(), mask.bottom()),
    );
    let right = Rect::from_min_max(
        egui::pos2(mask.right(), mask.top()),
        egui::pos2(img.right(), mask.bottom()),
    );
    for rect in [top, bottom, left, right] {
        if rect.width() > 0.0 && rect.height() > 0.0 {
            painter.rect_filled(rect, cr, dim);
        }
    }
}

/// 把图像**只画在椭圆内**：一个以遮罩中心为扇心的三角扇，逐顶点采纹理。
///
/// 先整块压暗、再这样把椭圆内的图重画回来，就等于「只压暗椭圆外」——
/// 比拿矩形去近似椭圆干净得多。
fn ellipse_image(painter: &egui::Painter, texture: egui::TextureId, img: Rect, mask: Rect) {
    if img.width() <= 0.0 || img.height() <= 0.0 {
        return;
    }
    let uv = |point: Pos2| {
        egui::pos2(
            (point.x - img.left()) / img.width(),
            (point.y - img.top()) / img.height(),
        )
    };
    const SEGMENTS: usize = 64;
    let center = mask.center();
    let (rx, ry) = (mask.width() * 0.5, mask.height() * 0.5);

    let mut mesh = egui::Mesh::with_texture(texture);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: uv(center),
        color: Color32::WHITE,
    });
    for k in 0..=SEGMENTS {
        let t = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let point = egui::pos2(center.x + rx * t.cos(), center.y + ry * t.sin());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: point,
            uv: uv(point),
            color: Color32::WHITE,
        });
    }
    for k in 0..SEGMENTS as u32 {
        mesh.indices.extend_from_slice(&[0, k + 1, k + 2]);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// 拖动之后的新遮罩（图像像素坐标）。
///
/// 纯几何，单测直接打它 —— 界面起不来，但这段逻辑能自动守住。
fn resize(mask: Mask, grab: Grab, point: Pos2, min: f32, img: Vec2, square: bool) -> Mask {
    let px = point.x;
    let py = point.y;
    let img_w = img.x;
    let img_h = img.y;
    let (left, top) = (mask.x, mask.y);
    let (right, bottom) = (mask.x + mask.w, mask.y + mask.h);
    let mut out = match grab {
        Grab::Move => {
            let dx = (px - (left + right) * 0.5).clamp(-left, img_w - right);
            let dy = (py - (top + bottom) * 0.5).clamp(-top, img_h - bottom);
            Mask {
                x: left + dx,
                y: top + dy,
                w: mask.w,
                h: mask.h,
            }
        }
        Grab::Nw => Mask {
            x: px.min(right - min),
            y: py.min(bottom - min),
            w: right - px.min(right - min),
            h: bottom - py.min(bottom - min),
        },
        Grab::Ne => Mask {
            x: left,
            y: py.min(bottom - min),
            w: px.max(left + min).min(img_w) - left,
            h: bottom - py.min(bottom - min),
        },
        Grab::Sw => Mask {
            x: px.min(right - min),
            y: top,
            w: right - px.min(right - min),
            h: py.max(top + min).min(img_h) - top,
        },
        Grab::Se => Mask {
            x: left,
            y: top,
            w: px.max(left + min).min(img_w) - left,
            h: py.max(top + min).min(img_h) - top,
        },
    };
    out.x = out.x.clamp(0.0, (img_w - min).max(0.0));
    out.y = out.y.clamp(0.0, (img_h - min).max(0.0));
    out.w = out.w.clamp(min, img_w - out.x);
    out.h = out.h.clamp(min, img_h - out.y);

    if square {
        let side = out.w.max(out.h);
        // 以拖动角为锚点向里收。移动模式就保持左上角不动。
        match grab {
            Grab::Nw => {
                let right = out.x + out.w;
                let bottom = out.y + out.h;
                out.w = side.min(right);
                out.h = side.min(bottom);
                out.x = right - out.w;
                out.y = bottom - out.h;
            }
            _ => {
                out.w = side.min(img_w - out.x);
                out.h = side.min(img_h - out.y);
            }
        }
    }
    out
}

fn load_texture(ctx: &egui::Context, node_id: &str, url: &str) -> Option<egui::TextureHandle> {
    let (width, height, rgba) = starrytools_core::image_io::decode_preview_data_url(url)?;
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    // 和卡片缩略图一样用最近邻 —— 像素画放大后不会被糊掉。
    Some(ctx.load_texture(
        format!("prompt:{node_id}"),
        image,
        egui::TextureOptions::NEAREST,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Mask {
        Mask {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        }
    }

    fn img(w: f32, h: f32) -> Vec2 {
        Vec2::new(w, h)
    }

    #[test]
    fn dragging_a_corner_resizes_the_mask() {
        let mask = resize(
            full(),
            Grab::Se,
            Pos2::new(40.0, 60.0),
            4.0,
            img(100.0, 100.0),
            false,
        );
        assert_eq!((mask.x, mask.y), (0.0, 0.0));
        assert_eq!((mask.w, mask.h), (40.0, 60.0));
    }

    #[test]
    fn resizing_cannot_go_below_the_minimum() {
        let mask = resize(full(), Grab::Se, Pos2::ZERO, 4.0, img(100.0, 100.0), false);
        assert!(mask.w >= 4.0 && mask.h >= 4.0, "至少留 {MIN_PX} 像素");
    }

    #[test]
    fn moving_is_kept_inside_the_image() {
        let start = Mask {
            x: 10.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        };
        let mask = resize(
            start,
            Grab::Move,
            Pos2::new(999.0, 999.0),
            4.0,
            img(100.0, 100.0),
            false,
        );
        assert_eq!((mask.x, mask.y), (80.0, 80.0), "移到右下角就贴边");
        assert_eq!((mask.w, mask.h), (20.0, 20.0), "移动不改变大小");
    }

    #[test]
    fn shift_forces_a_square() {
        let mask = resize(
            full(),
            Grab::Se,
            Pos2::new(60.0, 30.0),
            4.0,
            img(200.0, 200.0),
            true,
        );
        assert_eq!(mask.w, mask.h, "Shift 下长宽相等");
        assert_eq!(mask.w, 60.0, "取拖得更多的那一边");
    }

    #[test]
    fn a_corner_drag_never_escapes_the_image() {
        let mask = resize(
            full(),
            Grab::Nw,
            Pos2::new(-50.0, -50.0),
            4.0,
            img(100.0, 80.0),
            false,
        );
        assert!(mask.x >= 0.0 && mask.y >= 0.0);
        assert!(mask.x + mask.w <= 100.0 && mask.y + mask.h <= 80.0);
    }
}
