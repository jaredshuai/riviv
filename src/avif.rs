//! AVIF 解码臂(#220,ADR 0008):oxideav-avif `=0.0.15` 的纯逻辑面。
//!
//! 分层与 `svg.rs` 同构:本模块只放纯逻辑——魔数嗅探(复用
//! oxideav 的 `probe`,判据与解码接受面零漂移)、色度接受集
//! 判定(ADR 0008 D3:集外一律拒绝加载,不猜色)、YUV→RGB16
//! 转换纯函数(深位深源走 `ContentSpace::F16Srgb` 深源路径的
//! 输入构件)。文件读取/预算闸/sink 协议在 `loader.rs` 的 AVIF
//! arm(`avif_decode_to_sink`),不在这里。
//!
//! 深度语义(ADR 0008 D3):10/12-bit 输入读 `AvifImage` 公有
//! 原生 u16 平面(16-bit LE 低 n 位有效),自建转换进满量程
//! 65535 的 RGB16 容器(`DeepSamples::Rgb16`/`Rgba16` 的输入
//! 形状);打包输出 `to_rgba8` 仅 8-bit,深路径不得用。转换
//! kernel 与所依赖 oxideav-heif 0.0.13 `rgb.rs` 的
//! `for_each_pixel` 逐样本同构(f64、同一次序、round-to-nearest
//! 后 clamp;子采样色度 co-sited nearest 复制,无插值)——同构
//! 是钉测里 `to_rgba8` 交叉 oracle 能逐像素对上的前提。range
//! 公式按源码实录:Limited 的 Y 轴 offset/scale = 16·2^(n−8) /
//! 219·2^(n−8),C 轴 mid/scale = 2^(n−1) / 224·2^(n−8)(ITU
//! 标准形;ADR 0008 D3 初稿把 Y 轴 offset 写作 64·2^(n−8) 是
//! 笔误,n=8 代入即不成立,回填时订正)。

use oxideav_avif::{AvifImage, ColorInfo, ColorRange};

/// 单条 AVIF 输入的体积预算(字节),与 SVG 臂同值:防整文件
/// 读入的 hostile 大输入;消费级 AVIF(12 MP q50)远小于此。
pub(crate) const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;

/// 魔数嗅探(ISOBMFF `ftyp` box,brand ∈ {avif, avis, avio},
/// av1-avif §6):直接复用 oxideav 的 `probe`——纯函数、零分配、
/// 短输入 fail-closed——判据与 `decode` 的接受面出自同一实现,
/// 不可能漂移(自建一份反而引入两个 brand 集合的对齐义务)。
/// 窗口:ftyp 的 major brand 在字节 8,compatible brands 紧随,
/// 512 字节(loader 的统一前缀读窗口,与 `svg::SNIFF_WINDOW`
/// 同宽)远大于任何真实 brand 列表;窗口尾截断时 fail-closed,
/// 流交回 image crate 走 undetermined-format 的用户级失败。
pub(crate) fn sniff(head: &[u8]) -> bool {
    oxideav_avif::probe(head)
}

/// 色度接受集判定(ADR 0008 D3 明文裁定,集外拒绝 = ADR 0001
/// 用户级:保留旧图 + 状态栏 flash,不猜色):matrix ∈ {0
/// identity 直通, 1 BT.709, 6 BT.601} × range ∈ {Full, Limited}
/// × primaries ∈ {1, 2(unspecified 按 1)} × transfer ∈ {13,
/// 2(unspecified 按 13)}。primaries/transfer 不参与转换数值
/// (接受后当 sRGB 直读,与 `F16Srgb` 容器一致),只做门:BT.2020
/// ncl、PQ/HLG transfer、宽 primaries 一律拒绝。`decode` 产出的
/// `ColorInfo` 一定 effective(`from_colr`:文件 nclx,缺席按
/// MIAF 默认),无「文件缺席」的第三态需要在这里区分。
pub(crate) fn accept_color(color: &ColorInfo) -> Result<(), String> {
    let name = |code: u8, kind: &str| {
        let resolved = match (kind, code) {
            ("matrix", 0) => "0 identity",
            (_, 2) => "2 unspecified",
            _ => "",
        };
        match resolved {
            "" => format!("{kind} {code}"),
            r => format!("{kind} {r}"),
        }
    };
    if !matches!(color.matrix, 0 | 1 | 6) {
        return Err(format!(
            "colour matrix {} is outside the accepted set (0/1/6) — refusing to guess the colour transform",
            name(color.matrix, "matrix")
        ));
    }
    if !matches!(color.range, ColorRange::Full | ColorRange::Limited) {
        // `decode` 从不产出 Unspecified(from_flag 二值);防御性
        // 拒绝而非猜一个(与整个判定门的哲学一致)。
        return Err(format!(
            "colour range {:?} is outside the accepted set (Full/Limited)",
            color.range
        ));
    }
    if !matches!(color.primaries, 1 | 2) {
        return Err(format!(
            "colour primaries {} is outside the accepted set (1, or 2 as 1) — refusing to guess the colour transform",
            name(color.primaries, "primaries")
        ));
    }
    if !matches!(color.transfer, 13 | 2) {
        return Err(format!(
            "colour transfer {} is outside the accepted set (13, or 2 as 13) — refusing to guess the colour transform",
            name(color.transfer, "transfer")
        ));
    }
    Ok(())
}

