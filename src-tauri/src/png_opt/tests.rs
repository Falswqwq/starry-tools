//! PNG 优化的测试。
//!
//! 每一条都盯着同一个底线：**优化完的像素必须和原来一模一样**。除此之外再看
//! 挑中的颜色类型对不对、文件有没有变小、元数据留得对不对。

use super::*;
use image::{ImageBuffer, Rgb, Rgba};

fn encode_png(image: &DynamicImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("写出基准 PNG");
    bytes
}

/// 跑一遍优化，并且顺手验一次逐像素相等。
fn run(image: &DynamicImage, scheme: Scheme) -> Optimized {
    let source = encode_png(image);
    let optimized = optimize(image, Some(&source), &Options::new(scheme, Strip::Safe))
        .unwrap_or_else(|err| panic!("优化失败：{}", err.0));
    assert!(verify(image, &optimized.bytes), "优化后的像素和原图对不上",);
    assert!(
        optimized.bytes.len() <= source.len(),
        "优化后反而更大了：{} → {}",
        source.len(),
        optimized.bytes.len(),
    );
    optimized
}

/// 从 IHDR 里读出（位深，颜色类型）。
fn color_shape(png: &[u8]) -> (u8, u8) {
    let reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .expect("能读回来");
    let info = reader.info();
    (info.bit_depth as u8, info.color_type as u8)
}

fn chunk_kinds(png: &[u8]) -> Vec<String> {
    read_chunks(png)
        .expect("结构完整")
        .into_iter()
        .map(|(kind, _)| String::from_utf8_lossy(&kind).to_string())
        .collect()
}

fn rgba_image(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> DynamicImage {
    DynamicImage::ImageRgba8(ImageBuffer::from_fn(width, height, |x, y| {
        Rgba(pixel(x, y))
    }))
}

/// 一张颜色多、又带透明的图 —— 调色板装不下，只能老老实实存 RGBA。
fn rich_rgba() -> DynamicImage {
    rgba_image(32, 32, |x, y| {
        [
            (x * 7) as u8,
            (y * 11) as u8,
            ((x * y) % 251) as u8,
            (x * 8) as u8,
        ]
    })
}

// ---------------------------------------------------------------------------
// 九、逐像素一致
// ---------------------------------------------------------------------------

#[test]
fn rich_rgba_image_is_lossless_and_keeps_alpha() {
    let image = rich_rgba();
    let optimized = run(&image, Scheme::Balanced);

    // 颜色太多，进不了调色板；有透明，所以不能丢 alpha。
    assert_eq!(color_shape(&optimized.bytes), (8, 6), "应该是 RGBA 8 位");
    assert!(optimized.improved, "这种图至少能小一点");
}

#[test]
fn every_scheme_gives_the_same_pixels() {
    let image = rich_rgba();
    for scheme in [Scheme::Fast, Scheme::Balanced, Scheme::Maximum] {
        let optimized = run(&image, scheme);
        assert!(optimized.verified, "{scheme:?} 没有通过校验");
    }
}

#[test]
fn zopfli_scheme_keeps_pixels_and_does_not_grow() {
    let image = rgba_image(24, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, 90, 255]);
    let optimized = run(&image, Scheme::Zopfli);
    assert!(
        optimized.strategy.contains("zopfli"),
        "{}",
        optimized.strategy
    );
}

// ---------------------------------------------------------------------------
// 一、颜色类型 / 位深
// ---------------------------------------------------------------------------

#[test]
fn opaque_image_never_keeps_an_alpha_channel() {
    let image = rgba_image(32, 32, |x, y| [(x * 7) as u8, (y * 11) as u8, 200, 255]);
    let optimized = run(&image, Scheme::Balanced);
    let (_, color_type) = color_shape(&optimized.bytes);
    assert_ne!(color_type, 6, "整张不透明就不该存成 RGBA");
    assert_ne!(color_type, 4, "整张不透明就不该存成灰度 + 透明");
}

#[test]
fn grayscale_image_is_stored_as_grayscale() {
    // 20 级灰阶：是灰度，但不是 17 的倍数之外的规律，索引色和灰度都值得试试。
    let image = rgba_image(20, 20, |x, y| {
        let value = ((x + y) * 13) as u8;
        [value, value, value, 255]
    });
    let optimized = run(&image, Scheme::Balanced);
    let (_, color_type) = color_shape(&optimized.bytes);
    assert!(
        color_type == 0 || color_type == 3,
        "灰度图该走灰度或索引色，实际颜色类型 {color_type}",
    );
    // 灰度只有 20 级，用不着 8 位存。
    let (bit_depth, _) = color_shape(&optimized.bytes);
    assert!(bit_depth <= 8);
}

