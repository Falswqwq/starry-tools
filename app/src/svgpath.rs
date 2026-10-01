//! 一点点 SVG path 解析 —— 只为把 lucide 那套图标的原始 `d` 数据画出来。
//!
//! 目标不是做个通用解析器，而是把 lucide 用到的那几个命令（`M L H V C S Q T A Z`，
//! 大小写都有）折线化：曲线按固定段数采样、圆弧按端点参数化算法转成中心角再采样。
//! 这样界面上画的就是**原始图形**，而不是手描的近似。
//!
//! 坐标系就是图标的 24×24 视口，缩放交给调用方。

use std::f32::consts::TAU;

enum Tok {
    Cmd(char),
    Num(f32),
}

fn tokenize(d: &str) -> Vec<Tok> {
    let cs: Vec<char> = d.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c == ' ' || c == ',' || c == '\n' || c == '\t' || c == '\r' {
            i += 1;
        } else {
            let start = i;
            if cs[i] == '+' || cs[i] == '-' {
                i += 1;
            }
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i < cs.len() && cs[i] == '.' {
                i += 1;
                while i < cs.len() && cs[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < cs.len() && (cs[i] == 'e' || cs[i] == 'E') {
                i += 1;
                if i < cs.len() && (cs[i] == '+' || cs[i] == '-') {
                    i += 1;
                }
                while i < cs.len() && cs[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = cs[start..i].iter().collect();
            out.push(Tok::Num(text.parse().unwrap_or(0.0)));
        }
    }
    out
}

fn num(tokens: &[Tok], j: &mut usize) -> f32 {
    if let Some(Tok::Num(v)) = tokens.get(*j) {
        *j += 1;
        *v
    } else {
        0.0
    }
}

/// 把一条 `d` 折线化成若干条子路径（每条是一串 24×24 坐标里的点）。
pub fn flatten(d: &str) -> Vec<Vec<(f32, f32)>> {
    let tokens = tokenize(d);
    let mut paths: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut last_cubic: Option<(f32, f32)> = None;
    let mut last_quad: Option<(f32, f32)> = None;
    let mut cmd = ' ';
    let mut j = 0;

    while j < tokens.len() {
        if let Tok::Cmd(c) = tokens[j] {
            cmd = c;
            j += 1;
            if c == 'Z' || c == 'z' {
                if cur.len() > 1 {
                    cur.push((sx, sy));
                }
                if !cur.is_empty() {
                    paths.push(std::mem::take(&mut cur));
                }
                x = sx;
                y = sy;
            }
            continue;
        }

        let rel = cmd.is_ascii_lowercase();
        let abs = |v: f32, base: f32| if rel { base + v } else { v };
        match cmd.to_ascii_uppercase() {
            'M' => {
                x = abs(num(&tokens, &mut j), x);
                y = abs(num(&tokens, &mut j), y);
                sx = x;
                sy = y;
                if !cur.is_empty() {
                    paths.push(std::mem::take(&mut cur));
                }
                cur.push((x, y));
                // 后面再来坐标对，按 lineto 处理。
                cmd = if rel { 'l' } else { 'L' };
                last_cubic = None;
                last_quad = None;
            }
            'L' => {
                x = abs(num(&tokens, &mut j), x);
                y = abs(num(&tokens, &mut j), y);
                cur.push((x, y));
                last_cubic = None;
                last_quad = None;
            }
            'H' => {
                x = abs(num(&tokens, &mut j), x);
                cur.push((x, y));
                last_cubic = None;
                last_quad = None;
            }
            'V' => {
                y = abs(num(&tokens, &mut j), y);
                cur.push((x, y));
                last_cubic = None;
                last_quad = None;
            }
            'C' => {
                let c1 = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                let c2 = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                let end = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                cubic(&mut cur, (x, y), c1, c2, end, 16);
                last_cubic = Some(c2);
                last_quad = None;
                x = end.0;
                y = end.1;
            }
            'S' => {
                let c1 = match last_cubic {
                    Some((lx, ly)) => (2.0 * x - lx, 2.0 * y - ly),
                    None => (x, y),
                };
                let c2 = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                let end = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                cubic(&mut cur, (x, y), c1, c2, end, 16);
                last_cubic = Some(c2);
                last_quad = None;
                x = end.0;
                y = end.1;
            }
            'Q' => {
                let q = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                let end = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                quad(&mut cur, (x, y), q, end, 16);
                last_quad = Some(q);
                last_cubic = None;
                x = end.0;
                y = end.1;
            }
            'T' => {
                let q = match last_quad {
                    Some((lx, ly)) => (2.0 * x - lx, 2.0 * y - ly),
                    None => (x, y),
                };
                let end = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                quad(&mut cur, (x, y), q, end, 16);
                last_quad = Some(q);
                last_cubic = None;
                x = end.0;
                y = end.1;
            }
            'A' => {
                let rx = num(&tokens, &mut j);
                let ry = num(&tokens, &mut j);
                let rot = num(&tokens, &mut j);
                let large = num(&tokens, &mut j) != 0.0;
                let sweep = num(&tokens, &mut j) != 0.0;
                let end = (abs(num(&tokens, &mut j), x), abs(num(&tokens, &mut j), y));
                arc(&mut cur, (x, y), rx, ry, rot, large, sweep, end);
                last_cubic = None;
                last_quad = None;
                x = end.0;
                y = end.1;
            }
            _ => {
                // 认不出的命令：吃掉一个数，免得死循环。
                j += 1;
            }
        }
    }

    if !cur.is_empty() {
        paths.push(cur);
    }
    paths
}

fn cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    n: usize,
) {
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let mt = 1.0 - t;
        let x = mt * mt * mt * p0.0
            + 3.0 * mt * mt * t * p1.0
            + 3.0 * mt * t * t * p2.0
            + t * t * t * p3.0;
        let y = mt * mt * mt * p0.1
            + 3.0 * mt * mt * t * p1.1
            + 3.0 * mt * t * t * p2.1
            + t * t * t * p3.1;
        out.push((x, y));
    }
}

fn quad(out: &mut Vec<(f32, f32)>, p0: (f32, f32), q: (f32, f32), p3: (f32, f32), n: usize) {
    let c1 = (
        p0.0 + 2.0 / 3.0 * (q.0 - p0.0),
        p0.1 + 2.0 / 3.0 * (q.1 - p0.1),
    );
    let c2 = (
        p3.0 + 2.0 / 3.0 * (q.0 - p3.0),
        p3.1 + 2.0 / 3.0 * (q.1 - p3.1),
    );
    cubic(out, p0, c1, c2, p3, n);
}

/// 圆弧（SVG 端点参数化）→ 采样点。
#[allow(clippy::too_many_arguments)]
fn arc(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    mut rx: f32,
    mut ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    p1: (f32, f32),
) {
    if rx == 0.0 || ry == 0.0 {
        out.push(p1);
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    let (sp, cp) = (rot_deg.to_radians()).sin_cos();
    let dx2 = (p0.0 - p1.0) / 2.0;
    let dy2 = (p0.1 - p1.1) / 2.0;
    let x1p = cp * dx2 + sp * dy2;
    let y1p = -sp * dx2 + cp * dy2;

    let mut rx2 = rx * rx;
    let mut ry2 = ry * ry;
    let lambda = x1p * x1p / rx2 + y1p * y1p / ry2;
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
        rx2 = rx * rx;
        ry2 = ry * ry;
    }

    let sign = if large != sweep { 1.0 } else { -1.0 };
    let numerator = (rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p).max(0.0);
    let denominator = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let coef = sign * (numerator / denominator.max(1e-12)).sqrt();
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cp * cxp - sp * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sp * cxp + cp * cyp + (p0.1 + p1.1) / 2.0;

    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let a = (dot / len.max(1e-12)).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let u = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let v = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta1 = angle(1.0, 0.0, u.0, u.1);
    let mut dtheta = angle(u.0, u.1, v.0, v.1);
    if !sweep && dtheta > 0.0 {
        dtheta -= TAU;
    }
    if sweep && dtheta < 0.0 {
        dtheta += TAU;
    }

    let n = (((dtheta.abs() / TAU) * 48.0).ceil() as usize).max(2);
    for k in 1..=n {
        let t = theta1 + dtheta * (k as f32 / n as f32);
        let (st, ct) = t.sin_cos();
        out.push((
            cx + rx * ct * cp - ry * st * sp,
            cy + rx * ct * sp + ry * st * cp,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_line_gives_two_points() {
        let paths = flatten("M5 12h14");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0][0], (5.0, 12.0));
        assert_eq!(*paths[0].last().unwrap(), (19.0, 12.0));
    }

    #[test]
    fn relative_commands_accumulate() {
        let paths = flatten("m18 15-6-6-6 6");
        assert_eq!(paths[0][0], (18.0, 15.0));
        assert_eq!(*paths[0].last().unwrap(), (6.0, 15.0));
    }

    #[test]
    fn a_closed_arc_ends_where_asked() {
        // loader-circle：一段圆弧，终点应当落在 (21,12) 附近。
        let paths = flatten("M21 12a9 9 0 1 1-6.219-8.56");
        let last = *paths[0].last().unwrap();
        assert!((last.0 - 14.781).abs() < 0.2, "{last:?}");
        assert!((last.1 - 3.44).abs() < 0.2, "{last:?}");
        // 采样点足够多，看起来才圆。
        assert!(paths[0].len() > 20);
    }

    #[test]
    fn z_closes_the_subpath() {
        let paths = flatten("M2 2h4v4z");
        assert_eq!(*paths[0].first().unwrap(), (2.0, 2.0));
        assert_eq!(*paths[0].last().unwrap(), (2.0, 2.0));
    }
}