/// 深位深源(10/12-bit)的转换:`AvifImage` 原生平面 → 满量程
/// 65535 的交错 RGB16(或带 alpha 的 RGBA16)`u16` 缓冲,即
/// `DeepSamples::Rgb16`/`Rgba16` 的输入形状(`w * h * 3` /
/// `w * h * 4` 个样本)。调用前提:`accept_color` 已通过(matrix
/// ∈ {0,1,6}、range ∈ {Full,Limited})。
///
/// kernel 与 oxideav-heif 0.0.13 `rgb.rs::for_each_pixel` 逐样本
/// 同构(见模块文档);mono(含 packed YA)复制三分量;alpha
/// 满量程映射(`a * 65535 / (2^n − 1)` 四舍五入),与 `to_rgba8`
/// 的 alpha 缩放(`a / max * 255`)同一归一。
pub(crate) fn deep_rgb16(image: &AvifImage) -> Vec<u16> {
    let fmt = image.format;
    let w = image.width as usize;
    let h = image.height as usize;
    let depth = u32::from(image.bit_depth);
    let max = f64::from((1u32 << depth) - 1);
    let sh = f64::from(1u32 << (depth - 8));
    let (y_off, y_scale, c_scale) = if image.color.range == ColorRange::Limited {
        (16.0 * sh, 219.0 * sh, 224.0 * sh)
    } else {
        (0.0, max, max)
    };
    let mid = f64::from(1u32 << (depth - 1));
    // identity(0) 无 Kr/Kb——平面即 G,B,R;1 = BT.709,6 = BT.601
    // (H.273 Table 4)。`None` 只对 identity 合法:调用前提是
    // `accept_color` 已过(集外矩阵拒绝在先)——debug_assert 把这
    // 个前提钉成强制,防未来调用方绕门把集外矩阵静默当 GBR 读
    //(补偿评审 P3-6)。
    let kr_kb = match image.color.matrix {
        1 => Some((0.2126, 0.0722)),
        6 => Some((0.299, 0.114)),
        _ => {
            debug_assert_eq!(image.color.matrix, 0, "accept_color gates the matrix");
            None
        }
    };
    let mono = fmt.plane_count() == 1;
    let (sx, sy) = fmt.chroma_subsampling();
    let (sx, sy) = (sx as usize, sy as usize);
    let bps = fmt.bytes_per_sample();
    // 非平面序的 alpha:planar `Yuva*` 的尾平面;packed YA 的
    // luma+alpha 交错在平面 0(样本序 x*2 / x*2+1)。
    let alpha_plane = if fmt.has_alpha() && !fmt.is_packed_ya() {
        Some(fmt.plane_count() - 1)
    } else {
        None
    };
    let sample = |plane: usize, x: usize, y: usize| -> f64 {
        let p = &image.planes[plane];
        let i = y * p.stride + x * bps;
        f64::from(if bps == 1 {
            p.data[i] as u16
        } else {
            u16::from_le_bytes([p.data[i], p.data[i + 1]])
        })
    };
    let quant = |v: f64| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let mut out = Vec::with_capacity(w * h * if fmt.has_alpha() { 4 } else { 3 });
    for y in 0..h {
        for x in 0..w {
            let rgb = if mono {
                let luma = if fmt.is_packed_ya() {
                    sample(0, x * 2, y)
                } else {
                    sample(0, x, y)
                };
                let g = (luma - y_off) / y_scale;
                [g, g, g]
            } else {
                let yv = sample(0, x, y);
                let cb = sample(1, x >> sx, y >> sy);
                let cr = sample(2, x >> sx, y >> sy);
                match kr_kb {
                    None => [
                        (cr - y_off) / y_scale,
                        (yv - y_off) / y_scale,
                        (cb - y_off) / y_scale,
                    ],
                    Some((kr, kb)) => {
                        let ey = (yv - y_off) / y_scale;
                        let pb = (cb - mid) / c_scale;
                        let pr = (cr - mid) / c_scale;
                        let r = ey + 2.0 * (1.0 - kr) * pr;
                        let b = ey + 2.0 * (1.0 - kb) * pb;
                        let g = (ey - kr * r - kb * b) / (1.0 - kr - kb);
                        [r, g, b]
                    }
                }
            };
            out.push(quant(rgb[0]));
            out.push(quant(rgb[1]));
            out.push(quant(rgb[2]));
            if fmt.has_alpha() {
                let a = match alpha_plane {
                    Some(p) => sample(p, x, y),
                    None => sample(0, x * 2 + 1, y),
                };
                out.push((a / max * 65535.0).round() as u16);
            }
        }
    }
    out
}

