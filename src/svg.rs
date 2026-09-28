//! SVG 解码臂(#152;beyond-original — 上游 GDI+ 零 SVG,见 README
//! Differences 与 ADR 0005):魔数嗅探 → `usvg::Tree` 解析 → tiny-skia
//! 光栅化 → 直乘 RGBA 进共享帧管线(合成背景 → Srgb master,同
//! clipboard DIB 臂的一帧流形状)。纯逻辑层 — 无 Win32、无 unsafe;
//! resvg 0.48 经重导出提供 `usvg`/`tiny_skia`/`fontdb` 全部类型。
//!
//! 三个载重决策,均出自 s-svg.md「决策影响」§3 的实测留档:
//!
//! - **fontdb 必须显式装系统字体**:`usvg::Options::default()` 携带空
//!   fontdb,`<text>` 会静默渲染成空(A1 sample2 实测);系统字体首扫
//!   35–132 ms,进程内只做一次(`OnceLock` 共享)。
//! - **`resources_dir` 保持 `None`**:外链(本地图片等)天然隔离,加载
//!   侧最多一次 stat(A1 攻击面实测:外链不存在优雅跳过)。
//! - **光栅目标面由调用方 clamp**:tiny-skia 无面积上限,usvg 对
//!   自然尺寸 10^5 的 viewBox 也照单全收(A1 攻击面:无 clamp 时进入
//!   40 GB 零初始化分配 8 s 未完成被强杀)。`raster_target` 是这道
//!   闸:fit≥1 钉在自然尺寸(光栅面即显示语义的「100%」,放大它 =
//!   小图标撑满整窗,破坏位图格式的 no-upscale fit 同语义),缩小方
//!   向按视口 fit 且下限保住 1024 级中间档(交互缩放的重栅留档配方,
//!   s-svg.md §D),上限压进帧字节预算。
//!
//! svgz(gzip 封装)**刻意不放行**:usvg 0.48 的 `decompress_svgz` 是
//! 无界 `read_to_end`(解压炸弹会膨胀到 OOM 才停),嗅探只认文本签名;
//! `.svgz` 落到 image crate 的 undetermined-format 用户级失败,与该臂
//! 存在之前的行为一致(ADR 0005 录案)。

use std::sync::{Arc, OnceLock};

use resvg::usvg;

/// 嗅探窗口:文件头读取的字节数。合法 SVG 的序言(XML 声明/DOCTYPE/
/// 注释)远短于此;窗口内找不到 `<svg` 根即不认(fail-closed — 宁可
/// 用户级失败,不误入解析器)。
pub(crate) const SNIFF_WINDOW: usize = 512;

/// 中间档长边(s-svg.md §D:1024 级重光栅 0.88 ms,4096 级 12.3 ms 已
/// 逼近 16.7 ms 帧预算):自然尺寸大于它的缩小方向 fit 至少保住这一档,
/// 给加载后的缩放留出清晰度余量。
const INTERMEDIATE_MAX_DIM: f32 = 1024.0;

/// 视口未知(`(0, 0)` 快照,如创建前的窗口)时的兜底显示面。
const DEFAULT_VIEWPORT: (u32, u32) = (1024, 768);

/// 单条 SVG 输入的解析预算(字节)。usvg 自身只限元素个数(1M 上限,
/// 实体炸弹另有环检测),不限输入体积 — 读满整个文件前先量长短,
/// 超限按用户级失败直报而不是把几百 MB 文本喂进解析器。
pub(crate) const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;

