# ADR 0008: AVIF 输入面路线 = oxideav,编解码器类体积破例

- 日期: 2026-10-22
- 状态: 已接受(用户拍板 2026-10-22「接受」+1.59 MiB;重审证据 = #215 / `docs/spikes/s-avif.md`「重审(2026-10-21,#215)」节;实现票 #220;观察票 #138 随本决策关闭)
- 范围: AVIF 输入面(still + `avis` 序列)的解码路线裁定与依赖原则③的编解码器类破例;**不含** HDR 输出/gain map 消费(#136 已解绑)。深度语义:10/12-bit 输入走现有 `ContentSpace::F16Srgb` 深源路径(见 D3)。
- 上游权威: s-avif.md(2026-09-21 三路线表 + 2026-10-21 重审节)、#136 票面 F7/F8 与处置评论、AGENTS「依赖原则」、s1-platform-floor(新增 DLL 静态导入须 dumpbin 证据并写 ADR)

## 背景

AVIF 输入面自 #136 解绑后挂起为 L3 观察票 #138,触发器 (a)=出现 MIT/BSD 兼容、能过依赖原则的纯 Rust 全链。#215 重审(证据全部 2026-10-21 当日亲查 + 本机探针)结论:

- **imazen 系否决未解除**:rav1d-safe 的 GitHub license 字段显 BSD-2 是探测器读了 fork 带入的上游 COPYING;权威源(Cargo.toml 0.7.0 prep + LICENSE-AGPL3/LICENSE-COMMERCIAL 随包发行 + crates.io 元数据)仍 `AGPL-3.0-only OR LicenseRef-Imazen-Commercial`,zenavif 仍 AGPL——对 MIT 的 riviv 一票否决维持。(#215 票面触发证据第 1 条「rav1d 许可变更」经复核为误报。)
- **memorysafety/rav1d 单本体非候选**:BSD-2 不变,但 v1.1.0(2025-05)后 17 个月无新 release,裸 C 风格 unsafe API + 无容器 demux 现状未变。
- **oxideav = 真触发成立**:全 MIT(avif 0.0.15 / av1 0.1.21 / heif 0.0.12 / core 0.1.40,MSRV 1.80),clean-room 语料自报 155 流 + 114/114 真实静图字节精确。本机探针三 PASS:纯 cargo 零外部工具链(依赖树零 `*-sys`/cc/bindgen,heif 以 default-features=false 引入不拖 h264/h265/h266)、跨编码器互操作(ravif/rav1e 语料,四象限 l1=1,<1 ms)、10-bit 4:4:4+alpha 无损往返逐值精确(Yuva444P10Le)。
- **唯一硬门不过 = 体积**:纯解码臂双 bin 差分 **+1,671,680 B ≈ +1.59 MiB ≈ 4.1× 依赖原则③的 400 KB 基准线**(400 KB 本就按「解码格式」校准,ADR 0005 D1)。

## 决策

### D1. 路线 = oxideav-avif(精确锁版本),其余路线维持否决

唯一直接依赖 `oxideav-avif = "=0.0.15"`(registry feature 默认开,带 pixel decode);传递 av1/heif/core 面以重审探针树为准。否决项维持:dav1d/C(违反①外部工具链)、WIC(装机漂移违反⑥,仅存「有则用」加速且永不作加载决策)、imazen 系(AGPL)。依赖原则⑤为**记录性偏离**:image 同族的解码 feature(`avif-native`)即 dav1d/C 路线已被①否决,纯 Rust 解码在 image 生态不存在;oxideav 自仿 image 合同 API(probe/info/decode),生态外、合同内。

### D2. 体积破例 = 编解码器类独立锚,不回写放松③

本破例锚值 **+1.59 MiB(探针口径:独立探针 exe 双 bin 差分)**,库内实际增量以实现票 #220 双臂实录为准并回填本 ADR(先例:resvg 探针 3.92 MiB → 库内 3.38 MiB)。**#220 回填(2026-10-22):库内实测 = 双臂 `cargo build --release` 字节差分 +1,632,768 B ≈ +1.56 MiB(arm A = master b976e9b = 6,897,152 B;arm B = #220 合并前代码态 = 8,529,920 B;worktree 干净基线)**——方向与先例一致(库内略小于探针锚)。传递面实录:avif 0.0.15 解析为 av1 0.1.21 / heif **0.0.13** / core **0.1.41**(重审时节记载 0.0.12/0.1.40,fetch 时 heif 已发 0.0.13 并带动 core 0.1.41;同一 MIT 家族,`cargo tree --all-features` 全树仍零 `*-sys`/cc/bindgen/h264/h265/h266)。与 ADR 0005 的关系:**不援引其先例**——0005 D1 的独立阈值以「渲染引擎非编解码器」为正当性来源,本 ADR 的正当性 = 用户对 AVIF 格式覆盖面的直接拍板(2026-10-22「接受」);400 KB 基准线对后续编解码器**原样有效**,下一个超限者仍须各自 ADR + 拍板。

### D3. 管线与深度语义

同管线接入不旁路:魔数嗅探(ftyp brand)进现有嗅探层、`Metadata.icc` 进现有 icm 链(带 ICC 样张为 #220 必测项)、`avis` 序列走现有帧时序契约。**深度语义与 PNG16 同例走深源路径**:10/12-bit 输入读 `AvifImage` 公有原生 u16 平面(P10/P12,16-bit LE),自建 YUV→RGB16 纯函数转换后进入现有 `ContentSpace::F16Srgb` master(#142/#140 已落,深度不截断;oxideav 打包输出仅 8-bit `to_rgba8`,深路径必须取原生平面);8-bit 量化只发生在既有文档化的深源回退点,不为本格式新增截断(初稿误按 #136 时代的 `into_rgba8` 截断语义落笔,cubic #221 P1 订正——现行 loader 的 `into_rgba8` 仅存于动画 8-bit 恒等臂与 radiance 线性光臂,均有注释钉死)。**深位深 `avis` 交叉格(10/12-bit 序列)= 与 PNG16 动画同例降级静图**(首帧走深源;现行动画管线 8-bit-only,逐帧深源属未来帧管线域,不为本票扩)(补偿评审 F1 钉死)。**YUV→RGB16 纯函数规格(cubic #222 两 P2 落成明文裁定,不再留待办)**:有效色度 = 文件 nclx,缺席时按 MIAF 默认(与所依赖 oxideav-avif 0.0.15 `ColorInfo::miaf_default` 对齐亲核 = Full range / primaries 1 / transfer 13 / matrix 6);**接受集** = matrix ∈ {0 identity 直通, 1 BT.709, 6 BT.601} × range ∈ {Full, Limited} × primaries ∈ {1, 2→按 1} × transfer ∈ {13, 2→按 13};**接受集外一律拒绝加载**(ADR 0001 用户级:保留旧图+状态栏 flash 理由——含 BT.2020 ncl、PQ/HLG transfer、宽 primaries 不猜色,与 #130 judge 的 never-a-guess 同哲学;README Differences 记录)。**Limited range 公式**(ITU 标准形,n=8+2 或 8+4;**#220 订正:Y 轴 offset 初稿误书 64·2^(n−8),n=8 代入即不成立,以所依赖 oxideav-heif 0.0.13 `rgb.rs` 源码实录为准 = 16·2^(n−8)**——8-bit 16 / 10-bit 64 / 12-bit 256):Y′=(Y−16·2^(n−8))/(219·2^(n−8)),C′=(C−2^(n−1))/(224·2^(n−8));Full:Y′=Y/(2ⁿ−1),C′=(C−2^(n−1))/(2ⁿ−1)(**#220 二次订正,采 cubic #223:Full 的 C 轴初稿误书 2C/(2ⁿ−1)−1——swing ±1 且 C=128 不落 0;源码实录为 mid 中心形,C=2^(n−1) → 0**);RGB=clip₀₋₁(矩阵核)·65535 进 RGB16 容器(矩阵核与色度 co-sited nearest 复制亦按该源码逐样本同构实现)。钉测 = 黑/白/彩条期望值由公式生成后写死断言,**加 oxideav `to_rgba8` 交叉 oracle**(同文件,自算 RGB16 降 8-bit vs 其打包输出,实测逐样本差 ≤1、均值差 <0.5)+ 亲核订正:黑/白钉值与 oxideav 侧自带测试同款(Y=16→0 / Y=235→白,10-bit 64/940)。错误路径遵循 ADR 0001 用户级(保留旧图、不退出、不弹框)。

### D4. S1 合规:导入面证据为 #220 回填项

oxideav 全纯 Rust,**预期零新 DLL 静态导入**;但按 s1-platform-floor 纪律,#220 仍须附 `dumpbin /imports` 双臂 diff 证据回填本 ADR——若出现新 DLL 即停,另走 ADR。**#220 回填(2026-10-22):双臂 `dumpbin -IMPORTS` 全量 DLL 列表逐行 diff = 零差异**(arm A/arm B 同 D2;每臂 26 个原始导入行,含 `KERNEL32.dll`/`kernel32.dll` 大小写双形,即 **25 个唯一 DLL**——与 ADR 0007 记载的同一导入面 25 一致;kernel32/user32/gdi32/…/winhttp.dll 全集一致,证据留档 PR)——预期达成,零新 DLL 静态导入,S1 地板不动。

### D5. 风险承接(全部 #220 验收项)

API churn(0.0.x 无 semver 承诺,半年 12 版)→ 精确锁 + 升级独立票;单主作者 bus factor → MIT 可 fork,锁版本冻结面可控;纯标量无 SIMD(12 MP 自报 0.31 s 串行)→ 查看器场景可接受,`ExecutionContext` 线程预算为后继可选项;真实样张覆盖(自报 114/114)→ #220 本地 QA 矩阵补验(ICC、第三方 10-bit、grid)。

## 后果

- exe 体积将增长约 1.6 MiB 量级(#220 实录 +1,632,768 B ≈ +1.56 MiB,见 D2;AGENTS 体积基线数字随发版更新)。
- #138 关闭(其触发器 (a) 经重审成立并已消费);AVIF 输入面从观察态转实施态(#220)。
- 本 ADR 不为任何后续编解码器(如 JXL/HEIC 解码)预设先例——各自 ADR + 拍板。
