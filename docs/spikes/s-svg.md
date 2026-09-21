# S-SVG:resvg vs D2D `ID2D1SvgDocument` 对照 spike

- 票:#97(M7-S0)
- 日期:2026-09-21;机器:Win11 build 26200.9457,d2d1.dll 10.0.26100.8875
- 全量证据报告(本文件的数据源,含全部命令与原始输出、探针源码全文):
  `%TEMP%\riviv-s0-svg\A1-report.md`(resvg 侧)、`%TEMP%\riviv-s0-svg\A2-report.md`
  (D2D 侧);样本集与全部 PNG 在 `%TEMP%\riviv-s0-svg\{samples,out-minimal,out-full,out-size,out-d2d}\`
- 背景:原版纯 GDI+ 零 SVG(#97 票面已核实),SVG 整体是 beyond-original;
  WIC 无 SVG 解码器(三方咨询一致),候选仅 resvg(纯 Rust)与 D2D SVG
  (系统引擎,SVG 1.1 子集)两条路。

## 结论一句话

**推迟**——保真路线唯一可行的是 **resvg full 档(+3.92 MiB,超依赖原则③
单格式 ≤+400KB 建议近 10 倍,须 ADR 破例并经用户拍板后才开产品票);
**D2D SVG 路线 no-go**(子集无 text、无 filter、无外链,能力面随 OS
build 漂移,且 API 挂在 `ID2D1DeviceContext5` = Win10 1703+,高于 S1
钉死的 1607 地板);两路线的攻防与计时结论留档,光栅化时机策略定为
**缓存解析树 + 仅在尺寸变化时重光栅 + 调用方 clamp 目标面**。

## 证据块

### A. 版本/依赖/编译/体积(resvg 侧,scratch 探针,同 riviv profile `lto=thin`+`strip`)

resvg 0.48.1(usvg 0.48.1 / tiny-skia 0.12.0 / roxmltree 0.21.1);
许可:**resvg/usvg = Apache-2.0 OR MIT,tiny-skia = BSD-3-Clause**
(registry Cargo.toml 实录,与 riviv MIT 兼容)。

| 档位 | feature 面 | crates | 冷编译 | exe delta(vs empty 130,048 B) |
|---|---|---|---|---|
| minimal(全关) | 无 text/system-fonts/raster-images/svgz;**filters 恒编译不可关** | 53 | 5.32 s | +1,391,616 B ≈ **+1.33 MiB** |
| full(全默认) | +text+system-fonts+raster-images | 94 | 17.95 s | +4,105,216 B ≈ **+3.92 MiB** |

riviv 基线 exe = 3,099,648 B(S1),full 档增量 ≈ 基线的 1.3 倍——
单 exe 轻量定位的实质代价。方法学:scratch 探针差分(未模拟跨 crate
LTO 合并),是近似上界。

### B. 同批样本双路线渲染分类(256px;分类由像素扫描+输出 PNG 双证,
主会话另用 System.Drawing 独立复核 nonWhite 计数一致)

| 样本 | resvg full | D2D SVG | 说明 |
|---|---|---|---|
| sample1-icon(纯矢量图标) | ✅ 正确 | ✅ 正确 | D2D 需手动 world transform 才 fit(见 D) |
| sample2-text(`<text>`) | ✅ 正确(nonWhite=158;**必须显式 `load_system_fonts()`**,默认空 fontdb) | ❌ **失败**(全白,nonWhite=0;子集无 text,元素静默丢弃) | 文字是真实 SVG 高频要素 |
| sample3-filter(feGaussianBlur) | ✅ 模糊生效(非白 1638,扩散带 30+px) | ⚠️ **降级**(非白 162,形状在但零模糊 1px 硬边) | D2D 忽略 filter 属性 |
| sample4-extref(本地外链 png) | 默认失败;显式 `resources_dir` 后 ✅ 棋盘 | ❌ 失败(IStream 解析,无资源目录概念,不可恢复) | resvg 可选恢复,D2D 无机制 |
| sample5-hugeviewbox(10^5 viewBox) | ✅ 正确 | ✅ 正确 | 双方缩放几何正确 |

### C. 攻击面(billion laughs / 外链不存在 / 自然尺寸 10^5×10^5)

| 攻击 | resvg | D2D |
|---|---|---|
| DTD 实体炸弹(10^10) | **毫秒级解析拒绝**(roxmltree `EntityLoopDetector` 每实体引用 ≤255,`roxmltree-0.21.1/src/parse.rs:470-486`;usvg 另有 1M 元素上限)。注意 usvg 显式 `allow_dtd: true`,靠环检测兜底 | **亚秒 `E_INVALIDARG` 拒绝**(不接受该类 DTD;错误信息少) |
| 外链不存在 | 优雅跳过 exit 0,默认 `resources_dir=None` 时最多一次 stat(usvg 源码 `parser/image.rs:85-110`),**天然外链隔离** | 优雅跳过 exit 0,IStream 形态无资源解析 |
| 自然尺寸巨图 | **无静态面积上限**:进入 40 GB 零初始化分配,8s 窗口未完成被超时强杀(TIMEOUT exit 3)→ **调用方必须 clamp 光栅目标面** | **WIC `CreateBitmap` 亚秒快速失败** `0x80070216`(算术溢出守卫),忘了 clamp 也不挂死 |

### D. 光栅化计时(µs,Instant 实测;「重光栅策略」的证据)

resvg(parse≈建树,render≈光栅):parse 恒定 0.3–0.5 ms;render
256px=0.22 ms / 1024=0.88 ms / **4096=12.3 ms(逼近 16.7 ms 帧预算)**;
像素量 ×256 时 render 仅 ×56(矢量覆盖率主导)。
D2D:`CreateSvgDocument` 恒定 ~0.5–0.7 ms,draw 0.4–1.9 ms @256,
1024 下仍 1.5 ms 且边缘 1px AA(矢量保留、按目标分辨率重栅格)。
**D2D 视口陷阱(A2 新发现)**:根元素带 width/height 时,
`CreateSvgDocument` viewport 参数与 `SetViewportSize` 都不缩放内容,
必须调用方设 `Matrix3x2::scale` world transform(对照实验五组证实)。

### E. 本机 D2D SVG 支持判定

WIC RT QI `ID2D1DeviceContext5` 成功、`CreateSvgDocument` 成功
(Win11 26200);API 下限 = Win10 1703(S1 地板 1607 → 若任何场景
采用须 QI 探测降级,行为还随用户 OS build 漂移)。

## 决策影响

1. **SVG 产品票暂不开**(推迟)。开票前置条件:用户接受 full 档体积
   代价 → 写 ADR(依赖原则③破例:SVG 是完整渲染引擎,与编解码器
   不同类,预算阈值单独定)+ 明确 beyond-original 的 Differences 条目。
2. **D2D SVG 否决永久在案**:text/filter 双缺使「正确渲染」不可达,
   且属系统子集(随 OS 版本变化)与 1703+ 能力门,违反行为确定性。
   其视口陷阱与快速失败特性记录于此,防止未来误重启该路线;
   `Matrix3x2::scale` world-transform 配方若未来他用已验证可行。
3. **resvg 集成要点留档**(供未来产品票直接引用,均 A1/A2 实测):
   - `usvg::Options::default()` 携带**空 fontdb**,必须显式
     `fontdb_mut().load_system_fonts()`(首扫 35–132 ms,进程内一次);
   - `resources_dir` 默认 `None` = 天然外链隔离(推荐保持,安全默认);
   - tiny-skia 无光栅面积上限,**目标面必须 clamp**(建议按视口/窗口
     客户区,D2D 侧同场景的 0x80070216 快速失败不可依赖);
   - 复用 `usvg::Tree`、仅尺寸变化时重光栅;4096 级重光栅 12 ms,
     交互缩放时可退 1024 级中间档再后台出全分辨率。
4. #98(APNG)不受本结论影响;AVIF 见 s-avif.md(许可问题更硬)。

## 降级路径

- 用户若近期要 SVG 且不接受 +3.92 MiB:唯一中间档是 minimal
  (+1.33 MiB,无 text 无光栅图)——文字缺失与 D2D 同病,不推荐。
- 等待生态:MIT/Apache 轻量 SVG 引擎出现、或 resvg 体积显著下降时
  复评;届时重跑 A1/A2 探针(源码全文在两份报告附录,%TEMP% 失效后
  按 A1-report §8 / A2-report §9 重建即可)。
- 若 #98 APNG 后解码面仍有余量且用户拍板,SVG 是 M7 内第一个候选
  复评点;否则顺延 M8 与 HDR 解冻同期再审。