/// SVG 魔数嗅探 — image crate 的 `with_guessed_format` 不识 SVG,这道
/// 闸在它之前(s-svg.md 裁定 A 的接入点:同一嗅探语义,扩展名无关)。
/// 只认「文本序言 + `<svg` 根」的形状:剥 BOM 与 XML 空白后,以
/// `<?xml` / `<!DOCTYPE` / `<!--` 开头的须在窗口内出现 `<svg` 根
/// (带元素边界字节,`<svgfoo` 不算);其余须直接以 `<svg` 根开头。
/// PNG/JPEG 等二进制魔数与中段碰巧含 `<svg` 的流都不会命中 — 闸门
/// 看的是流的形状,不是内容扫描。gzip 魔数不在其列(svgz 排除,见
/// 模块文档)。
pub(crate) fn sniff(prefix: &[u8]) -> bool {
    let mut s = prefix;
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        s = &s[3..];
    }
    s = trim_xml_leading_whitespace(s);
    if s.starts_with(b"<?xml") || s.starts_with(b"<!DOCTYPE") || s.starts_with(b"<!--") {
        return contains_svg_root(s);
    }
    starts_with_svg_root(s)
}

/// 判断 `<svg` 后跟的元素边界字节(空格/`>`/`/`/制表/换行/命名空间
/// 冒号);窗口尾正好切在 `<svg` 上也认(512 字节头恰好收在根标签
/// 开头的流几乎必是 SVG,误认的代价只是一次注定失败的解析 — 用户级
/// 失败,旧图保留,ADR 0001)。
fn svg_root_boundary(bytes: &[u8], pos: usize) -> bool {
    let after = pos + 4;
    after >= bytes.len()
        || matches!(
            bytes[after],
            b' ' | b'>' | b'/' | b'\t' | b'\r' | b'\n' | b':'
        )
}

fn starts_with_svg_root(bytes: &[u8]) -> bool {
    bytes.starts_with(b"<svg") && svg_root_boundary(bytes, 0)
}

fn contains_svg_root(bytes: &[u8]) -> bool {
    let mut from = 0;
    while let Some(found) = bytes[from..].windows(4).position(|w| w == b"<svg") {
        let pos = from + found;
        if svg_root_boundary(bytes, pos) {
            return true;
        }
        from = pos + 1;
    }
    false
}

