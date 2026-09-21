# S-AVIF:解码路线对比 spike(三路线带数字)

- 票:#97(M7-S0)
- 日期:2026-09-21;机器:Win11 build 26200.9457
- 全量证据报告(数据源,命令+原始输出+探针源码):
  `%TEMP%\riviv-s0-avif\B-report.md`;语料 `test.avif`(64×64 四象限,
  ravif 0.13.0 经 image 0.25.10 纯 Rust 编码,13 ms / 364 B,
  fyp brand=`avif` 实录)在 `%TEMP%\riviv-s0-avif\` 下,三路线共用。
- 取舍原则(票面):单 exe、行为不随用户装机变、CI 三闸可复现 >
  解码速度 > 体积。

## 结论一句话

**推迟到 M8,与 HDR 同期**——三条路线当前全部不过门槛:A(dav1d,C)
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
| **B. 系统 WIC**(HEIF+AV1) | 本机实测解码成功(64×64 四角像素全对,32bppBGR);WIC 无独立 AVIF decoder,走「Microsoft HEIF Decoder + AV1 MFT」;装机前提:AV1VideoExtension 2.0.30 + HEIFImageExtension 1.2.48(本机已装;扩展缺失分支本机未复现) | ≈ +0(系统 COM) | 高(纯 API);但 CI 测不了「用户没装扩展」的行为 | **最弱**:随装机变 | 系统组件;免费但装机率非 100% |
| **C. 纯 Rust**(rav1d 系) | rav1d 1.1.0(memorysafety,BSD-2)仅裸 OBU 的 unsafe C 风格 API、无容器 demux;**全链 zenavif 0.1.6 实测解码成功**(5 ms,四角全对,零外部工具);oxideav-avif 0.0.11(MIT)自认「orphan-rebuild scaffold 不成熟」 | ≈ +3.67 MiB(route-c.exe 原值;riviv 实际增量会小) | **最高**(cargo build 零外部依赖零网络) | 好(代码自带) | **rav1d-safe+zenavif = AGPL-3.0-only OR 商业双授 → 对 MIT 的 riviv 一票否决**;oxideav 不可依赖 |

关键单点证据:
- A 失败原文:`dav1d-sys-0.8.3\build.rs:82 panicked: PkgConfig(...The pkg-config command could not be found)`;
  `SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=always` 后 `git clone ... 1.5.0` 成功 → `meson failed: program not found`。
- B 成功原文:`CreateDecoderFromFilename(test.avif): OK` / `GetSize: 64x64`
  / 四角 BGRA 与源四象限一致;`MFTEnumEx` input=AV1 定向查询 `count=1 → AV1VideoExtension`。
- C 成功原文:`DECODE_OK backend=zenavif(rav1d-safe) size=64x64 elapsed_ms=5`,四角 RGB 对;
  许可字段:zenavid/rav1d-safe = `AGPL-3.0-only OR LicenseRef-Imazen-Commercial`(crates.io API)。

## 决策影响

1. **M7 不开 AVIF 产品票**;重审点 = M8 色彩输出包启动时(HDR/10-bit
   输入与输出变换同一设计面,届时 A 路线的 C 工具链代价与 B 路线的
   「探测+引导装扩展」混合形态可一并重评)。
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
