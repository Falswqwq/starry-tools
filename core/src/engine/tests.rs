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
    let mut params = defaults(crate::nodes::read::KIND);
    params.insert("path".into(), serde_json::json!(path.to_string_lossy()));
    node(id, crate::nodes::read::KIND, params)
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

    // 输出类型是**自动推断**出来的：选了 png 就是 PNG，选了 jpg 就是 JPEG。
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
fn a_configuringless_input_is_any() {
    // 还没选文件：类型待定，输出就是 ANY —— 节点库卡片上看到的就是它。
    let input = registry().get(crate::nodes::read::KIND).unwrap();
    let ports = input.outputs_for(&input.kind.default_params());
    assert_eq!(ports[0].ty, PortType::Any);
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

    // 推断出是 JPEG，而目标端口只收 PNG —— 编辑期就拦下。
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
    let mut params = defaults(crate::nodes::read::KIND);
    params.insert("path".into(), serde_json::json!("/definitely/not/here.png"));
    workflow
        .nodes
        .push(node("in", crate::nodes::read::KIND, params));
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
    let expected_count =
        18 + usize::from(cfg!(feature = "onnx")) + usize::from(cfg!(feature = "video"));
    assert_eq!(
        kinds.len(),
        expected_count,
        "内置工具的数量（改动时请一并更新这条）"
    );
    for (id, enabled) in [
        ("background_removal", cfg!(feature = "onnx")),
        ("video_compress", cfg!(feature = "video")),
    ] {
        assert_eq!(
            kinds.iter().any(|kind| kind["id"] == id),
            enabled,
            "{id} 的存在与否必须匹配其 feature",
        );
    }

    let convert = kinds
        .iter()
        .find(|kind| kind["id"] == "convert_image")
        .unwrap();
    assert_eq!(convert["name"], "图像格式转换");
    assert_eq!(convert["isSource"], false);
    assert_eq!(convert["inputs"][0]["ty"], "img");
    assert_eq!(convert["inputs"][0]["required"], true);
    // 输出格式跟着「目标格式」走，清单（节点库卡片）里给的是泛化的 IMG。
    assert_eq!(convert["outputs"][0]["ty"], "img");

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

    let read = kinds.iter().find(|kind| kind["id"] == "read").unwrap();
    assert_eq!(read["name"], "读取");
    assert_eq!(read["isSource"], true);
    // 输出声明为通配（ANY）：具体类型自动推断。
    assert_eq!(read["outputs"][0]["ty"], "any");
    assert_eq!(read["params"][0]["control"], "select");
    assert_eq!(read["defaults"]["valueType"], "image");

    let file_param = read["params"]
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
    assert_eq!(upscale["inputs"][0]["ty"], "png");
    assert_eq!(upscale["params"][0]["control"], "number");
    assert_eq!(upscale["params"][0]["unit"], "%");
    assert_eq!(upscale["defaults"]["percent"], 200.0);

    let compress = kinds
        .iter()
        .find(|kind| kind["id"] == "compress_image")
        .unwrap();
    assert_eq!(compress["name"], "图像压缩");
    assert_eq!(compress["inputs"][0]["ty"], "img");
    // 输出格式跟着压缩方式走，卡片上是泛化的 IMG。
    assert_eq!(compress["outputs"][0]["ty"], "img");
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
    assert_eq!(crop["inputs"][0]["ty"], "png");
    assert_eq!(crop["outputs"][0]["ty"], "png");
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
    assert_eq!(modes, vec!["ratio", "size", "content", "shape"]);
    // 「形状裁切」是一个紫色（阻塞）节点：切到 shape 时才拦住运行。
    let crop_spec = registry().get("crop_image").unwrap();
    let mut shape_params = defaults("crop_image");
    shape_params.insert("mode".into(), serde_json::json!("shape"));
    assert!(crop_spec.is_interactive(&shape_params));
    assert!(!crop_spec.is_interactive(&defaults("crop_image")));
    assert!(
        crop["interactive"].is_boolean(),
        "卡片要能知道默认是不是紫色"
    );

    let border = kinds
        .iter()
        .find(|kind| kind["id"] == "border_image")
        .unwrap();
    assert_eq!(border["name"], "边框/边距");
    let color = border["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["id"] == "color")
        .unwrap();
    assert_eq!(color["control"], "color", "颜色参数用的是一块取色器");

    let save = kinds
        .iter()
        .find(|kind| kind["id"] == "save_output")
        .unwrap();
    assert_eq!(save["name"], "保存到目录");
    assert_eq!(save["inputs"][0]["ty"], "file");
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
    workflow.edges.push(edge("e1", "in", "out", "save", "file"));

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
    workflow.edges.push(edge("e2", "rn", "out", "save", "file"));

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
        .push(edge("e3", "c", "image", "save", "file"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert!(
        target.join("shot.jpg").exists(),
        "名字该跟着走，扩展名该按编码后的实际格式写"
    );
}

#[test]
fn read_reads_a_text_file_as_text() {
    let dir = workspace("read-text");
    let path = dir.join("note.txt");
    std::fs::write(&path, "你好，世界").unwrap();

    let mut workflow = Workflow::new("读文本");
    let mut params = defaults(crate::nodes::read::KIND);
    params.insert("valueType".into(), serde_json::json!("text"));
    params.insert("textPath".into(), serde_json::json!(path.to_string_lossy()));
    workflow
        .nodes
        .push(node("in", crate::nodes::read::KIND, params));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    // 文本模式推断出 Text。
    assert_eq!(resolved.nodes[0].outputs[0].ty, PortType::Text);

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert_eq!(report.nodes[0].outputs[0].ty, PortType::Text);
}

#[test]
fn rename_passes_a_non_image_value_through_untouched() {
    // 重命名是通用节点：文本、数字都能接，类型和内容都不变。
    let dir = workspace("rename-text");

    let mut workflow = Workflow::new("重命名文本");
    let mut text_params = defaults(crate::nodes::literal::KIND_TEXT);
    text_params.insert("value".into(), serde_json::json!("hello"));
    workflow
        .nodes
        .push(node("in", crate::nodes::literal::KIND_TEXT, text_params));

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

// ---- 参数端口（参数改从上游取）----------------------------------------------

#[test]
fn params_get_optional_input_ports() {
    // 参数端口是**推导**出来的：节点只要声明了参数，就自动多一个可选输入。
    let spec = registry().get(crate::nodes::upscale::KIND).unwrap();
    let params = spec.kind.default_params();
    let ports = spec.kind.inputs_for(&params);
    let ids: Vec<&str> = ports.iter().map(|port| port.id.as_str()).collect();

    assert!(ids.contains(&"image"), "声明里的输入还在：{ids:?}");
    assert!(
        ids.contains(&"param:percent"),
        "数字参数该多一个端口：{ids:?}"
    );
    assert!(
        !ids.contains(&"param:filter"),
        "下拉框不该被上游喂（合法值是一张固定的表）：{ids:?}"
    );

    let percent = ports
        .iter()
        .find(|port| port.id == "param:percent")
        .unwrap();
    assert_eq!(percent.ty, PortType::Number);
    assert_eq!(percent.param.as_deref(), Some("percent"));
    assert!(percent.is_param() && !percent.required);
}

#[test]
fn source_nodes_have_no_param_ports() {
    // 字面量节点本身就是那个值，没有上游可接数据 —— 但会有一个「空输入」用来排先后。
    for id in [
        crate::nodes::literal::KIND_TEXT,
        crate::nodes::literal::KIND_NUMBER,
        crate::nodes::literal::KIND_BOOL,
    ] {
        let spec = registry().get(id).unwrap();
        assert!(spec.kind.is_source, "{id} 该是起点节点");
        let ports = spec.kind.inputs_for(&spec.kind.default_params());
        assert_eq!(ports.len(), 1, "{id} 只该有那个空输入");
        assert_eq!(ports[0].ty, PortType::Void);
        assert!(
            ports.iter().all(|port| !port.is_param()),
            "{id} 不该有参数端口"
        );
    }
}

#[test]
fn a_linked_param_port_overrides_the_param() {
    // 「数字」节点接到「缩放图像」的 percent 端口上：真正生效的是上游那个数字，
    // 节点自己存的那个值被盖掉。
    let dir = workspace("param-port");
    let png = image_fixture(&dir, "png", 2, 2);

    let mut workflow = Workflow::new("参数接上游");
    workflow.nodes.push(input_node("in", &png));

    let mut upscale = defaults(crate::nodes::upscale::KIND);
    upscale.insert("percent".into(), serde_json::json!(100));
    workflow
        .nodes
        .push(node("up", crate::nodes::upscale::KIND, upscale));

    let mut number = defaults(crate::nodes::literal::KIND_NUMBER);
    number.insert("value".into(), serde_json::json!(400));
    workflow
        .nodes
        .push(node("num", crate::nodes::literal::KIND_NUMBER, number));

    workflow.edges.push(edge("e1", "in", "out", "up", "image"));
    workflow
        .edges
        .push(edge("e2", "num", "out", "up", "param:percent"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let path = report
        .nodes
        .iter()
        .find(|result| result.node_id == "up")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    let out = image::open(path).unwrap();
    assert_eq!(
        (out.width(), out.height()),
        (8, 8),
        "应当按上游那个 400%（而不是本地的 100%）放大"
    );
}

/// 把一个图像写进工作流、跑「边框/边距」、把产物读回来。
fn border_result(name: &str, image: &RgbaImage, thickness: i64, color: &str) -> RgbaImage {
    use crate::nodes::border as bd;

    let dir = workspace(name);
    let path = dir.join("source.png");
    image.save(&path).unwrap();

    let mut workflow = Workflow::new("描边");
    workflow.nodes.push(input_node("in", &path));
    let mut params = defaults(bd::KIND);
    params.insert("thickness".into(), serde_json::json!(thickness));
    params.insert("color".into(), serde_json::json!(color));
    workflow.nodes.push(node("bd", bd::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "bd", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out_path = report
        .nodes
        .iter()
        .find(|result| result.node_id == "bd")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    image::open(out_path).unwrap().to_rgba8()
}

#[test]
fn progress_streams_node_by_node() {
    use crate::progress::{Progress, ProgressEvent};

    let dir = workspace("progress");
    let png = image_fixture(&dir, "png", 4, 3);

    let mut workflow = Workflow::new("进度");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "up", "image"));

    let (tx, rx) = std::sync::mpsc::sync_channel(crate::progress::PROGRESS_CAPACITY);
    let progress = Progress::new(tx);
    let report = run_with(&workflow, None, &dir.join("out"), None, Some(&progress)).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let events: Vec<ProgressEvent> = rx.try_iter().collect();
    let started: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            ProgressEvent::Started { node_id } => Some(node_id.as_str()),
            _ => None,
        })
        .collect();
    let finished = events
        .iter()
        .filter(|event| matches!(event, ProgressEvent::Finished { .. }))
        .count();
    // 每个节点各一条 Started + 一条 Finished，且顺序就是拓扑序。
    assert_eq!(started, vec!["in", "up"]);
    assert_eq!(finished, 2);
}

/// 进度事件要把每个节点的**结果**也带过来 —— 界面靠它把缩略图跟着流程一张张贴上卡片，
/// 而不是等整张图跑完。
#[test]
fn progress_carries_each_nodes_result_as_it_finishes() {
    use crate::progress::{Progress, ProgressEvent};

    let dir = workspace("progress-result");
    let png = image_fixture(&dir, "png", 4, 3);

    let mut workflow = Workflow::new("进度结果");
    workflow.nodes.push(input_node("in", &png));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow.edges.push(edge("e1", "in", "out", "up", "image"));

    let (tx, rx) = std::sync::mpsc::sync_channel(crate::progress::PROGRESS_CAPACITY);
    let progress = Progress::new(tx);
    run_with(&workflow, None, &dir.join("out"), None, Some(&progress)).unwrap();

    let finished: Vec<crate::engine::NodeRunResult> = rx
        .try_iter()
        .filter_map(|event| match event {
            ProgressEvent::Finished { result } => Some(result),
            _ => None,
        })
        .collect();

    let up = finished
        .iter()
        .find(|result| result.node_id == "up")
        .expect("应当收到「up」的结果");
    assert_eq!(up.status, NodeStatus::Ok);
    // 缩略图必须**在进度里就有** —— 这正是「边跑边展示」的关键。
    assert!(
        up.outputs.iter().any(|output| output.preview.is_some()),
        "进度里的结果就该带着缩略图：{:?}",
        up.outputs
    );
}

#[test]
fn color_analysis_outputs_a_palette_text() {
    use crate::nodes::palette as pa;

    let dir = workspace("palette-extract");
    let png = image_fixture(&dir, "png", 16, 16);

    let mut workflow = Workflow::new("抽色板");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(pa::KIND);
    params.insert("mode".into(), serde_json::json!("extract"));
    params.insert("colors".into(), serde_json::json!(4));
    workflow.nodes.push(node("an", pa::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "an", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let output = &report
        .nodes
        .iter()
        .find(|result| result.node_id == "an")
        .unwrap()
        .outputs[0];

    assert_eq!(output.ty, PortType::Text, "色板就是一段文本");
    let palette = output.palette.clone().expect("引擎该把颜色拆出来");
    assert!(!palette.is_empty() && palette.len() <= 4, "{:?}", palette);
    assert!(palette.iter().all(|c| c.starts_with('#') && c.len() == 7));
    assert!(output.summary.starts_with("色板"), "{}", output.summary);
}

#[test]
fn color_analysis_over_an_any_image_input_takes_any_format() {
    use crate::nodes::palette as pa;

    // 输入声明的是 Image(Any)，所以 JPEG 不先转 PNG 也能直接接。
    let dir = workspace("palette-any");
    let jpg = image_fixture(&dir, "jpg", 12, 12);

    let mut workflow = Workflow::new("直接分析 jpg");
    workflow.nodes.push(input_node("in", &jpg));
    workflow
        .nodes
        .push(node("an", pa::KIND, defaults(pa::KIND)));
    workflow.edges.push(edge("e1", "in", "out", "an", "image"));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    assert!(run(&workflow, None, &dir.join("out")).unwrap().ok);
}

#[test]
fn a_palette_feeds_a_colour_parameter_by_its_first_colour() {
    use crate::model::node_kind::param_port_id;
    use crate::nodes::border as bd;
    use crate::nodes::palette as pa;

    let dir = workspace("palette-into-colour");
    // 一张全红的图：统计出来的色板第一行就是红。
    let path = dir.join("red.png");
    RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]))
        .save(&path)
        .unwrap();

    let mut workflow = Workflow::new("色板当颜色");
    workflow.nodes.push(input_node("in", &path));
    let mut analysis = defaults(pa::KIND);
    analysis.insert("mode".into(), serde_json::json!("count"));
    analysis.insert("threshold".into(), serde_json::json!(0));
    workflow.nodes.push(node("an", pa::KIND, analysis));
    workflow
        .nodes
        .push(node("bd", bd::KIND, defaults(bd::KIND)));
    workflow.edges.push(edge("e1", "in", "out", "an", "image"));
    workflow.edges.push(edge("e3", "in", "out", "bd", "image"));
    // 色板接到「颜色」参数端口上，应取第一个颜色。
    workflow
        .edges
        .push(edge("e2", "an", "palette", "bd", &param_port_id("color")));

    let resolved = resolve(&workflow);
    assert!(errors(&resolved.issues).is_empty(), "{:?}", resolved.issues);
    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let out_path = report
        .nodes
        .iter()
        .find(|result| result.node_id == "bd")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    let out = image::open(out_path).unwrap().to_rgba8();
    assert_eq!(
        out.get_pixel(0, 0).0,
        [255, 0, 0, 255],
        "边框用了色板第一个颜色"
    );
}

#[test]
fn remove_color_keyed_the_matching_pixels_transparent() {
    use crate::nodes::remove_color as rc;

    let dir = workspace("remove-color");
    // 3×1：纯白、近白、红。
    let path = dir.join("key.png");
    let mut image = RgbaImage::from_pixel(3, 1, Rgba([255, 255, 255, 255]));
    image.put_pixel(1, 0, Rgba([240, 250, 255, 255]));
    image.put_pixel(2, 0, Rgba([255, 0, 0, 255]));
    image.save(&path).unwrap();

    let mut workflow = Workflow::new("去底色");
    workflow.nodes.push(input_node("in", &path));
    let mut params = defaults(rc::KIND);
    params.insert("color".into(), serde_json::json!("#ffffff"));
    params.insert("threshold".into(), serde_json::json!(16));
    workflow.nodes.push(node("rc", rc::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "rc", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out_path = report
        .nodes
        .iter()
        .find(|result| result.node_id == "rc")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    let out = image::open(out_path).unwrap().to_rgba8();

    assert_eq!(out.get_pixel(0, 0).0[3], 0, "纯白被剔透");
    assert_eq!(out.get_pixel(1, 0).0[3], 0, "在阈值内的近白也被剔透");
    assert_eq!(
        out.get_pixel(2, 0).0,
        [255, 0, 0, 255],
        "红不在范围内，原样留着"
    );
}

#[test]
fn border_grows_an_opaque_image_into_a_frame() {
    // 整张不透明 → 相当于画框：四边各加 1 像素，边距是红色。
    let image = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
    let out = border_result("border-opaque", &image, 1, "#ff0000");
    assert_eq!(out.dimensions(), (6, 6));
    assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255], "新边距是边框色");
    assert_eq!(out.get_pixel(5, 5).0, [255, 0, 0, 255]);
    assert_eq!(out.get_pixel(1, 1).0, [10, 20, 30, 255], "原图一个像素不改");
    assert_eq!(out.get_pixel(4, 4).0, [10, 20, 30, 255]);
}

#[test]
fn border_outlines_transparent_content_in_place() {
    // 7×7 全透明，中间 3×3 是白色 —— 四周留白够宽，尺寸不该变，描的是内容边界。
    let mut image = RgbaImage::from_pixel(7, 7, Rgba([0, 0, 0, 0]));
    for y in 2..5 {
        for x in 2..5 {
            image.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    let out = border_result("border-sprite", &image, 1, "#ff0000");
    assert_eq!(out.dimensions(), (7, 7), "内容四周有留白，尺寸不变");
    assert_eq!(out.get_pixel(3, 3).0, [255, 255, 255, 255], "内容保留");
    assert_eq!(
        out.get_pixel(1, 3).0,
        [255, 0, 0, 255],
        "紧贴内容的一圈被描上"
    );
    assert_eq!(out.get_pixel(0, 0).0, [0, 0, 0, 0], "远处仍透明");
}

#[test]
fn border_outlines_the_inside_of_a_hole() {
    // 甜甜圈：5×5 白色实心，正中间挖一个透明的洞 —— 洞里也该被描上。
    let mut image = RgbaImage::from_pixel(5, 5, Rgba([255, 255, 255, 255]));
    image.put_pixel(2, 2, Rgba([0, 0, 0, 0]));
    let out = border_result("border-donut", &image, 1, "#00ff00");
    // 内容贴着四边，所以画布向外扩了 1 像素（5 × 5 → 7 × 7）。
    assert_eq!(out.dimensions(), (7, 7));
    assert_eq!(out.get_pixel(3, 3).0, [0, 255, 0, 255], "洞被从内侧描上");
    assert_eq!(out.get_pixel(1, 1).0, [255, 255, 255, 255], "实体保留");
}

#[test]
fn border_with_a_transparent_colour_just_pads() {
    let image = RgbaImage::from_pixel(3, 3, Rgba([9, 9, 9, 255]));
    let out = border_result("border-padding", &image, 2, "transparent");
    assert_eq!(out.dimensions(), (7, 7));
    assert_eq!(
        out.get_pixel(0, 0).0,
        [0, 0, 0, 0],
        "透明色就只是加了一圈透明边距"
    );
    assert_eq!(out.get_pixel(3, 3).0, [9, 9, 9, 255]);
}

#[test]
fn transform_flips_then_rotates_a_png() {
    use crate::nodes::transform as tf;

    let dir = workspace("transform");
    let png = image_fixture(&dir, "png", 3, 2);

    let mut workflow = Workflow::new("变换");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(tf::KIND);
    params.insert("flipHorizontal".into(), serde_json::json!(true));
    params.insert("rotate".into(), serde_json::json!("90"));
    workflow.nodes.push(node("tf", tf::KIND, params));
    workflow.edges.push(edge("e1", "in", "out", "tf", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let path = report
        .nodes
        .iter()
        .find(|result| result.node_id == "tf")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    let out = image::open(path).unwrap().to_rgba8();

    // 与「先左右翻转、再顺时针 90°」逐像素对照 —— 顺序不能反。
    let source = image::open(&png).unwrap().to_rgba8();
    let expected = image::imageops::rotate90(&image::imageops::flip_horizontal(&source));
    assert_eq!((out.width(), out.height()), (2, 3), "90° 要长宽对调");
    assert_eq!(out.dimensions(), expected.dimensions());
    assert!(out.pixels().eq(expected.pixels()), "像素应当完全一致");
}

#[test]
fn transform_with_default_params_is_a_no_op() {
    use crate::nodes::transform as tf;

    let dir = workspace("transform-noop");
    let png = image_fixture(&dir, "png", 4, 3);

    let mut workflow = Workflow::new("变换默认");
    workflow.nodes.push(input_node("in", &png));
    workflow
        .nodes
        .push(node("tf", tf::KIND, defaults(tf::KIND)));
    workflow.edges.push(edge("e1", "in", "out", "tf", "image"));

    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let result = report
        .nodes
        .iter()
        .find(|result| result.node_id == "tf")
        .unwrap();
    assert!(
        result.warnings.iter().any(|w| w.contains("原样通过")),
        "什么都没选时会提示原样通过：{:?}",
        result.warnings
    );
}

#[test]
fn shape_crop_asks_the_interface_and_honours_the_answer() {
    use crate::interaction::{Interaction, InteractionKind, InteractionResponse};
    use std::sync::mpsc;

    let dir = workspace("shape-crop");
    let png = image_fixture(&dir, "png", 8, 8);

    let mut workflow = Workflow::new("形状裁切");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::crop::KIND);
    params.insert("mode".into(), serde_json::json!("shape"));
    params.insert("shape".into(), serde_json::json!("ellipse"));
    workflow
        .nodes
        .push(node("crop", crate::nodes::crop::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "crop", "image"));

    // 预先把答案塞进通道：运行到紫色节点时它会发一条请求、然后阻塞地收这条答案。
    let (request_tx, request_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();
    response_tx
        .send(InteractionResponse::Crop {
            x: 1,
            y: 1,
            width: 6,
            height: 6,
        })
        .unwrap();
    let interaction = Interaction::new(request_tx, response_rx);

    let report = run_with(&workflow, None, &dir.join("out"), Some(&interaction), None).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    // 请求确实发出来了，而且认得出是谁在等。
    let request = request_rx.try_recv().expect("紫色节点应当发来一条请求");
    assert_eq!(request.node_id, "crop");
    assert_eq!(request.node_name, "图像裁切");
    assert!(matches!(request.kind, InteractionKind::CropShape(_)));

    // 尺寸严格按用户框的那一块（6 × 6），而不是参数里的默认比例。
    let cropped = report
        .nodes
        .iter()
        .find(|result| result.node_id == "crop")
        .unwrap();
    let path = cropped.outputs[0].path.clone().unwrap();
    let out = image::open(path).unwrap();
    assert_eq!((out.width(), out.height()), (6, 6));
}

#[test]
fn a_blocking_node_without_an_interface_fails_cleanly() {
    let dir = workspace("shape-crop-no-ui");
    let png = image_fixture(&dir, "png", 4, 4);

    let mut workflow = Workflow::new("无界面的形状裁切");
    workflow.nodes.push(input_node("in", &png));
    let mut params = defaults(crate::nodes::crop::KIND);
    params.insert("mode".into(), serde_json::json!("shape"));
    workflow
        .nodes
        .push(node("crop", crate::nodes::crop::KIND, params));
    workflow
        .edges
        .push(edge("e1", "in", "out", "crop", "image"));

    // 无头运行：没有界面可问，应当明确报错而不是 panic。
    let report = run(&workflow, None, &dir.join("out")).unwrap();
    assert!(!report.ok);
    let cropped = report
        .nodes
        .iter()
        .find(|result| result.node_id == "crop")
        .unwrap();
    assert!(matches!(cropped.status, NodeStatus::Failed));
    assert!(
        cropped.error.as_deref().unwrap_or("").contains("界面"),
        "{:?}",
        cropped.error
    );
}

// ---------------------------------------------------------------------------
// 逻辑节点与列表
// ---------------------------------------------------------------------------

/// 「顺序执行」的输出个数跟着 `outs` 参数走；它不需要数据输入，因此自动带一个空输入。
#[test]
fn sequence_ports_follow_its_count_param() {
    let spec = registry().get(crate::nodes::sequence::KIND).unwrap();
    let mut params = spec.kind.default_params();
    assert_eq!(spec.outputs_for(&params).len(), 1, "默认就一个输出");

    params.insert("outs".into(), serde_json::json!(3.0));
    let ports = spec.outputs_for(&params);
    assert_eq!(
        ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["out1", "out2", "out3"]
    );
    assert!(ports[0].required, "第一个是必须输出");
    assert!(ports[1..].iter().all(|p| !p.required), "其余的都可删");
    assert!(ports.iter().all(|p| p.ty == PortType::Void));
    // 输出都挂在参数栏的对应行上 —— 节点顶部不该再有输出。
    assert_eq!(
        ports.iter().map(|p| p.panel_row).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2)]
    );

    // 不需要数据输入 —— 自动一个空输入。
    let inputs = spec.kind.inputs_for(&params);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].id, crate::model::node_kind::VOID_INPUT_PORT);
    assert_eq!(inputs[0].ty, PortType::Void);
}

/// 「遍历」的端口：一个循环体输出 + 可选的「结束」；默认不限制遍历数目。
#[test]
fn iterate_has_a_single_body_output_and_an_optional_finally() {
    let spec = registry().get(crate::nodes::iterate::KIND).unwrap();
    let params = spec.kind.default_params();
    let ports = spec.outputs_for(&params);
    assert_eq!(
        ports.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["item", "finally"]
    );
    // 「元素」是必填的（不接就等于循环体什么都不做）；「结束」可选。
    assert!(ports[0].required, "元素是必填输出");
    assert!(!ports[1].required, "结束时可选输出");
    assert_eq!(spec.kind.inputs_for(&params)[0].ty, PortType::any_list());
    assert_eq!(params["max"], serde_json::json!(0.0), "默认 0 = 不限制");
}

/// 「遍历」对每个元素把下游那一支跑一遍。
#[test]
fn iterate_runs_the_body_once_per_element() {
    let src = workspace("iterate-src");
    image_fixture(&src, "png", 2, 2);
    // 第二个文件（同一种格式，类型才对得上「缩放图像」）。
    std::fs::copy(src.join("input.png"), src.join("second.png")).unwrap();

    let mut dir_params = defaults(crate::nodes::read_dir::KIND);
    dir_params.insert("path".into(), serde_json::json!(src.to_string_lossy()));
    let mut up_params = defaults(crate::nodes::upscale::KIND);
    up_params.insert("percent".into(), serde_json::json!(200.0));

    let mut workflow = Workflow::new("循环");
    workflow
        .nodes
        .push(node("d", crate::nodes::read_dir::KIND, dir_params));
    workflow.nodes.push(node(
        "it",
        crate::nodes::iterate::KIND,
        defaults(crate::nodes::iterate::KIND),
    ));
    workflow
        .nodes
        .push(node("up", crate::nodes::upscale::KIND, up_params));
    workflow.edges.push(edge("e1", "d", "out", "it", "list"));
    workflow.edges.push(edge(
        "e2",
        "it",
        crate::nodes::iterate::ITEM_PORT,
        "up",
        "image",
    ));

    let out = workspace("iterate-out");
    let report = run(&workflow, None, &out).unwrap();
    assert!(report.ok, "{:?}", report.nodes);

    let it = report.nodes.iter().find(|r| r.node_id == "it").unwrap();
    assert!(
        it.warnings.iter().any(|w| w.contains("遍历了 2 个元素")),
        "{:?}",
        it.warnings
    );
    let up = report.nodes.iter().find(|r| r.node_id == "up").unwrap();
    assert_eq!(up.status, NodeStatus::Ok);

    // 循环体跑了两轮 —— 自动输出目录里该有两份产物。
    let produced: Vec<_> = std::fs::read_dir(report.output_dir.as_ref().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(produced.len(), 2, "两轮该落两份");
}

/// 「最大遍历数目」限制跑几轮；0 是不限制。
#[test]
fn iterate_honours_the_maximum() {
    let src = workspace("iterate-max-src");
    image_fixture(&src, "png", 2, 2);
    std::fs::copy(src.join("input.png"), src.join("second.png")).unwrap();

    let mut dir_params = defaults(crate::nodes::read_dir::KIND);
    dir_params.insert("path".into(), serde_json::json!(src.to_string_lossy()));
    let mut it_params = defaults(crate::nodes::iterate::KIND);
    it_params.insert("max".into(), serde_json::json!(1.0));

    let mut workflow = Workflow::new("限量");
    workflow
        .nodes
        .push(node("d", crate::nodes::read_dir::KIND, dir_params));
    workflow
        .nodes
        .push(node("it", crate::nodes::iterate::KIND, it_params));
    workflow.nodes.push(node(
        "up",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow.edges.push(edge("e1", "d", "out", "it", "list"));
    workflow.edges.push(edge(
        "e2",
        "it",
        crate::nodes::iterate::ITEM_PORT,
        "up",
        "image",
    ));

    let out = workspace("iterate-max-out");
    let report = run(&workflow, None, &out).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let it = report.nodes.iter().find(|r| r.node_id == "it").unwrap();
    assert!(
        it.warnings.iter().any(|w| w.contains("遍历了 1 个元素")),
        "{:?}",
        it.warnings
    );
    let produced = std::fs::read_dir(report.output_dir.as_ref().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(produced, 1, "限量到 1 就只跑一轮");
}

/// 嵌套的遍历：外层收到内层循环的节点，`direct` / `reach` 要能正确划分。
#[test]
fn nested_loops_partition_into_direct_scopes() {
    let mut workflow = Workflow::new("嵌套");
    let it = defaults(crate::nodes::iterate::KIND);
    let rn = defaults(crate::nodes::rename::KIND);
    for (id, kind, params) in [
        ("outer", crate::nodes::iterate::KIND, it.clone()),
        ("x", crate::nodes::rename::KIND, rn.clone()),
        ("inner", crate::nodes::iterate::KIND, it.clone()),
        ("y", crate::nodes::rename::KIND, rn.clone()),
        ("z", crate::nodes::rename::KIND, rn.clone()),
    ] {
        workflow.nodes.push(node(id, kind, params));
    }
    // outer.item → x → inner.list → inner.item → y → z
    workflow.edges.push(edge(
        "e1",
        "outer",
        crate::nodes::iterate::ITEM_PORT,
        "x",
        "in",
    ));
    workflow.edges.push(edge("e2", "x", "out", "inner", "list"));
    workflow.edges.push(edge(
        "e3",
        "inner",
        crate::nodes::iterate::ITEM_PORT,
        "y",
        "in",
    ));
    workflow.edges.push(edge("e4", "y", "out", "z", "in"));
    // 循环体 y 还有一个**外部**依赖 w —— 该被排到内层循环之前。
    workflow
        .nodes
        .push(node("w", crate::nodes::rename::KIND, rn.clone()));
    workflow.edges.push(edge("e5", "w", "out", "y", "in"));

    let sets = super::loop_sets(registry(), &workflow);
    let outer = &sets["outer"];
    assert_eq!(
        sorted(&outer.direct),
        vec!["inner", "x"],
        "内层循环自己留在外层里"
    );
    assert!(outer.reach.contains("y") && outer.reach.contains("z"));
    let inner = &sets["inner"];
    assert_eq!(sorted(&inner.direct), vec!["y", "z"]);
    assert_eq!(inner.reach.len(), 2, "内层不该把外层的节点算进来");

    // 内层的外部依赖 w 该排到内层之前。
    let ordering = super::loop_ordering(registry(), &workflow);
    assert!(ordering.contains(&("w", "inner")), "{:?}", ordering);
    assert!(
        ordering.contains(&("w", "outer")),
        "嵌套体用到的外部依赖必须在外层之前"
    );
    assert!(!ordering.contains(&("x", "outer")));
}

fn loop_fixture(src: &Path) -> Workflow {
    let mut workflow = Workflow::new("loop regression");
    let mut params = defaults(crate::nodes::read_dir::KIND);
    params.insert("path".into(), serde_json::json!(src.to_string_lossy()));
    workflow
        .nodes
        .push(node("source", crate::nodes::read_dir::KIND, params));
    workflow.nodes.push(node(
        "loop",
        crate::nodes::iterate::KIND,
        defaults(crate::nodes::iterate::KIND),
    ));
    workflow.nodes.push(node(
        "body",
        crate::nodes::upscale::KIND,
        defaults(crate::nodes::upscale::KIND),
    ));
    workflow
        .edges
        .push(edge("list", "source", "out", "loop", "list"));
    workflow
        .edges
        .push(edge("item", "loop", "item", "body", "image"));
    workflow
}

#[test]
fn empty_loop_runs_zero_iterations_and_still_signals_finally() {
    use crate::registry::RegistryBuilder;
    let mut builder = RegistryBuilder::with_builtins().unwrap();
    builder
        .register_node(test_spec(
            "empty_source",
            vec![],
            vec![PortDef::new("out", "list", PortType::any_list())],
            |_| Ok(ValueMap::from([("out".into(), Value::List(vec![]))])),
        ))
        .unwrap();
    let registry = builder.freeze().unwrap();
    let mut workflow = loop_fixture(&workspace("empty-loop"));
    workflow.nodes[0] = node("source", "empty_source", Params::new());
    workflow.nodes.push(node(
        "done",
        crate::nodes::literal::KIND_TEXT,
        defaults(crate::nodes::literal::KIND_TEXT),
    ));
    workflow.edges.push(edge(
        "done",
        "loop",
        "finally",
        "done",
        crate::model::node_kind::VOID_INPUT_PORT,
    ));
    for target in [None, Some("body"), Some("done")] {
        let report = run_with_registry(
            &registry,
            &workflow,
            target,
            &workspace("empty-loop-out"),
            None,
            None,
        )
        .unwrap();
        assert!(report.ok, "{:?} {:?}", report.issues, report.nodes);
        assert!(!report.nodes.iter().any(|result| result.node_id == "body"));
        let loop_result = report
            .nodes
            .iter()
            .find(|result| result.node_id == "loop")
            .unwrap();
        assert_eq!(loop_result.status, NodeStatus::Ok);
        assert_eq!(loop_result.warnings, vec!["遍历了 0 个元素"]);
        assert_eq!(
            report.nodes.iter().any(|result| result.node_id == "done"),
            target != Some("body"),
        );
    }
}

#[test]
fn nested_empty_loops_do_not_execute_bodies_or_reuse_previous_items() {
    use crate::registry::RegistryBuilder;
    let mut builder = RegistryBuilder::with_builtins().unwrap();
    builder
        .register_node(test_spec(
            "nested_empty_source",
            vec![],
            vec![PortDef::new("out", "list", PortType::any_list())],
            |_| {
                Ok(ValueMap::from([(
                    "out".into(),
                    Value::List(vec![
                        Value::List(vec![]),
                        Value::List(vec![Value::text("only item")]),
                        Value::List(vec![]),
                    ]),
                )]))
            },
        ))
        .unwrap();
    let registry = builder.freeze().unwrap();
    let mut workflow = Workflow::new("nested empty loops");
    workflow.nodes = vec![
        node("source", "nested_empty_source", Params::new()),
        node(
            "outer",
            crate::nodes::iterate::KIND,
            defaults(crate::nodes::iterate::KIND),
        ),
        node(
            "inner",
            crate::nodes::iterate::KIND,
            defaults(crate::nodes::iterate::KIND),
        ),
        node(
            "body",
            crate::nodes::rename::KIND,
            defaults(crate::nodes::rename::KIND),
        ),
        node(
            "inner_done",
            crate::nodes::literal::KIND_TEXT,
            defaults(crate::nodes::literal::KIND_TEXT),
        ),
        node(
            "outer_done",
            crate::nodes::literal::KIND_TEXT,
            defaults(crate::nodes::literal::KIND_TEXT),
        ),
    ];
    workflow.edges = vec![
        edge("list", "source", "out", "outer", "list"),
        edge("nested", "outer", "item", "inner", "list"),
        edge("item", "inner", "item", "body", "in"),
        edge(
            "inner_done",
            "inner",
            "finally",
            "inner_done",
            crate::model::node_kind::VOID_INPUT_PORT,
        ),
        edge(
            "outer_done",
            "outer",
            "finally",
            "outer_done",
            crate::model::node_kind::VOID_INPUT_PORT,
        ),
    ];
    let (tx, rx) = std::sync::mpsc::sync_channel(crate::progress::PROGRESS_CAPACITY);
    let progress = Progress::new(tx);
    let report = run_with_registry(
        &registry,
        &workflow,
        None,
        &workspace("nested-empty-out"),
        None,
        Some(&progress),
    )
    .unwrap();
    assert!(report.ok, "{:?} {:?}", report.issues, report.nodes);
    let inner = report
        .nodes
        .iter()
        .find(|result| result.node_id == "inner")
        .unwrap();
    assert_eq!(
        inner.warnings,
        vec!["遍历了 0 个元素", "遍历了 1 个元素", "遍历了 0 个元素"]
    );
    let body = report
        .nodes
        .iter()
        .find(|result| result.node_id == "body")
        .unwrap();
    assert_eq!(body.status, NodeStatus::Ok);
    assert_eq!(body.outputs[0].summary, "only item");
    let finished: Vec<_> = rx
        .try_iter()
        .filter_map(|event| match event {
            crate::progress::ProgressEvent::Finished { result } => Some(result),
            _ => None,
        })
        .collect();
    for (id, count) in [
        ("body", 1),
        ("inner", 3),
        ("inner_done", 3),
        ("outer_done", 1),
    ] {
        assert_eq!(
            finished
                .iter()
                .filter(|result| result.node_id == id)
                .count(),
            count,
            "{id}"
        );
    }
    for id in ["inner_done", "outer_done"] {
        assert_eq!(
            report
                .nodes
                .iter()
                .find(|result| result.node_id == id)
                .unwrap()
                .status,
            NodeStatus::Ok
        );
    }
}

#[test]
fn connected_loop_limit_overrides_the_saved_parameter() {
    let src = workspace("wired-max");
    image_fixture(&src, "png", 2, 2);
    std::fs::copy(src.join("input.png"), src.join("second.png")).unwrap();
    let mut workflow = loop_fixture(&src);
    let mut params = defaults(crate::nodes::literal::KIND_NUMBER);
    params.insert("value".into(), serde_json::json!(1));
    workflow
        .nodes
        .push(node("limit", crate::nodes::literal::KIND_NUMBER, params));
    workflow
        .edges
        .push(edge("max", "limit", "out", "loop", "param:max"));
    let report = run(&workflow, None, &workspace("wired-max-out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert!(report
        .nodes
        .iter()
        .find(|r| r.node_id == "loop")
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains("遍历了 1 个元素")));
}

#[test]
fn loop_merge_keeps_execution_errors_above_skips_and_successes() {
    use crate::registry::RegistryBuilder;
    for order in [[1.0, 2.0, 3.0], [2.0, 1.0, 3.0]] {
        let mut builder = RegistryBuilder::with_builtins().unwrap();
        builder
            .register_node(test_spec(
                "merge_source",
                vec![],
                vec![PortDef::new("out", "list", PortType::any_list())],
                move |_| {
                    Ok(ValueMap::from([(
                        "out".into(),
                        Value::List(order.into_iter().map(Value::Number).collect()),
                    )]))
                },
            ))
            .unwrap();
        builder
            .register_node(test_spec(
                "merge_gate",
                vec![PortDef::new("in", "in", PortType::Number).required()],
                vec![PortDef::new("out", "out", PortType::Number)],
                |args| {
                    let value = args.input("in")?.as_number()?;
                    if value == 2.0 {
                        Ok(ValueMap::new())
                    } else {
                        Ok(ValueMap::from([("out".into(), Value::Number(value))]))
                    }
                },
            ))
            .unwrap();
        builder
            .register_node(test_spec(
                "merge_body",
                vec![PortDef::new("in", "in", PortType::Number).required()],
                vec![PortDef::new("out", "out", PortType::Number)],
                |args| {
                    let value = args.input("in")?.as_number()?;
                    args.warnings.push(format!("executed {value}"));
                    if value == 1.0 {
                        Err(crate::error::NodeError::new("actual execution failure"))
                    } else {
                        Ok(ValueMap::from([("out".into(), Value::Number(value))]))
                    }
                },
            ))
            .unwrap();
        let registry = builder.freeze().unwrap();
        let mut workflow = Workflow::new("merge error priority");
        workflow.nodes = vec![
            node("source", "merge_source", Params::new()),
            node(
                "loop",
                crate::nodes::iterate::KIND,
                defaults(crate::nodes::iterate::KIND),
            ),
            node("gate", "merge_gate", Params::new()),
            node("body", "merge_body", Params::new()),
        ];
        workflow.edges = vec![
            edge("list", "source", "out", "loop", "list"),
            edge("item", "loop", "item", "gate", "in"),
            edge("body", "gate", "out", "body", "in"),
        ];
        let (tx, rx) = std::sync::mpsc::sync_channel(crate::progress::PROGRESS_CAPACITY);
        let progress = Progress::new(tx);
        let report = run_with_registry(
            &registry,
            &workflow,
            None,
            &workspace("merge-priority-out"),
            None,
            Some(&progress),
        )
        .unwrap();
        assert!(!report.ok);
        let body = report
            .nodes
            .iter()
            .find(|result| result.node_id == "body")
            .unwrap();
        assert_eq!(body.status, NodeStatus::Failed);
        assert_eq!(body.error.as_deref(), Some("actual execution failure"));
        assert_eq!(body.warnings, vec!["executed 1", "executed 3"]);
        assert_eq!(body.outputs[0].summary, "3");
        let statuses: Vec<_> = rx
            .try_iter()
            .filter_map(|event| match event {
                crate::progress::ProgressEvent::Finished { result } if result.node_id == "body" => {
                    Some(result.status)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            if order[0] == 1.0 {
                vec![NodeStatus::Failed, NodeStatus::Skipped, NodeStatus::Ok]
            } else {
                vec![NodeStatus::Skipped, NodeStatus::Failed, NodeStatus::Ok]
            }
        );
    }
}

#[test]
fn loop_failures_do_not_block_later_iterations() {
    let src = workspace("iteration-reset");
    image_fixture(&src, "jpg", 2, 2);
    image_fixture(&src, "png", 2, 2);
    let mut workflow = loop_fixture(&src);
    workflow.nodes.push(node(
        "after",
        crate::nodes::rename::KIND,
        defaults(crate::nodes::rename::KIND),
    ));
    workflow
        .edges
        .push(edge("after", "body", "image", "after", "in"));
    let root = workspace("iteration-reset-out");
    let report = run(&workflow, None, &root).unwrap();
    assert!(
        !report.ok,
        "earlier failure must remain in the aggregate report"
    );
    let after = report.nodes.iter().find(|r| r.node_id == "after").unwrap();
    assert!(
        !after.outputs.is_empty(),
        "second iteration must recover: {:?}",
        report.nodes
    );
    let product = after.outputs[0].path.as_ref().unwrap();
    assert!(Path::new(product).exists());
    workflow
        .nodes
        .retain(|node| node.id == "source" || node.id == "loop");
    workflow.edges.retain(|edge| edge.target == "loop");
    assert!(run(&workflow, None, &root).unwrap().ok);
    assert!(
        !Path::new(product).exists(),
        "failed runs must still persist artifact ownership"
    );
}

#[test]
fn partial_loop_run_never_executes_sibling_or_finally_branches() {
    let src = workspace("partial-loop");
    image_fixture(&src, "png", 2, 2);
    let mut workflow = loop_fixture(&src);
    workflow.nodes.push(node(
        "sibling",
        crate::nodes::rename::KIND,
        defaults(crate::nodes::rename::KIND),
    ));
    workflow.nodes.push(node(
        "done",
        crate::nodes::rename::KIND,
        defaults(crate::nodes::rename::KIND),
    ));
    workflow
        .edges
        .push(edge("sibling", "loop", "item", "sibling", "in"));
    workflow
        .edges
        .push(edge("done", "loop", "finally", "done", "in"));
    let report = run(&workflow, Some("body"), &workspace("partial-loop-out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert!(report
        .nodes
        .iter()
        .all(|r| report.order.contains(&r.node_id)));
    assert!(!report
        .nodes
        .iter()
        .any(|r| r.node_id == "sibling" || r.node_id == "done"));
    let report = run(&workflow, Some("done"), &workspace("partial-finally-out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    for id in ["body", "sibling", "done"] {
        assert!(
            report.nodes.iter().any(|r| r.node_id == id),
            "finally must wait for its complete body"
        );
    }
}

#[test]
fn unsafe_rename_stems_fail_before_saving() {
    for name in [
        "../escape",
        "..\\escape",
        "C:escape",
        "/absolute",
        "CON",
        "nul.txt",
        "COM1.png",
        "LPT9",
        "trailing.",
        "trailing ",
    ] {
        let dir = workspace("unsafe-rename");
        let (_, report) = rename_pipeline(&dir, name, true);
        assert_eq!(
            report
                .nodes
                .iter()
                .find(|r| r.node_id == "rn")
                .unwrap()
                .status,
            NodeStatus::Failed,
            "{name}"
        );
    }
}

#[test]
fn product_manifest_preserves_user_files_and_isolates_workflow_ids() {
    let src = workspace("manifest-source");
    image_fixture(&src, "png", 2, 2);
    let mut workflow = loop_fixture(&src);
    let root = workspace("manifest-root");
    let first = run(&workflow, None, &root).unwrap();
    let dir = PathBuf::from(first.output_dir.unwrap());
    let product = first
        .nodes
        .iter()
        .find(|r| r.node_id == "body")
        .unwrap()
        .outputs[0]
        .path
        .clone()
        .unwrap();
    std::fs::write(dir.join("01-user.txt"), b"user").unwrap();
    std::fs::write(dir.join("100-user.png"), b"user").unwrap();
    workflow.nodes.retain(|n| n.id != "body");
    workflow.edges.retain(|e| e.target != "body");
    let second = run(&workflow, None, &root).unwrap();
    assert!(second.ok);
    assert!(!Path::new(&product).exists());
    assert_eq!(std::fs::read(dir.join("01-user.txt")).unwrap(), b"user");
    assert_eq!(std::fs::read(dir.join("100-user.png")).unwrap(), b"user");
    let mut other = workflow.clone();
    other.id = "another-id".into();
    let other = run(&other, None, &root).unwrap();
    assert_ne!(second.output_dir, other.output_dir);
}

fn test_spec(
    id: &str,
    inputs: Vec<PortDef>,
    outputs: Vec<PortDef>,
    run: impl Fn(&mut NodeArgs<'_>) -> Result<ValueMap, crate::error::NodeError> + Send + Sync + 'static,
) -> crate::registry::NodeSpec {
    let mut kind = registry()
        .get(crate::nodes::rename::KIND)
        .unwrap()
        .kind
        .clone();
    kind.id = id.into();
    kind.name = id.into();
    kind.inputs = inputs;
    kind.outputs = outputs;
    kind.params.clear();
    crate::registry::NodeSpec::fixed(kind, run)
}

#[test]
fn custom_control_metadata_supports_nested_loops_and_external_dependencies() {
    use crate::registry::{ControlFlow, NodeSpec, RegistryBuilder};
    use std::sync::{Arc, Mutex};
    let mut builder = RegistryBuilder::with_builtins().unwrap();
    builder
        .register_node(test_spec(
            "nested_source",
            vec![],
            vec![PortDef::new("out", "list", PortType::any_list())],
            |_| {
                Ok(ValueMap::from([(
                    "out".into(),
                    Value::List(vec![
                        Value::List(vec![Value::Number(1.0), Value::Number(2.0)]),
                        Value::List(vec![Value::Number(3.0)]),
                    ]),
                )]))
            },
        ))
        .unwrap();
    let mut kind = registry()
        .get(crate::nodes::iterate::KIND)
        .unwrap()
        .kind
        .clone();
    kind.id = "custom_loop".into();
    kind.inputs = vec![PortDef::new("batch", "batch", PortType::any_list()).required()];
    kind.outputs = vec![
        PortDef::new("current", "current", PortType::Any),
        PortDef::new("done", "done", PortType::Void),
    ];
    kind.params[0].id = "ceiling".into();
    builder
        .register_node(
            NodeSpec::fixed(kind, |_| panic!("loop callback must not run")).control_flow(
                ControlFlow::Loop {
                    list_input: "batch".into(),
                    item_output: "current".into(),
                    finally_output: "done".into(),
                    max_param: "ceiling".into(),
                },
            ),
        )
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    builder
        .register_node(test_spec(
            "record",
            vec![
                PortDef::new("in", "in", PortType::Number).required(),
                PortDef::new("external", "external", PortType::Number).required(),
            ],
            vec![],
            move |args| {
                captured
                    .lock()
                    .unwrap()
                    .push(args.input("in")?.as_number()? + args.input("external")?.as_number()?);
                Ok(ValueMap::new())
            },
        ))
        .unwrap();
    let registry = builder.freeze().unwrap();
    let mut workflow = Workflow::new("custom nested");
    workflow.nodes = vec![
        node("source", "nested_source", Params::new()),
        node("outer", "custom_loop", Params::new()),
        node("inner", "custom_loop", Params::new()),
        node("body", "record", Params::new()),
        node(
            "external",
            crate::nodes::literal::KIND_NUMBER,
            Params::from_iter([("value".into(), serde_json::json!(10))]),
        ),
    ];
    workflow.edges = vec![
        edge("a", "source", "out", "outer", "batch"),
        edge("b", "outer", "current", "inner", "batch"),
        edge("c", "inner", "current", "body", "in"),
        edge("d", "external", "out", "body", "external"),
    ];
    let report = run_with_registry(
        &registry,
        &workflow,
        None,
        &workspace("custom-nested"),
        None,
        None,
    )
    .unwrap();
    assert!(report.ok, "{:?} {:?}", report.issues, report.nodes);
    assert_eq!(*seen.lock().unwrap(), vec![11.0, 12.0, 13.0]);
    seen.lock().unwrap().clear();
    workflow.nodes.push(node(
        "limit",
        crate::nodes::literal::KIND_NUMBER,
        Params::from_iter([("value".into(), serde_json::json!(1))]),
    ));
    workflow
        .edges
        .push(edge("limit", "limit", "out", "inner", "param:ceiling"));
    let report = run_with_registry(
        &registry,
        &workflow,
        Some("body"),
        &workspace("custom-nested-partial"),
        None,
        None,
    )
    .unwrap();
    assert!(report.ok, "{:?} {:?}", report.issues, report.nodes);
    assert_eq!(*seen.lock().unwrap(), vec![11.0, 13.0]);
}

#[test]
fn loop_parameter_inputs_share_runtime_type_and_failure_checks() {
    use crate::registry::RegistryBuilder;
    for fail in [false, true] {
        let src = workspace("bad-loop-max");
        image_fixture(&src, "png", 2, 2);
        let mut builder = RegistryBuilder::with_builtins().unwrap();
        builder
            .register_node(test_spec(
                "bad_limit",
                vec![],
                vec![PortDef::new("out", "limit", PortType::Any)],
                move |_| {
                    if fail {
                        Err(crate::error::NodeError::new("failed limit"))
                    } else {
                        Ok(ValueMap::from([(
                            "out".into(),
                            Value::text("not a number"),
                        )]))
                    }
                },
            ))
            .unwrap();
        let registry = builder.freeze().unwrap();
        let mut workflow = loop_fixture(&src);
        workflow
            .nodes
            .push(node("limit", "bad_limit", Params::new()));
        workflow
            .edges
            .push(edge("limit", "limit", "out", "loop", "param:max"));
        let report = run_with_registry(
            &registry,
            &workflow,
            None,
            &workspace("bad-loop-max-out"),
            None,
            None,
        )
        .unwrap();
        assert!(!report.ok);
        assert_eq!(
            report
                .nodes
                .iter()
                .find(|r| r.node_id == "loop")
                .unwrap()
                .status,
            if fail {
                NodeStatus::Skipped
            } else {
                NodeStatus::Failed
            }
        );
        assert_eq!(
            report
                .nodes
                .iter()
                .find(|r| r.node_id == "body")
                .unwrap()
                .status,
            NodeStatus::Skipped
        );
    }
}

#[test]
fn sequence_ordering_uses_custom_prefixes() {
    use crate::registry::{ControlFlow, RegistryBuilder};
    let mut builder = RegistryBuilder::with_builtins().unwrap();
    builder
        .register_node(
            test_spec(
                "custom_sequence",
                vec![],
                vec![
                    PortDef::new("step1", "one", PortType::Void),
                    PortDef::new("step2", "two", PortType::Void),
                ],
                |_| {
                    Ok(ValueMap::from([
                        ("step1".into(), Value::Void),
                        ("step2".into(), Value::Void),
                    ]))
                },
            )
            .control_flow(ControlFlow::Sequence {
                output_prefix: "step".into(),
                blocking_prefix: "wait".into(),
            }),
        )
        .unwrap();
    let registry = builder.freeze().unwrap();
    let mut workflow = Workflow::new("custom sequence");
    workflow.nodes = vec![
        node("b", crate::nodes::literal::KIND_TEXT, Params::new()),
        node("a", crate::nodes::literal::KIND_TEXT, Params::new()),
        node("seq", "custom_sequence", Params::new()),
    ];
    workflow.edges = vec![
        edge(
            "b",
            "seq",
            "step2",
            "b",
            crate::model::node_kind::VOID_INPUT_PORT,
        ),
        edge(
            "a",
            "seq",
            "step1",
            "a",
            crate::model::node_kind::VOID_INPUT_PORT,
        ),
    ];
    let report = run_with_registry(
        &registry,
        &workflow,
        None,
        &workspace("custom-sequence"),
        None,
        None,
    )
    .unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    assert_eq!(report.order, vec!["seq", "a", "b"]);
    workflow.nodes[2]
        .params
        .insert("wait1".into(), serde_json::json!(false));
    let report = run_with_registry(
        &registry,
        &workflow,
        None,
        &workspace("custom-sequence-free"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(report.order, vec!["seq", "b", "a"]);
}

fn sorted(set: &HashSet<String>) -> Vec<String> {
    let mut values: Vec<String> = set.iter().cloned().collect();
    values.sort();
    values
}

/// 「图像分割」把 sprite 图切成一个列表（每块一张图）。
#[test]
fn split_image_cuts_a_sprite_sheet_into_a_list() {
    let dir = workspace("split-src");
    let path = dir.join("sheet.png");
    let mut sheet = RgbaImage::from_pixel(6, 6, Rgba([0, 0, 0, 0]));
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        sheet.put_pixel(x, y, Rgba([255, 0, 0, 255]));
    }
    for (x, y) in [(4, 4), (5, 4), (4, 5), (5, 5)] {
        sheet.put_pixel(x, y, Rgba([0, 0, 255, 255]));
    }
    sheet.save(&path).unwrap();

    let mut workflow = Workflow::new("分割");
    workflow.nodes.push(input_node("in", &path));
    workflow.nodes.push(node(
        "split",
        crate::nodes::split::KIND,
        defaults(crate::nodes::split::KIND),
    ));
    workflow
        .edges
        .push(edge("e1", "in", "out", "split", "image"));

    let report = run(&workflow, None, &workspace("split-out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out = &report.nodes[1].outputs[0];
    assert_eq!(
        out.ty,
        PortType::list_of(PortType::Image(ImageFormat::Png)),
        "输出该是 List[Png]"
    );
    assert_eq!(out.summary, "列表 · 2 项");
}

/// 列表值的类型跟着元素走；不同元素类型降级成 `List[Any]`。
#[test]
fn a_list_value_derives_its_element_type() {
    let same = Value::List(vec![Value::text("a"), Value::text("b")]);
    assert_eq!(same.port_type(), PortType::list_of(PortType::Text));

    let mixed = Value::List(vec![Value::text("a"), Value::Number(1.0)]);
    assert_eq!(mixed.port_type(), PortType::list_of(PortType::Any));

    assert_eq!(Value::List(vec![]).port_type(), PortType::any_list());
}

/// 「顺序执行」的输出序号决定分支先后：即使「后一个分支」的节点先加入列表，也要后跑。
#[test]
fn sequence_orders_branches_by_output_index() {
    let mut seq_params = defaults(crate::nodes::sequence::KIND);
    seq_params.insert("outs".into(), serde_json::json!(2.0));

    let mut text_params = defaults(crate::nodes::literal::KIND_TEXT);
    text_params.insert("value".into(), serde_json::json!("second"));

    let mut workflow = Workflow::new("顺序");
    // 故意让第二个分支的节点先出现 —— 拓扑序得靠排序约束把它压到后面。
    workflow.nodes.push(node(
        "b2",
        crate::nodes::literal::KIND_TEXT,
        text_params.clone(),
    ));
    text_params.insert("value".into(), serde_json::json!("first"));
    workflow
        .nodes
        .push(node("b1", crate::nodes::literal::KIND_TEXT, text_params));
    workflow
        .nodes
        .push(node("s", crate::nodes::sequence::KIND, seq_params));
    workflow.edges.push(edge(
        "e2",
        "s",
        "out2",
        "b2",
        crate::model::node_kind::VOID_INPUT_PORT,
    ));
    workflow.edges.push(edge(
        "e1",
        "s",
        "out1",
        "b1",
        crate::model::node_kind::VOID_INPUT_PORT,
    ));

    let report = run(&workflow, None, &workspace("sequence-order")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let pos = |id: &str| report.order.iter().position(|n| n == id).unwrap();
    assert!(pos("s") < pos("b1"), "顺序执行先跑");
    assert!(
        pos("b1") < pos("b2"),
        "out1 的分支该先跑：{:?}",
        report.order
    );
}

/// 把 out1 的阻塞开关关掉：两段之间不再有先后约束。
#[test]
fn sequence_non_blocking_output_drops_the_ordering() {
    let mut seq_params = defaults(crate::nodes::sequence::KIND);
    seq_params.insert("outs".into(), serde_json::json!(2.0));
    seq_params.insert("block1".into(), serde_json::json!(false));

    let mut second = defaults(crate::nodes::literal::KIND_TEXT);
    second.insert("value".into(), serde_json::json!("second"));
    let mut first = defaults(crate::nodes::literal::KIND_TEXT);
    first.insert("value".into(), serde_json::json!("first"));

    let mut workflow = Workflow::new("非阻塞");
    // 这里故意让 b2 在 b1 之前 —— 非阻塞时顺序就按插入序，不再被“抢”过来。
    workflow
        .nodes
        .push(node("b2", crate::nodes::literal::KIND_TEXT, second));
    workflow
        .nodes
        .push(node("b1", crate::nodes::literal::KIND_TEXT, first));
    workflow
        .nodes
        .push(node("s", crate::nodes::sequence::KIND, seq_params));
    workflow.edges.push(edge(
        "e2",
        "s",
        "out2",
        "b2",
        crate::model::node_kind::VOID_INPUT_PORT,
    ));
    workflow.edges.push(edge(
        "e1",
        "s",
        "out1",
        "b1",
        crate::model::node_kind::VOID_INPUT_PORT,
    ));

    let report = run(&workflow, None, &workspace("seq-nonblock")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let pos = |id: &str| report.order.iter().position(|n| n == id).unwrap();
    assert!(
        pos("b2") < pos("b1"),
        "非阻塞就不该把 b1 抢到前面：{:?}",
        report.order
    );
}

/// 「读取目录」把目录里的图像读成一个列表；混合格式收窄到 `List[Img]`。
#[test]
fn read_dir_collects_a_list_of_images() {
    let dir = workspace("read-dir-src");
    image_fixture(&dir, "png", 2, 2);
    image_fixture(&dir, "jpg", 3, 3);

    let mut params = defaults(crate::nodes::read_dir::KIND);
    params.insert("path".into(), serde_json::json!(dir.to_string_lossy()));
    let mut workflow = Workflow::new("读目录");
    workflow
        .nodes
        .push(node("d", crate::nodes::read_dir::KIND, params));

    let report = run(&workflow, None, &workspace("read-dir-out")).unwrap();
    assert!(report.ok, "{:?}", report.nodes);
    let out = &report.nodes[0].outputs[0];
    assert_eq!(
        out.ty,
        PortType::list_of(PortType::trait_("img")),
        "PNG + JPEG 混合应收窄为 List[Img]"
    );
    assert_eq!(out.summary, "列表 · 2 项");
}
