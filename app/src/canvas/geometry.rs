//! 刀光的几何。
//!
//! 刻意写成不碰界面的纯函数 —— 「哪几条连线该被切断」是这件事里
//! 唯一会出错的部分，值得单独测。
//!
//! 不一样的是：那边得先从 DOM 里把 SVG `<path>` 读出来再 `getPointAtLength` 采样，
//! 所以 `onlyRenderVisibleElements` 不能开；这边连线的控制点本来就在我们手上，
//! 采样是自己算的，也不受虚拟化影响。

use eframe::egui::Pos2;

/// 一条三次贝塞尔的四个控制点。
pub type Cubic = [Pos2; 4];

/// 采样步数的上下限 —— 曲率大的地方密一点，但别为了几像素的精度采上千次。
const STEPS_MIN: usize = 8;
const STEPS_MAX: usize = 64;

/// 曲线在 `t` 处的点。
pub fn at(curve: &Cubic, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Pos2::new(
        b0 * curve[0].x + b1 * curve[1].x + b2 * curve[2].x + b3 * curve[3].x,
        b0 * curve[0].y + b1 * curve[1].y + b2 * curve[2].y + b3 * curve[3].y,
    )
}

/// 按控制多边形的长度估一个采样步数。
pub fn steps_for(curve: &Cubic) -> usize {
    let rough = (curve[1] - curve[0]).length()
        + (curve[2] - curve[1]).length()
        + (curve[3] - curve[2]).length();
    (rough / 8.0).ceil() as usize
}

/// 采样成折线。`t` 是均匀取的，所以第 `i` 个点对应 `t = i / (n - 1)`。
pub fn sample(curve: &Cubic, steps: usize) -> Vec<Pos2> {
    let steps = steps.clamp(STEPS_MIN, STEPS_MAX);
    (0..=steps)
        .map(|i| at(curve, i as f32 / steps as f32))
        .collect()
}

/// 把曲线在 `t` 处切成两段（de Casteljau）。
///
/// 断裂动画要用：断口两侧各自缩回去，比在整条路径上用虚线掩码干净得多。
pub fn split(curve: &Cubic, t: f32) -> (Cubic, Cubic) {
    let lerp = |a: Pos2, b: Pos2| a + (b - a) * t;

    let p01 = lerp(curve[0], curve[1]);
    let p12 = lerp(curve[1], curve[2]);
    let p23 = lerp(curve[2], curve[3]);
    let p012 = lerp(p01, p12);
    let p123 = lerp(p12, p23);
    let mid = lerp(p012, p123);

    ([curve[0], p01, p012, mid], [mid, p123, p23, curve[3]])
}

/// 沿方向把线段延长，用来做收刀时的「跟出」。
pub fn extend(from: Pos2, to: Pos2, extra: f32) -> Pos2 {
    let d = to - from;
    let len = d.length();
    if len == 0.0 {
        return to;
    }
    from + d * ((len + extra) / len)
}