/// 按行裁左上子矩形(序列降级的 display-dims 裁剪):`src_w × h`
/// 的交错行缓冲(`ch` 样本/像素)取前 `dst_w × dst_h` 像素。调用
/// 前提:`dst_w ≤ src_w`(更大的目标由调用侧拒绝,绝不缩放)。
pub(crate) fn crop_top_left<T: Copy>(
    buf: Vec<T>,
    src_w: usize,
    ch: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<T> {
    debug_assert!(dst_w <= src_w);
    // The identity shape returns the buffer untouched (no copy) — the
    // animation arm calls this per frame, and the display==coded case
    // (every honest file) must not pay an extra full-canvas memcpy.
    if dst_w == src_w && dst_w.saturating_mul(dst_h).saturating_mul(ch) == buf.len() {
        return buf;
    }
    if dst_w == src_w {
        // The common no-pad shape: the whole-buffer prefix is exactly
        // the first dst_h rows.
        buf.into_iter()
            .take(dst_w.saturating_mul(dst_h).saturating_mul(ch))
            .collect()
    } else {
        let row = dst_w * ch;
        let mut out = Vec::with_capacity(row * dst_h);
        for y in 0..dst_h {
            let start = y * src_w * ch;
            out.extend_from_slice(&buf[start..start + row]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_avif::{AvifImage, ColorRange, EncodeOptions, PixelFormat, Plane};

    // ---- 嗅探(钉 oxideav probe 在锁版本上的行为,防升级漂移)----

    fn ftyp(major: &[u8; 4], compat: &[&[u8; 4]]) -> Vec<u8> {
        let mut v = Vec::new();
        let size = 16 + 4 * compat.len();
        v.extend_from_slice(&(size as u32).to_be_bytes());
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(major);
        v.extend_from_slice(&[0, 0, 0, 0]);
        for c in compat {
            v.extend_from_slice(*c);
        }
        v
    }

    #[test]
    fn the_sniff_admits_the_three_avif_brands_wherever_they_sit() {
        // av1-avif §6 的三个 brand:major 或 compatible 位置都认。
        assert!(sniff(&ftyp(b"avif", &[b"mif1", b"miaf"])));
        assert!(sniff(&ftyp(b"mif1", &[b"miaf", b"avif"])));
        assert!(sniff(&ftyp(b"avis", &[b"msf1", b"mif1"])));
        assert!(sniff(&ftyp(b"avio", &[])));
    }

    #[test]
    fn non_avif_streams_and_short_heads_stay_outside_the_gate() {
        // 非 AVIF 的 HEIF(heic)、PNG 魔数、短头、无 ftyp 形状的
        // 流都不得进门(fail-closed:流走 image crate 的既有路径)。
        assert!(!sniff(&ftyp(b"heic", &[b"mif1", b"miaf"])));
        assert!(!sniff(&ftyp(b"mif1", &[b"heic"])));
        assert!(!sniff(b""));
        assert!(!sniff(b"\0\0\0\x10ftypavif")); // 15 字节:少一个都不认
        assert!(!sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0"));
    }

    #[test]
    fn the_display_crop_takes_the_top_left_rectangle_row_by_row() {
        // 4×2 的 RGBA 行缓冲(32 字节)裁到 2×1:每行取前 2 像素,
        // 第二行丢弃。全宽目标走前缀捷径(不重排),窄目标走按行
        // 路径——两条路径的断言分开钉。
        let src: Vec<u8> = (0..32u8).collect(); // 4 像素 × 2 行
        let cropped = crop_top_left(src.clone(), 4, 4, 2, 1);
        assert_eq!(cropped, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        let full_width = crop_top_left(src, 4, 4, 4, 1);
        assert_eq!(full_width, (0..16u8).collect::<Vec<_>>());
        // 更大的目标不在本函数的合同里(debug_assert),调用侧
        // 先拒绝;同尺寸目标是恒等。
        let src = vec![7u16; 12];
        assert_eq!(crop_top_left(src.clone(), 2, 3, 2, 2), src);
    }

    // ---- 接受集(每字段一个接受代表 + 一个拒绝代表)----

    fn color(range: ColorRange, primaries: u8, transfer: u8, matrix: u8) -> ColorInfo {
        ColorInfo::new(range, primaries, transfer, matrix)
    }

    #[test]
    fn the_accepted_chroma_set_takes_both_ranges_and_both_unspecifieds() {
        // 接受集的四个轴各自取边界:matrix 0/1/6、range Full/Limited、
        // primaries 1/2、transfer 13/2——2(unspecified)按 1/13 接受。
        for c in [
            color(ColorRange::Full, 1, 13, 0),
            color(ColorRange::Full, 1, 13, 1),
            color(ColorRange::Full, 1, 13, 6),
            color(ColorRange::Limited, 1, 13, 1),
            color(ColorRange::Limited, 1, 13, 6),
            color(ColorRange::Limited, 2, 2, 6),
            color(ColorRange::Full, 2, 13, 1),
            color(ColorRange::Full, 1, 2, 0),
        ] {
            assert!(accept_color(&c).is_ok(), "{c:?}");
        }
    }

    #[test]
    fn chroma_outside_the_set_is_refused_per_field() {
        // ADR 0008 D3 的拒绝清单各取一个代表:BT.2020 ncl(matrix 9)、
        // unspecified matrix 2、宽 primaries(9 BT.2020 / 12 P3)、
        // PQ(16)/HLG(18) transfer。消息必须点名拒绝的字段值。
        let cases = [
            (color(ColorRange::Full, 1, 13, 9), "matrix 9"),
            (color(ColorRange::Full, 1, 13, 2), "matrix 2"),
            (color(ColorRange::Full, 1, 13, 12), "matrix 12"),
            (color(ColorRange::Full, 9, 13, 6), "primaries 9"),
            (color(ColorRange::Full, 12, 13, 6), "primaries 12"),
            (color(ColorRange::Full, 1, 16, 6), "transfer 16"),
            (color(ColorRange::Full, 1, 18, 6), "transfer 18"),
        ];
        for (c, needle) in cases {
            let err = accept_color(&c).unwrap_err();
            assert!(err.contains(needle), "{c:?}: {err}");
        }
    }

    // ---- 转换 kernel:公式生成期望值写死断言 ----

    /// 8-bit 平面(stride = 宽)。
    fn u8_plane(w: usize, data: &[u8]) -> Plane {
        Plane::new(w, data.to_vec())
    }
    /// 16-bit LE 平面(stride = 宽 × 2 字节)。
    fn u16_plane(w: usize, data: &[u16]) -> Plane {
        Plane::new(w * 2, data.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    #[test]
    fn limited_range_luma_extremes_map_to_the_container_extremes() {
        // BT.709 limited 8-bit(oxideav 侧同一钉测的四角):Y=16 → 0,
        // Y=235 → 65535,中性 chroma(Cb=Cr=128)下 Y=128 →
        // (128−16)/219 × 65535 = 33515.62 → 33516。
        let planes = vec![
            u8_plane(3, &[16, 235, 128]),
            u8_plane(3, &[128, 128, 128]),
            u8_plane(3, &[128, 128, 128]),
        ];
        let img = AvifImage::new(3, 1, PixelFormat::Yuv444P, planes)
            .unwrap()
            .with_color(color(ColorRange::Limited, 1, 13, 1));
        let out = deep_rgb16(&img);
        assert_eq!(out[0], 0, "Y=16 limited → black");
        assert_eq!(out[4], 65535, "Y=235 limited → white");
        assert_eq!(out[8], 33516, "Y=128 limited → (128−16)/219×65535 rounded");
    }

    #[test]
    fn limited_range_10bit_uses_the_scaled_itu_offsets() {
        // n=10 的 ITU 标准形:offset = 16·2² = 64,scale = 219·4 = 876,
        // chroma mid = 2⁹ = 512(scale 224·4 = 896)。Y=64 → 0,
        // Y=940 → 65535,Y=512 → (512−64)/876 × 65535 = 33515.62 → 33516
        //(与 8-bit 的 Y=128 同一归一值——两轴公式是同一分式)。
        let planes = vec![
            u16_plane(3, &[64, 940, 512]),
            u16_plane(3, &[512, 512, 512]),
            u16_plane(3, &[512, 512, 512]),
        ];
        let img = AvifImage::new(3, 1, PixelFormat::Yuv444P10Le, planes)
            .unwrap()
            .with_color(color(ColorRange::Limited, 1, 13, 1));
        let out = deep_rgb16(&img);
        assert_eq!(out[0], 0);
        assert_eq!(out[4], 65535);
        assert_eq!(out[8], 33516);
    }

    #[test]
    fn full_range_10bit_widens_by_the_full_scale_ratio() {
        // Full:Y/1023 → ×65535。0 → 0;1023 → 65535;
        // 512 → 512×65535/1023 = 32799.53 → 32800。
        let planes = vec![
            u16_plane(3, &[0, 1023, 512]),
            u16_plane(3, &[512, 512, 512]),
            u16_plane(3, &[512, 512, 512]),
        ];
        let img = AvifImage::new(3, 1, PixelFormat::Yuv444P10Le, planes)
            .unwrap()
            .with_color(color(ColorRange::Full, 1, 13, 1));
        let out = deep_rgb16(&img);
        assert_eq!(out[0], 0);
        assert_eq!(out[4], 65535);
        assert_eq!(out[8], 32800);
    }

    #[test]
    fn identity_matrix_reads_the_planes_as_gbr_verbatim() {
        // H.273 MC=0:plane0=G, plane1=B, plane2=R(H.273 方程 41-43,
        // oxideav identity 臂同款);Full 8-bit 的 RGB16 = 码值 × 257
        //(65535 = 255 × 257,整数精确)。
        let planes = vec![
            u8_plane(1, &[200]), // G
            u8_plane(1, &[100]), // B
            u8_plane(1, &[50]),  // R
        ];
        let img = AvifImage::new(1, 1, PixelFormat::Yuv444P, planes)
            .unwrap()
            .with_color(color(ColorRange::Full, 1, 13, 0));
        assert_eq!(deep_rgb16(&img), vec![50 * 257, 200 * 257, 100 * 257]);
    }

    #[test]
    fn subsampled_chroma_is_replicated_cosited_nearest() {
        // 4:2:0:单个色度样本复制到 2×2 块的四个像素(co-sited
        // nearest,无插值——与 oxideav kernel 同款)。
        let planes = vec![
            u8_plane(2, &[64, 192, 32, 224]),
            u8_plane(1, &[128]),
            u8_plane(1, &[128]),
        ];
        let img = AvifImage::new(2, 2, PixelFormat::Yuv420P, planes)
            .unwrap()
            .with_color(color(ColorRange::Full, 1, 13, 1));
        let out = deep_rgb16(&img);
        // 全 Full + 中性色度 → RGB = Y×257,每像素独立 luma。
        assert_eq!(out[0], 64 * 257);
        assert_eq!(out[3], 192 * 257);
        assert_eq!(out[6], 32 * 257);
        assert_eq!(out[9], 224 * 257);
    }

    #[test]
    fn mono_replicates_the_luma_and_the_packed_ya_carries_alpha() {
        // Gray10Le:luma 复制三分量;Ya8(packed Y-A 交错):alpha
        // 随行(样本序 x*2 / x*2+1)。
        let img = AvifImage::new(
            2,
            1,
            PixelFormat::Gray10Le,
            vec![u16_plane(2, &[512, 1023])],
        )
        .unwrap()
        .with_color(color(ColorRange::Full, 1, 13, 6));
        assert_eq!(
            deep_rgb16(&img),
            [32800; 3]
                .iter()
                .chain([65535; 3].iter())
                .copied()
                .collect::<Vec<_>>()
        );

        let img = AvifImage::new(
            2,
            1,
            PixelFormat::Ya8,
            vec![u8_plane(4, &[200, 255, 100, 0])],
        )
        .unwrap()
        .with_color(color(ColorRange::Full, 1, 13, 6));
        assert_eq!(
            deep_rgb16(&img),
            vec![
                200 * 257,
                200 * 257,
                200 * 257,
                65535,
                100 * 257,
                100 * 257,
                100 * 257,
                0
            ]
        );
    }

    #[test]
    fn alpha_widens_by_the_full_scale_ratio_of_its_depth() {
        // alpha 满量程映射:10-bit 1023 → 65535、512 → 32800(与
        // luma Full 同一归一)、0 → 0;12-bit 4095 → 65535。
        let planes = vec![
            u16_plane(2, &[0, 0]),
            u16_plane(2, &[0, 0]),
            u16_plane(2, &[0, 0]),
            u16_plane(2, &[1023, 512]),
        ];
        let img = AvifImage::new(2, 1, PixelFormat::Yuva444P10Le, planes)
            .unwrap()
            .with_color(color(ColorRange::Full, 1, 13, 1));
        let out = deep_rgb16(&img);
        assert_eq!(out[3], 65535);
        assert_eq!(out[7], 32800);

        let planes = vec![
            u16_plane(1, &[0]),
            u16_plane(1, &[0]),
            u16_plane(1, &[0]),
            u16_plane(1, &[4095]),
        ];
        let img = AvifImage::new(1, 1, PixelFormat::Yuva444P12Le, planes)
            .unwrap()
            .with_color(color(ColorRange::Full, 1, 13, 1));
        assert_eq!(deep_rgb16(&img)[3], 65535);
    }

    #[test]
    fn the_reference_formula_generates_a_pinned_colour_bar() {
        // BT.709 limited 8-bit 红条(100% 红:Y=63/Cb=102/Cr=240 的
        // 编码侧逆):解码 ey=47/219=0.214612、pb=−26/224、pr=0.5——
        // R = ey+1.5748·0.5 = 1.00201 → clip 65535;
        // B = ey−1.8556·0.116071 = −0.0008 → clip 0;
        // G 用未 clip 的 R/B 中间值(oxideav kernel 同款):
        // (0.214612 − 0.2126·1.00201 + 0.0722·0.0008)/0.7152 =
        // 0.002296 × 65535 = 150.46 → 150。
        let planes = vec![u8_plane(1, &[63]), u8_plane(1, &[102]), u8_plane(1, &[240])];
        let img = AvifImage::new(1, 1, PixelFormat::Yuv444P, planes)
            .unwrap()
            .with_color(color(ColorRange::Limited, 1, 13, 1));
        assert_eq!(deep_rgb16(&img), vec![65535, 150, 0]);
    }

    // ---- 交叉 oracle:与 oxideav `to_rgba8` 的独立第三方对齐 ----

    /// 自算 RGB16 降到 8-bit(`(v · 255 + 32767) / 65535`,oxideav
    /// `to_rgba8` 两级缩放的整数同形);无 alpha 的布局按 `to_rgba8`
    /// 的合同补不透明 255,alpha 已是满量程映射、同式缩放。
    fn rgb16_to_rgba8(samples: &[u16], has_alpha: bool) -> Vec<u8> {
        let scaled = |v: u16| ((u32::from(v) * 255 + 32767) / 65535) as u8;
        if has_alpha {
            samples.iter().map(|&v| scaled(v)).collect()
        } else {
            samples
                .chunks(3)
                .flat_map(|px| [scaled(px[0]), scaled(px[1]), scaled(px[2]), 255])
                .collect()
        }
    }

    #[test]
    fn the_kernel_agrees_with_oxideav_to_rgba8_per_pixel() {
        // ADR 0008 D3 的交叉 oracle:同一 `AvifImage`,自算 RGB16 降
        // 8-bit vs oxideav 的打包输出。kernel 同构 + 两级舍入,逐像素
        // 差 ≤ 1。覆盖:8/10/12-bit × Full/Limited × matrix 0/1/6 ×
        // 4:4:4/4:2:0/mono(含 packed YA)× alpha。
        struct Case {
            image: AvifImage,
        }
        let cases: Vec<Case> = vec![
            // 8-bit BT.709 limited 4:2:0。
            Case {
                image: AvifImage::new(
                    4,
                    2,
                    PixelFormat::Yuv420P,
                    vec![
                        u8_plane(4, &[16, 82, 144, 235, 63, 128, 200, 240]),
                        u8_plane(2, &[102, 160]),
                        u8_plane(2, &[240, 128]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Limited, 1, 13, 1)),
            },
            // 8-bit BT.601 full 4:4:4。
            Case {
                image: AvifImage::new(
                    3,
                    1,
                    PixelFormat::Yuv444P,
                    vec![
                        u8_plane(3, &[10, 128, 250]),
                        u8_plane(3, &[60, 128, 200]),
                        u8_plane(3, &[200, 130, 20]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Full, 1, 13, 6)),
            },
            // 8-bit identity full 4:4:4(GBR 平面)。
            Case {
                image: AvifImage::new(
                    3,
                    1,
                    PixelFormat::Yuv444P,
                    vec![
                        u8_plane(3, &[10, 128, 250]),
                        u8_plane(3, &[60, 128, 200]),
                        u8_plane(3, &[200, 130, 20]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Full, 1, 13, 0)),
            },
            // 8-bit identity full 4:4:4 + alpha(Yuva)。
            Case {
                image: AvifImage::new(
                    2,
                    1,
                    PixelFormat::Yuva444P,
                    vec![
                        u8_plane(2, &[10, 250]),
                        u8_plane(2, &[60, 200]),
                        u8_plane(2, &[200, 20]),
                        u8_plane(2, &[255, 17]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Full, 1, 13, 0)),
            },
            // 10-bit BT.709 limited 4:2:0。
            Case {
                image: AvifImage::new(
                    4,
                    2,
                    PixelFormat::Yuv420P10Le,
                    vec![
                        u16_plane(4, &[64, 350, 512, 940, 70, 420, 700, 900]),
                        u16_plane(2, &[100, 800]),
                        u16_plane(2, &[900, 400]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Limited, 1, 13, 1)),
            },
            // 10-bit BT.601 full 4:4:4 + alpha。
            Case {
                image: AvifImage::new(
                    2,
                    1,
                    PixelFormat::Yuva444P10Le,
                    vec![
                        u16_plane(2, &[100, 900]),
                        u16_plane(2, &[400, 600]),
                        u16_plane(2, &[512, 1023]),
                        u16_plane(2, &[1023, 300]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Full, 1, 13, 6)),
            },
            // 12-bit BT.709 limited 4:4:4。
            Case {
                image: AvifImage::new(
                    2,
                    1,
                    PixelFormat::Yuv444P12Le,
                    vec![
                        u16_plane(2, &[256, 3600]),
                        u16_plane(2, &[600, 2048]),
                        u16_plane(2, &[3800, 1000]),
                    ],
                )
                .unwrap()
                .with_color(color(ColorRange::Limited, 1, 13, 1)),
            },
            // mono:Gray8 与 packed Ya8。
            Case {
                image: AvifImage::new(3, 1, PixelFormat::Gray8, vec![u8_plane(3, &[0, 128, 255])])
                    .unwrap()
                    .with_color(color(ColorRange::Full, 1, 13, 6)),
            },
            Case {
                image: AvifImage::new(
                    2,
                    1,
                    PixelFormat::Ya8,
                    vec![u8_plane(4, &[200, 255, 100, 0])],
                )
                .unwrap()
                .with_color(color(ColorRange::Limited, 1, 13, 6)),
            },
        ];
        for (i, case) in cases.iter().enumerate() {
            let ours = rgb16_to_rgba8(&deep_rgb16(&case.image), case.image.format.has_alpha());
            let theirs = case.image.to_rgba8();
            assert_eq!(ours.len(), theirs.len(), "case {i}");
            let mut worst: i32 = 0;
            let mut mean: f64 = 0.0;
            for (a, b) in ours.iter().zip(&theirs) {
                let d = i32::from(*a) - i32::from(*b);
                worst = worst.max(d.abs());
                mean += d.abs() as f64;
            }
            mean /= ours.len() as f64;
            assert!(worst <= 1, "case {i}: worst per-sample delta {worst}");
            assert!(mean < 0.5, "case {i}: mean |delta| {mean}");
        }
    }

    // ---- 端到端:encode → decode → deep_rgb16(纯依赖,无 Win32)----

    #[test]
    fn a_lossless_identity_roundtrip_recovers_the_deep_planes_exactly() {
        // 重审 R-report 同款:10-bit 4:4:4+alpha 无损(q=0)往返,
        // identity 矩阵下平面即 GBR——往返后转换应逐样本还原
        // (identity + Full = 码值直读:G/B/R/alpha × 64.06 满量程
        // 映射的逆就是它自身)。
        let w = 4usize;
        let h = 2usize;
        let g: Vec<u16> = (0..w * h).map(|i| (i * 101 % 1024) as u16).collect();
        let b: Vec<u16> = (0..w * h).map(|i| (i * 197 % 1024) as u16).collect();
        let r: Vec<u16> = (0..w * h).map(|i| (i * 331 % 1024) as u16).collect();
        let a: Vec<u16> = (0..w * h).map(|i| (i * 409 % 1024) as u16).collect();
        let img = AvifImage::new(
            w as u32,
            h as u32,
            PixelFormat::Yuva444P10Le,
            vec![
                u16_plane(w, &g),
                u16_plane(w, &b),
                u16_plane(w, &r),
                u16_plane(w, &a),
            ],
        )
        .unwrap()
        .with_color(color(ColorRange::Full, 1, 13, 0));
        let file = oxideav_avif::encode(&img, &EncodeOptions::default()).unwrap();
        let decoded = oxideav_avif::decode(&file).unwrap();
        assert_eq!(decoded.bit_depth, 10);
        assert!(decoded.format.has_alpha());
        accept_color(&decoded.color).unwrap();
        let out = deep_rgb16(&decoded);
        for i in 0..w * h {
            let px = &out[i * 4..i * 4 + 4];
            // 满量程映射的两级舍入(round(n·65535/1023) 再无第二次
            // 舍入)对码值本身可差 1:断言 |v − n·65535/1023| ≤ 1。
            let want = |n: u16| -> i64 { (i64::from(n) * 65535 + 511) / 1023 };
            assert!(
                (i64::from(px[0]) - want(r[i])).abs() <= 1
                    && (i64::from(px[1]) - want(g[i])).abs() <= 1
                    && (i64::from(px[2]) - want(b[i])).abs() <= 1
                    && (i64::from(px[3]) - want(a[i])).abs() <= 1,
                "pixel {i}: {:?}",
                px
            );
        }
    }

    #[test]
    fn a_limited_range_file_decodes_with_its_signalled_chroma() {
        // 消费级 AVIF 多 limited 的补缺语料:手写 BT.709 limited 的
        // YUV 平面(still_of 原样编码 planar 平面 + colr,不经
        // RGB↔YUV 转换)——黑(16,128,128)/白(235,128,128)条
        //(4:2:0 的偶数宽高约束 → 2×2)。
        let planes = vec![
            u8_plane(2, &[16, 235, 16, 235]),
            u8_plane(1, &[128]),
            u8_plane(1, &[128]),
        ];
        let img = AvifImage::new(2, 2, PixelFormat::Yuv420P, planes)
            .unwrap()
            .with_color(color(ColorRange::Limited, 1, 13, 1));
        let file = oxideav_avif::encode(&img, &EncodeOptions::default()).unwrap();
        let decoded = oxideav_avif::decode(&file).unwrap();
        assert_eq!(decoded.color.range, ColorRange::Limited);
        assert_eq!(decoded.color.matrix, 1);
        let out = deep_rgb16(&decoded);
        assert_eq!(out[0], 0, "black bar");
        assert_eq!(out[3], 65535, "white bar");
    }
}
