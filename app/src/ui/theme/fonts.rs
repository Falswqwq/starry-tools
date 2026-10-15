//! 西文等宽字体与中文回退；Windows 路径来自系统环境，不写死盘符。

use eframe::egui;
use std::path::PathBuf;

pub const FONT_CONTROL: f32 = 11.5;
pub const FONT_BUTTON: f32 = 11.0;
pub const FONT_SMALL: f32 = 10.5;
pub const FONT_HEADING: f32 = 12.0;

fn candidates(cjk: bool) -> Vec<(PathBuf, u32)> {
    let mut out = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
        let fonts = PathBuf::from(windows).join("Fonts");
        let names: &[&str] = if cjk {
            &["msyh.ttc", "msyh.ttf", "simhei.ttf", "simsun.ttc"]
        } else {
            &["CascadiaMono.ttf", "consola.ttf"]
        };
        out.extend(names.iter().map(|name| (fonts.join(name), 0)));
    }
    let linux: &[(&str, u32)] = if cjk {
        &[
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 7),
            (
                "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                0,
            ),
        ]
    } else {
        &[
            (
                "/usr/share/fonts/truetype/jetbrains/JetBrainsMonoNerdFont-Regular.ttf",
                0,
            ),
            (
                "/usr/share/fonts/truetype/jetbrains/JetBrainsMonoNLNerdFont-Regular.ttf",
                0,
            ),
            ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0),
        ]
    };
    out.extend(
        linux
            .iter()
            .map(|(path, index)| (PathBuf::from(path), *index)),
    );
    out
}

fn find_font(cjk: bool) -> Option<(u32, Vec<u8>)> {
    candidates(cjk)
        .into_iter()
        .find_map(|(path, index)| std::fs::read(path).ok().map(|bytes| (index, bytes)))
}

pub(super) fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut changed = false;
    for (name, cjk) in [("ui", false), ("cjk", true)] {
        let Some((index, bytes)) = find_font(cjk) else {
            eprintln!("[theme] 找不到 {name} 候选字体，使用内置回退");
            continue;
        };
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        fonts
            .font_data
            .insert(name.to_owned(), std::sync::Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let family = fonts.families.entry(family).or_default();
            if cjk {
                family.push(name.to_owned());
            } else {
                family.insert(0, name.to_owned());
            }
        }
        changed = true;
    }
    if changed {
        ctx.set_fonts(fonts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font;

    #[test]
    fn cjk_font_has_han_glyphs() {
        let Some((index, bytes)) = find_font(true) else {
            return;
        };
        let font =
            ab_glyph::FontRef::try_from_slice_and_index(&bytes, index).expect("候选中文字体可解析");
        for c in "重命名图像压缩裁切保存目录输入输出参数".chars() {
            assert_ne!(font.glyph_id(c).0, 0, "中文字体缺少 {c}");
        }
    }

    #[test]
    fn windows_fonts_follow_the_system_environment_when_available() {
        let Some(windows) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))
        else {
            return;
        };
        let fonts = PathBuf::from(windows).join("Fonts");
        assert!(candidates(true)
            .iter()
            .any(|(path, _)| path == &fonts.join("msyh.ttc")));
        assert!(candidates(false)
            .iter()
            .any(|(path, _)| path == &fonts.join("consola.ttf")));
    }

    #[test]
    fn linux_fallbacks_remain_available() {
        assert!(candidates(true)
            .iter()
            .any(|(path, index)| path.ends_with("NotoSansCJK-Regular.ttc") && *index == 2));
        assert!(candidates(false)
            .iter()
            .any(|(path, _)| path.ends_with("DejaVuSansMono.ttf")));
    }
}