#[test]
fn bilevel_image_compresses_to_one_bit() {
    let image = rgba_image(32, 32, |x, y| {
        if (x + y) % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let optimized = run(&image, Scheme::Balanced);
    assert_eq!(
        color_shape(&optimized.bytes),
        (1, 0),
        "两极灰度就该是 1 位灰度"
    );
}

#[test]
fn colour_counts_pick_the_smallest_bit_depth() {
    // 颜色数 → 期望的索引位深。
    for (count, expected_bits) in [(1usize, 1u8), (2, 1), (4, 2), (16, 4), (256, 8)] {
        // 颜色打散着铺：既不凑成平滑的行（那样 RGB 配 DPCM 也能压得很小），
        // 又保证每种颜色都真的出现过。红绿分量不一样，免得被当成灰度图走捷径。
        let image = rgba_image(256, 256, |x, y| {
            let index = scramble(x, y) % count;
            [index as u8, 3, 5, 255]
        });

        let optimized = run(&image, Scheme::Balanced);
        let (bit_depth, color_type) = color_shape(&optimized.bytes);
        assert_eq!(color_type, 3, "{count} 色应该走索引色，实际 {color_type}");
        assert_eq!(
            bit_depth, expected_bits,
            "{count} 色该用 {expected_bits} 位"
        );
    }
}

/// 跟着坐标走的散列，把颜色铺得「没规律」一点。
fn scramble(x: u32, y: u32) -> usize {
    let mut value = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263));
    value ^= value >> 13;
    value = value.wrapping_mul(1_274_126_177);
    (value ^ (value >> 16)) as usize
}

#[test]
fn a_tiny_image_may_prefer_the_simplest_encoding() {
    // 16×16、三种颜色：索引色的数据虽然小，可调色板和 tRNS 的头占了大头，
    // 压完反而比 RGBA 大。这时候就该老老实实选 RGBA —— 目标是「最小」，
    // 不是「看上去最聪明」。这个用例是把「真的比大小」这条规矩钉住。
    let palette = [[0, 0, 0, 0], [255, 0, 0, 255], [16, 32, 64, 255]];
    let image = rgba_image(16, 16, |x, y| palette[((x / 2 + y / 3) % 3) as usize]);
    let optimized = run(&image, Scheme::Balanced);
    assert!(optimized.verified);
    assert_eq!(
        color_shape(&optimized.bytes),
        (8, 6),
        "这种尺寸下 RGBA 反而最小"
    );
}

#[test]
fn transparent_pixel_art_keeps_its_transparency() {
    let palette = [[0, 0, 0, 0], [255, 0, 0, 255], [0, 128, 255, 255]];
    let image = rgba_image(64, 64, |x, y| palette[((x / 2 + y / 3) % 3) as usize]);
    let optimized = run(&image, Scheme::Balanced);

    assert_eq!(
        color_shape(&optimized.bytes),
        (2, 3),
        "3 色 + 透明该是 2 位索引"
    );
    // 透明像素必须还是完全透明。
    let decoded = image::load_from_memory(&optimized.bytes)
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded.get_pixel(0, 0).0[3], 0, "透明像素被改掉了");
}

#[test]
fn sixteen_bit_images_stay_sixteen_bit() {
    let image = DynamicImage::ImageRgb16(ImageBuffer::from_fn(16, 16, |x, y| {
        Rgb([(x * 4096) as u16, (y * 4096) as u16, 30000])
    }));
    let optimized = run(&image, Scheme::Balanced);
    let (bit_depth, color_type) = color_shape(&optimized.bytes);
    assert_eq!(bit_depth, 16, "16 位的图不能偷偷降到 8 位");
    assert_eq!(color_type, 2, "没有透明就该是 RGB");
}

// ---------------------------------------------------------------------------
// 五、元数据
// ---------------------------------------------------------------------------

/// 往 PNG 里塞一个 tEXt 块。
fn with_text_chunk(png: &[u8], key: &str, value: &str) -> Vec<u8> {
    let chunks = read_chunks(png).expect("结构完整");
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE);
    for (kind, data) in chunks {
        if &kind == b"IEND" {
            let mut payload = key.as_bytes().to_vec();
            payload.push(0);
            payload.extend_from_slice(value.as_bytes());
            write_chunk(&mut out, b"tEXt", &payload);
        }
        write_chunk(&mut out, &kind, &data);
    }
    out
}

/// 让 png 编码器顺手写一个 sRGB 块出来。
fn with_srgb_chunk(image: &DynamicImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(image.to_rgba8().as_raw()).unwrap();
    }
    bytes
}