fn trim_xml_leading_whitespace(mut bytes: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = bytes.split_first() {
        if matches!(first, b' ' | b'\t' | b'\r' | b'\n') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

/// 加载期光栅目标面决策:
/// 1. **fit≤1 取 fit、fit≥1 取自然尺寸** — 光栅面即应用眼中的自然尺
///    寸(DEFAULT fit 从不过 100% 放大,README Differences):放大到
///    视口的光栅会被显示语义原样展示,一个 48×48 图标就撑满整窗 —
///    与位图格式同语义要求 fit≥1 时必须钉在自然尺寸(放大方向的清
///    晰度余量属重栅 follow-up,ADR 0005 D6)。缩小方向按视口等比
///    fit:巨 viewBox 缩到窗口(40 GB 分配不可能发生)。
/// 2. **中间档下限** — 缩小方向上,自然尺寸长边超过 1024 的至少保
///    住 1024 级中间档:小窗口里的巨大 SVG 也拿到缩放余量;自然尺寸
///    本就不超过中间档的按自然尺寸(不加戏)。
/// 3. **帧字节预算上限** — 目标面 `w*h*4` 压进 `max_frame_bytes`
///    (等比收缩;防御纵深 — 视口来自窗口客户区,常规远够小,但本函
///    数的契约不依赖调用者的善意,预算收缩的欠账记为重栅级议题)。
///    逐轴再夹一个 16384 硬顶,吸收自然尺寸在乘法前制造的溢出面
///    (1e5 级 viewBox 实测在案)。
pub(crate) fn raster_target(
    natural: (f32, f32),
    viewport: (u32, u32),
    max_frame_bytes: usize,
) -> (u32, u32) {
    let natural = (natural.0.max(1.0), natural.1.max(1.0));
    let viewport = if viewport.0 == 0 || viewport.1 == 0 {
        DEFAULT_VIEWPORT
    } else {
        viewport
    };
    let fit = (viewport.0 as f32 / natural.0).min(viewport.1 as f32 / natural.1);
    // fit >= 1: never above natural (the display's no-upscale fit makes a
    // larger raster dead weight — it would BE the shown "100%" size);
    // fit < 1: the fit display, floored by the zoom-headroom intermediate.
    let intermediate = (INTERMEDIATE_MAX_DIM / natural.0.max(natural.1)).min(1.0);
    let scale = if fit >= 1.0 {
        1.0
    } else {
        fit.max(intermediate)
    };
    let mut target = scaled(natural, scale);

    // 荒诞自然尺寸硬顶:常规 SVG 自然尺寸 ≤ 数千,16384 只拦病态值
    // (1e5 级 viewBox);此路径本就不是真实几何,等比不保可接受。
    const ABSURD_DIM_CAP: u32 = 16384;
    target.0 = target.0.min(ABSURD_DIM_CAP);
    target.1 = target.1.min(ABSURD_DIM_CAP);

    // 字节预算:u64 内必不溢出(16384^2 * 4 < 2^32),等比 sqrt 收缩,
    // floor 保证收缩后严格不超(floor 之积 ≤ 精确值之积)。
    let bytes = target.0 as u64 * target.1 as u64 * 4;
    if bytes > max_frame_bytes as u64 {
        let shrink = (max_frame_bytes as f64 / bytes as f64).sqrt();
        target.0 = ((target.0 as f64) * shrink).floor() as u32;
        target.1 = ((target.1 as f64) * shrink).floor() as u32;
    }
    (target.0.max(1), target.1.max(1))
}

fn scaled(natural: (f32, f32), scale: f32) -> (u32, u32) {
    (
        (natural.0 * scale).round().max(1.0) as u32,
        (natural.1 * scale).round().max(1.0) as u32,
    )
}

/// 重栅判定 — 显示面把光栅面任一轴上采样超过 2× 即需要重光栅
/// (放大超过 2 倍后矢量源与位图源的观感差开始刺眼;2× 恰好不触发,
/// 避免贴着阈值反复抖动)。这是 s-svg.md §3 留档配方(复用 Tree、仅
/// 尺寸变化重栅、交互缩放退 1024 中间档后台出全分辨率)的判定核心。
/// 本票交付判定逻辑 + 测试钉面,UI 接线(窗口持有 Tree、后台重栅、
/// 无闪烁换面)属后续票 — 在那之前生产侧无调用点,dead_code 豁免即
/// 该状态的显式登记(消费方落地时移除)。
#[allow(dead_code)]
pub(crate) fn needs_reraster(raster: (u32, u32), display: (u32, u32)) -> bool {
    display.0 as f64 / raster.0.max(1) as f64 > 2.0
        || display.1 as f64 / raster.1.max(1) as f64 > 2.0
}

/// 解析 + 光栅化:SVG 字节 →(宽,高,直乘 RGBA)帧缓冲,长度恰为
/// `w * h * 4`(tiny-skia 产物是预乘 RGBA,`take_demultiplied` 还原
/// 直乘后交给共享帧管线 — `composite_over_background_in_place` 的
/// 语义按直乘写)。产物色彩空间 = sRGB(resvg 自己的文档保证),无
/// ICC 可言,icm 链结构性缺席 — 与无 profile 的 BMP/ICO/DIB 同类。
/// 每个失败都是用户级(ADR 0001:保留旧图、不退出、不弹框)。
pub(crate) fn decode(
    bytes: &[u8],
    viewport: (u32, u32),
    max_frame_bytes: usize,
) -> Result<(u32, u32, Vec<u8>), String> {
    // 两个安全默认显式钉住(模块文档三决策之二、之三):
    // resources_dir=None — 外链隔离;
    // 系统字体 — 默认空 fontdb 会把 <text> 渲染成空。
    let options = usvg::Options {
        resources_dir: None,
        fontdb: system_fontdb(),
        ..Default::default()
    };
    // 嗅探只认文本签名,gzip 分支不可达;from_data 的 svgz 路径因此
    // 不会被触发(排除理由见模块文档)。
    let tree = usvg::Tree::from_data(bytes, &options)
        .map_err(|e| format!("failed to parse as SVG: {e}"))?;

    let size = tree.size();
    let natural = (nonzero(size.width()), nonzero(size.height()));
    let target = raster_target(natural, viewport, max_frame_bytes);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(target.0, target.1)
        .ok_or("raster target rejected by tiny-skia")?;
    let scale = resvg::tiny_skia::Transform::from_scale(
        target.0 as f32 / natural.0,
        target.1 as f32 / natural.1,
    );
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    Ok((target.0, target.1, pixmap.take_demultiplied()))
}

fn nonzero(v: f32) -> f32 {
    if v > 0.0 { v } else { 1.0 }
}

/// 进程级系统字体库:首扫 35–132 ms(s-svg.md §D),`OnceLock` 保证
/// 进程内一次;之后每次 SVG 解析共享同一 `Arc`(usvg 只读它)。
fn system_fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- sniff:形状闸,不是内容扫描 ----

    #[test]
    fn a_bare_svg_root_opens_the_svg_arm() {
        assert!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"));
    }

    #[test]
    fn an_xml_prolog_with_svg_root_opens_the_svg_arm() {
        assert!(sniff(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"x\"><rect/></svg>"
        ));
    }

    #[test]
    fn a_doctype_with_an_entity_prelude_opens_the_svg_arm() {
        // billion-laughs 形态的序言也走这里 — 解析器侧的环检测
        // (roxmltree 每实体引用 ≤255)是那条路径的防线,闸门不预判。
        assert!(sniff(b"<!DOCTYPE svg [<!ENTITY a \"x\">]><svg></svg>"));
    }

    #[test]
    fn a_comment_before_the_root_opens_the_svg_arm() {
        assert!(sniff(b"<!-- drawn by hand --><svg></svg>"));
    }

    #[test]
    fn a_bom_and_indentation_still_reach_the_root() {
        assert!(sniff("\u{FEFF}\n\t <svg height=\"10\"></svg>".as_bytes()));
    }

    #[test]
    fn a_window_ending_exactly_at_the_root_tag_still_sniffs() {
        // 根标签正好切在窗口尾:认(误认的代价只是一次注定失败的解析)。
        let mut prefix = vec![b' '];
        prefix.extend_from_slice(b"<svg");
        assert!(sniff(&prefix));
    }

    #[test]
    fn an_svgfoo_root_is_not_svg() {
        assert!(!sniff(b"<svgfoo xmlns=\"urn:x\"></svgfoo>"));
    }

    #[test]
    fn an_uppercase_root_is_not_svg() {
        // XML 大小写敏感:SVG 根必须是小写 <svg>。
        assert!(!sniff(b"<SVG></SVG>"));
    }

    #[test]
    fn a_gzip_stream_is_not_admitted_svgz_is_excluded() {
        assert!(!sniff(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn a_png_stream_is_not_svg() {
        assert!(!sniff(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]));
    }

    #[test]
    fn a_plain_xml_document_is_not_svg() {
        assert!(!sniff(b"<?xml version=\"1.0\"?><catalog></catalog>"));
    }

    #[test]
    fn an_empty_stream_is_not_svg() {
        assert!(!sniff(b""));
    }

    #[test]
    fn a_prolog_whose_window_ends_before_any_root_is_not_admitted() {
        // fail-closed:序言后面 512 字节内没出现根 → 不认,宁可让
        // image crate 走 undetermined-format 的用户级失败。
        let mut prefix = b"<?xml version=\"1.0\"?>".to_vec();
        prefix.extend(std::iter::repeat_n(b' ', SNIFF_WINDOW));
        assert!(!sniff(&prefix));
    }

    #[test]
    fn binary_junk_with_an_embedded_svg_tag_is_not_svg() {
        // 闸门看流的形状:中段含 <svg 的二进制流(碰巧的相机原始数据
        // 等)不命中 — 只有序言形状走到 contains 扫描。
        assert!(!sniff(b"\x00\x01\x02<svg></svg>\xff"));
    }

    // ---- raster_target:目标面三段规则 ----

    #[test]
    fn an_icon_that_fits_keeps_its_natural_face() {
        // 24×24 图标在 1920×1080 视口:fit 45× 但光栅钉在自然尺寸 —
        // 光栅面就是显示语义里的「100%」,放大它 = 图标撑满整窗
        //(位图格式同语义;放大方向的清晰度余量属重栅 follow-up)。
        assert_eq!(
            raster_target((24.0, 24.0), (1920, 1080), usize::MAX),
            (24, 24)
        );
    }

    #[test]
    fn a_huge_viewbox_rasterizes_at_the_fit_size_not_the_natural_100k() {
        // spike 攻击面三号的数值:10^5 viewBox 无 clamp 时 40 GB 分配。
        assert_eq!(
            raster_target((100_000.0, 100_000.0), (1920, 1080), usize::MAX),
            (1080, 1080)
        );
    }

    #[test]
    fn a_downscaled_natural_keeps_a_1024_intermediate_for_zoom_headroom() {
        // 4000×3000 自然尺寸在 200×150 小窗:fit 显示 200×150,但目标
        // 保 1024 长边中间档 → 1024×768(缩小方向才有中间档)。
        assert_eq!(
            raster_target((4000.0, 3000.0), (200, 150), usize::MAX),
            (1024, 768)
        );
    }

    #[test]
    fn a_natural_at_or_below_the_intermediate_rasterizes_at_natural() {
        // 800×600 自然尺寸、更小的窗口:中间档不会把目标顶过自然尺寸。
        assert_eq!(
            raster_target((800.0, 600.0), (400, 300), usize::MAX),
            (800, 600)
        );
    }

    #[test]
    fn a_wide_natural_shrinks_with_the_aspect_preserved() {
        // 300×200 在 1920×1080:fit≥1 → 自然尺寸;在 300×200 视口恰好
        // 1:1;在 150×100 视口:fit=0.5,但自然尺寸 ≤ 中间档 → 地板顶
        // 回自然尺寸 300×200(显示面仍由 renderer 按 fit 缩到 150×100,
        // 与位图同语义,光栅白得缩放余量)。
        assert_eq!(
            raster_target((300.0, 200.0), (1920, 1080), usize::MAX),
            (300, 200)
        );
        assert_eq!(
            raster_target((300.0, 200.0), (300, 200), usize::MAX),
            (300, 200)
        );
        assert_eq!(
            raster_target((300.0, 200.0), (150, 100), usize::MAX),
            (300, 200)
        );
    }

    #[test]
    fn an_unknown_viewport_falls_back_to_the_default_face() {
        // (0,0) = 未知视口(创建前请求):默认面下 fit≥1 → 自然尺寸。
        assert_eq!(raster_target((24.0, 24.0), (0, 0), usize::MAX), (24, 24));
    }

    #[test]
    fn an_extreme_aspect_does_not_inflate_the_short_axis() {
        // 30000×200 横幅在 1920×1080:fit 由宽主导。
        assert_eq!(
            raster_target((30_000.0, 200.0), (1920, 1080), usize::MAX),
            (1920, 13)
        );
    }

    #[test]
    fn the_target_never_exceeds_the_frame_byte_budget() {
        // 自然 8192²(×4 = 1 GiB)> 512 MiB 帧预算 → 等比收缩到预算内
        //(约 5793²)— 预算收缩的欠账记为重栅级议题(ADR 0005 D4)。
        let budget = 512 * 1024 * 1024;
        let t = raster_target((8192.0, 8192.0), (8192, 8192), budget);
        assert!(t.0 as u64 * t.1 as u64 * 4 <= budget as u64);
        assert_eq!(t.0, t.1, "等比收缩保持正方形");
    }

    #[test]
    fn an_absurd_natural_is_capped_before_the_byte_math() {
        // 1e5 自然尺寸 + 荒诞视口(fit≥1 走自然):16384 硬顶先夹,字节
        // 预算再压 — 乘法前的溢出面不可能到达。
        let t = raster_target((100_000.0, 100_000.0), (u32::MAX, u32::MAX), usize::MAX);
        assert_eq!(t, (16384, 16384));
    }

    #[test]
    fn a_degenerate_one_pixel_natural_still_decides_a_face() {
        assert_eq!(raster_target((1.0, 1.0), (1920, 1080), usize::MAX), (1, 1));
    }

    // ---- needs_reraster:重栅配方的判定核心 ----

    #[test]
    fn a_display_upscaling_past_two_needs_a_reraster() {
        assert!(needs_reraster((500, 500), (1500, 500)));
        assert!(needs_reraster((500, 500), (500, 1001)));
    }

    #[test]
    fn a_two_times_upscale_exactly_does_not_reraster() {
        // 严格大于才触发:贴着 2× 不抖动。
        assert!(!needs_reraster((500, 500), (1000, 1000)));
    }

    #[test]
    fn a_shrinking_display_never_needs_a_reraster() {
        assert!(!needs_reraster((1024, 768), (300, 200)));
    }

    // ---- decode:end-to-end 纯逻辑(无 Win32)----

    #[test]
    fn a_red_square_decodes_to_a_transparent_corner_and_a_red_center() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <rect x="2" y="2" width="4" height="4" fill="red"/>
        </svg>"##;
        // 8×8 自然尺寸在 64×64 视口:fit≥1 → 光栅 = 自然尺寸 8×8
        //(位图同语义;显示面按 no-upscale fit 就是 8×8)。
        let (w, h, rgba) = decode(svg, (64, 64), usize::MAX).unwrap();
        assert_eq!((w, h), (8, 8));
        assert_eq!(rgba.len(), 8 * 8 * 4);
        let px = |x: usize, y: usize| {
            let i = (y * 8 + x) * 4;
            (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3])
        };
        // rect(2,2,4,4) 原生坐标;中心红(sRGB red 直读)、画布外透明
        //(直乘语义:透明像素 RGB=0)。
        assert_eq!(px(4, 4), (255, 0, 0, 255));
        assert_eq!(px(0, 0), (0, 0, 0, 0));
    }

    #[test]
    fn a_broken_document_fails_user_level_with_a_parse_reason() {
        let err = decode(b"<svg><unclosed>", (64, 64), usize::MAX).unwrap_err();
        assert!(err.contains("failed to parse as SVG"), "{err}");
    }

    #[test]
    fn a_non_svg_xml_root_fails_user_level() {
        // 形状像 svg(嗅探会放行 DOCTYPE 形态 + 窗口内有 <svg)但根
        // 不是 svg 的文档,在解析器侧按用户级失败。
        let err = decode(b"<!DOCTYPE svg><catalog/>", (64, 64), usize::MAX).unwrap_err();
        assert!(err.contains("failed to parse as SVG"), "{err}");
    }

    #[test]
    fn the_frame_budget_clamps_the_raster_not_the_load() {
        // 预算 4 KiB:64×64 自然面(16 KiB)等比压到 32×32(4 KiB 恰满),
        // 载入不失败、缓冲长宽自洽。
        let (w, h, rgba) = decode(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect width="64" height="64" fill="blue"/></svg>"#,
            (64, 64),
            4096,
        )
        .unwrap();
        assert_eq!((w, h), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);
    }
}
