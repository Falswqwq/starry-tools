//! 引擎的测试：真的建图、真的跑、真的检查产物。

use super::*;
use crate::model::params::Params;
use crate::model::port_type::ImageFormat;
use crate::model::workflow::{Edge, NodeInstance, Position};
use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
use std::path::PathBuf;

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("starrytools-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn image_fixture(dir: &Path, ext: &str, width: u32, height: u32) -> PathBuf {
    let path = dir.join(format!("input.{ext}"));
    let image = if ext == "jpg" {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            Rgb([(x * 40) as u8, (y * 40) as u8, 200])
        }))
    } else {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x * 40) as u8, (y * 40) as u8, 200, 255])
        }))
    };
    image
        .save_with_format(&path, image::ImageFormat::from_extension(ext).unwrap())
        .unwrap();
    path
}

fn defaults(kind: &str) -> Params {
    registry().get(kind).unwrap().kind.default_params()
}

fn node(id: &str, kind: &str, params: Params) -> NodeInstance {
    NodeInstance {
        id: id.to_string(),
        kind: kind.to_string(),
        position: Position::default(),
        params,
    }
}

fn edge(id: &str, source: &str, source_port: &str, target: &str, target_port: &str) -> Edge {
    Edge {
        id: id.to_string(),
        source: source.to_string(),
        source_port: source_port.to_string(),
        target: target.to_string(),
        target_port: target_port.to_string(),
    }
}

fn input_node(id: &str, path: &Path) -> NodeInstance {
    let mut params = defaults(crate::nodes::input::KIND);
    params.insert("path".into(), serde_json::json!(path.to_string_lossy()));
    node(id, crate::nodes::input::KIND, params)
}

fn errors(issues: &[Issue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(|issue| issue.message.as_str())
        .collect()
}

#[test]
fn input_port_type_follows_the_selected_file() {
    let dir = workspace("resolve-input");
    let png = image_fixture(&dir, "png", 4, 3);
    let jpg = image_fixture(&dir, "jpg", 4, 3);

    let mut workflow = Workflow::new("类型跟着文件走");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(input_node("in2", &jpg));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);

    assert_eq!(
        resolved.nodes[0].outputs[0].ty,
        PortType::Image(ImageFormat::Png)
    );
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Jpeg)
    );
}

#[test]
fn png_only_input_rejects_a_jpeg_source() {
    let dir = workspace("type-mismatch");
    let jpg = image_fixture(&dir, "jpg", 4, 3);

    let mut workflow = Workflow::new("jpeg 直接接放大");
    workflow.nodes.push(input_node("in", &jpg));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "up", "image"));

    let resolved = resolve(&workflow);
    assert!(!resolved.runnable);
    assert!(
        errors(&resolved.issues)
            .iter()
            .any(|message| message.contains("类型不匹配")),
        "{:?}",
        resolved.issues
    );

    // 检查没过就不该真的跑起来。
    let report = run(&workflow, None, &dir).unwrap();
    assert!(!report.ok);
    assert!(report.nodes.is_empty());
}

