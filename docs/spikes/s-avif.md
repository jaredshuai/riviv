# S-AVIF:解码路线对比 spike(三路线带数字)

- 票:#97(M7-S0);重审:#215(2026-10-21,见下方「重审」节)
- 日期:2026-09-21;机器:Win11 build 26200.9457(重审探针机:Win11 26300,rustc 1.98.0)
- 全量证据报告(数据源,命令+原始输出+探针源码):
  `%TEMP%\riviv-s0-avif\B-report.md`;语料 `test.avif`(64×64 四象限,
  ravif 0.13.0 经 image 0.25.10 纯 Rust 编码,13 ms / 364 B,
  ftyp brand=`avif` 实录)在 `%TEMP%\riviv-s0-avif\` 下,三路线共用。
- 取舍原则(票面):单 exe、行为不随用户装机变、CI 三闸可复现 >
  解码速度 > 体积。

## 结论一句话

**推迟到 M8,与 HDR 同期**(追记 2026-09-26 #136:已解绑——HDR/WCG 输出不再依赖
AVIF;AVIF 输入面=移出 M8 的 L3 观察票 #138,详见下方「决策影响」第 1 条)。
**追记 2026-10-21 #215 重审:三条路线两死一生——imazen 系否决未解除(rav1d-safe
Cargo.toml 仍 AGPL 双授,GitHub BSD-2 字段是 fork 带入的上游 COPYING 探测噪声,
#215 触发证据第 1 条实为误报);oxideav 路线(MIT 纯 Rust 全链)本地实测①⑥通过、
④可接待实现票、⑤记录性偏离,唯体积 +1.59 MiB≈4.2× 基准线须独立 ADR 破例 +
用户拍板——证据与结论见文末「重审(2026-10-21,#215)」节。**
原三路线结论(2026-09-21)保留如下:
三条路线当前全部不过门槛:A(dav1d,C)
需 meson/nasm/pkg-config 外部工具链且构建期联网(违反依赖原则①,
本机链接失败、增量 N/A);B(系统 WIC)本机虽解码成功,但前提是用户
装了两个商店扩展(AV1VideoExtension+HEIFImageExtension),行为随装机
漂移(违反原则⑥,票面已点名 WIC AVIF/HEIF 不合格);C(纯 Rust)唯一
全链 zenavif 是 **AGPL-3.0-only/商业双授**(riviv=MIT,许可一票否决),
MIT 侧 oxideav 自认脚手架不成熟。AVIF 位深/gain map 与 HDR 语义耦合
(三方咨询共识),M8 色彩输出包解冻时一并重审。

## 证据块(对比总表,数字均 B-report 实测,N/A=不可得)

| 路线 | 关键事实(实测) | 体积代价 | CI 可复现 | 行为确定性 | 成熟度/许可 |
|---|---|---|---|---|---|
| **A. image `avif-native`**(dav1d,C) | 本机默认构建失败:pkg-config 缺失,dav1d-sys 0.8.3 **无 feature 面**,默认 pkg-config 找系统 dav1d≥1.3.0;源码编走 `SYSTEM_DEPS_DAV1D_BUILD_INTERNAL`(git clone dav1d 1.5.0 成功→死于无 meson;dav1d 需 nasm≥2.14;编完解析 .pc 仍要 pkg-config) | **N/A**(链不上) | 推导:GHA 需 `pip install meson`+`choco nasm pkgconfiglite`+构建期联网 clone;可做但依赖面最重 | 好(自带 C) | 最高:dav1d=业界标准,image 官方 feature,mp4parse demux 现成;MIT/BSD 系 |
| **B. 系统 WIC**(HEIF+AV1) | 本机实测解码成功(64×64 四角像素全对,32bppBGR);WIC 无独立 AVIF decoder,走「Microsoft HEIF Decoder + AV1 MFT」;装机前提:AV1VideoExtension 2.0.30 + HEIFImageExtension 1.2.48(本机已装;扩展缺失分支本机未复现) | ≈ +0(系统 COM) | 高(纯 API);但除非 CI 专门关掉扩展,测不到扩展缺失分支的行为 | **最弱**:随装机变 | 系统组件;免费但装机率非 100% |
| **C. 纯 Rust**(rav1d 系) | rav1d 1.1.0(memorysafety,BSD-2)仅裸 OBU 的 unsafe C 风格 API、无容器 demux;**全链 zenavif 0.1.6 实测解码成功**(5 ms,四角全对,零外部工具);oxideav-avif 0.0.11(MIT)自认「orphan-rebuild scaffold 不成熟」 | ≈ +3.67 MiB(route-c.exe 原值;riviv 实际增量会小) | **最高**(cargo build 零外部依赖零网络) | 好(代码自带) | **rav1d-safe+zenavif = AGPL-3.0-only OR 商业双授 → 对 MIT 的 riviv 一票否决**;oxideav 不可依赖 |

关键单点证据:
- A 失败原文:`dav1d-sys-0.8.3\build.rs:82 panicked: PkgConfig(...The pkg-config command could not be found)`;
  `SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=always` 后 `git clone ... 1.5.0` 成功 → `meson failed: program not found`。
- B 成功原文:`CreateDecoderFromFilename(test.avif): OK` / `GetSize: 64x64`
  / 四角 BGRA 与源四象限一致;`MFTEnumEx` input=AV1 定向查询 `count=1 → AV1VideoExtension`。
- C 成功原文:`DECODE_OK backend=zenavif(rav1d-safe) size=64x64 elapsed_ms=5`,四角 RGB 对;
  许可字段:zenavif 0.1.6(探针实测所用)与 rav1d-safe 系 = `AGPL-3.0-only OR LicenseRef-Imazen-Commercial`(crates.io API 元数据,后者取 0.6.0 最新版字段;探针实测链为 rav1d-safe 0.5.7)。

## 决策影响

1. **M7 不开 AVIF 产品票**(历史结论保留);**M8 重审已发生并裁定解绑**
   (2026-09-26,#136 三份外部裁定同向 + 处置评论 5846511549):HDR/WCG
   输出不再 riding AVIF——依赖方向修正(loader `into_rgba8()` 截断使
   「先 AVIF」的 10-bit 输入当天撞 D10 结构性损失;中间格式先行,
   #137 探针 + ADR 0003 落地 L1)。**AVIF 输入面移出 M8 = L3 观察票
   #138**,触发器:(a) 生态——出现 MIT/BSD 兼容纯 Rust 全链(imazen
   系改 MIT 或同等);(b) 用户拍板——C1(dav1d)工具链实测(届时先
   补本表 A 行 N/A 体积格 + dumpbin 导入表复查,S1 纪律);(c) 里程碑
   收尾复核兜底。**触发前勿开工**(与 #138 互指,两边同源)。
2. 依赖原则⑥的实证注脚:WIC 的 AVIF/HEIF 解码 = 商店扩展装配率
   问题,连「装机≈100%」都达不到,登记为**永不作为唯一路径**(探测
   登记/降级提示可以,正如三方咨询已裁定)。
3. 纯 Rust 生态观察留档:rav1d(memorysafety,BSD-2)本体许可干净但
   API 是裸 C 风格 unsafe + 无 demux,直接产品化成本 = 自建
   mp4parse 胶水 + YUV→RGB + CICP/ICC 色彩管理;若未来 imazen 系
   改 MIT 或出现 MIT 全链,重跑 route-c 探针即可(B-report §四)。
4. 语料生成配方留档:`image` crate `avif` feature(ravif/rav1e 纯
   Rust)13 ms 编 64×64——未来 AVIF 冒烟素材可直接照此自造(同
   smoke80 手写 GIF 纪律),无需外部工具。

## 降级路径

- M8 重审时若选 A:CI 侧配方已推导(meson+nasm+pkgconfiglite+联网
  clone),需先在本机装齐工具链实测「静态链进单 exe 的增量 KB」补上
  本表 N/A 格,并按 S1 规则 dumpbin 复查导入表(dav1d 若引 DLL 静态
  导入即违反 S1 纪律,须 ADR)。
- 若选 B 混合形态:WIC 探针代码照 B-report route-b 复用(全部 HRESULT
  实测口径),但仅作「有则用」加速路径,加载决策不得依赖它。
- 无论如何,AVIF 解码票开工前先读本表与 s1-platform-floor.md 的
  决策影响节。

## 重审(2026-10-21,#215)

- 触发:#138 触发器 (a) 疑似满足(#215 票面 2026-10-07 GitHub 元数据)+ (c)
  里程碑收尾兜底;本节为重审产出,纪律 = 重审 ≠ 开工。
- 全部生态事实 2026-10-21 当日亲查(GitHub API / crates.io API / 仓库文件);
  全部数字为本机实测(rustc 1.98.0,Win11 26300;探针工程 `%TEMP%\riviv-s215\`,
  语料与探针产物同目录,勿入库)。

### 触发证据复核(两条)

1. **「rav1d 许可变更」= 误报实锤。** #215 票面记载 memorysafety/rav1d license
   字段「现示 BSD-2-Clause → 许可面变化」——但 memorysafety/rav1d 自始即 BSD-2
   (上表 C 行 2026-09-21 已记载);真正的字段变化发生在 **imazen/rav1d-safe**,
   而那是 **GitHub 探测器读了 fork 时原样带入的上游 COPYING**(BSD-2,2026-02-04
   fork 提交即存在):其 Cargo.toml(0.7.0 prep 主干,2026-10-10 `release prep`
   提交)仍写 `license = "AGPL-3.0-only OR LicenseRef-Imazen-Commercial"`,
   LICENSE-AGPL3 / LICENSE-COMMERCIAL 双授文件仍在仓库且随包发行,CHANGELOG 无
   改授记录,crates.io 0.6.0 元数据(2026-09-08 发布)不变。**权威源 = Cargo.toml
   + 许可文件,imazen 系(rav1d-safe/zenavif)一票否决未解除**(zenavif 仓库
   license 字段实查仍 AGPL-3.0,2026-10-19 仍在推送)。
   memorysafety/rav1d 本体:BSD-2 不变,但 v1.1.0(2025-05-06)后 **17 个月无新
   release**,裸 C 风格 unsafe API + 无容器 demux 现状未变(main 分支活跃,
   2026-10-12 有 push)——单本体仍非候选,定性不变。
2. **oxideav 活跃化 = 真触发。** 分仓结构成形(OxideAV/oxideav-workspace +
   -core + -av1 + -heif + -avif,全 MIT,MSRV 统一 1.80),发版节奏
   avif 半年 15 版(0.0.4 2026-04-25 → 0.0.15 2026-10-17),av1 0.1.21 /
   heif 0.0.12 均本周在发。crates.io 上 oxideav-av1 的「orphan-rebuild scaffold」
   描述**已过期**:仓库自述 clean-room rebuild 已落地(r390 起独立一致性语料
   16/16 字节精确,逐版增长),workspace README codecs 表现值 AV1 解码
   「✅ 100%——155 流语料 + 114/114 真实静图字节精确;lacks u16 in-loop + SIMD
   kernels(纯标量 Rust)」、AVIF 行「✅ ~99%——8/10/12-bit 全布局、alpha、
   grid/overlay/iden,fuzzed」。

### 本地探针(三面全绿)

| 核查面 | 方法 | 结果 |
|---|---|---|
| ①纯 Rust 静态链零外部工具链 | 探针工程 `cargo build --release`(oxideav-avif 0.0.15 默认 feature) | **PASS**:全程零 nasm/meson/pkg-config;`cargo tree` oxideav 侧 = avif+av1+heif+core+bytemuck/serde_json/thiserror,**零 `*-sys`/cc/bindgen/cmake**;heif 以 `default-features = false` 被引入,**不拖 h264/h265/h266** |
| 跨编码器互操作 | image 0.25.10(ravif 0.13/rav1e 0.8.1)自造 64×64 四象限语料(339 B,ftyp brand=avif 实录,同 B-report 配方)→ oxideav `probe`/`info`/`decode` | **PASS**:`decode` <1 ms(zenavif 臂同语料 5 ms),四象限像素 l1=1(有损舍入级) |
| 10-bit 深度路径 | oxideav 自编码 10-bit 4:4:4+alpha 无损(q=0,623 B)→ 解码 | **PASS**:`info.bit_depth=10`、`has_alpha=true`;`decode` 得 `Yuva444P10Le` 四平面 16-bit LE,luma/alpha 样本**逐值精确往返**,`<1 ms` |
| 体积量级 | 纯解码臂 exe(仅链 oxideav,无 image——与语料生成臂 3,412,480 B 对比可证)− 空基线 | **+1,671,680 B ≈ +1.59 MiB**(1,795,584 − 123,904;release, cg16) |

- API 面顺带核实(读取已编译版本源码):image-crate 合同(probe/info/decode/
  decode_rgba8);`ImageInfo` 携带 `bit_depth`/`has_icc`/`frames`/`is_sequence`;
  深位深解码输出 = 原生 u16 平面(P10/P12 全布局 + alpha,`AvifImage` 公有字段);
  `avis` 序列有 `decode_all`。管线接线(④)无结构性障碍。

### 依赖原则逐项(oxideav 路线)

| 原则 | 判定 | 依据 |
|---|---|---|
| ① 纯 Rust+静态链+零外部工具链 | **过(实测)** | 上表探针 |
| ② 新依赖落纯逻辑层 | 过(设计) | 解码纯逻辑进 loader 管线,unsafe 壳不加码 |
| ③ 体积 ≤ +400 KB/格式 | **超限:+1.59 MiB ≈ 4.2× 基准线 → 须独立 ADR(编解码器类,阈值与理由单独论证)+ 用户拍板** | 上表探针;400 KB 阈值本就按「解码格式」校准(ADR 0005 D1),codec 超限不能援引渲染引擎先例——ADR 0005 D1 明言其破例「不回写放松原则③」;对照量级:resvg +3.38 MiB、zenavif 臂 +3.67 MiB,oxideav 为至今测得最便宜的全链 AVIF |
| ④ 同管线(魔数+icm+帧时序) | 可过(实现票落实) | 魔数=ftyp brand 探测;ICC=`has_icc`+`Metadata.icc`(API 已核实,本地未解带 ICC 样张=已知缺口,实现票 QA 项);帧时序=`avis`/`decode_all` 对齐现帧管线 |
| ⑤ 优先 image 同族 | **记录性偏离** | image 0.25 的同族解码 feature = `avif-native`(dav1d/C,即本表路线 A)已被①否决,纯 Rust 解码在 image 生态不存在;oxideav 为唯一过①的解码路线,且刻意自仿 image 合同 API(生态外、合同内) |
| ⑥ 装机≈100% 无授权费 | 过(构造性) | 代码随行、MIT,行为不随装机变(与 WIC 相反) |

### 风险与缓解

1. **API churn**:avif 0.0.x 无 semver 稳定承诺,半年 15 版 → riviv 侧锁精确
   版本,升级走独立票。
2. **单主作者**(OxideAV,Mark Karpeles)bus factor → MIT 可 fork,锁版本后
   冻结面可控。
3. **纯标量无 SIMD**:12 MP 静图自报 0.31 s 串行 / 0.097 s@8 workers;查看器
   单图加载场景可接受,`ExecutionContext` 线程预算实现票可后加。
4. **真实世界样张覆盖**:自报 114/114 字节精确;本地仅 rav1e 编码互操作 + 自
   往返,未持有第三方 10-bit 样张 → 实现票 QA 矩阵项。

### 重审结论

**观察票 #138 触发器 (a) 在 oxideav 分支成立**(imazen 分支不成立,见上)。
硬门逐面:①(纯 Rust/零工具链)实测过、⑥(装机无关)构造性过;④(同管线)
为设计判断 + API 亲核,**ICC 路径未本地实测,留实现票 QA 落实**;⑤为记录性
偏离(见上表理由);唯③超限须破例。**产出 = 建议开 AVIF 输入面实现票(oxideav
路线),前置条件 = 用户拍板体积破例(编解码器类独立 ADR,建议锚值 +1.59 MiB
实测)——拍板前不开工。**WIC 地位
不变(仅「有则用」加速,永不作加载决策);imazen 系与 C 工具链路线维持否决。
#138 可在实现票开立时关闭(其 (a) 触发经本重审成立)。
