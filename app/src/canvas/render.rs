//! 背景、连线、刀光和节点卡片的绘制与动画外观；绘制顺序在此集中维护。
use super::{
    Graph, BODY_PAD, CHECKER, DOT_GAP, HEADER_H, NOTICE_GAP, NOTICE_ICON, NOTICE_PAD, NOTICE_STACK,
    OVERTAKE_REACH, PALETTE_H, PREVIEW_H, PROGRESS_H,
};
use crate::canvas::geometry::{self, Cubic};
use crate::canvas::layout::*;
use crate::canvas::node::{kind_of, PortRef};
use crate::state::run::Marks;
use crate::ui::controls::mask_corners;
use crate::ui::icons::{self, IconFn};
use crate::ui::motion::{CANVAS_ENTER, CANVAS_OVERTAKE, CANVAS_SEVER};
use crate::ui::{easing, theme, widgets};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
};
use egui::epaint::CubicBezierShape;
use starrytools_core::engine::NodeStatus;
use starrytools_core::model::node_kind::{NodeKindInfo, PortDef};

impl Graph {
    pub(super) fn draw_canvas(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        now: f64,
        show_fps: bool,
    ) {
        let painter = ui.painter_at(rect);

        // ---- 背景小点 ----
        // 只在点距够大（没缩太远）时才画，省得小到看不清还白花功夫。
        // 全部拼进**一个 Mesh**：几千个 `circle_filled` 会让 epaint 每帧重新三角化
        // 几千个形状，平移时位置又全变、缓存也命中不了 —— 用一个网格一次画出去，
        // 图形一模一样，每帧的开销则从「几千个形状」降到「一个」+ 拼顶点的循环。
        let step = DOT_GAP * self.view.zoom;
        if step > 5.0 {
            let texture = dot_texture(ui.ctx());
            let mut mesh = egui::Mesh::with_texture(texture.id());
            let ox = self.view.pan.x.rem_euclid(step);
            let oy = self.view.pan.y.rem_euclid(step);
            let mut y = rect.top() - step + oy;
            while y < rect.bottom() + step {
                let mut x = rect.left() - step + ox;
                while x < rect.right() + step {
                    push_dot(&mut mesh, egui::pos2(x, y), 1.0, theme::CANVAS_DOT);
                    x += step;
                }
                y += step;
            }
            painter.add(egui::Shape::mesh(mesh));
        }

        // ---- 连线 ----
        for i in 0..self.wires.len() {
            self.draw_wire(&painter, kinds, marks, i, now);
        }

        // ---- 正在断开的连线（它们已经不在 `wires` 里了） ----
        self.draw_severing(&painter, now);

        // ---- 正在拉的那根线：画在节点**下面**，这样线尾会被端点盖住 ----
        self.draw_connecting(&painter, kinds, now);

        // ---- 节点 ----
        let pointer = ui.input(|input| input.pointer.hover_pos());
        self.anim
            .entering
            .retain(|_, started| now - *started < CANVAS_ENTER);
        let enter: Vec<f32> = self
            .nodes
            .iter()
            .map(|node| self.enter_factor(&node.id, now))
            .collect();
        // 先把所有阴影铺一遍，再把卡片本体盖上去 —— 两张卡叠在一起时，
        // 上面那张的阴影不会落到下面那张身上，看着才像真的盖住了。
        for (i, &factor) in enter.iter().enumerate() {
            if factor >= 1.0 {
                self.draw_node_shadow(&painter, kinds, i);
            } else {
                let mut faded = ui.painter_at(rect);
                faded.set_opacity(factor);
                self.draw_node_shadow(&faded, kinds, i);
            }
        }
        // 卡片 + 它自己的参数控件，**逐个节点**画 ——
        // 控件必须夹在卡片之间，否则下面那张卡的控件会盖到上面那张卡上。
        let mut controls_changed = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            {
                let style = ui.style_mut();
                for (text_style, size) in [
                    (egui::TextStyle::Body, 11.0),
                    (egui::TextStyle::Button, 11.0),
                    (egui::TextStyle::Monospace, 11.0),
                    (egui::TextStyle::Small, 9.5),
                ] {
                    style
                        .text_styles
                        .insert(text_style, FontId::monospace(size * self.view.zoom));
                }
                style.spacing.button_padding = egui::vec2(5.0, 1.0) * self.view.zoom;
                style.spacing.interact_size.y = 18.0 * self.view.zoom;
                style.spacing.icon_width = 9.0 * self.view.zoom;
                style.spacing.item_spacing = egui::vec2(4.0, 3.0) * self.view.zoom;
            }
            for (i, &factor) in enter.iter().enumerate() {
                let mut faded = ui.painter_at(rect);
                if factor < 1.0 {
                    faded.set_opacity(factor);
                }
                self.draw_node(&faded, kinds, marks, i, pointer, now);
                // 参数控件紧跟在这张卡后面注册 —— 这就是原版的 `nodrag`，
                // 同时也让叠放顺序和卡片一致。
                if self.draw_node_controls(ui, kinds, i, now) {
                    controls_changed = true;
                }
            }
        });
        if controls_changed {
            self.touch_content();
        }

        // ---- 刀光在最上面 ----
        self.draw_slash(&painter, now);

        // ---- 节点右键菜单（是另一层浮层，和 painter 的先后无关） ----
        self.draw_menu(ui);

        // ---- 左下角：缩放控件 ----
        self.zoom_controls(ui, rect, kinds);

        // ---- 右下角：帧率读数 ----
        if show_fps {
            self.draw_fps(ui);
        }
    }

    /// 记一帧的时间，算出平滑后的 fps。
    ///
    /// 空闲一段时间后的第一帧间隔会很大（甚至几秒），那种不算 —— 否则一恢复
    /// 交互，读数会被那一帧拖到个位数。
    pub(super) fn measure_fps(&mut self, now: f64) {
        if let Some(last) = self.anim.last_frame {
            let delta = now - last;
            if delta > 0.0 && delta <= 0.5 {
                let instant = (1.0 / delta) as f32;
                self.anim.fps = if self.anim.fps <= 0.0 {
                    instant
                } else {
                    self.anim.fps * 0.9 + instant * 0.1
                };
            }
        }
        self.anim.last_frame = Some(now);
    }

    /// 右下角一个灰色的帧率读数，保留到整数位。
    ///
    /// 数字和 `fps` 都自己排版：数字在一个固定宽度里**右对齐**，紧跟着单位 ——
    /// 位数变化（60 → 100）时数字从右边长出去，`fps` 一动不动，才不会左右抖。
    pub(super) fn draw_fps(&self, ui: &egui::Ui) {
        if self.anim.fps <= 0.0 {
            return;
        }
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("fps"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -8.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(&ctx, |ui| {
                let font = FontId::monospace(11.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 15.0), Sense::hover());
                let painter = ui.painter();
                let unit = painter.layout_no_wrap("fps".to_string(), font.clone(), theme::INK_3);
                let unit_x = rect.right() - unit.size().x;
                painter.galley(
                    egui::pos2(unit_x, rect.center().y - unit.size().y * 0.5),
                    unit.clone(),
                    theme::INK_3,
                );
                let number = painter.layout_no_wrap(
                    format!("{}", self.anim.fps.round() as i64),
                    font,
                    theme::INK_3,
                );
                painter.galley(
                    egui::pos2(
                        unit_x - 5.0 - number.size().x,
                        rect.center().y - number.size().y * 0.5,
                    ),
                    number,
                    theme::INK_3,
                );
            });
    }

    /// 左下角一竖条缩放控件（放大 / 缩小 / 适应画面）。
    pub(super) fn zoom_controls(
        &mut self,
        ui: &mut egui::Ui,
        canvas: Rect,
        kinds: &[NodeKindInfo],
    ) {
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("zoom-controls"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                let size = egui::vec2(29.0, 29.0 * 3.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                widgets::card_shadow(ui.painter(), rect, theme::R_CARD);
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(theme::R_CARD), theme::HAIRLINE);
                let inner = rect.shrink(1.0);
                let h = inner.height() / 3.0;
                let icons: [IconFn; 3] = [icons::zoom_in, icons::zoom_out, icons::fit_view];
                for (k, icon) in icons.into_iter().enumerate() {
                    let b = Rect::from_min_size(
                        egui::pos2(inner.left(), inner.top() + k as f32 * h),
                        egui::vec2(inner.width(), h - if k < 2 { 1.0 } else { 0.0 }),
                    );
                    let resp = ui.interact(b, ui.id().with(k), Sense::click());
                    let hot = resp.hovered();
                    let radius = CornerRadius {
                        nw: if k == 0 { theme::R_CARD - 1 } else { 0 },
                        ne: if k == 0 { theme::R_CARD - 1 } else { 0 },
                        sw: if k == 2 { theme::R_CARD - 1 } else { 0 },
                        se: if k == 2 { theme::R_CARD - 1 } else { 0 },
                    };
                    ui.painter().rect_filled(
                        b,
                        radius,
                        if hot {
                            theme::SURFACE_2
                        } else {
                            theme::SURFACE
                        },
                    );
                    icon(
                        ui.painter(),
                        widgets::icon_rect(b, 14.0),
                        if hot { theme::ACCENT } else { theme::INK_3 },
                    );
                    if resp.clicked() {
                        match k {
                            0 => self.zoom_by(1.2, canvas),
                            1 => self.zoom_by(1.0 / 1.2, canvas),
                            _ => self.fit_view(canvas, kinds),
                        }
                    }
                }
            });
    }

    /// 入场动画的进度（0→1，缓出）。不在 `entering` 里就是 1。
    pub(super) fn enter_factor(&self, id: &str, now: f64) -> f32 {
        match self.anim.entering.get(id) {
            Some(started) => {
                let t = ((now - started) / CANVAS_ENTER).clamp(0.0, 1.0) as f32;
                easing::ease_out_quad(t)
            }
            None => 1.0,
        }
    }

    pub(super) fn draw_wire(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        i: usize,
        now: f64,
    ) {
        let wire = &self.wires[i];
        let curve = self.wire_points(kinds, wire);
        let screen: Cubic = curve.map(|p| self.view.to_screen(p));

        // 被刀光扫到：先铺一层更宽的发光，再画本体，并轻轻搏动；
        // 刀口切到的那一点再闪一下白光 —— 像真的被切开。
        if let Some(slash) = &self.interaction.slash {
            if let Some((_, cut_t)) = slash.doomed.iter().find(|(index, _)| *index == i) {
                let pulse = 0.55 + 0.45 * (now * easing::BREATH).sin() as f32;
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(9.0 * self.view.zoom, theme::danger_alpha(0.16 * pulse)),
                ));
                painter.add(CubicBezierShape::from_points_stroke(
                    screen,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.4 * self.view.zoom, theme::danger_alpha(pulse)),
                ));
                // 刀口那一点：白光亮一下。
                let cut = geometry::at(&screen, *cut_t);
                let flash = (3.0 + 2.0 * pulse) * self.view.zoom;
                painter.circle_filled(cut, flash, theme::white_alpha(220));
                painter.circle_filled(cut, flash * 2.4, theme::white_alpha(40));
                return;
            }
        }

        // 静态检查说这条线接不上 → 红色虚线。
        if marks.invalid_edges.contains(&wire.id) {
            self.dashed_curve(
                painter,
                &curve,
                Stroke::new((1.6 * self.view.zoom).max(1.0), theme::DANGER),
                5.0 * self.view.zoom,
                4.0 * self.view.zoom,
            );
            return;
        }

        // 两端都跑成功了 → 点亮成强调色，一眼能看出信号流到哪儿了。
        let ran = marks.ok_nodes.contains(&self.nodes[wire.from].id)
            && marks.ok_nodes.contains(&self.nodes[wire.to].id);

        painter.add(CubicBezierShape::from_points_stroke(
            screen,
            false,
            Color32::TRANSPARENT,
            Stroke::new(
                ((if self.nodes[wire.from]
                    .outputs
                    .iter()
                    .any(|port| port.id == wire.from_port && port.exec)
                {
                    2.5
                } else {
                    1.5
                }) * self.view.zoom)
                    .max(1.0),
                if ran {
                    theme::ACCENT
                } else if self.nodes[wire.from]
                    .outputs
                    .iter()
                    .any(|port| port.id == wire.from_port && port.exec)
                {
                    theme::INK_2
                } else {
                    theme::WIRE
                },
            ),
        ));
    }

    /// 虚线贝塞尔。egui 没有虚线描边，就沿采样点按累计长度切段画 ——
    /// 按累计长度（而不是每小段各自重头算）才能让相位连续。
    pub(super) fn dashed_curve(
        &self,
        painter: &egui::Painter,
        curve: &Cubic,
        stroke: Stroke,
        dash: f32,
        gap: f32,
    ) {
        let period = dash + gap;
        if period <= 0.0 {
            return;
        }

        let points: Vec<Pos2> = geometry::sample(curve, geometry::steps_for(curve))
            .into_iter()
            .map(|p| self.view.to_screen(p))
            .collect();

        let mut travelled = 0.0;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = (b - a).length();
            if length <= 0.001 {
                continue;
            }
            let dir = (b - a) / length;
            let mut at = 0.0;
            while at < length {
                let phase = (travelled + at) % period;
                let step = if phase < dash {
                    (dash - phase).min(length - at)
                } else {
                    (period - phase).min(length - at)
                };
                if phase < dash {
                    painter.line_segment([a + dir * at, a + dir * (at + step)], stroke);
                }
                at += step;
            }
            travelled += length;
        }
    }

    pub(super) fn draw_severing(&self, painter: &egui::Painter, now: f64) {
        for entry in &self.anim.severing {
            let progress = ((now - entry.started) / CANVAS_SEVER).clamp(0.0, 1.0) as f32;
            let eased = easing::ease_out_quad(progress);
            let fade = 1.0 - ((progress - 0.5) / 0.5).max(0.0);
            let stroke = Stroke::new(2.4 * self.view.zoom, theme::danger_alpha(fade));

            let (left, right) = geometry::split(&entry.curve, entry.t);

            // 两截各自从断口往回缩：左半留前 `keep`，右半留后 `keep`。
            let keep = 1.0 - eased;
            if keep > 0.01 {
                self.stroke_curve(painter, &geometry::split(&left, keep).0, stroke);
                self.stroke_curve(painter, &geometry::split(&right, 1.0 - keep).1, stroke);
            }

            // 断口的那一下光。
            let point = self.view.to_screen(geometry::at(&entry.curve, entry.t));
            let radius = (3.0 + eased * 20.0) * self.view.zoom;
            let alpha = (1.0 - progress * 2.4).max(0.0);
            painter.circle_filled(point, radius, theme::danger_alpha(alpha * 0.9));
        }
    }

    pub(super) fn stroke_curve(&self, painter: &egui::Painter, curve: &Cubic, stroke: Stroke) {
        let points: Vec<Pos2> = geometry::sample(curve, geometry::steps_for(curve))
            .into_iter()
            .map(|p| self.view.to_screen(p))
            .collect();
        painter.line(points, stroke);
    }

    pub(super) fn draw_slash(&self, painter: &egui::Painter, now: f64) {
        let Some(slash) = &self.interaction.slash else {
            return;
        };

        // 收刀之后：刀尖往前送一段，同时变淡变细，像力道用完了。
        let (tip, fade, scale) = match slash.release {
            None => (slash.to, 1.0, 1.0),
            Some(start) => {
                let t = ((now - start) / CANVAS_OVERTAKE).clamp(0.0, 1.0) as f32;
                let eased = easing::ease_out_cubic(t);
                (
                    geometry::extend(slash.from, slash.to, OVERTAKE_REACH * eased),
                    1.0 - eased.powf(1.6),
                    1.0 - eased * 0.45,
                )
            }
        };
        if fade <= 0.01 {
            return;
        }

        // 尾巴透明、刀尖最亮。egui 没有描边渐变，就切成小段自己调透明度。
        const SEGMENTS: usize = 14;
        for i in 0..SEGMENTS {
            let t0 = i as f32 / SEGMENTS as f32;
            let t1 = (i + 1) as f32 / SEGMENTS as f32;
            let a = slash.from.lerp(tip, t0);
            let b = slash.from.lerp(tip, t1);
            let bright = (t0 + t1) * 0.5;

            painter.line_segment(
                [a, b],
                Stroke::new(16.0 * scale, theme::accent_alpha(bright * 0.16 * fade)),
            );
            painter.line_segment(
                [a, b],
                Stroke::new(3.5 * scale, theme::accent_alpha(bright * fade)),
            );
        }

        // 刀尖。命中的时候亮一点、大一点。
        let radius = if slash.doomed.is_empty() { 3.0 } else { 4.5 };
        painter.circle_filled(tip, radius * scale, theme::accent_alpha(fade));
    }

    /// 卡片底部的预览条：先铺棋盘格（透出底下的透明像素），再把图像按比例放进去。
    ///
    /// `bg` 是这块预览**周围**的背景色 —— 圆角外面要补回它，见 [`mask_corners`]。
    pub(super) fn draw_preview(
        &self,
        painter: &egui::Painter,
        inner: Rect,
        texture: &egui::TextureHandle,
        bg: Color32,
    ) {
        let cr = CornerRadius::same(theme::R_CTL);
        painter.rect_filled(inner, cr, theme::SURFACE_3);

        // 棋盘格：只在格子上画第二种颜色，比每格画两个矩形省一半。
        let cell = CHECKER * self.view.zoom;
        if cell > 2.0 {
            let mut row = 0usize;
            let mut y = inner.top();
            while y < inner.bottom() {
                let mut col = 0usize;
                let mut x = inner.left();
                while x < inner.right() {
                    if (row + col).is_multiple_of(2) {
                        let rect = Rect::from_min_max(
                            egui::pos2(x, y),
                            egui::pos2(
                                (x + cell).min(inner.right()),
                                (y + cell).min(inner.bottom()),
                            ),
                        );
                        painter.rect_filled(rect, CornerRadius::ZERO, theme::SURFACE_2);
                    }
                    x += cell;
                    col += 1;
                }
                y += cell;
                row += 1;
            }
        }

        // 图像按比例居中放进去 —— 像素画用最近邻，贴图里已经设好了。
        let size = texture.size_vec2();
        if size.x > 0.0 && size.y > 0.0 {
            let scale = (inner.width() / size.x).min(inner.height() / size.y);
            let fitted = Rect::from_center_size(inner.center(), size * scale);
            painter.image(
                texture.id(),
                fitted,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // 棋盘格和图片都是直角矩形，会盖到圆角外面 —— 不补的话四角会露出灰角。
        mask_corners(painter, inner, theme::R_CTL as f32, bg);

        painter.rect_stroke(
            inner,
            cr,
            Stroke::new(1.0, theme::HAIRLINE),
            StrokeKind::Inside,
        );
    }

    /// 正在拉的那根线。靠近合法端口就吸附到端口中心。
    pub(super) fn draw_connecting(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        now: f64,
    ) {
        let Some(connect) = &self.interaction.connect else {
            return;
        };

        let a = self.view.to_screen(self.port_pos(kinds, connect.from));
        // 有落点候选就直接吸到那个端口上 —— 手感上就是「啪」地贴过去。
        let b = match connect.target {
            Some(target) => self.view.to_screen(self.port_pos(kinds, target)),
            None => connect.to,
        };
        let snapped = connect.target.is_some();
        // 从输出端往外拉，控制点朝右；从输入端往外拉就朝左 —— 看起来才像「拽出一根线」。
        let reach = ((b.x - a.x).abs() * 0.55).max(48.0);
        let (c1, c2) = if connect.from.input {
            (egui::pos2(a.x - reach, a.y), egui::pos2(b.x + reach, b.y))
        } else {
            (egui::pos2(a.x + reach, a.y), egui::pos2(b.x - reach, b.y))
        };

        let color = if snapped { theme::ACCENT } else { theme::INK_3 };
        let width = (1.8 * self.view.zoom).max(1.2);

        // 还没接到端口上：整根线一呼一吸地泛蓝光，提示「尚未连接」。
        let pulse = if snapped {
            0.0
        } else {
            let wave = 0.5 + 0.5 * (now * easing::BREATH).sin() as f32;
            // 外圈光晕
            painter.add(CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(
                    width + 7.0 * self.view.zoom,
                    theme::accent_alpha(0.04 + 0.10 * wave),
                ),
            ));
            wave
        };

        painter.add(CubicBezierShape::from_points_stroke(
            [a, c1, c2, b],
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        ));
        // 叠一层会呼吸的强调色，让「还没接上」这件事一眼看得出来。
        if !snapped {
            painter.add(CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, theme::accent_alpha(0.14 + 0.40 * pulse)),
            ));
        }
        // 自由端（还没吸附到端口）自己一明一灭。吸附上之后不另画点 ——
        // 落点那一端由端口自己的强调动画负责（`port_halo`），两端才完全一致。
        if !snapped {
            painter.circle_filled(
                b,
                (5.0 + pulse * 2.5) * self.view.zoom,
                theme::accent_alpha(0.16 + 0.18 * pulse),
            );
            painter.circle_filled(b, 3.0 * self.view.zoom, color);
        }
    }

    /// 只画卡片的投影。单独一遍，为了不和别的卡片的填色打架。
    pub(super) fn draw_node_shadow(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        i: usize,
    ) {
        widgets::card_shadow(painter, self.screen_rect(kinds, i), theme::R_CARD);
    }

    pub(super) fn draw_node(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        i: usize,
        pointer: Option<Pos2>,
        now: f64,
    ) {
        let node = &self.nodes[i];
        let r = self.screen_rect(kinds, i);
        let cr = CornerRadius::same(theme::R_CARD);
        let z = self.view.zoom;

        // 卡片里面的字一律剪到卡片内 —— 长警告 / 长错误、甚至文件名也不会溢出去。
        let body = painter.with_clip_rect(r);

        // 边线颜色说几件事：跑失败了、跑成功了、还是平常；紫色则说明这是一个
        // 「会拦住运行等你动手」的节点（运行到它时会额外搏动高亮）。
        // **边线放到最后画** —— 里面的各段底色是铺满整宽的，先画会被它们盖住。
        let interactive = crate::catalog::is_interactive(&node.kind, &node.params);
        let waiting = marks.waiting.as_deref() == Some(node.id.as_str());
        // 「正在跑的那个」—— 运行中牌子的蓝色边框。
        let running = marks.running.as_deref() == Some(node.id.as_str());
        // 这个节点自己报上来的实时进度（视频压缩这类长任务才有）。
        let step = marks
            .step
            .as_ref()
            .filter(|(id, _)| id == &node.id)
            .map(|(_, step)| step);
        // 要模型但本地还没有：整个节点禁用，卡片上挂一个下载面板（见 `draw_node_controls`）。
        // 要一个外部程序（如 ffmpeg）但本地没有：一样禁用，卡片上挂一句提示。
        // 这两项都在 `refresh_ports` 里缓存好了 —— 绘制路径上不再碰磁盘。
        let model_missing = node.requirements.model_missing;
        let tool_missing = node.requirements.tool_missing;
        // 节点专色：拦住运行的节点用框架的「交互」专色（紫），
        // 其余按节点自己**声明**的节点类（如逻辑类）取色；没声明的走正常主题边线。
        let accent = if interactive {
            Some(theme::interactive_color())
        } else {
            kind_of(kinds, node)
                .and_then(|kind| kind.accent.as_deref())
                .and_then(theme::accent_color)
        };

        // 白底（投影已经在上一遍里铺好了）。
        painter.rect_filled(r, cr, theme::SURFACE);

        let border = if marks.failed_nodes.contains(&node.id) {
            theme::DANGER
        } else if running {
            theme::ACCENT
        } else if model_missing || tool_missing {
            theme::WARN
        } else if let Some(ink) = accent {
            ink
        } else if marks.ok_nodes.contains(&node.id) {
            theme::ACCENT
        } else {
            theme::HAIRLINE
        };
        // 正在等用户操作 / 正在跑 / 有专色：把边框加粗一档。
        let border_width = if waiting {
            3.0
        } else if running || accent.is_some() || model_missing || tool_missing {
            1.5
        } else {
            1.0
        };

        // ---- 头部 ----
        let head_h = HEADER_H * z;
        let cy = r.top() + head_h * 0.5;
        painter.line_segment(
            [
                egui::pos2(r.left(), r.top() + head_h),
                egui::pos2(r.right(), r.top() + head_h),
            ],
            Stroke::new(1.0, theme::HAIRLINE),
        );

        let title =
            painter.layout_no_wrap(node.title.clone(), FontId::monospace(12.0 * z), theme::INK);
        let x = r.left() + 10.0 * z;
        painter.galley(egui::pos2(x, cy - title.size().y * 0.5), title, theme::INK);

        // 状态 + 工具（鼠标悬停或选中才露出工具）。
        let hovered =
            pointer.is_some_and(|p| r.contains(p)) && self.hovered_node(kinds, pointer) == Some(i);
        let show_tools = hovered || self.selected == Some(i);
        let (play_rect, del_rect) = self.node_tool_rects(kinds, i);
        let can_run = self.can_run_node(kinds, i);
        let right_edge = if show_tools {
            (if can_run {
                play_rect.left()
            } else {
                del_rect.left()
            }) - 6.0 * z
        } else {
            r.right() - 8.0 * z
        };

        if running {
            // 跑到它了：右边一个转圈的箭头 + 「运行中」，一眼看出光标在哪儿。
            // 进度（帧数 / 百分比 / 进度条）画在卡片下方的信息区，不挤在这里。
            let spinner = Rect::from_center_size(
                egui::pos2(right_edge - 6.0 * z, cy),
                egui::Vec2::splat(10.0 * z),
            );
            icons::loader(painter, spinner, theme::ACCENT, (now * 1.2) as f32 % 1.0);
            painter.text(
                egui::pos2(right_edge - 13.0 * z, cy),
                Align2::RIGHT_CENTER,
                "运行中",
                FontId::monospace(9.5 * z),
                theme::ACCENT,
            );
        } else if model_missing {
            // 要模型但本地没有：右边一个「需模型」，提示这个节点还不能跑。
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                "需模型",
                FontId::monospace(9.5 * z),
                theme::WARN,
            );
        } else if tool_missing {
            // 要外部程序但本地没有：把工具名也报出来，好知道去装什么。
            let label = match kind_of(kinds, node).and_then(|kind| kind.requires_tool.as_deref()) {
                Some(tool) => format!("需 {tool}"),
                None => "缺工具".to_string(),
            };
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                label,
                FontId::monospace(9.5 * z),
                theme::WARN,
            );
        } else if node.requirements.tool_checking {
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                "检查工具…",
                FontId::monospace(9.5 * z),
                theme::INK_3,
            );
        } else if let Some(run) = &node.run {
            let (text, color) = match run.status {
                NodeStatus::Ok => ("完成", theme::OK),
                NodeStatus::Failed => ("出错", theme::DANGER),
                NodeStatus::Skipped => ("跳过", theme::INK_3),
            };
            painter.text(
                egui::pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                format!("{text} {}ms", run.ms),
                FontId::monospace(9.5 * z),
                color,
            );
        }

        if show_tools {
            for (rect, icon) in [
                (play_rect, icons::play as IconFn),
                (del_rect, icons::x as IconFn),
            ] {
                if rect == play_rect && !can_run {
                    continue;
                }
                let hot = pointer.is_some_and(|p| rect.contains(p));
                if hot {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same((theme::R_CTL as f32 * z) as u8),
                        theme::SURFACE_3,
                    );
                }
                icon(
                    painter,
                    widgets::icon_rect(rect, 12.0 * z),
                    if hot { theme::INK } else { theme::INK_2 },
                );
            }
        }

        // ---- 端口（输入在左、输出在右，同一行对齐） ----
        // 注意这几行放到函数**最后**去画：端口要盖在卡片边线上，不能被它切一刀。

        // ---- 参数区底色（浅灰，圆底） ----
        // 参数区以下各段的高度只算一次（`card_sections`），下面按顺序往下画。
        let sections = card_sections(kinds, node, &self.notes);
        let params_last = sections.total() <= 0.0;
        let params_bottom =
            r.top() + (node_body_top(node) + params_height_of(kinds, node, &self.notes)) * z;
        if let Some(kind) = kind_of(kinds, node) {
            let ph = params_height(kind, &node.params, &self.notes);
            if ph > 0.0 {
                let top = r.top() + node_body_top(node) * z;
                let rect =
                    Rect::from_min_size(egui::pos2(r.left(), top), egui::vec2(r.width(), ph * z));
                painter.rect_filled(
                    rect,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: if params_last { theme::R_CARD } else { 0 },
                        se: if params_last { theme::R_CARD } else { 0 },
                    },
                    theme::SURFACE_2,
                );
                painter.line_segment(
                    [egui::pos2(rect.left(), top), egui::pos2(rect.right(), top)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
            }
        }

        // ---- 「缺东西」面板的底色（紧贴参数区，控件由 `draw_node_controls` 画） ----
        if sections.requirement > 0.0 {
            let rect = Rect::from_min_size(
                egui::pos2(r.left(), params_bottom),
                egui::vec2(r.width(), sections.requirement * z),
            );
            painter.rect_filled(
                rect,
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: theme::R_CARD,
                    se: theme::R_CARD,
                },
                theme::WARN_SOFT,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left(), params_bottom),
                    egui::pos2(rect.right(), params_bottom),
                ],
                Stroke::new(1.0, theme::WARN_LINE),
            );
        }

        // ---- 参数区以下的几段 ----
        let body_pad = if sections.requirement > 0.0 {
            0.0
        } else if sections.total() > 0.0 {
            BODY_PAD
        } else {
            0.0
        };
        let mut y = params_bottom + (sections.requirement + body_pad) * z;

        // ---- 运行中的进度（长任务才有这一段）：帧计数 + 百分比 + 会波的进度条 ----
        if sections.progress > 0.0 {
            if let Some(step) = step {
                let last = sections.warn == 0.0
                    && sections.error == 0.0
                    && sections.actions == 0.0
                    && !node.preview
                    && sections.palette == 0.0;
                let panel = Rect::from_min_size(
                    egui::pos2(r.left(), y),
                    egui::vec2(r.width(), sections.progress * z),
                );
                painter.rect_filled(
                    panel,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: if last { theme::R_CARD } else { 0 },
                        se: if last { theme::R_CARD } else { 0 },
                    },
                    theme::ACCENT_SOFT,
                );
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );

                let inner = Rect::from_min_max(
                    egui::pos2(r.left() + 10.0 * z, y + 7.0 * z),
                    egui::pos2(r.right() - 10.0 * z, y + PROGRESS_H * z - 7.0 * z),
                );
                let percent = match step.fraction {
                    Some(fraction) => format!("{}%", (fraction * 100.0).round() as i32),
                    None => "…".to_string(),
                };
                let left_text = match (step.frame, step.total_frames) {
                    (Some(frame), Some(total)) => format!("帧 {frame}/{total}"),
                    (Some(frame), None) => format!("帧 {frame}"),
                    _ => "处理中".to_string(),
                };
                let right_text = match &step.text {
                    Some(text) => format!("{percent} · {text}"),
                    None => percent,
                };
                let font = FontId::monospace(9.5 * z);
                painter.text(
                    egui::pos2(inner.left(), inner.top()),
                    Align2::LEFT_TOP,
                    left_text,
                    font.clone(),
                    theme::INK_2,
                );
                painter.text(
                    egui::pos2(inner.right(), inner.top()),
                    Align2::RIGHT_TOP,
                    right_text,
                    font,
                    theme::INK_2,
                );

                let bar = Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - 12.0 * z),
                    egui::pos2(inner.right(), inner.bottom()),
                );
                draw_progress_bar(&body, bar, step.fraction, now);
            }
            y += sections.progress * z;
        }

        if node.preview {
            if let Some(texture) = self.textures.card.get(&node.id) {
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
                let inner = Rect::from_min_max(
                    egui::pos2(r.left() + 10.0 * z, y + 8.0 * z),
                    egui::pos2(r.right() - 10.0 * z, y + PREVIEW_H * z - 9.0 * z),
                );
                self.draw_preview(painter, inner, texture, theme::SURFACE);
            }
            y += sections.preview * z;
        }

        // ---- 色板：一排小色块 ----
        if let Some(palette) = &node.palette {
            if !palette.is_empty() {
                painter.line_segment(
                    [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                    Stroke::new(1.0, theme::HAIRLINE),
                );
                draw_palette(
                    painter,
                    Rect::from_min_size(
                        egui::pos2(r.left() + 10.0 * z, y + 8.0 * z),
                        egui::vec2(r.width() - 20.0 * z, PALETTE_H * z - 16.0 * z),
                    ),
                    palette,
                    z,
                );
            }
            y += sections.palette * z;
        }

        // ---- 运行提示 ----
        // 不再是一排橙色的「!」，而是一块淡蓝底 + 信息图标 + 会折行的正文：
        // 读完是「知道发生了什么」而不是「出事了」。
        if sections.warn > 0.0 {
            if let Some(run) = &node.run {
                let last = sections.error == 0.0 && sections.actions == 0.0;
                let panel = Rect::from_min_size(
                    egui::pos2(r.left(), y),
                    egui::vec2(r.width(), sections.warn * z),
                );
                painter.rect_filled(
                    panel,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: if last { theme::R_CARD } else { 0 },
                        se: if last { theme::R_CARD } else { 0 },
                    },
                    theme::ACCENT_SOFT,
                );
                let mut ny = y + NOTICE_PAD * z;
                for warning in &run.warnings {
                    let icon = Rect::from_min_size(
                        egui::pos2(r.left() + 10.0 * z, ny),
                        egui::Vec2::splat(NOTICE_ICON * z),
                    );
                    icons::info(&body, icon, theme::ACCENT);
                    if let Some(block) = self.notes.get(warning) {
                        draw_block(
                            &body,
                            block,
                            z,
                            icon.right() + NOTICE_GAP * z,
                            ny,
                            theme::INK_2,
                        );
                    }
                    ny += (notice_height(warning, &self.notes) + NOTICE_STACK) * z;
                }
            }
            y += sections.warn * z;
        }

        if sections.error > 0.0 {
            if let Some(run) = &node.run {
                if let Some(error) = &run.error {
                    let last = sections.actions == 0.0;
                    let rect = Rect::from_min_size(
                        egui::pos2(r.left(), y),
                        egui::vec2(r.width(), sections.error * z),
                    );
                    painter.rect_filled(
                        rect,
                        CornerRadius {
                            nw: 0,
                            ne: 0,
                            sw: if last { theme::R_CARD } else { 0 },
                            se: if last { theme::R_CARD } else { 0 },
                        },
                        theme::DANGER_SOFT,
                    );
                    // 错误也是会折行的 —— 同样按定死的折行画，不靠定值。
                    if let Some(block) = self.notes.get(error) {
                        draw_block(
                            &body,
                            block,
                            z,
                            r.left() + 10.0 * z,
                            y + NOTICE_PAD * z,
                            theme::DANGER,
                        );
                    }
                }
            }
            y += sections.error * z;
        }

        if sections.actions > 0.0 {
            if let Some(run) = &node.run {
                if let Some(file) = &run.file {
                    painter.line_segment(
                        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                        Stroke::new(1.0, theme::HAIRLINE),
                    );
                    let (reveal, save) = self.node_action_rects(kinds, i);
                    for (rect, icon) in [
                        (reveal, icons::folder_search as IconFn),
                        (save, icons::download as IconFn),
                    ] {
                        let hot = pointer.is_some_and(|p| rect.contains(p));
                        if hot {
                            painter.rect_filled(
                                rect,
                                CornerRadius::same((theme::R_CTL as f32 * z) as u8),
                                theme::SURFACE_3,
                            );
                        }
                        icon(
                            painter,
                            widgets::icon_rect(rect, 12.0 * z),
                            if hot { theme::INK } else { theme::INK_3 },
                        );
                    }
                    let name = std::path::Path::new(file)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| file.clone());
                    body.text(
                        egui::pos2(save.right() + 6.0 * z, save.center().y),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::monospace(9.0 * z),
                        theme::INK_3,
                    );
                }
            }
        }

        // 卡片边线最后画 —— 否则会被参数区那层铺满整宽的灰底盖住（失败时尤其明显）。
        painter.rect_stroke(r, cr, Stroke::new(border_width, border), StrokeKind::Inside);
        if self.selected == Some(i) {
            painter.rect_stroke(
                r.expand(2.5),
                CornerRadius::same(theme::R_CARD + 2),
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
        }

        // ---- 端口最后画：压在边线上面 ----
        self.draw_ports(painter, kinds, marks, i, r, z, pointer, now);
    }

    /// 画一个节点所有端口。**最后画** —— 端口要压住卡片边线，不能被它切一刀。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_ports(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        i: usize,
        r: Rect,
        z: f32,
        pointer: Option<Pos2>,
        now: f64,
    ) {
        let node = &self.nodes[i];
        let rows = node.inputs.len().max(node.outputs.len());
        for k in 0..rows {
            if let Some(port) = node.inputs.get(k) {
                self.draw_port(painter, kinds, marks, i, k, port, true, r, z, pointer, now);
            }
            if let Some(port) = node.outputs.get(k) {
                self.draw_port(painter, kinds, marks, i, k, port, false, r, z, pointer, now);
            }
        }
    }

    /// 画一个端口：卡片边线上的圆点 + 徽标 + 标签。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_port(
        &self,
        painter: &egui::Painter,
        kinds: &[NodeKindInfo],
        marks: &Marks,
        i: usize,
        k: usize,
        port: &PortDef,
        input: bool,
        r: Rect,
        z: f32,
        _pointer: Option<Pos2>,
        now: f64,
    ) {
        let here = PortRef {
            node: i,
            port: k,
            input,
        };
        let p = if input {
            self.view.to_screen(self.in_port(kinds, i, k))
        } else {
            self.view.to_screen(self.out_port(kinds, i, k))
        };
        let linked = self.port_linked(i, &port.id, input);
        // 出错的（非法连线 / 必填未接）→ 红；**可选**（非必填）端口→浅蓝
        // （参数端口、以及「顺序执行」「遍历」那些可选输出都算）；必填→主题蓝。
        let bad = self.port_invalid(marks, i, &port.id, input) || (port.required && !linked);
        let dot = if bad {
            theme::DANGER
        } else if port.required {
            theme::ACCENT
        } else {
            theme::ACCENT_LIGHT
        };
        let type_ink = if port.exec {
            theme::INK_2
        } else {
            theme::badge_color(port.ty)
        };

        // 拉线的**起点**和它可能落下的**落点** —— 两端都用同一套「正要连上」的动画
        // （同一个 `port_pending`），看起来才是一件东西。
        let is_source = self
            .interaction
            .connect
            .as_ref()
            .is_some_and(|connect| connect.from == here);
        let hovering = self.interaction.hovered_port == Some(here);
        let valid = self.is_valid_target(here);

        // 端点「被强调」时统一的一套观感 —— 拉线的**起点**、它可能落下的**落点**、
        // 以及只是被悬停的端点，圈的大小都一样，只有「正要连上」多一圈搏动的描边。
        let pending = is_source || valid;
        if pending || hovering {
            port_halo(painter, p, z, now, pending, port.exec);
        }

        // 圆点 / 三角形随强调状态轻微呼吸；光晕与本体用同一个节奏。
        let pulse = 0.5 + 0.5 * (now * easing::BREATH).sin() as f32;
        let scale = self.port_scale(here)
            * if pending || hovering {
                1.0 + pulse * 0.10
            } else {
                1.0
            };
        if port.exec {
            let radius = 5.0 * z * scale;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p + egui::vec2(-radius, -radius),
                    p + egui::vec2(radius, 0.0),
                    p + egui::vec2(-radius, radius),
                ],
                dot,
                Stroke::new(1.0, theme::SURFACE),
            ));
        } else {
            painter.circle_filled(p, 5.0 * z * scale, theme::SURFACE);
            painter.circle_filled(p, 3.0 * z * scale, dot);
        }
        if bad {
            stroke_port_shape(
                painter,
                p,
                6.0 * z * scale,
                Stroke::new(1.0, theme::DANGER),
                port.exec,
            );
        }

        // 参数端口 / 参数区下方的输入：只出小圆点，徽标与标签由参数那一行自己画。
        // 挂在参数栏上的**输出**不提前返回：每一行都要有自己的 Exec 徽标和序号，
        // 位置跟着端口圆点走，而不是在参数块中间画一次。
        if port.is_param() || port.panel_input_row.is_some() {
            return;
        }

        // 徽标（chip）：带色的小框。
        let badge = painter.layout_no_wrap(
            if port.exec {
                "Exec".to_owned()
            } else {
                port.badge().to_owned()
            },
            FontId::monospace(8.0 * z),
            type_ink,
        );
        let label = painter.layout_no_wrap(
            port.label.clone(),
            FontId::monospace(10.0 * z),
            if bad { theme::DANGER } else { theme::INK_3 },
        );
        let pad = 4.0 * z;
        let chip_w = badge.size().x + pad * 2.0;
        let chip_h = badge.size().y + 2.0 * z;
        let gap = 4.0 * z;

        let chip = if input {
            Rect::from_min_size(
                egui::pos2(r.left() + 10.0 * z, p.y - chip_h * 0.5),
                egui::vec2(chip_w, chip_h),
            )
        } else {
            // 输出这一侧：[徽标][标签][圆点]，整体往右靠。
            let right = r.right() - 10.0 * z;
            Rect::from_min_size(
                egui::pos2(right - label.size().x - gap - chip_w, p.y - chip_h * 0.5),
                egui::vec2(chip_w, chip_h),
            )
        };

        painter.rect_stroke(
            chip,
            CornerRadius::same((3.0 * z) as u8),
            Stroke::new(1.0, type_ink),
            StrokeKind::Inside,
        );
        // 徽标与标签都按**墨迹中心**对齐到胶囊中线 ——
        // 汉字墨迹在框里偏下、拉丁徽标偏上，各自按框高校正就会显得「下端对齐」。
        painter.galley(
            egui::pos2(
                chip.center().x - badge.size().x * 0.5,
                crate::ui::widgets::ink_top(&badge, chip.center().y),
            ),
            badge,
            type_ink,
        );

        let label_color = if bad { theme::DANGER } else { theme::INK_3 };
        painter.galley(
            egui::pos2(
                chip.right() + gap,
                crate::ui::widgets::ink_top(&label, chip.center().y),
            ),
            label,
            label_color,
        );
    }
}

