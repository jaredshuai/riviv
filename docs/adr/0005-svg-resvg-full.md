# ADR 0005: SVG 解码臂——resvg full 档 size-exception 破例(依赖原则③)

- 日期: 2026-09-28
- 状态: 已接受(用户体积拍板 2026-09-27 已给;首个 PR 体积实录过保险丝,见 F4′)
- 范围: 依赖原则③的破例与阈值另立、resvg 0.48.1 full 档接入的边界决策(嗅探门/svgz 排除/光栅目标面/输入预算)、重栅配方留档;**不含**交互式重栅的 UI 接线(follow-up)
- 票: #152(施工图 = #150 处置评论 issuecomment-5854869778 方向 A + [s-svg](../spikes/s-svg.md)「决策影响」§3)

## 背景

上游 voidImageViewer 是纯 GDI+ 查看器,**零 SVG 能力**;SVG 整体是 beyond-original 增强。S0 spike(#97,[s-svg](../spikes/s-svg.md))裁定:保真路线唯一可行 = resvg full 档(D2D SVG 路 no-go:text/filter 双缺、随 OS build 漂移、1703+ 高于 1607 地板);但 full 档 +3.92 MiB 超出依赖原则③「单解码格式 ≤ +400 KB」近 10 倍,须 ADR 破例 + 用户拍板后才开工。2026-09-27 用户拍板接受 +3.92 MiB(#150 处置评论方向 A),#152 开票,本 ADR 落案。

## F4 体积数据(spike 实测 + 本 PR 实录)

| 项 | 数值 | 出处 |
|---|---|---|
| spike minimal 档(全关) | +1,391,616 B ≈ +1.33 MiB / 53 crates | s-svg.md A 表(scratch 探针差分,近似上界) |
| spike full 档(全默认) | +4,105,216 B ≈ +3.92 MiB / 94 crates | 同上 |
| S1 基线 | 3,099,648 B @ 559f1cc | AGENTS.md 依赖原则③ |
| 本 PR 前基线(master ff45203) | 3,227,648 B | **本会话亲验**(stash 后干净树 `cargo build --release` 实录,与 #157 会话交接数字一致) |
| **本 PR 后** | **6,770,688 B** | 本会话亲验(同 profile:`lto=thin`+`strip`) |
| **真实增量** | **+3,543,040 B ≈ 3.38 MiB** | 低于 spike 近似上界 3.92 MiB(scratch 差分未模拟跨 crate LTO 合并,方向一致) |

**F4′ 保险丝(#152 清单项 1)**:真实增量 3.38 MiB < 5 MiB 阈,**未触发**「ADR 附修订条款重新交用户拍板」分支;若后续维护性增长使 SVG 族总增量越过 5 MiB,按票面规则重新交拍板。**新基线参照 = 6,767,648 B**(已回写 AGENTS.md 依赖原则③基线行,作后续依赖决策参照)。

## 决策

### D1. 依赖原则③破例:完整渲染引擎与编解码器不同类,阈值单独定

依赖原则③的 ≤ +400 KB 阈值按「解码格式」校准——一个编解码器(png/gif/webp 同族)的合理预算。SVG 的 resvg 是**完整渲染引擎**(XML 解析 + CSS + 文本排版 + 字体库 + 滤镜 + 光栅化),能力面与编解码器不同量级;用编解码器的阈值去卡渲染引擎,等于宣布该能力类永远不可入——这不是阈值想回答的问题。破例成立,但**阈值单独定**(本 ADR 录案 + 用户拍板 = 阈值的正当性来源),不回写放松原则③本身:下一个渲染引擎类的依赖仍须各自 ADR + 拍板。

### D2. 依赖形态:resvg 0.48.1 单直接依赖,feature 面与 spike 实测档位逐一相同

- 唯一直接依赖 `resvg = "0.48.1"`(默认 feature = svgz/text/system-fonts/memmap-fonts/raster-images,与 spike「full 档」相同);`usvg`/`tiny_skia`/`fontdb` 一律经 `resvg::` 重导出取用,**不另列直接依赖**——防 feature 面漂移(spike 的 +3.92 MiB 即该 feature 组合的实测)。
- 许可:resvg/usvg = Apache-2.0 OR MIT,tiny-skia = BSD-3-Clause(s-svg.md A 表 registry 实录)——与 riviv MIT 兼容。
- 纯 Rust、零外部工具链、零新 DLL 静态导入(依赖原则①⑥保持;PE 导入面不变)。
- 落位纯逻辑层(`src/svg.rs` 无 Win32、无 unsafe;依赖原则②),解码走共享帧管线不旁路(依赖原则④:魔数嗅探门、帧时序契约 = 单帧流同 DIB 臂;icm 链结构性缺席——SVG 无嵌入 profile,resvg 产物即 sRGB,与 BMP/ICO/DIB 同类)。

### D3. 嗅探门:文本形状闸,svgz 刻意排除

- **接入点**:image crate 的 `with_guessed_format` 不识 SVG,`svg::sniff` 在其前(文件路径与 `stdin:` 双臂同 gate)——同「内容优先于扩展名」语义,改名/无扩展名的 .svg 照样解码。
- **形状闸,非内容扫描**:剥 BOM/空白后只认「XML 序言(`<?xml`/`<!DOCTYPE`/`<!--`)+ 窗口内 `<svg` 根(带元素边界字节)」或「直接 `<svg` 根」;`<svgfoo`、大写 `<SVG>`、中段碰巧含 `<svg` 的二进制流都不命中。PNG 等二进制魔数永不误入。序言后 512 字节窗口内无根 = fail-closed(落到 image crate 的 undetermined-format 用户级失败)。
- **svgz 排除**:usvg 0.48 的 `decompress_svgz` 是无界 `read_to_end`(usvg-0.48.1/src/parser/mod.rs:180-189)——gzip 炸弹会膨胀到 OOM;嗅探不放行 gzip 魔数,`.svgz` 与本臂存在之前同形(undetermined-format 用户级失败)。**重开条件**:上游给解压加界(或自包 bounded reader)时复评,重跑 s-svg 攻击面探针。

### D4. 光栅目标面:请求时视口决策 + 三段规则(纯函数 `svg::raster_target`)

tiny-skia 无光栅面积上限,usvg 对自然尺寸 10^5 的 viewBox 照单全收(spike 攻击面:无 clamp 时 40 GB 零初始化分配、8 s 未完成被强杀)——**调用方必须 clamp**,落为纯函数:

1. **fit≥1 钉在自然尺寸**(规则的主轴):应用的 DEFAULT fit 从不过 100% 放大(README Differences),而光栅面就是显示语义里的「自然尺寸/100%」——放大到视口的光栅会被显示语义原样展示,一个 48×48 图标就撑满整窗,破坏与位图格式的同语义。放大方向的清晰度余量属重栅 follow-up(D6),不在加载期买。
2. **fit<1 按视口 fit + 中间档下限**:缩小方向等比 fit;自然尺寸长边 >1024 的至少保 1024 级(s-svg §D:1024 级 0.88 ms,4096 级 12.3 ms 逼近 16.7 ms 帧预算)——小窗口里的巨大 SVG 也拿到缩放余量;自然尺寸 ≤1024 的按自然尺寸(便宜且 100% 缩放就绪,显示面仍由 renderer 按 fit 缩,与位图同形)。
3. **帧字节预算上限**:`w*h*4` 压进 `MAX_TOTAL_FRAME_BYTES`(512 MiB)——超预算自然尺寸(如 8192²=1 GiB)等比收缩进预算,收缩欠账记为重栅级议题;逐轴 16384 硬顶在乘法前吸收病态自然尺寸(1e5 级 viewBox 实测在案)。

### D5. 输入预算与失败类别

- 单条 SVG 输入 ≤128 MiB(usvg 只限元素个数(1M 上限)与实体环检测(每实体引用 ≤255,roxmltree),不限输入体积——读满前先量长短,超限用户级直报)。实体炸弹(billion laughs)在解析器环检测层毫秒级拒绝(spike 攻击面实测),闸门不预判、照常放行给解析器。
- 一切失败用户级(ADR 0001 用户级类:保留旧图、窗口不退出、不弹框)。

### D6. 重栅配方:留档不接线(follow-up)

s-svg §3 配方 = **复用 `usvg::Tree`、仅尺寸变化时重光栅、交互缩放退 1024 中间档后台出全分辨率**。本 PR 交付判定核心纯函数 `svg::needs_reraster`(display 对 raster 任一轴上采样 >2× 即需重栅,2× 恰不触发防抖;`#[allow(dead_code)]` 显式登记「消费方待接线」状态)并以测试钉面;**UI 接线(窗口持有 Tree、后台重栅、无闪烁换面、保持 zoom/pan)为独立后续票**——现有 reload 通道(F5 语义:clear + re-open)会重置缩放/触发 last_cache 命中,不适合承载。加载后的缩放行为:与位图同形(缩放现有光栅面;fit≥1 时光栅=自然尺寸,放大即位图式插值)。

### D7. fontdb 与外链

- 系统字体:`usvg::Options::default()` 携带**空** fontdb(`<text>` 会静默渲染成空,spike sample2 实测)——显式 `load_system_fonts()`,`OnceLock` 进程内一次(首扫 35–132 ms,spike §D)。
- 外链:`resources_dir=None` 保持默认(spike 攻击面:外链不存在优雅跳过、天然隔离,加载侧最多一次 stat)。

## 后续票(follow-ups)

1. **交互式重栅接线**(D6):窗口侧 Tree 持有 + 后台重栅通道 + zoom/pan 保持——独立票,验收含 `needs_reraster` 的 UI 侧消费。
2. **svgz 重评**(D3):上游 bounded decompress 出现时复评。
3. 冒烟面扩展(若后续需要):SVG 已进 smoke 矩阵(5 样本 + 攻击面三件,#152 清单项 5)。