/// JS 的 `Math.sign`。**不能用 `f32::signum`** —— 那个把 `0.0` 判成 `+1`，
/// 下面「端点正好落在线段上」那种退化情形就永远进不去了。
fn sign(x: f32) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// 叉积，用来判断点在直线的哪一侧。
fn orient(p: Pos2, q: Pos2, r: Pos2) -> f32 {
    (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
}

/// `r` 是否落在 `p`-`q` 的包围盒里。
fn within(p: Pos2, q: Pos2, r: Pos2) -> bool {
    r.x >= p.x.min(q.x) && r.x <= p.x.max(q.x) && r.y >= p.y.min(q.y) && r.y <= p.y.max(q.y)
}

/// 两条线段是否相交（允许端点搭在另一条上）。
pub fn segments_intersect(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
    let s1 = sign(orient(a, b, c));
    let s2 = sign(orient(a, b, d));
    let s3 = sign(orient(c, d, a));
    let s4 = sign(orient(c, d, b));

    // 严格相交：两端各在对方两侧。
    if s1 * s2 < 0 && s3 * s4 < 0 {
        return true;
    }

    // 退化：某个端点正好落在线段上。
    (s1 == 0 && within(a, b, c))
        || (s2 == 0 && within(a, b, d))
        || (s3 == 0 && within(c, d, a))
        || (s4 == 0 && within(c, d, b))
}

/// 两条线段的交点。平行或共线时返回 `None`（那种情况按端点处理就够了）。
fn intersection_point(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> Option<Pos2> {
    let r = b - a;
    let s = d - c;
    let denominator = r.x * s.y - r.y * s.x;
    if denominator == 0.0 {
        return None;
    }
    let t = ((c.x - a.x) * s.y - (c.y - a.y) * s.x) / denominator;
    Some(a + r * t)
}

/// 折线与线段的**第一处**相交。
///
/// 返回断点在曲线上的参数 `t`（0–1，按折线分段位置换算，不是弧长比例 ——
/// 断裂动画要的是参数，不是长度）和交点（流坐标）。
pub fn first_hit(poly: &[Pos2], a: Pos2, b: Pos2) -> Option<(f32, Pos2)> {
    let last = poly.len().saturating_sub(1);
    if last == 0 {
        return None;
    }

    for i in 1..=last {
        let p = poly[i - 1];
        let q = poly[i];
        if !segments_intersect(a, b, p, q) {
            continue;
        }

        let hit = intersection_point(a, b, p, q).unwrap_or(p);

        // 交点在这小段里的位置，用来把分段下标还原成曲线的参数 t。
        let seg = (q - p).length();
        let frac = if seg > 0.0 {
            ((hit - p).length() / seg).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t = ((i - 1) as f32 + frac) / last as f32;

        return Some((t, hit));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    /// 一条从 (0,0) 到 (100,0) 的直线（用退化成直线的贝塞尔表示）。
    fn flat() -> Cubic {
        [
            pos2(0.0, 0.0),
            pos2(33.0, 0.0),
            pos2(66.0, 0.0),
            pos2(100.0, 0.0),
        ]
    }

    /// 一条向下弯的贝塞尔。
    fn curved() -> Cubic {
        [
            pos2(0.0, 0.0),
            pos2(0.0, 80.0),
            pos2(100.0, 80.0),
            pos2(100.0, 0.0),
        ]
    }

    #[test]
    fn sampling_hits_both_ends() {
        let points = sample(&curved(), 16);
        assert_eq!(points.len(), 17);
        assert!((points[0] - pos2(0.0, 0.0)).length() < 0.01);
        assert!((points[16] - pos2(100.0, 0.0)).length() < 0.01);
    }

    #[test]
    fn a_vertical_slash_cuts_a_flat_wire() {
        let poly = sample(&flat(), 32);
        let hit = first_hit(&poly, pos2(50.0, -20.0), pos2(50.0, 20.0));
        let (t, point) = hit.expect("应当切中");
        assert!((point.x - 50.0).abs() < 0.5, "切点 x = {}", point.x);
        assert!((t - 0.5).abs() < 0.05, "参数 t = {t}");
    }

    #[test]
    fn a_slash_that_misses_finds_nothing() {
        let poly = sample(&flat(), 32);
        assert!(first_hit(&poly, pos2(50.0, 20.0), pos2(50.0, 40.0)).is_none());
        assert!(first_hit(&poly, pos2(200.0, -20.0), pos2(200.0, 20.0)).is_none());
    }

    #[test]
    fn the_first_hit_is_the_near_end_not_the_far_one() {
        // 一条来回折的折线：先在第 1 段被切到，就不该报到第 3 段去。
        let poly = vec![
            pos2(0.0, 0.0),
            pos2(10.0, 0.0),
            pos2(10.0, 10.0),
            pos2(0.0, 10.0),
            pos2(0.0, 20.0),
        ];
        // 竖线 x = 5 会同时穿过第 0 段（y=0）和第 2 段（y=10）。
        let (t, point) = first_hit(&poly, pos2(5.0, -5.0), pos2(5.0, 25.0)).expect("应当切中");
        assert!(point.y < 1.0, "应当报近端那个，实际 y = {}", point.y);
        assert!(t < 0.3, "参数应当靠前，实际 t = {t}");
    }

    #[test]
    fn a_touching_endpoint_counts_as_an_intersection() {
        // 刀尖正好落在连线端点上 —— 原实现特意算了这种退化情形。
        assert!(segments_intersect(
            pos2(0.0, 0.0),
            pos2(10.0, 0.0),
            pos2(10.0, 0.0),
            pos2(10.0, 10.0)
        ));
    }

    #[test]
    fn parallel_segments_do_not_intersect() {
        assert!(!segments_intersect(
            pos2(0.0, 0.0),
            pos2(10.0, 0.0),
            pos2(0.0, 5.0),
            pos2(10.0, 5.0)
        ));
    }

    #[test]
    fn splitting_keeps_the_shape() {
        let curve = curved();
        let (left, right) = split(&curve, 0.35);

        // 两段拼起来必须和原曲线重合。
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let whole = at(&curve, t * 0.35);
            let piece = at(&left, t);
            assert!(
                (whole - piece).length() < 0.05,
                "t = {t}：整条 {whole:?} vs 左半 {piece:?}"
            );
        }

        // 切点本身也要对得上。
        assert!((left[3] - right[0]).length() < 0.001);
        assert!((left[3] - at(&curve, 0.35)).length() < 0.01);
    }

    #[test]
    fn extending_pushes_the_tip_out() {
        let from = pos2(0.0, 0.0);
        let to = pos2(10.0, 0.0);
        let tip = extend(from, to, 5.0);
        assert!((tip - pos2(15.0, 0.0)).length() < 0.001);
        // 长度为 0 时原样返回，别除零。
        assert_eq!(extend(from, from, 100.0), from);
    }
}
