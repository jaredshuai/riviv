# ADR 0004: AC-aware 宽色域输出面(L2)——F16 宽域中间格式 + scRGB 声明输出

- 日期: 2026-09-27(草案同日过审)
- 状态: 已接受并过线(用户 2026-09-27 过审裁毕开放项;P-A~F 探针电池同日执行:**P-A/B/C/E PASS;P-D 当前态断言过线 + D4 FINDING 记档修订——其两态对照(SDR-WCG vs HDR)与兼容助手两态转 QA 人工项,「bit1 sufficient」结论限当前态**;P-F 设计记档结案——实现票族照「影响面」清单开)
- 范围: master 宽域编码、Stage 1 目的地、决策表第三维(输出面)、交换链格式与色彩空间声明、失败回退、验收口径;**不含** HDR 峰值亮度输入面(PQ/gain map = L3/#138)、AVIF、HDR10 路线
- 上游权威: #136 处置(L2 = 独立 ADR + 用户拍板门;type 15 是否转消费在此定)、ADR 0003(D4 域半留档 + D2 升级路径三断点)、#127 处置表 D1–D10(M8 核心表,本 ADR 重设计其前提)、#134(ac 位钉测)、ADR 0002(渲染栈)

## 背景

L1(#140–#144)交付了 D10 结构性损失的**精度半**:f16 gamma-sRGB master 全链无降级。**域半仍在**:嵌入宽色域 ICC 的图在 Stage 1(ICC→sRGB,mscms u16 定点裁 [0,1])就被裁到 sRGB 域,后段无人可救——ACM 机器上 OS 代管(决策表 DwmAcm 行)也只是把 sRGB 内容正确地放上宽色域屏,**屏能显示的更艳颜色从未到达过它**。ADR 0003 探针②已证:mscms 全部浮点输出格式被拒(`BM_32b_scRGB` GLE=50;`R16G16B16A16_FLOAT` dst GLE=2021)——「目的地=线性 scRGB」的 CMM 通道**不存在**,这是 L1 选 gamma 臂时把域半留给 L2 的原因。

开发机现态(#134 P1 + ADR 0003 探针④亲测留档):type 9 flags=0x3(`advancedColorEnabled`=1)、10bpc;type 15 raw=0xF3、`activeColorMode=2(HDR)`、wideColor supported+enabled——**AC 输出臂的验证环境齐备**。

## 核心洞察:域半有两条交付腿,共享同一个地基

宽色域值要在 Stage 1 **活下来**,唯一现实路径是给 mscms 一个**宽域定点目的地**(u16 [0,1] 落在宽域容器内,超 sRGB 值不越界);活下来之后往哪儿送,分两条腿:

- **腿一(legacy 宽色域屏,非 ACM)**:现有 gpu_effect 显示段(#130)把 master 直映射到显示 profile——今天它收到的已经是被裁过的 sRGB,换成宽域 master 后**同一 pass 直接解裁**,无需 ACM。
- **腿二(ACM 机器)**:riviv 声明 AC-aware(交换链 FP16 + `SetColorSpace1` scRGB),输出线性 scRGB,面板映射交还 OS。

## 决策

### D1. 目标矩阵(三内容类 × 显示环境;普通内容零改动是硬约束)

| 内容类 | sRGB 屏(None 行) | legacy 宽屏(gpu_effect 行) | ACM 机器(DwmAcm 行→AC 臂) | WARP |
|---|---|---|---|---|
| 普通 8-bit(日常多数) | 现状逐字节不变 | 现状不变 | 现状不变(OS 代管,D2 姿势保持) | 现状不变 |
| F16Srgb(>8bit 直系/等效 ICC) | 现状不变(L1) | 现状不变(L1) | **现状不变**(sRGB 内容 OS 代管已正确) | 现状不变 |
| **F16P3(宽 ICC 嵌入,新)** | effect P3→sRGB(新恒通行) | **effect P3→display(域半腿一)** | **scRGB 输出臂(域半腿二)** | Stage 1 目的地退回 sRGB(见 D5) |

依据:AC 臂对 sRGB 内容零收益(OS 已做同一件事),只对「有超域内容可保」的图付切换代价——arm 判据天然 = L1 的宽 ICC gate(ADR 0003 D1 第一支),普通内容的全部现有行/字节合同/1:1 契约原样保持。

### D2. master 宽域编码:Display-P3-D65 容器,BM_16b_RGB 目的地

- Stage 1 目的地 profile 从系统 sRGB 换为**内嵌自建 P3-D65 matrix-shaper ICC v2**(自产字节,零许可负担、确定性、过 icm.rs 既有 RGB v2/v4 gate),输出仍 `BM_16b_RGB`(gamma 编码随目的地 profile TRC),CPU 转码 u16→f16 复用 #142 纯函数形态 → 新 `ContentSpace::F16P3`(transform_stage.rs:204 枚举扩值;四层缓存键/直读接缝照 #141 分派模式)。
- **容器选 P3 而非更宽(ProPhoto/AP1)的推荐理由**:现实显示端点(含 HDR 面板)全部 ≤ ~P3,超出 P3 的源色在任何现存屏上都显示不出来(OS/效果段会再映射掉);P3-D65 实 primaries、D65 白、sRGB 曲线,effect 侧验证简单;体积/预算与 F16Srgb 全同(8 B/px,#144 记账不变)。更宽容器 = 更少裁剪但买不到可见收益;**已裁(2026-09-27 用户):容器定 P3,更宽(ProPhoto 级)记为后续升级方向**——容器语义封在 ContentSpace 与 Stage 1 目的地两个单一落点之后,升级 = 独立小票(换容器 profile 常量 + 旧图重派生),不动输出面两腿。
- mscms 接受任意 ICC 目的地是现产机制(sRGB 目的地即此用法),但 **P3 目的地实测留探针 P-A**。

### D3. 输出面:AC 臂 = `R16G16B16A16_FLOAT` 交换链 + `SetColorSpace1(RGB_FULL_G10_NONE_P709)`

- 交换链格式落点 = `gpu.rs:421` `SWAPCHAIN_FORMAT`(单一常量,现钉测 gpu.rs:2499–2504 随之扩展为「legacy 臂仍恒 B8G8R8A8_UNORM」双臂断言);色彩空间 = DXGI linear scRGB(709 primaries, G1.0, FP16 扩展域),D2D render target 同步 FP16。
- 绘制链:F16P3 master → 合成(f16 位图,现状臂)→ ColorManagement effect(P3 ICC 源 context → scRGB 目的,相对比色,双端显式,BEST)→ FP16 target。SDR 亮度语义:scRGB 1.0 = OS SDR 参考白,宽色域照片亮度仍 SDR 档(**真 HDR 峰值属 L3 输入面,本 ADR 不许诺**)。
- `SetColorSpace1` 是**自家缓冲区解释声明**,不是系统显示设置写侧(#134「不调 Set*」纪律的边界:禁的是 `SetDisplayConfig`/`ColorProfileSetDisplayDefaultAssociation` 类系统状态;声明自己缓冲区 = 成为 AC-aware 应用的定义动作)。实现票须附 `dumpbin /imports` 证据(dxgi 预期已在导入表,SwapChain3 经 QI,零新增 DLL 待证)。
- **否决 HDR10 路线**(`R10G10B10A2` + P2020/PQ):PQ 定量编码对 SDR 源照片是纯损失,固定 P3-D65 容器无扩展域,PQ 编码 pass 纯增;只对已有 PQ 源(=L3)才有意义。

### D4. 决策表重设计:加第三维,ac 位升为臂判据(对 D3/#127 的显式修订)

- 表扩为 `desired_output(backend, query, ac, content_class) -> {surface, stage}` 纯函数,穷举钉测照 #127 形态;OutputIdentity 指纹加 surface 项(gen 语义:#132 通道原样承载,内容类切换铸新 gen 属预期行为,实现票写断言)。
- **ac 位(type 9 bit1)从「诊断标签」升为 AC 臂主判据之一**——修订 #127 D3,理由:D3 的语境是「是否叠 app 侧 sRGB→display 段」(那时 ACM 位确实两义:兼容助手 on-but-不代管);AC 臂问的是另一件事——「riviv 是否接管自己内容的输出解释」,bit1 正是它的定义信号,且 #134 已把它接进指纹/新鲜度(name-OR-ac 单梯),事件通道现成。sRGB 内容的行**仍以 WCS 查询为主判据**(D3 原文不动)。
- **type 15 裁定(本 ADR 定,#136 留问)**:维持「诊断储备,不转消费」;P-D 实证 type 9 bit1 ≡ type 15 bit1(active)同真,单一门 sufficient 的当前态证据成立。
- **P-D 实测记档(2026-09-27,修订 #127 D2 的一半)**:本机 ACM+HDR 态下 WCS getter **非空**返回 `TPLCD_8BAF_AdobeRGB.icm`(subtype 7)+ HDR 校准 profile(subtype 8,机器 2026-08-24 生成),5/5 稳定——「AC 态 getter 一律返空 = OS 代管」的等价式在本机形态**不成立**(面板厂商关联,#127 P2 时代已在)。后果:sRGB 内容行在本机走 GpuEffect(现产自 #130 起即如此,#144 冒烟 S1 同态),其面板端正确性(OS 是否同时代管 = dump 不可见的 DWM 段)挂 QA 人工项——**非 L2 引入的回归**;AC 臂门(bit1)不受此发现影响。兼容助手边界情形(ACM 位 on + getter 返合成 profile)同族处置:AC 声明优先,AC 臂照走。

### D5. WARP 与 Stage 1 目的地的后端耦合

WARP 无 effect(#127 硬互斥)→ F16P3 在 WARP 上无人可映射(P3 halves 当 sRGB 显示 = 假色,**不可接受**)→ **Stage 1 目的地按后端择型**:WARP 会话宽 ICC 图退回 sRGB 目的地(= 现产 F16Srgb 形态,域被裁但色正确)。代价:WARP 会话无域半,与 L1 的 WARP→None 精度残余同族,记 README 已知限制。后端在栈创建时已知(#126 ladder),Stage 1 时序上可得;后端中途迁移(hw→WARP 升降级)时宽 master 经既有重载通道重派生——**已裁:重解码**;机制 = #90 ladder latch 翻转时 F16P3 master 全部失效、当屏图重派生一次,成本 = 一次重解码 + 9.35 ms/1080p 帧转码(#137 探针③实测,罕见路径有界,见 D8 P-F)。

### D6. 失败阶梯(复用 D4 哲学:desired/effective 分离 + 单向棘轮)

- FP16 交换链创建失败 / `SwapChain3` QI 失败 / `SetColorSpace1` 失败:**绘制前**判明,本会话单向落 legacy 臂(面包屑,不 fatal——复用 gpu.rs `PREPARE_ESCALATION_FAILURES=3` 棘轮先例的会话级 latch 形态)。
- effect 失败(AC 臂上):不能落 None(P3 halves 当 scRGB = 假色)——latch 掉 surface 本身,回 legacy 臂 + 该帧宽 master 的 legacy 交付(effect P3→sRGB/display 仍是 effect,同 latch 风险;彻底失效 = 经 D5 的重派生通道回 F16Srgb)。整链保底形态 = 现产 L1 输出(裁域但处处色正确),**永不假色**。

### D7. 契约重述:legacy 臂不变,AC 臂新契约

- legacy 臂:1:1 五件套(ADR 0002 D6)、hw/warp dump 字节一致(#126 S1 域)、smoke 既有断言**全部原样**(回归零扰动是 D1 硬约束的验收面)。
- AC 臂:1:1 = **half 精确直通**(NEAREST 下 master halves 逐位到达交换链,oracle = 纯 Rust P3→scRGB 参考矩阵;阈值 P-B 已标定:分档 ≤4 f16 ULP(|v|≥0.02)+ ≤5e-4 绝对(|v|<0.02),单值式 ≤2e-3 线性;替代 #130 的 mscms CPU 参考——scRGB 无 CMM 通道,探针②)。dump 通道 FP16 readback(`CopyFromRenderTarget` 16F 面)为断言入口;屏采/WGC 不进 riviv.exe(#126 rescope 原裁定不变)。

### D8. 探针电池(承诺门,照 #136 V2 纪律:过线前不开实现票)

| # | 问题 | 判据 |
|---|---|---|
| P-A | mscms `BM_16b_RGB` 以自建 P3 ICC 为目的地:P3 外源色映射不越 [0,1]、sRGB 域内与现链一致 | 全档样本通过;失败 → D2 容器重议 |
| P-B | D2D `CreateColorContext(P3 ICC)` + effect P3→scRGB 于 FP16 target(hw;WARP 对照) | 可建可画;数值 vs 纯 Rust 参考容差标定(仿 #130 maxDiff 形态) |
| P-C | FP16 交换链创建 + `SetColorSpace1(scRGB)` + Present(开发机,ACM/HDR 开发态) | 成功且屏上内容正确(SDR 白不漂);`SwapChain3` QI 与 dxgi 导入表现状 |
| P-D | type 9 bit1 / type 15 在 SDR-WCG 与 HDR 两态、兼容助手两态的读数对照 | bit1 单一 sufficient;否则 D4 翻案 |
| P-E | 新增 pass 吞吐(AC 臂整链 paint;P3→sRGB 恒通行) | ≤8ms 阈(1080p,#127 P3 口径) |
| P-F | 后端迁移时宽 master 重派生路径(warp 降级/热迁移) | 机制可行、成本可记 |

全败回退边界(照 #136 预案):L2 转 Unscheduled 带触发,M8 以 L1 收尾。

**电池结果(2026-09-27,子代理串行执行、主会话逐针复核,留档 `%TEMP%\riviv-l2\`)**:

- **P-A PASS**:自建 P3-D65 ICC v2(TRC = sRGB 形 1024 项 curv 表;para 形态被 mscms 拒 GLE=2011)作 `BM_16b_RGB` 目的地——超 sRGB 源色落容器内部(承重样点距 65535 余量 ~2982 LSB;同色 sRGB 目的地钉边界=现产裁剪复现);P3→P3 恒等往返 ≥65523;**实现注意**:icm.rs fixture 的 P3 原色是未适配 D65 值,生产须存 Bradford-D50 适配值(与 Apple DisplayP3 已知值一致)。
- **P-B PASS**:两臂(hw/WARP)首试即成,规范形态无需 fallback;CreateColorContext 回读 6672 字节逐字节一致;数值 vs 纯 Rust 参考 max 8.213e-4 线性(≈1.71 f16 ULP),hw≡warp 逐位同;**超域值端到端无钳制**(>1.0 保留 4 通道、负值 7 通道、boundary-pinned=0)——scRGB 臂承重前提成立。
- **P-C PASS**:ACM/HDR 开发态 FP16 交换链 + `SetColorSpace1(scRGB)` 全链首试 rc=0;屏采双臂(scRGB 线性填值 vs legacy sRGB 编码填值)逐位同(255/188/118,两跑字节一致)——SDR 白零漂;实证两条隐含契约:D2D 对 FP16 target 直写线性值(无二次线性化)、DWM 映射用精确 sRGB OETF。**dumpbin:dxgi.dll 不在静态导入表**(QI/SetColorSpace1/Present 全 COM vtable,经 d3d11 传递加载)——零新增 DLL 导入,平台地板无扰动。
- **P-D(部分过线:D1/D2/D3/D5 PASS 于当前 ACM+HDR 态,D4 FINDING;判据的两态对照与兼容助手两态**未执行**,转 QA 人工项)**:type 9/15 全字段与 #134 基线逐位一致、5/5 稳定;bit1 两 API 同真(**当前态**单门 sufficient 实证;type 15 bit1 义为 active,与 type 9 enabled 同名不同义)。**FINDING:ACM+HDR 态 getter 非空**(TPLCD AdobeRGB @subtype 7 + HDR 校准 profile @subtype 8)——「getter 空 ⟺ OS 代管」在本机形态不成立,处置与影响面见 D4 记档;SDR-WCG/HDR 真切换 + 兼容助手两态 = QA 人工项(`%TEMP%\riviv-l2\pd\pd-report.md` 一键复跑配方)。
- **P-E PASS**:E4 整链热路径 @1080p median **3.998 ms** ≤8 ms 阈(三完整跑 2.218/4.000/3.998,≥2× 余量;sanity 断言证明计时打在真 effect 图上——AC p3-red 实测 +1.2246 与 P-B 参考跨针一致);E5 legacy 恒通行 0.007 ms(几乎免费);热上传臂与 #137 基线重合(+2%/−4%,环境可比);**计时口径注意:本机驱动 EndDraw 之后 Flush 返 D2DERR_WRONG_STATE,有效形态 = BeginDraw→Draw→Flush→EndDraw(实现票照此)**;4K 整链 36.5 ms(66 MB 主图上传主导,记录无门)。
- **P-F 设计记档结案(无新实测,成本数字引 #137 探针③)**:后端迁移走 #90 既有 ladder latch;翻转时 F16P3 master 失效、当屏图经既有重载通道重派生一次;成本 = 一次重解码 + 9.35 ms/1080p 帧转码,罕见路径(10s 内 3 次设备失败才触发)有界。

### D9. 验收口径(实现票级)

- 纯函数:决策表三维修订的穷举钉测(含「普通内容全行 = 现产枚举逐位」回归钉)、P3 转码 u16→f16、自建 P3 profile 字节的确定性断言(哈希钉)。
- 冒烟:既有矩阵**全量原跑**(ACM off 态 = 现产合同逐字节);AC 臂 = 开发机 live anchor(照 smoke144 S1 口径)FP16 dump + half-oracle 断言 + 面包屑(`surface=ac-srgb` 类)断言;S3(HDR off)类人工项照旧进 QA 清单。
- 四门禁;README Roadmap M8 L2 行 + Differences(AC-aware 声明、F16P3、WARP 残余、SDR 亮度边界)+ commit why。

### D10. 明确不做

HDR 峰值亮度(PQ/gain-map 输入 = L3/#138);HDR10(D3 已否决);系统显示状态写侧(仅 `SetColorSpace1` 自声明例外);AVIF;显示段用户闸(#127 再议维持);type 15 消费(D4 维持储备);更宽容器(P3 定案,留开放项翻案权)。

## 影响面(接缝逐名;≈4–5 票,分段可回退)

1. `transform_stage.rs:204` ContentSpace +F16P3;决策表/指纹/新鲜度(#132 通道)扩维;
2. Stage 1(icm.rs/loader.rs)目的地择型 + 自建 P3 ICC 常量 + u16→f16 转码臂;
3. `gpu.rs:421` SWAPCHAIN_FORMAT 双臂 + FP16 target + effect 源 context 扩 P3 + SetColorSpace1 + 棘轮(D6);
4. 直读接缝分派(status RGB/clipboard/GDI dump 照 #141 模式)+ dump FP16 readback;
5. 冒烟矩阵 + README/Differences + QA 清单(收口票,M8 收官 = L1+L2)。

## 已裁项(2026-09-27 用户)

1. 容器定 **P3**;更宽(ProPhoto 级)为后续升级方向(D2,单一落点、独立小票);
2. **type 15 维持诊断储备**(D4,照推荐);
3. 后端迁移重派生 = **重解码**(D5/P-F,照推荐:罕见路径正确性优先);
4. AC 臂内容类切换**接受 gen churn**(照推荐:#132 先例,身份转移本就是变更信号)。
