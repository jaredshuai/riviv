# ADR 0003: FP16 中间格式 master(L1)——精度链升级,输出面不动

- 日期: 2026-09-26
- 状态: 已接受(#137 四探针过线;定向权威 = #136 处置评论 issuecomment-5846511549 的 A′ 分层)
- 范围: master 中间格式、Stage 1 ICM 输出链、四层缓存字节量与键、CPU/GPU 吞吐预算;**不含输出面**(交换链与决策表零改动)

## 背景

M8 前五票(#126/#127/#130/#132/#134)落地后,决策表 §D10 记录了 8-bit 时代的结构性限制:全管线像素钉死 sRGB 8-bit——宽色域源在 Stage 1 相对色度映射后、深位深源(PNG16)在 loader `into_rgba8()`(loader.rs:585/661)截断后,可用精度都只剩 8 bit,宽色域屏上的带化与量化不可避免。原相位序把这一限制的解法押在「HDR riding AVIF 10-bit input」上;#136 的三份外部裁定(V1/V2/V3)同向推翻——**依赖方向反了:中间格式必须先于一切 >8-bit 输入**。#137 的四探针(源码与原始输出留档 `%TEMP%\riviv-p137\`)验证了 L1 的可行性,本 ADR 据此定型 L1。

## 探针证据(#137,全部本会话亲跑,release,Win11 build 26200)

| # | 问题 | 结果 |
|---|---|---|
| ① P6 | sRGB 8-bit 码值经 f32→f16→f32 往返是否无损 | **无损**:256/256 码值全数回归;最大量化误差 2.432e-4(码值 239),仅半步长(1.961e-3)的 1/8。手写 IEEE half 换算(round-to-nearest-even、subnormal、Inf/NaN)过全部参考点——该换算即 L1 实现的候选纯函数 |
| ② ICM 浮点字格式 | `TranslateBitmapBits` 能否直接产出浮点 master | **不能**:icm.h 26100 转录的全部浮点/10bit/FP16 输出格式被拒——`BM_32b_scRGB/scARGB`(0x0601/02)src/dst 双侧 `GLE=50 NOT_SUPPORTED`;`S2DOT13FIXED_scRGB/scARGB`(0x0603/04)、`R10G10B10A2(_XR)`(0x0701/02)、`R16G16B16A16_FLOAT`(0x0703)`GLE=2021 ERROR_COLORSPACE_MISMATCH`;浮点 **src** → 8-bit dst 同样 50(格式族级拒绝,非 dst 侧限制)。**降级路径实证可用**:`BM_16b_RGB`(0x000A,16-bit gamma 定点、BGR 序)src/dst 全通(8-bit `BM_BGRTRIPLETS` 对照同场通过)——V3 风险 2 预案(CMM 高精度定点出 + CPU 纯函数化 f16 换算)转正 |
| ③ FP16 内存·吞吐 | FP16 master 的四层缓存内存与上传/合成开销(1080p,n=15,#127 P3 的 8 ms 阈) | **GPU 侧全绿**:WARP 冷上传 FP16 4.41 ms(BGRA8 2.00)/ 热上传 CopyFromMemory 0.44 ms(0.20)/ 合成 FP16 源→BGRA8 目标 0.04 ms(0.00)——显示路径全部远低阈。**CPU 侧(decode 一次性,非 paint 路径)**:16b→f16 转码(gamma 保持)med 9.35 ms;+sRGB 线性化(朴素 powf)44.76 ms(LUT 可降,但线性臂另有 D2 的否决理由)。**内存**:四层缓存(master/LevelCache/上传键/tile)每层 ×2——1080p 帧 8.30→16.59 MB,4K 33.18→66.36 MB |
| ④ type 15 复跑 | `GET_ADVANCED_COLOR_INFO_2` 读取通道现态 | 与 #134 基线**逐位一致**:size=36 首试 rc=0,raw=0xF3(supported/active/hdrSupported/hdrUserEnabled/wideColorSupported/wideColorUserEnabled 全 1),10 bpc,RGB,`activeColorMode=2(HDR)`;type 9 对照 flags=0x3——L2 的前置读取通道稳定可用 |

## 决策

### D1. master 择型:按输入择型(清单项 1)

FP16 master **只对「有可保内容」的图启用**,gate = 以下任一:

- Stage 1 实际施加了变换(嵌入 ICC 非 sRGB 等价——16-bit CMM 输出有精度可保);
- 源位深 >8 bit(PNG16 等直系——`into_rgba8()` 的截断有精度可保)。

普通未标签 8-bit 图(日常多数)保持现 BGRA8 路径**零改动**——它们从 FP16 得不到任何收益,不该付 2× 内存(四层缓存每层翻倍)与每帧 9.35 ms 的 decode 转码。双格式并存靠四层缓存键携带 content-space 标记隔离(#127 `ContentSpace` 枚举从恒 `Srgb` 扩出 `F16Srgb` 值);status RGB 读数、clipboard CF_BITMAP、GDI dump 等直读 master 的接缝按标记分派读法。

### D2. 编码择型:gamma-sRGB-f16(清单项 2)

master 编码 = **sRGB EOTF 编码值的 f16 表示**(非线性光,gamma 域),不选线性 scRGB。四条实测依据:

1. **mscms 只能出 gamma**:`BM_16b_RGB` 输出实测 gamma 编码(gray≈0.50);浮点族(唯一线性 scRGB 直达通道)全拒(探针②)。线性臂的 CMM 侧根本不存在。
2. **CPU 成本**:gamma 保持转码 9.35 ms/帧(1080p);朴素线性化 44.76 ms(每 px 3 次 `powf(2.4)`)——LUT 可降但多一环、多 256 KB 常驻表,为 L1 换不来可见收益(见第 4 条)。
3. **输出面零改动的兑现**:f16 gamma 值 → `B8G8R8A8` gamma 值是**同编码直通**——DrawBitmap 采样时自然量化回 8-bit(探针③合成臂 0.04 ms),NoProfile/None/WARP 全部行无需任何 encode pass。线性 scRGB 则要求所有输出行先做 linear→sRGB 编码,打破「决策表零加行」。
4. **线性 scRGB 的真收益是超域(sRGB 外)值保留**,而超域保留需要「目的 = scRGB/宽域」的变换目的——mscms 无浮点输出、16-bit 定点无符号又裁在 [0,1](探针②旁证),**在 L1 的 CMM 链下不可达**;超域显示属输出面议题,归 L2(AC-aware 输出,独立 ADR + 用户拍板门)。L1 若选线性臂 = 付全部成本(编码 pass + LUT + 更高转码)只买精度,不买域——那不如 gamma 臂同精度零成本。

升级路径记录(不承诺):L2 若过门引入线性 scRGB/HDR 输出,编码升级的断裂点 = 四层缓存 content-space 标记值、Stage 1 CPU 转码函数、effect 输入 context——三者都有单一落点,可整层换。

### D3. 输出面不动(清单项 3,D2 保持声明)

交换链恒 `B8G8R8A8_UNORM`、永不 `_SRGB`、不调 `SetColorSpace1`——#127 决策表零加行,`#136` 处置 L1 字面兑现。FP16 master 的合成输入用 `R16G16B16A16_FLOAT` bitmap(探针③实测 WARP 可建可画),输出量化由采样/合成自然完成。

### D4. D10 解消范围(清单项 4)

- **解(本层交付)**:精度半——带化/量化。16-bit 链全程:Stage 1 出 16-bit(CMM)→ f16 master → `gpu_effect` 行的 f16 输入(`ColorManagement` effect 内部以高精度做 sRGB→display 变换,输入精度从 8 bit 升至 f16)。legacy 宽色域屏(sRGB 域内显示)上的带化消失——**这就是 D10「由显示段救」的机理**。
- **不解(记为 L2 议题)**:域半——超 sRGB 色域值的保留与显示(真宽色域呈现)需要 AC-aware/线性 scRGB 输出面,#136 处置 L2 独立 ADR + 用户拍板门。
- **已知限制**:`WARP→None` 行不解裁(gpu_effect 硬互斥 WARP;cpu 格已被 #127 P3 吞吐探针撤,不可达)——WARP 会话的 FP16 master 量化回 8-bit 输出,与现产同形。

### D5. 回 8-bit 的落点与回退边界(清单项 5,V3 风险 2)

- **WARP/None 行的量化回 8-bit** = 合成本身(DrawBitmap 到 `B8G8R8A8` 目标的采样量化,0.04 ms,无额外 pass,无额外代码路径——探针③实证)。
- **CPU 转码失败回退**:mscms `BM_16b_RGB` 输出失败或转码异常 → 该帧降级走现产 8-bit 路径(ADR 0001 用户级失败形态:旧图保留,面包屑,不 fatal)。
- **全败回退**:探针电池若曾全败,相位六转 Unscheduled(案 B,M8 以前五票收尾)——本 ADR 存在即该分支未触发。

### D6. Stage 1 新链与实现票接口

变换路径(嵌入 ICC 图):mscms `TranslateBitmapBits` 目的格式改 `BM_16b_RGB`(BGR u16×3,gamma)→ CPU 纯函数转码(BGR→RGB 序转 + u16/65535→f32→f16 + alpha 恢复为 1.0)→ f16 master。直系路径(无 ICC 的 >8-bit 源):decoder 16-bit → f32 → f16 直转,不经 mscms。

实现票族(过线后照 #136 处置分派,3–6 票,每票独立可回退,纯函数先行):

1. `loader.rs:585/661` `into_rgba8()` 截断移除 + f16 换算纯函数(探针①的手写换算为候选)+ master 择型 gate;
2. 四层缓存 content-space 键 + 直读接缝分派(status RGB/clipboard/dump);
3. Stage 1 `BM_16b_RGB` 链 + CPU 转码;
4. GPU 上传键 `R16G16B16A16_FLOAT` 臂 + gpu_effect f16 输入;
5. 预算重算(512 MiB 帧预算下 f16 帧的 4096 帧与字节两腿重推导)+ 冒烟矩阵(ACM 三态 × HDR on/off,P3 解裁断言更新)。

## 后果

- 四层缓存内存对 FP16 图 ×2(1080p 16.59 MB/层持有;LevelCache 128 MiB 与 tile 预算按层各自吃满更早、逐出更频——可接受,gate 保证只有受益图进入)。
- decode 路径每帧 +9.35 ms(1080p)一次性成本(现产 mscms 8-bit 变换本身同量级,#127 P3 实测 9.61 ms);paint 路径净增 ≈ +0.24 ms(热上传差)——交互不受扰。
- GDI dump(CF_BITMAP/CF_DIB)从 f16 master 量化回 8-bit 输出,与现产字节同形(接缝分派的一支)。
- 本 ADR 不引入新依赖(纯函数换算,零 crate);不触碰 min-OS 地板(无新导入)。