/// 背景点阵用的小圆贴图：一张 `SIDE×SIDE` 的白色圆，带一点抗锯齿。
///
/// 按 `(图标, 尺寸)` 的思路缓存进 `ctx.data`，只建一次。
fn dot_texture(ctx: &egui::Context) -> egui::TextureHandle {
    const SIDE: usize = 16;
    let id = egui::Id::new("starrytools-canvas-dot");
    if let Some(handle) = ctx.data_mut(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let center = (SIDE as f32 - 1.0) / 2.0;
    // 圆几乎顶满贴图，这样贴到屏幕上的实际半径才和原来 `circle_filled(1.0)` 接近。
    let radius = SIDE as f32 / 2.0 - 0.5;
    let mut pixels = Vec::with_capacity(SIDE * SIDE);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            pixels.push(theme::white_alpha((coverage * 255.0) as u8));
        }
    }
    let handle = ctx.load_texture(
        "canvas-dot",
        egui::ColorImage::new([SIDE, SIDE], pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
    handle
}

/// 往网格里塞一颗小圆（用一个采样小圆贴图的四边形）。
fn push_dot(mesh: &mut egui::Mesh, center: Pos2, radius: f32, color: Color32) {
    let base = mesh.vertices.len() as u32;
    let half = egui::vec2(radius, radius);
    let (min, max) = (center - half, center + half);
    let vertex = |pos: Pos2, uv: Pos2| egui::epaint::Vertex { pos, uv, color };
    mesh.vertices.push(vertex(min, egui::pos2(0.0, 0.0)));
    mesh.vertices
        .push(vertex(egui::pos2(max.x, min.y), egui::pos2(1.0, 0.0)));
    mesh.vertices.push(vertex(max, egui::pos2(1.0, 1.0)));
    mesh.vertices
        .push(vertex(egui::pos2(min.x, max.y), egui::pos2(0.0, 1.0)));
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// 一排小色块，展示一个色板。色块太多就只画能放下的那些。
fn draw_palette(painter: &egui::Painter, band: Rect, palette: &[[u8; 4]], z: f32) {
    if palette.is_empty() || band.width() <= 0.0 || band.height() <= 0.0 {
        return;
    }
    let gap = 3.0 * z;
    let min_w = 4.0 * z;
    let max_w = 22.0 * z;
    let radius = CornerRadius::same((2.0 * z).round().max(1.0) as u8);
    let count = palette.len();

    // 放得下就一个不落；放不下就按能放的数量截断，别挤成一团。
    let width = (band.width() - gap * (count as f32 - 1.0)) / count as f32;
    let (shown, width) = if width >= min_w {
        (count, width.min(max_w))
    } else {
        let fits = (((band.width() + gap) / (min_w + gap)).floor() as usize).clamp(1, count);
        (
            fits,
            (band.width() - gap * (fits as f32 - 1.0)) / fits as f32,
        )
    };

    let mut x = band.left();
    for color in palette.iter().take(shown) {
        let cell = Rect::from_min_size(egui::pos2(x, band.top()), egui::vec2(width, band.height()));
        painter.rect_filled(
            cell,
            radius,
            Color32::from_rgb(color[0], color[1], color[2]),
        );
        painter.rect_stroke(
            cell,
            radius,
            Stroke::new(1.0, theme::HAIRLINE),
            StrokeKind::Inside,
        );
        x += width + gap;
    }
}

/// 端点光晕：形状跟端口一致（圆点用圆，Exec 三角用三角），并随呼吸伸缩。
///
/// `pending`（拉线起点 / 可落点）时外扩更明显并带一圈描边；普通悬停只做轻微呼吸。
/// 要改这份动画，只改这里一处。
fn port_halo(painter: &egui::Painter, p: Pos2, z: f32, now: f64, pending: bool, exec: bool) {
    let pulse = 0.5 + 0.5 * (now * easing::BREATH).sin() as f32;
    let base = 8.5 * z;
    if pending {
        paint_port_shape(
            painter,
            p,
            base * (1.0 + pulse * 0.24),
            theme::accent_alpha(0.18),
            exec,
        );
        stroke_port_shape(
            painter,
            p,
            base * 0.88,
            Stroke::new(1.5, theme::ACCENT),
            exec,
        );
    } else {
        paint_port_shape(
            painter,
            p,
            base * (1.0 + pulse * 0.10),
            theme::accent_alpha(0.12 + pulse * 0.08),
            exec,
        );
    }
}

/// Exec 端口标记是朝右的三角形；数据端口是圆。光晕必须用同一轮廓。
fn port_outline(center: Pos2, radius: f32, exec: bool) -> Vec<Pos2> {
    if exec {
        vec![
            center + egui::vec2(-radius, -radius),
            center + egui::vec2(radius, 0.0),
            center + egui::vec2(-radius, radius),
        ]
    } else {
        Vec::new()
    }
}

fn paint_port_shape(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    color: egui::Color32,
    exec: bool,
) {
    if exec {
        painter.add(egui::Shape::convex_polygon(
            port_outline(center, radius, true),
            color,
            Stroke::NONE,
        ));
    } else {
        painter.circle_filled(center, radius, color);
    }
}

fn stroke_port_shape(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    stroke: Stroke,
    exec: bool,
) {
    if exec {
        painter.add(egui::Shape::closed_line(
            port_outline(center, radius, true),
            stroke,
        ));
    } else {
        painter.circle_stroke(center, radius, stroke);
    }
}

/// 一条「胶片」进度条：一条实心蓝条，里面一条条细白纹随进度向右流动，
/// 前沿一道细亮线。全是纯色细线，**不用渐变、也没有圆点**。
///
/// 动画落在**条身**上（细纹在走），所以哪怕进度一时不动，它看着也是活的。
/// `fraction` 是 `None` 时画**不确定态**：整块胶片在轨道上来回扫。
/// 时间从 `now` 来，与帧率无关；跑着的时候外壳会持续申请重绘。
fn draw_progress_bar(painter: &egui::Painter, rect: Rect, fraction: Option<f32>, now: f64) {
    if rect.width() <= 6.0 || rect.height() <= 3.0 {
        return;
    }
    let r = (rect.height() * 0.28).round() as u8;
    let radius = CornerRadius::same(r);
    // 轨道：浅灰底 + 一道发丝边框。略方的角，比胶囊更像一条胶片。
    painter.rect_filled(rect, radius, theme::SURFACE_3);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::HAIRLINE),
        StrokeKind::Inside,
    );

    // 胶片上的细纹：间距固定，整体随 `now` 向右匀速流动（与帧率无关）。
    const SPACING: f32 = 9.0;
    let offset = (now as f32 * 26.0).rem_euclid(SPACING);

    // 画一段胶片：实心蓝底 + 流动细纹 + 前沿细亮线。
    let draw_strip = |painter: &egui::Painter, strip: Rect| {
        if strip.width() <= 1.5 {
            return;
        }
        // 结束那头贴着轨道右端时（进度跑满）才收圆角，否则前沿是切平的。
        let flush = strip.right() >= rect.right() - 1.0;
        let corners = CornerRadius {
            nw: r,
            sw: r,
            ne: if flush { r } else { 0 },
            se: if flush { r } else { 0 },
        };
        let clip = painter.with_clip_rect(strip);
        clip.rect_filled(strip, corners, theme::ACCENT);

        let mut x = strip.left() - SPACING + offset;
        while x < strip.right() {
            if x > strip.left() + 1.0 {
                clip.line_segment(
                    [
                        egui::pos2(x, strip.top() + 2.5),
                        egui::pos2(x, strip.bottom() - 2.5),
                    ],
                    Stroke::new(1.0, theme::white_alpha(56)),
                );
            }
            x += SPACING;
        }

        if strip.width() > 2.5 {
            clip.line_segment(
                [
                    egui::pos2(strip.right() - 1.0, strip.top()),
                    egui::pos2(strip.right() - 1.0, strip.bottom()),
                ],
                Stroke::new(2.0, theme::INK_ON_COLOR),
            );
        }
    };

    match fraction {
        Some(fraction) => {
            let width = rect.width() * fraction.clamp(0.0, 1.0);
            if width <= 1.5 {
                return;
            }
            let strip = Rect::from_min_size(rect.min, egui::vec2(width, rect.height()));
            draw_strip(painter, strip);
        }
        None => {
            // 不确定态：整块胶片在轨道上来回扫，细纹照旧在它自己身上流。
            let block_w = rect.width() * 0.3;
            let sweep = (now * 0.5).rem_euclid(1.0) as f32;
            let x = rect.left() - block_w + sweep * (rect.width() + block_w);
            let strip = Rect::from_min_size(
                egui::pos2(x, rect.top()),
                egui::vec2(block_w, rect.height()),
            );
            draw_strip(painter, strip);
        }
    }
}