#[test]
fn runs_convert_then_upscale() {
    let dir = workspace("pipeline");
    let jpg = image_fixture(&dir, "jpg", 8, 6);
    let outputs = dir.join("out");

    let mut workflow = Workflow::new("jpeg 转 png 再放大三倍");
    workflow.nodes.push(input_node("in", &jpg));
    workflow.nodes.push(node(
        "to_png",
        crate::nodes::convert::KIND,
        defaults(crate::nodes::convert::KIND),
    ));
    let mut upscale_params = defaults(crate::nodes::upscale::KIND);
    upscale_params.insert("percent".into(), serde_json::json!(300));
    workflow
        .nodes
        .push(node("up", crate::nodes::upscale::KIND, upscale_params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "to_png", "image"));
    workflow
        .edges
        .push(edge("e2", "to_png", "image", "up", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);

    // 输入节点输出 JPEG，图像格式转换之后输出 PNG —— 类型是静态可推的。
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Png)
    );

    let report = run(&workflow, None, &outputs).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert_eq!(report.order, vec!["in", "to_png", "up"]);

    let convert_result = &report.nodes[1];
    assert_eq!(convert_result.status, NodeStatus::Ok);
    assert!(convert_result.outputs[0].summary.contains("8 × 6"));
    assert!(convert_result.outputs[0].preview.is_some());

    // 转换节点确实产出了 PNG 字节。
    let converted_path = convert_result.outputs[0].path.as_ref().unwrap();
    let bytes = std::fs::read(converted_path).unwrap();
    assert_eq!(&bytes[..4], b"\x89PNG", "转换结果应该真的是 PNG");

    // 放大节点把 8 × 6 变成 24 × 18。
    let upscale_result = &report.nodes[2];
    assert_eq!(
        upscale_result.status,
        NodeStatus::Ok,
        "{:?}",
        upscale_result.error
    );
    assert!(upscale_result.outputs[0].summary.contains("24 × 18"));
    let upscaled = std::fs::read(upscale_result.outputs[0].path.as_ref().unwrap()).unwrap();
    assert_eq!(&upscaled[..4], b"\x89PNG");
}

#[test]
fn required_input_must_be_connected() {
    let mut workflow = Workflow::new("漏了连线");
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));

    let resolved = resolve(&workflow);
    assert!(!resolved.runnable);
    assert!(
        errors(&resolved.issues)
            .iter()
            .any(|message| message.contains("必填的输入")),
        "{:?}",
        resolved.issues
    );
}

#[test]
fn cycles_are_reported_instead_of_hanging() {
    let dir = workspace("cycle");
    let png = image_fixture(&dir, "png", 4, 3);

    let mut workflow = Workflow::new("环");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "a",
        crate::nodes::convert::KIND,
        defaults(crate::nodes::convert::KIND),
    ));
    workflow.nodes.push(node(
        "b",
        crate::nodes::convert::KIND,
        defaults(crate::nodes::convert::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "a", "image"));
    workflow.edges.push(edge("e2", "a", "image", "b", "image"));
    workflow.edges.push(edge("e3", "b", "image", "a", "image"));

    let resolved = resolve(&workflow);
    assert!(!resolved.runnable);
    assert!(
        errors(&resolved.issues)
            .iter()
            .any(|message| message.contains("环")),
        "{:?}",
        resolved.issues
    );
}

