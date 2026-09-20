# ADR 0002: 渲染栈现代化 —— D2D 视口 + WCS 色彩管理

- 日期: 2026-09-17
- 状态: 已接受(M6 规划票,#76–#82)
- 范围: 渲染管线(paint/surface/mip/stitch)、色彩管理、DPI 立场、验收基础设施

## 背景

M1–M5 全部 landed(上游功能面 + 愿望单 pass 完毕)后,用户定向 M6 = D2D/WCS 渲染栈——README「Unscheduled candidates」中标注「needs an ADR」的 #50 重开触发器。决策输入 = 两份独立的外部旗舰 AI 架构咨询(各带源码核对),关键断言经本会话逐条对**本地权威源**核验(windows-rs 0.62.2 crate 源码、本仓库源码、MS Learn 官方文档原文)。两份咨询的高置信一致点构成本 ADR 的决策基线;分歧点已裁决并记录。

## 决策

### D0. 动机定调(写错动机会做错每个取舍)

**D2D 的决定性理由是交互式高分辨率实时重采样吞吐(4K+ 图上丝平滑移缩放)与未来能力空间(HDR/效果链),不是「缩放质量」也不是「色彩管理」**——后两者各有更便宜的解:色彩管理与渲染栈无关(D1),纯 Rust CPU 重采样器也能显著超过 HALFTONE。纯 CPU 重采样路线已评估并拒绝(交互吞吐是产品级诉求;且 D2D 路线同时解锁 Stage 2 GPU 变换与效果链)。

### D1. ICM 与 D2D 解耦——#50 重开条件的前提修正

#50 wontfix 记录的重开条件「渲染栈离开裸 GDI」**前提错误**:`CreateMultiProfileTransform`/`TranslateBitmapBits` 是纯 CPU 管线(mscms),与渲染栈无关;windows-rs 0.62 `Win32_UI_ColorSystem` 全量 134 函数在绑,零第三方依赖。**lcms2 否决**(C 依赖,违反 cargo-only 约束,收益边际);D2D ColorManagement effect 后置到 Stage 2。因此 M6 拆两条互不阻塞的支线:**M6a(纯 CPU,#76/#77)先行,不等 D2D**。

### D2. 色彩管理分层:Stage 1 → sRGB(现在),Stage 2 → 显示器 profile(M7)

- **Stage 1(#77)**:嵌入式 ICC → sRGB,解码期 worker 一次施加(与上游 `GdipLoadImageFromStreamICM`「加载期施加」同构)。sRGB 是**设备无关的稳定中间点**——跨屏/profile 更换/icm 开关不需要重算,也不需要双份 master 驻留。
- **Stage 2(M7 候选)**:sRGB → 显示器 profile(每显示器查询 + 热更新 + ACM/HDR 检测 + GPU ColorManagement effect)。显示器空间变换才涉及 `OutputColorState` 与 generation 化失效。
- 变换顺序钉死:**decode(RGBA) → ICM → composite over bg → swizzle BGRA**。窗口背景色是 sRGB,与变换后图像同空间;不得把背景一起变换。无 profile / profile≡sRGB → 快速路径零成本跳过(identity 零差异是 L0 前提)。字节序:top-down BGRA `[B,G,R,x]` = **`BM_xRGBQUADS`**(官方 BMFORMAT 表:`BM_xBGRQUADS` 是 `[R,G,B,x]`——最易写反处,色块测试钉死)。支持合同:RGB ICC v2/v4;非 RGB profile(CMYK JPEG 等 decoder 已转色者)显式降级不误用。不调用任何写系统显示设置的 API(不 `ColorProfileSetDisplayDefaultAssociation`、不动 gamma);Stage 2 的显示器 profile 获取注意 `ColorProfileGetDisplayDefault` 最低 build **20348**(官方 Requirements,咨询二的 1809 说法错误)——届时动态解析 + `WcsGetDefaultColorProfile`/`GetICMProfileW` 回退。

### D3. CPU 帧真源(#76):`PixelFrame` 是 source of truth

worker 产出 `Box<[u8]>`(top-down BGRA,alpha 恒 255——pixels.rs:47 与 dib.rs 三路径已钉死的不变量);GDI 面(DIB/DC)降级为 UI 线程按需派生。它同时是:三 GDI 接缝(RGB 读出/Copy Image/旋转)的直读源(全部转纯函数或临时面,删 `GetPixel`/`GetDIBits`/`SetDIBits`/`unsafe impl Send`)、设备丢失恢复的免重解码前提、D2D 帧上传源。512 MiB 解码预算(loader.rs:37)语义不变。

### D4. 集成形态:视口专属子 HWND + DXGI flip(#78/#80)

DXGI flip 的 GDI 互操作禁令是 **per-HWND**(官方原文 "Use flip model in an HWND that is not also targeted by … GDI"),故:

- `riviv_view` 子 HWND 独占视口像素,`CreateSwapChainForHwnd` 挂它(`B8G8R8A8_UNORM`(禁 `_SRGB`)/BufferCount=2/`FLIP_DISCARD`/`ALPHA_MODE_IGNORE`/`SCALING_NONE`);chrome 三子窗口保持 GDI。
- 排除:DirectComposition+`WS_NOREDIRECTIONBITMAP`(顶层无重定向表面则 GDI 子窗口不显示=重写 chrome);顶层 flip(撞 chrome);`ID2D1HwndRenderTarget`(内部 blt-model、能力面窄);`ID2D1DCRenderTarget`(EndDraw 搬运 GDI 面,静态图可作应急兜底,动画/拖拽掉帧——两份咨询性能判断一致,数字待运行时)。子 HWND 的代价(输入路由重构:拖拽/滚轮/双击/mscroll/X 键/光标/拖放)是 M6 最大单票工作量,由 #78 独立承担并零像素差异验收。
- 窗口树不变式:父 `WS_CLIPCHILDREN`,子窗口 resize 晚于 chrome 高度变化,父窗口不再向视口区做任何 GDI 绘制。

### D5. 设备策略:WARP 是永久兜底,GDI 只是过渡(#80→#82)

`renderer = auto | d2d | warp | gdi`(默认 gdi,M6 末翻 auto);auto = 硬件 → WARP → (过渡期 GDI / 删除后 fatal)。WARP 是同一代码路径的枚举值,测试矩阵成本≈0;状态栏显示实际后端消灭「不可复现」类工单;不做驱动黑名单。失败三层(ADR 0001 的扩展):初始化失败=环境→温和降级;运行期 `D2DERR_RECREATE_TARGET`/`DEVICE_REMOVED`→从 master 重上传(不重解码);10s 内 3 次运行期失败→WARP;WARP 也败才 fatal。paint 路径内一律 degrade-not-fatal。**GDI 删除判据写死**(#82):auto 全环境初始化成功 + golden 冻结 + 一个稳定发布周期;不设无判据的「再保留一里程碑」。

### D6. 插值映射与 1:1 契约(#81)

两臂各 2 档是用户可见 ini 面(`shrink_blit_mode`/`mag_filter`,config.rs:52-53):1:1=NEAREST+整数矩形;shrink 0→NEAREST、1(HALFTONE 默认)→`HIGH_QUALITY_CUBIC`(不一致且更好,记 Differences);mag 0(COLORONCOLOR 默认)→NEAREST、1(HALFTONE)→LINEAR。`ID2D1DeviceContext::DrawBitmap` 收完整 `D2D1_INTERPOLATION_MODE` 六档(windows-rs 签名已核;咨询二「需 spike 验证是否要走 DrawImage」的说法过虑)。**1:1 五件套**(缺一即破):`SetUnitMode(PIXELS)` + identity 变换审计 + 整数矩形 + 位图/target/swapchain 三处 `B8G8R8A8_UNORM`(用 `_SRGB` 会线性化出 ±1 差) + NEAREST(+ALIASED/COPY)。1:1 的空间零重采样在**任何** icm 设置下都成立;RGB 字节不变仅颜色管线 identity 时成立——这是两条独立断言。`SetBrushOrgEx` parity 是**删除**不是迁移(D2D 无 dither)。

### D7. mip 与巨图(#81/#82)

常规尺寸(≤ `GetMaximumBitmapSize()` 运行时查询,勿硬编码 16384)单张位图直绘,mip 链退役;`mip.rs` 纯函数与 counterexample 测试**保留**(巨图分级数学继续服役)。**放弃 mip 选择 quirk 的 parity**(mip.rs 在案的 `render_h` vs `mip_wide` 比较怪癖):D2D 下级别选择不再是可观测行为,复刻 quirk=故意输出更差画面——记 Differences。深缩质量后备:`HIGH_QUALITY_CUBIC` 自带 prefilter,不足则 ANISOTROPIC / Scale 效果。巨图(> max,D2D 上限典型 16384 **小于** GDI 32768→巨图覆盖面变大):overview 位图 + LRU tile 缓存(GPU 驻留 O(视口)),tile 带 halo、同一全局映射与采样相位、接缝零容差;预算从 `MIP_GDI_OBJECT_BUDGET`(自设 1000 对象,surface.rs:57)换字节记账 + `IDXGIAdapter3::QueryVideoMemoryInfo` 真值。

> **#81 落地后记(2026-09-20,外部评审 AI1 P3-6)**:上段的 `MIP_GDI_OBJECT_BUDGET` 与链式预算已随 #81 删除(#82 的字节记账从零起算,不再以对象计数为起点);实现期 census 发现 GDI StretchBlt 的失败随 **face 宽度**而非单次调用 extent(4M 宽 face 可整矩形直绘、6.29M 宽 face 上 2^21 分片可绘、≥2^23 宽 face 仅 512px 分片可读——即上游 mip 生成历来依赖的形状),故 #81 为 GDI 臂 ≥2^22 源加了**临时 GiantRelief 中介**(每 paint 建一次 ~2^21 DIB、512px HALFTONE 分片生成、单 blit 出图;无链无缓存)作为 #82 D2D tiling 接管前的过渡——#82 设计 tile/预算时以本后记与 issue #81 的 census 数据(issuecomment-5748005538/5748006943)为准,勿再引用已删除的预算机制。

### D8. 时序与线程(#80)

D2D/D3D 对象只在 UI 线程(factory SINGLE_THREADED;D3D11 不加 SINGLETHREADED flag);worker 只产内存帧。**保持 invalidate→WM_PAINT 消息驱动**,不搞 Present 渲染循环(看图器 99% 静态);WM_PAINT 内:BeginPaint(忽略 rcPaint 整视口重绘)→ BeginDraw → Clear(bg)(letterbox 由 Clear 替代,「先 blit 后填」的防闪顺序及其整类 bug 归零)→ DrawBitmap → EndDraw(**HRESULT 必查**,设备丢失唯一上报点)→ Present(交互/动画 `Present(0,0)` 防 vsync 阻塞拖拽;OCCLUDED 停渲染轮询恢复)→ EndPaint。WM_SIZE:`SetTarget(None)`+释放全部引用→`ResizeBuffers`→重建→同步重绘一次。`MakeWindowAssociation(DXGI_MWA_NO_ALT_ENTER)` 必加。设备依赖对象收敛进一个 GpuStack struct 一起生一起死。

### D9. DPI 立场(#79)

现状=系统 DPI aware(window.rs:7079 `SetProcessDPIAware`,等价上游 manifest `dpiAware=true`,c-original/res/voidImageViewer.Manifest:19)——主缩放显示器上 1:1 今天就成立(咨询二「进程 DPI-unaware、1:1 现在是假的」经核验为**误**);残余缺口=无 PMv2→混合 DPI 多屏上异 DPI 屏会 DWM 位图拉伸,D2D 后会伪装成「D2D 不如 GDI 清晰」。升级 PMv2(winresource `set_manifest` 注入,不引 rc.exe)= 对上游的有意偏离,记 Differences;`WM_DPICHANGED` 布局链复用 #78 的子 HWND resize 时序;LOGPIXELS 读取点逐个审计防半迁移。

### D10. 验收基础设施(#80/#81)

分级:L0 字节精确(1:1+identity 颜色,dump 断言)/ L1 与 GDI golden 字节一致(整数倍 Nearest 放大、边界)/ L2 视觉等价(线性档 MAE+无接缝)/ L3 色彩不变量(纯 Rust 单测,离线解析期望值)。**主入口=进程内回读 dump**(`CPU_READ` bitmap→CopyFromRenderTarget→Map→PNG;GDI 栈同通道从 master 实现,两栈 golden 互比),**勿以 PrintWindow 为 D3D 内容契约**(默认 flag 不捕获 flip;`PW_RENDERFULLCONTENT` 行为先 spike 三环境);golden 语料固定 WARP 生成跨机器确定;RGB 状态栏读出升级为断言通道(master 直读)。冒烟脚本存量手段(BitBlt 抓 DC/GetPixel 采样)在 D2D 视口上失效——采集入口随 #80 换,断言逻辑尽量保留。

> **#81 落地后记(2026-09-20)**:上段「golden 语料固定 WARP 生成」在 #81 落地为 **GDI 臂生成**(票面原文「GDI 栈生成→冻结入仓」,`smoke/golden81/`)——L1 正确性由比 golden 更硬的独立 oracle 承担(整数放大 dump 与 `src[x/k,y/k]` 复制模型逐像素相等,与生成臂无关),golden 只做跨 build 漂移检测,且每轮冒烟同时断言 warp==gdi 逐字节(warp-vs-golden 传递成立)。#82 冻结语料时择一而定,并更新本后记。

### 非目标(M6 明确不做)

ICM Stage 2(每显示器变换/热更新/ACM/HDR)、HDR/WCG 输出链、GPU 旋转变换(已评估拒绝:master 朝向与显示分离会把局部改动扩散成横切关注点,接缝自洽性优先)、驱动黑名单、WARP 主动探测(只按失败降级)。

## 理由

两份独立咨询在结构决策上完全一致(子 HWND+flip、CPU 真源先行、WCS 系统 API、WARP 兜底、lcms2 否决、消息驱动保持、readback 验收),且关键外部断言全部通过本地核验:flip 的 per-HWND 限制(官方原文)、`BM_xRGBQUADS` 字节序(官方逐字节表)、`ColorProfileGetDisplayDefault` 20348(官方 Requirements)、`DrawBitmap` 六档(签名)、`Win32_UI_ColorSystem` 134 函数(crate 源码)、上游 manifest `dpiAware=true`(c-original 在档)。分歧处(显示器空间 CPU 重算 vs sRGB 分层;quirk parity 保真 vs 放弃;20348 vs 1809;DPI 现状定性)均已按证据裁决如上。

## 权衡与后果

- 输入路由重构(#78)是 M6 最大单票风险,换来的是渲染/胶合层永久解耦。
- 双栈过渡期内 GDI 路径只作 A/B 参考与逃生舱,不承担新质量特性;黄金语料替代代码成为长期参考。
- 放弃三处上游保真(mip 选择 quirk、SetBrushOrgEx、放大重锚 ±1px 相位)是「内部实现细节不再可观测」的推理结论,全部记 README Differences。
- 1:1 字节一致承诺的边界:颜色管线 identity 且无 OS DPI 拉伸(PMv2 后混合 DPI 屏成立)时;1:1 空间零重采样则无条件承诺。
- Stage 1 落地即修复上游 `icm` no-op(README #50 段落随 #77 改写);Stage 2 之前显示器非 sRGB 且未开系统色彩管理的场景仍有残差——记为已知限制。