#[test]
fn text_chunks_are_stripped_but_colour_chunks_are_kept() {
    let image = rich_rgba();
    let source = with_text_chunk(&with_srgb_chunk(&image), "Software", "StarryTools");

    let kept = optimize(
        &image,
        Some(&source),
        &Options::new(Scheme::Balanced, Strip::Safe),
    )
    .unwrap();
    let kinds = chunk_kinds(&kept.bytes);
    assert!(
        kinds.contains(&"sRGB".to_string()),
        "颜色相关的块该留着：{kinds:?}"
    );
    assert!(
        !kinds.contains(&"tEXt".to_string()),
        "文本块该去掉：{kinds:?}"
    );
    assert!(verify(&image, &kept.bytes));
}

#[test]
fn aggressive_stripping_drops_colour_chunks_too() {
    let image = rich_rgba();
    let source = with_srgb_chunk(&image);

    let stripped = optimize(
        &image,
        Some(&source),
        &Options::new(Scheme::Balanced, Strip::All),
    )
    .unwrap();
    let kinds = chunk_kinds(&stripped.bytes);
    assert!(
        !kinds.contains(&"sRGB".to_string()),
        "全剥离就该什么都不剩：{kinds:?}"
    );
    assert!(verify(&image, &stripped.bytes), "剥元数据不能动像素");
}

// ---------------------------------------------------------------------------
// 七、不做无意义的优化
// ---------------------------------------------------------------------------

#[test]
fn an_already_optimal_png_is_left_alone() {
    let image = rich_rgba();
    let first = run(&image, Scheme::Balanced);

    // 把上一轮的产物再喂一遍：同样的管线只会得出同样的大小，所以这次应当原样返回。
    let again = optimize(
        &image,
        Some(&first.bytes),
        &Options::new(Scheme::Balanced, Strip::Safe),
    )
    .unwrap();
    assert!(!again.improved, "压不动了就不该硬压");
    assert_eq!(again.bytes, first.bytes, "该原样把原图吐回来");
    assert_eq!(again.before, Some(first.bytes.len()));
    assert!(verify(&image, &again.bytes));
}

#[test]
fn savings_are_reported_against_the_original() {
    let image = rich_rgba();
    let source = encode_png(&image);
    let optimized = optimize(
        &image,
        Some(&source),
        &Options::new(Scheme::Maximum, Strip::Safe),
    )
    .expect("优化得起来");

    assert_eq!(optimized.before, Some(source.len()));
    assert_eq!(optimized.after(), optimized.bytes.len());
    let ratio = optimized.saved_ratio().expect("有原图就该算得出比例");
    assert!((0.0..1.0).contains(&ratio), "比例该在 0 和 1 之间：{ratio}");
}

#[test]
fn a_jpeg_source_still_produces_a_png() {
    // 输入本来就不是 PNG：没有可比的「之前」，但照样得给出一份合法的 PNG。
    let image = rgba_image(16, 16, |x, y| [(x * 15) as u8, (y * 15) as u8, 40, 255]);
    let optimized = optimize(&image, None, &Options::new(Scheme::Balanced, Strip::Safe)).unwrap();
    assert!(optimized.before.is_none());
    assert!(verify(&image, &optimized.bytes));
}

// ---------------------------------------------------------------------------
// 编码细节
// ---------------------------------------------------------------------------

#[test]
fn sub_byte_rows_are_padded_per_row() {
    // 宽度 3、1 位：每行 3 个值挤进 1 个字节，行与行不能串味儿。
    let packed = pack_rows(&[1, 0, 1, 0, 1, 0], 3, 2, 1);
    assert_eq!(packed.len(), 2);
    assert_eq!(packed[0], 0b1010_0000);
    assert_eq!(packed[1], 0b0100_0000);
}

#[test]
fn gray_bit_depth_picks_the_narrowest_fit() {
    assert_eq!(gray_bit_depth(&[0, 255]), 1);
    assert_eq!(gray_bit_depth(&[0, 85, 170, 255]), 2);
    assert_eq!(gray_bit_depth(&[0, 17, 238, 255]), 4);
    assert_eq!(gray_bit_depth(&[0, 3, 200]), 8);
}

#[test]
fn chunk_round_trip_preserves_everything() {
    let image = rich_rgba();
    let source = encode_png(&image);
    let chunks = read_chunks(&source).expect("结构完整");

    let mut rebuilt = Vec::new();
    rebuilt.extend_from_slice(&SIGNATURE);
    for (kind, data) in &chunks {
        write_chunk(&mut rebuilt, kind, data);
    }
    assert_eq!(rebuilt, source, "拆开再拼回去该是一个字节都不差");
}