#[test]
fn only_node_runs_just_its_upstream() {
    let dir = workspace("partial");
    let png = image_fixture(&dir, "png", 8, 6);

    let mut workflow = Workflow::new("只跑一半");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "to_png",
        crate::nodes::convert::KIND,
        defaults(crate::nodes::convert::KIND),
    ));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow.nodes.push(node(
        "other",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow
        .edges
        .push(edge("e1", "in", "out", "to_png", "image"));
    workflow
        .edges
        .push(edge("e2", "to_png", "image", "up", "image"));
    workflow
        .edges
        .push(edge("e3", "to_png", "image", "other", "image"));

    let report = run(&workflow, Some("to_png"), &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert_eq!(report.order, vec!["in", "to_png"]);
    assert!(!report.nodes.iter().any(|result| result.node_id == "other"));
}

#[test]
fn missing_file_is_reported_per_node() {
    let dir = workspace("missing-file");
    let mut workflow = Workflow::new("文件不见了");
    let mut params = defaults(crate::nodes::input::KIND);
    params.insert("path".into(), serde_json::json!("/definitely/not/here.png"));
    workflow
        .nodes
        .push(node("in", crate::nodes::input::KIND, params));
    workflow.nodes.push(node(
        "to_png",
        crate::nodes::convert::KIND,
        defaults(crate::nodes::convert::KIND),
    ));
    workflow
        .edges
        .push(edge("e1", "in", "out", "to_png", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(!report.ok);
    assert_eq!(report.nodes[0].status, NodeStatus::Failed);
    assert!(report.nodes[0]
        .error
        .as_ref()
        .unwrap()
        .contains("无法读取文件"));
    // 下游被跳过，而不是拿着空数据硬跑。
    assert_eq!(report.nodes[1].status, NodeStatus::Skipped);
}

#[test]
fn upscale_keeps_pixels_crisp_with_nearest_neighbour() {
    let dir = workspace("nearest");
    let png = image_fixture(&dir, "png", 2, 2);

    let mut workflow = Workflow::new("最近邻放大");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::upscale::KIND);
    params.insert("percent".into(), serde_json::json!(400));
    workflow
        .nodes
        .push(node("up", crate::nodes::upscale::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "up", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let path = report.nodes[1].outputs[0].path.as_ref().unwrap();
    let upscaled = image::open(path).unwrap().to_rgba8();
    assert_eq!((upscaled.width(), upscaled.height()), (8, 8));
    // 每个原始像素变成一个 4×4 的纯色方块，中间不该出现过渡色。
    let source = image::open(&png).unwrap().to_rgba8();
    for y in 0..8u32 {
        for x in 0..8u32 {
            assert_eq!(
                upscaled.get_pixel(x, y),
                source.get_pixel(x / 4, y / 4),
                "({x}, {y}) 应该原样复制自 ({}, {})",
                x / 4,
                y / 4
            );
        }
    }
}

#[test]
fn kind_metadata_is_a_stable_contract() {
    // 前端完全靠这份 JSON 渲染，形状变了要在这里显性失败。
    let json = serde_json::to_value(registry().kinds()).unwrap();
    let kinds = json.as_array().unwrap();
    assert_eq!(kinds.len(), 7);

    let convert = kinds
        .iter()
        .find(|kind| kind["id"] == "convert_image")
        .unwrap();
    assert_eq!(convert["name"], "图像格式转换");
    assert_eq!(convert["isSource"], false);
    assert_eq!(convert["inputs"][0]["ty"]["image"], "any");
    assert_eq!(convert["inputs"][0]["required"], true);
    // 输出端口跟着「目标格式」走，清单里给的是默认参数下的样子。
    assert_eq!(convert["outputs"][0]["ty"]["image"], "png");

    let format_param = convert["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "format")
        .expect("转换节点要有目标格式这个参数");
    assert_eq!(format_param["control"], "select");
    assert_eq!(format_param["default"], "png");
    let formats: Vec<&str> = format_param["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| option["value"].as_str().unwrap())
        .collect();
    assert!(formats.contains(&"jpeg"), "{formats:?}");
    assert!(formats.contains(&"webp"), "{formats:?}");
    assert!(!formats.contains(&"any"), "通配不是个能转出去的目标");

    // 画质只对 JPEG 有意义（WebP 在这个 crate 里只有无损）。
    let quality = convert["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "quality")
        .unwrap();
    assert_eq!(quality["visibleWhen"]["anyOf"][0], "jpeg");

    assert!(
        convert["notes"].as_array().unwrap().len() >= 3,
        "展开卡片要显示「注意事项」，每个工具都得写几条"
    );

    let rename = kinds.iter().find(|kind| kind["id"] == "rename").unwrap();
    assert_eq!(rename["name"], "重命名");
    assert_eq!(
        rename["category"], "通用",
        "重命名是个通用节点，不该待在图像分类里"
    );
    assert_eq!(rename["inputs"][0]["id"], "in");
    assert_eq!(rename["inputs"][0]["ty"], "any", "什么值都能接");
    assert_eq!(rename["outputs"][0]["id"], "out");
    assert_eq!(rename["outputs"][0]["ty"], "any");
    let auto_extension = rename["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "autoExtension")
        .expect("重命名节点要有自检测后缀名这个开关");
    assert_eq!(auto_extension["control"], "bool");
    assert_eq!(auto_extension["default"], true);

    let input = kinds.iter().find(|kind| kind["id"] == "input").unwrap();
    assert_eq!(input["isSource"], true);
    assert_eq!(input["outputs"][0]["ty"]["image"], "any");
    assert_eq!(input["params"][0]["control"], "select");
    assert_eq!(input["defaults"]["valueType"], "image");

    let file_param = input["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["control"] == "file")
        .unwrap();
    assert!(file_param["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|ext| ext == "png"));
    assert_eq!(file_param["visibleWhen"]["anyOf"][0], "image");

    let upscale = kinds.iter().find(|kind| kind["id"] == "upscale").unwrap();
    assert_eq!(upscale["name"], "缩放图像");
    assert_eq!(upscale["inputs"][0]["ty"]["image"], "png");
    assert_eq!(upscale["params"][0]["control"], "number");
    assert_eq!(upscale["params"][0]["unit"], "%");
    assert_eq!(upscale["defaults"]["percent"], 200.0);

    let compress = kinds
        .iter()
        .find(|kind| kind["id"] == "compress_image")
        .unwrap();
    assert_eq!(compress["name"], "图像压缩");
    assert_eq!(compress["inputs"][0]["ty"]["image"], "any");
    // 默认无损，输出就是 PNG。
    assert_eq!(compress["outputs"][0]["ty"]["image"], "png");
    let loss = compress["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "loss")
        .expect("压缩节点要有允许的损耗率这个参数");
    assert_eq!(loss["control"], "slider", "损耗率是个滑杆");
    assert_eq!(loss["unit"], "%");
    assert_eq!(loss["visibleWhen"]["anyOf"][0], "lossy");
    // 损耗率只对 JPEG 那条有损路线有意义，所以还挂了一个「而且」条件。
    assert_eq!(loss["visibleWhen"]["allOf"][0]["param"], "lossyFormat");
    assert_eq!(loss["visibleWhen"]["allOf"][0]["anyOf"][0], "jpeg");

    let params = compress["params"].as_array().unwrap();
    let find_param = |id: &str| {
        params
            .iter()
            .find(|param| param["id"] == id)
            .unwrap_or_else(|| panic!("压缩节点少了 {id} 这个参数"))
    };
    let options_of = |id: &str| -> Vec<String> {
        find_param(id)["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["value"].as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(
        options_of("scheme"),
        vec!["fast", "balanced", "maximum", "zopfli"],
        "无损那四档要都在"
    );
    assert_eq!(find_param("scheme")["visibleWhen"]["anyOf"][0], "lossless");
    assert_eq!(options_of("strip"), vec!["safe", "all"]);
    assert_eq!(options_of("lossyFormat"), vec!["jpeg", "palette"]);

    let colors = find_param("colors");
    assert_eq!(colors["control"], "slider");
    assert_eq!(colors["visibleWhen"]["anyOf"][0], "lossy");
    assert_eq!(colors["visibleWhen"]["allOf"][0]["anyOf"][0], "palette");

    let crop = kinds
        .iter()
        .find(|kind| kind["id"] == "crop_image")
        .unwrap();
    assert_eq!(crop["name"], "图像裁切");
    assert_eq!(crop["inputs"][0]["ty"]["image"], "png");
    assert_eq!(crop["outputs"][0]["ty"]["image"], "png");
    let modes: Vec<&str> = crop["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "mode")
        .unwrap()["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| option["value"].as_str().unwrap())
        .collect();
    assert_eq!(modes, vec!["ratio", "size", "content"]);

    let save = kinds
        .iter()
        .find(|kind| kind["id"] == "save_output")
        .unwrap();
    assert_eq!(save["name"], "保存到目录");
    assert_eq!(save["inputs"][0]["ty"]["image"], "any");
    assert_eq!(save["outputs"][0]["ty"], "text");
    let directory_param = save["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["control"] == "file")
        .unwrap();
    assert_eq!(
        directory_param["directory"], true,
        "保存节点的文件参数选的是一个目录"
    );
}

#[test]
fn converter_output_type_follows_the_target_format() {
    let dir = workspace("convert-format");
    let png = image_fixture(&dir, "png", 8, 6);

    let mut workflow = Workflow::new("转成别的格式");
    workflow.nodes.push(input_node("in", &png));
    let mut convert_params = defaults(crate::nodes::convert::KIND);
    convert_params.insert("format".into(), serde_json::json!("jpeg"));
    workflow
        .nodes
        .push(node("conv", crate::nodes::convert::KIND, convert_params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "conv", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Jpeg),
        "选了 JPEG，输出端口就该是 Image(Jpeg)"
    );

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    // 产物得真的是一个 JPEG（自己看字节头，别信标签）。
    let path = report.nodes[1].outputs[0].path.as_ref().unwrap();
    assert!(path.ends_with(".jpg"), "{path}");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..2], b"\xff\xd8", "JPEG 的字节头是 FFD8");
    assert_eq!(
        report.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Jpeg)
    );
}

#[test]
fn legacy_convert_to_png_ids_still_run() {
    // 节点改过 id。老存档里写的还是 convert_to_png，得能照常解析和执行，
    // 缺的「目标格式」由默认值补成 png —— 行为和以前一样。
    let dir = workspace("legacy-id");
    let jpg = image_fixture(&dir, "jpg", 4, 3);

    let mut workflow = Workflow::new("老存档");
    workflow.nodes.push(input_node("in", &jpg));
    workflow.nodes.push(node(
        "conv",
        "convert_to_png",
        registry()
            .get("convert_to_png")
            .unwrap()
            .kind
            .default_params(),
    ));
    workflow
        .edges
        .push(edge("e1", "in", "out", "conv", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    // 报给前端的要是归一后的 id，这样它才查得到元数据。
    assert_eq!(resolved.nodes[1].kind, "convert_image");
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Png)
    );

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let bytes = std::fs::read(report.nodes[1].outputs[0].path.as_ref().unwrap()).unwrap();
    assert_eq!(&bytes[..4], b"\x89PNG");
}

#[test]
fn compress_lossless_keeps_every_pixel() {
    let dir = workspace("compress-lossless");
    let png = image_fixture(&dir, "png", 5, 4);

    let mut workflow = Workflow::new("无损压缩");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "c",
        crate::nodes::compress::KIND,
        defaults(crate::nodes::compress::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "c", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let source = image::open(&png).unwrap().to_rgba8();
    let result = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(source.dimensions(), result.dimensions());
    // 无损就是无损：一个像素都不许变。
    for (before, after) in source.pixels().zip(result.pixels()) {
        assert_eq!(before, after);
    }
}

#[test]
fn compress_output_type_follows_the_mode() {
    let dir = workspace("compress-mode");
    let png = image_fixture(&dir, "png", 8, 6);

    let mut workflow = Workflow::new("有损压缩");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::compress::KIND);
    params.insert("mode".into(), serde_json::json!("lossy"));
    params.insert("loss".into(), serde_json::json!(40));
    workflow
        .nodes
        .push(node("c", crate::nodes::compress::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "c", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Jpeg),
        "选了有损，输出端口就该是 Image(Jpeg)"
    );

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let bytes = std::fs::read(report.nodes[1].outputs[0].path.as_ref().unwrap()).unwrap();
    assert_eq!(&bytes[..2], b"\xff\xd8", "有损模式的产物得真的是 JPEG");
}

/// PNG 的 IHDR 里，第 24 个字节是位深，第 25 个是颜色类型。
fn png_shape(bytes: &[u8]) -> (u8, u8) {
    assert_eq!(&bytes[..4], b"\x89PNG", "这得是一张 PNG");
    (bytes[24], bytes[25])
}

/// 一张颜色打散、不透明的图。
fn noisy_png(dir: &Path, side: u32) -> PathBuf {
    let path = dir.join("noisy.png");
    RgbaImage::from_fn(side, side, |x, y| {
        let mut value = x
            .wrapping_mul(374_761_393)
            .wrapping_add(y.wrapping_mul(668_265_263));
        value ^= value >> 13;
        value = value.wrapping_mul(1_274_126_177);
        let value = value ^ (value >> 16);
        Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
    })
    .save(&path)
    .unwrap();
    path
}

#[test]
fn lossless_mode_picks_an_indexed_encoding_for_pixel_art() {
    let dir = workspace("compress-indexed");
    let path = dir.join("sprite.png");
    let palette = [
        [10u8, 20, 30, 255],
        [200, 10, 10, 255],
        [0, 0, 0, 0],
        [250, 250, 250, 255],
    ];
    RgbaImage::from_fn(64, 64, |x, y| Rgba(palette[((x / 3 + y / 5) % 4) as usize]))
        .save(&path)
        .unwrap();

    let mut workflow = Workflow::new("像素画无损压缩");
    workflow.nodes.push(input_node("in", &path));
    workflow.nodes.push(node(
        "c",
        crate::nodes::compress::KIND,
        defaults(crate::nodes::compress::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "c", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let bytes = std::fs::read(report.nodes[1].outputs[0].path.as_ref().unwrap()).unwrap();
    let (depth, color_type) = png_shape(&bytes);
    assert_eq!(color_type, 3, "4 色像素画该被编成索引色");
    assert_eq!(depth, 2, "4 种颜色 2 位就够了");

    // 无损：逐像素得和原图完全一致。
    let original = image::open(&path).unwrap().to_rgba8();
    let optimized = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(original.as_raw(), optimized.as_raw());
}

#[test]
fn lossy_palette_mode_quantizes_to_an_indexed_png() {
    let dir = workspace("compress-palette");
    let path = noisy_png(&dir, 64);

    let mut workflow = Workflow::new("调色板量化");
    workflow.nodes.push(input_node("in", &path));
    let mut params = defaults(crate::nodes::compress::KIND);
    params.insert("mode".into(), serde_json::json!("lossy"));
    params.insert("lossyFormat".into(), serde_json::json!("palette"));
    params.insert("colors".into(), serde_json::json!(4));
    workflow
        .nodes
        .push(node("c", crate::nodes::compress::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "c", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert_eq!(
        resolved.nodes[1].outputs[0].ty,
        PortType::Image(ImageFormat::Png),
        "调色板那条路出来还是 PNG"
    );

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let bytes = std::fs::read(report.nodes[1].outputs[0].path.as_ref().unwrap()).unwrap();
    assert_eq!(png_shape(&bytes), (2, 3), "量化到 4 色就该是 2 位索引色");

    // 有损：颜色确实被并掉了，剩下不超过 4 种。
    let quantized = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    let mut colors: Vec<[u8; 4]> = quantized.pixels().map(|pixel| pixel.0).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() <= 4, "还剩 {} 种颜色", colors.len());
}

#[test]
fn crop_to_ratio_keeps_the_target_aspect() {
    let dir = workspace("crop-ratio");
    let png = image_fixture(&dir, "png", 8, 4);

    let mut workflow = Workflow::new("裁到比例");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::crop::KIND);
    params.insert("mode".into(), serde_json::json!("ratio"));
    params.insert("ratioWidth".into(), serde_json::json!(1));
    params.insert("ratioHeight".into(), serde_json::json!(1));
    workflow
        .nodes
        .push(node("crop", crate::nodes::crop::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "crop", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(
        (out.width(), out.height()),
        (4, 4),
        "8×4 图上取 1:1 就是 4×4"
    );
}

#[test]
fn crop_to_size_anchored_at_a_corner() {
    let dir = workspace("crop-anchor");
    let path = dir.join("corner.png");
    let mut canvas = RgbaImage::from_pixel(8, 4, Rgba([0, 0, 255, 255]));
    canvas.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
    canvas.save(&path).unwrap();

    let mut workflow = Workflow::new("裁到大小");
    workflow.nodes.push(input_node("in", &path));
    let mut params = defaults(crate::nodes::crop::KIND);
    params.insert("mode".into(), serde_json::json!("size"));
    params.insert("width".into(), serde_json::json!(3));
    params.insert("height".into(), serde_json::json!(2));
    params.insert("anchor".into(), serde_json::json!("topLeft"));
    workflow
        .nodes
        .push(node("crop", crate::nodes::crop::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "crop", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!((out.width(), out.height()), (3, 2));
    // 锚在左上角，那一角那个红点得原封不动地带出来。
    assert_eq!(out.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
}

#[test]
fn crop_to_content_trims_the_transparent_border() {
    let dir = workspace("crop-content");
    let path = dir.join("trim.png");
    // 6×5 全透明，中间画一块 3×2 的有色区域。
    let mut canvas = RgbaImage::new(6, 5);
    for y in 1..3 {
        for x in 2..5 {
            canvas.put_pixel(x, y, Rgba([10, 20, 30, 255]));
        }
    }
    canvas.save(&path).unwrap();

    let mut workflow = Workflow::new("裁到内容");
    workflow.nodes.push(input_node("in", &path));
    let mut params = defaults(crate::nodes::crop::KIND);
    params.insert("mode".into(), serde_json::json!("content"));
    workflow
        .nodes
        .push(node("crop", crate::nodes::crop::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "crop", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out = image::open(report.nodes[1].outputs[0].path.as_ref().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!((out.width(), out.height()), (3, 2), "只该剩下有色那一块");
    assert_eq!(out.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
}

#[test]
fn save_to_directory_writes_and_respects_overwrite() {
    let dir = workspace("save-dir");
    let png = image_fixture(&dir, "png", 4, 3);
    let target = dir.join("exports");

    let mut workflow = Workflow::new("保存产物");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::save::KIND);
    params.insert(
        "directory".into(),
        serde_json::json!(target.to_string_lossy()),
    );
    params.insert("overwrite".into(), serde_json::json!(false));
    workflow
        .nodes
        .push(node("save", crate::nodes::save::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "save", "image"));

    let first = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(first.ok, "{:?}", first.nodes);
    let written = first.nodes[1].outputs[0].summary.clone();
    assert!(
        written.ends_with("input.png"),
        "运行记录里要能看出写到了哪个文件：{written}"
    );
    assert!(target.join("input.png").exists(), "产物应该真的写出来了");

    // 再跑一次，不允许覆盖：应该另存成 input (2).png，不动原来那个。
    let second = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(second.ok, "{:?}", second.nodes);
    let rewritten = second.nodes[1].outputs[0].summary.clone();
    assert!(
        rewritten.ends_with("input (2).png"),
        "重名时应该换个名字：{rewritten}"
    );
    assert!(
        second.nodes[1]
            .warnings
            .iter()
            .any(|warning| warning.contains("已存在")),
        "改名了就该说一声，{:?}",
        second.nodes[1].warnings
    );
    assert!(target.join("input.png").exists(), "已有的文件不该被动");
}

/// 把一段「输入 → 重命名 → 保存」接起来，返回产物目录。
fn rename_pipeline(dir: &Path, name: &str, auto_extension: bool) -> (PathBuf, RunReport) {
    let png = image_fixture(dir, "png", 4, 3);
    let target = dir.join("exports");

    let mut workflow = Workflow::new("重命名");
    workflow.nodes.push(input_node("in", &png));

    let mut rename_params = defaults(crate::nodes::rename::KIND);
    rename_params.insert("name".into(), serde_json::json!(name));
    rename_params.insert("autoExtension".into(), serde_json::json!(auto_extension));
    workflow
        .nodes
        .push(node("rn", crate::nodes::rename::KIND, rename_params));

    let mut save_params = defaults(crate::nodes::save::KIND);
    save_params.insert(
        "directory".into(),
        serde_json::json!(target.to_string_lossy()),
    );
    workflow
        .nodes
        .push(node("save", crate::nodes::save::KIND, save_params));

    workflow.edges.push(edge("e1", "in", "out", "rn", "in"));
    workflow
        .edges
        .push(edge("e2", "rn", "out", "save", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    (target, report)
}

#[test]
fn rename_names_the_saved_file() {
    let dir = workspace("rename-save");
    let (target, report) = rename_pipeline(&dir, "hero-idle", true);
    assert!(report.ok, "{:?}", report.nodes);
    assert!(
        target.join("hero-idle.png").exists(),
        "应该按重命名节点给的名字落盘"
    );
}

#[test]
fn rename_without_auto_extension_uses_the_name_verbatim() {
    let dir = workspace("rename-verbatim");
    let (target, report) = rename_pipeline(&dir, "hero.sprite", false);
    assert!(report.ok, "{:?}", report.nodes);
    assert!(
        target.join("hero.sprite").exists(),
        "关掉自检测后缀名，名字就得原样用"
    );
}

#[test]
fn rename_survives_a_re_encode_and_follows_the_real_format() {
    // 名字要能跟着管线走过一次编码，而且扩展开关认的是**实际**格式：
    // 重命名时还是 PNG，压成有损之后就该写 .jpg。
    let dir = workspace("rename-reencode");
    let png = image_fixture(&dir, "png", 4, 3);
    let target = dir.join("exports");

    let mut workflow = Workflow::new("重命名后转格式");
    workflow.nodes.push(input_node("in", &png));

    let mut rename_params = defaults(crate::nodes::rename::KIND);
    rename_params.insert("name".into(), serde_json::json!("shot"));
    workflow
        .nodes
        .push(node("rn", crate::nodes::rename::KIND, rename_params));

    let mut compress_params = defaults(crate::nodes::compress::KIND);
    compress_params.insert("mode".into(), serde_json::json!("lossy"));
    workflow
        .nodes
        .push(node("c", crate::nodes::compress::KIND, compress_params));

    let mut save_params = defaults(crate::nodes::save::KIND);
    save_params.insert(
        "directory".into(),
        serde_json::json!(target.to_string_lossy()),
    );
    workflow
        .nodes
        .push(node("save", crate::nodes::save::KIND, save_params));

    workflow.edges.push(edge("e1", "in", "out", "rn", "in"));
    workflow.edges.push(edge("e2", "rn", "out", "c", "image"));
    workflow
        .edges
        .push(edge("e3", "c", "image", "save", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert!(
        target.join("shot.jpg").exists(),
        "名字该跟着走，扩展名该按编码后的实际格式写"
    );
}

#[test]
fn rename_passes_a_non_image_value_through_untouched() {
    // 重命名是通用节点：文本、数字都能接，类型和内容都不变。
    let dir = workspace("rename-text");

    let mut workflow = Workflow::new("重命名文本");
    let mut input_params = defaults(crate::nodes::input::KIND);
    input_params.insert("valueType".into(), serde_json::json!("text"));
    input_params.insert("text".into(), serde_json::json!("hello"));
    workflow
        .nodes
        .push(node("in", crate::nodes::input::KIND, input_params));

    let mut rename_params = defaults(crate::nodes::rename::KIND);
    rename_params.insert("name".into(), serde_json::json!("note"));
    workflow
        .nodes
        .push(node("rn", crate::nodes::rename::KIND, rename_params));

    workflow.edges.push(edge("e1", "in", "out", "rn", "in"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert_eq!(resolved.nodes[1].outputs[0].ty, PortType::Any);

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert_eq!(
        report.nodes[1].outputs[0].ty,
        PortType::Text,
        "运行期看得出来里面包的是文本"
    );
    // 名字会写进摘要，运行记录里一眼能看到它挂上了。
    assert!(
        report.nodes[1].outputs[0].summary.starts_with("note · "),
        "{:?}",
        report.nodes[1].outputs[0].summary
    );
}

#[test]
fn a_wildcard_output_may_feed_any_input_at_edit_time() {
    // 重命名的输出是通配，编辑期可以直接接到 PNG 端口上；运行期再由实际值说话。
    let dir = workspace("wildcard-downstream");
    let png = image_fixture(&dir, "png", 4, 3);

    let mut workflow = Workflow::new("通配往下接");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "rn",
        crate::nodes::rename::KIND,
        defaults(crate::nodes::rename::KIND),
    ));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));

    workflow.edges.push(edge("e1", "in", "out", "rn", "in"));
    workflow.edges.push(edge("e2", "rn", "out", "up", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert!(resolved.runnable);

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
}
