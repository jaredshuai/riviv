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

> **#81 落地后记(2026-09-20,外部评审 AI2)**:上段「默认 gdi,M6 末翻 auto」的翻默认已随 #81 落地——missing 键与未识别值双路均落 `auto`(详见 D7/D10 的同日后记与 README #81 条目)。迁移面注意:#80 保存的 ini 已显式写入 `renderer=gdi`(保存恒写当时值),这批 ini 升级后**保持 gdi**,翻默认只惠及无键/新建 ini。(#90 后记,外部评审 AI2 P3:本句「保持 gdi」的语义随 GDI 臂终结——`gdi` 值现于加载时迁移 `auto` 并留一行注记,见 D7 后记;终态 = 一切 ini 皆 D2D。)

### D6. 插值映射与 1:1 契约(#81)

两臂各 2 档是用户可见 ini 面(`shrink_blit_mode`/`mag_filter`,config.rs:52-53):1:1=NEAREST+整数矩形;shrink 0→NEAREST、1(HALFTONE 默认)→`HIGH_QUALITY_CUBIC`(不一致且更好,记 Differences);mag 0(COLORONCOLOR 默认)→NEAREST、1(HALFTONE)→LINEAR。`ID2D1DeviceContext::DrawBitmap` 收完整 `D2D1_INTERPOLATION_MODE` 六档(windows-rs 签名已核;咨询二「需 spike 验证是否要走 DrawImage」的说法过虑)。**1:1 五件套**(缺一即破):`SetUnitMode(PIXELS)` + identity 变换审计 + 整数矩形 + 位图/target/swapchain 三处 `B8G8R8A8_UNORM`(用 `_SRGB` 会线性化出 ±1 差) + NEAREST(+ALIASED/COPY)。1:1 的空间零重采样在**任何** icm 设置下都成立;RGB 字节不变仅颜色管线 identity 时成立——这是两条独立断言。`SetBrushOrgEx` parity 是**删除**不是迁移(D2D 无 dither)。

### D7. mip 与巨图(#81/#82)

常规尺寸(≤ `GetMaximumBitmapSize()` 运行时查询,勿硬编码 16384)单张位图直绘,mip 链退役;`mip.rs` 纯函数与 counterexample 测试**保留**(巨图分级数学继续服役)。**放弃 mip 选择 quirk 的 parity**(mip.rs 在案的 `render_h` vs `mip_wide` 比较怪癖):D2D 下级别选择不再是可观测行为,复刻 quirk=故意输出更差画面——记 Differences。深缩质量后备:`HIGH_QUALITY_CUBIC` 自带 prefilter,不足则 ANISOTROPIC / Scale 效果。巨图(> max,D2D 上限典型 16384 **小于** GDI 32768→巨图覆盖面变大):overview 位图 + LRU tile 缓存(GPU 驻留 O(视口)),tile 带 halo、同一全局映射与采样相位、接缝零容差;预算从 `MIP_GDI_OBJECT_BUDGET`(自设 1000 对象,surface.rs:57)换字节记账 + `IDXGIAdapter3::QueryVideoMemoryInfo` 真值。

> **#81 落地后记(2026-09-20,外部评审 AI1 P3-6)**:上段的 `MIP_GDI_OBJECT_BUDGET` 与链式预算已随 #81 删除(#82 的字节记账从零起算,不再以对象计数为起点);实现期 census 发现 GDI StretchBlt 的失败随 **face 宽度**而非单次调用 extent(4M 宽 face 可整矩形直绘、6.29M 宽 face 上 2^21 分片可绘、≥2^23 宽 face 仅 512px 分片可读——即上游 mip 生成历来依赖的形状),故 #81 为 GDI 臂 ≥2^22 源加了**临时 GiantRelief 中介**(每 paint 建一次 ~2^21 DIB、512px HALFTONE 分片生成、单 blit 出图;无链无缓存)作为 #82 D2D tiling 接管前的过渡——#82 设计 tile/预算时以本后记与 issue #81 的 census 数据(issuecomment-5748005538/5748006943)为准,勿再引用已删除的预算机制。

> **#82 落地后记(2026-09-20)**:巨图形态按上段落地,但**删除项拆分出去了**——上段「删除判据写死」的三条里,golden 未冻结、RDP/Hyper-V 的 auto 初始化无证据、发布周期未到(三条均于开工日核实),故 #82 只做巨图 + 预算 + 冻结前置,删除独立为 **#90**(与用户确认后拆分,见 issue #82 design comment 5749353703 与实测后记 5749498238)。落地形态与三条实测结论:
>
> 1. **源级 + 形态的每帧选择**(`tile::detail_level` / `tile::plan_frame`):级位图能装下设备(`GetMaximumBitmapSize()`)就单张直绘(级 0 = 原路径逐字节不变;级 ≥1 = overview,由 `mip::downscale_box` 从原图一遍箱式降采样,块边界取整数 `floor(x*dim/level_dim)` → 覆盖每个源像素);装不下就把**可见 dest 的原像**切成 1024px 网格 tile(halo 32 源像素 + 裁剪到逻辑内部区;枚举按可见区窗口化 → 巨图 1:1 的 GPU 驻留 O(视口))。一帧的 tile 超过字节预算时**深化级**(更浅的级 tile 更少更小),至 1×1 必落单张——压力降级整帧均匀,不做半覆盖。
> 2. **接缝口径是实测的,不是推理的**(本机 2026-09-20,`-dump-viewport` 逐像素对照):**1:1 分块与整张逐字节相等**(smoke82 S2a 以整文件哈希相等断言;"803,016 像素 0 差"是本机会话探针的记录值,非脚本断言);fractional 缩放(滤波档)**无结构接缝**——边界 ±2 列内最大列间阶跃与整张对照相同(平滑场实测 0.299 vs 0.299)。像素差**有界但不为零,且按内容分档**:平滑场上差异像素全为 ±1(约 10%,平均绝对差 0.035),高对比内容上亚像素相位差可把一个边缘像素移动 ≤21(同一次实测,硬边缘 fixture)——同一现象,不是位移。两条因此写进实现:tile 的**绘制**矩形必须亚像素(`src_to_dest_f`:f64 投影单次 f32 舍入;第一版取整使每块等效 scale 漂移 → 全画面 ±1),**裁切**矩形必须整数(共享源坐标 → 逐位相同的边界 → 精确分区);halo = **32 源像素的经验常数**(8 时 0.65× 缩放的抽头伸出块外被 clamp、边界列 maxΔ=37;按 scale 推导的变体在开发期试过一次、在 S2b 配置下实测更差(maxΔ 1 → 21)故未上船——该试验不可从仓库复现,且 S2b 是强制的 level-0 深缩(~0.4×)而非该常数的 (0.5,1] 调参域,故它**只否决该变体在该配置的表现,不建立 D2D 的一般性质**;外部评审 AI1 P2-2)。**在案残余**:各向异性帧(一轴迫使级 0、另一轴缩 > 约 2×)在缩小轴上的 32px 余量可能不足——仅在巨图 + panscan 分轴缩放 + 滤波档下可达,表现为该轴块边界 1-2px 明暗。故「零容差」在本实现的准确表述 = **1:1 逐字节 + 滤波档无结构接缝(像素差按内容分档记录)**,不可写成「分块与整张逐字节相等」,也不可写成「滤波档每通道 ≤1」。
> 3. **预算换轨完成**(overview 构建成本实测入档,release 2026-09-20:40000×256→级 5 = 4 ms;8000×6000→级 1 = 131 ms;最大可缓存 overview 16389×8189→级 1 = 402 ms):字节四类账本(cpu_source / cpu_display / inflight / gpu_resident)+ `IDXGIAdapter3::QueryVideoMemoryInfo(LOCAL)` 真值(adapter 非 Adapter3 或查询失败退自设上限),cap = `Budget/4` 夹到 [16 MiB, 256 MiB](集显 LOCAL 段是系统内存 → 靠夹取而非直接采用);压力阶梯 = LRU 驱逐 → 深化级 → 单张 overview,**不落 GDI**(预算按**帧工作集**计——含已驻留块;上传前先把本帧已驻留的键全部 touch 成热,故驱逐只能回收非本帧条目,不产生半覆盖帧;`-tile` 诊断强制帧**有意**绕过预算、可部分覆盖)。存活 D2D 会话内的上传失败(含深化后仍被拒的级)= **该帧 letterbox**,不切 GDI 不 fatal——ADR 0001 的「保留旧图」层管解码,渲染栈自身的失败链是 EndDraw/Present 阶梯(外部评审 AI1 P2-1/P2-5)。close 时一行统计(stderr)是冒烟的预算证据通道。
>
> 风险在案(外部评审 AI2 P2-3):自 #82 起 D2D 臂的巨图失败链即已是 **D2D-only**(旧 #80 门的 GDI 交接随门退役;prepare 失败 = 该帧 letterbox,不拆栈不切 GDI);且 tile 路径在 WARP 上几乎不可达(WARP max=2^23),实证覆盖是「硬件 + 本机」单臂——这是 #90 环境矩阵(RDP/Hyper-V/WARP)成为删除判据的又一理由。遗留交 #90:`GiantRelief` 与 GDI 巨图支(`STRETCH_SOURCE_STITCH_TRIGGER`/`SLICE`)、mag 臂 ≥2^22 的未验证带(#81 遗留,本票未碰)、golden 冻结后的删除本体。

> **#90 落地后记(2026-09-21)**:GDI 渲染主线已删(实现在 feat/90-gdi-removal;`Surface` 只剩 CPU 真源容器,`stitch.rs` 整文件退役——tile 自有纯数学,票面「tiling 仍在用 stitch」经核过时;chrome 的 GDI 永久保留)。**失败链终结形态**:init 失败=系统级 fatal(auto 已含 hw+warp 两臂诊断;`d2d`/`warp` 单驱动诊断同 fatal)——本 ADR 与 ADR 0001 的「初始化失败落 GDI」过渡语义随臂终结;运行时非丢失确定性错误、rebuild 失败、WARP 三连=**延迟 fatal**(paint 借约外开火,纯谓词 `stack_rebuild_allowed` 钉住 pending-fatal 先于 rebuild 的次序);`gpu_init_failed` latch 语义终结。#82 移交两件随票落地:prepare 连续 3 次失败喂入设备丢失 ladder(纯谓词 `prepare_verdict`)、tile 存储换 HashMap(诊断帧 O(n²) 扫描消除);Tiles↔Base 重传与 set_cap 钩子维持 #82 在案裁定。mag 臂 ≥2^22 未验证带随臂消失(#81 遗留闭合)。判据裁定:发布周期改判等价证据、RDP/Hyper-V 一键脚本用户实测为 merge 门(见 issue #90 评论 5754329845/5754373161)。**并纠正上文 #82 后记两句现行时表述**(外部评审 AI1 P3-1):「上传失败=该帧 letterbox、不 fatal」现为**有界升级**——连续 3 次 prepare 失败喂入设备丢失 ladder,终至延迟 fatal;「字节四类账本」的 `cpu_display` 类随 GDI face 刭,余三类(统计行 13→12 字段)。

### D8. 时序与线程(#80)

D2D/D3D 对象只在 UI 线程(factory SINGLE_THREADED;D3D11 不加 SINGLETHREADED flag);worker 只产内存帧。**保持 invalidate→WM_PAINT 消息驱动**,不搞 Present 渲染循环(看图器 99% 静态);WM_PAINT 内:BeginPaint(忽略 rcPaint 整视口重绘)→ BeginDraw → Clear(bg)(letterbox 由 Clear 替代,「先 blit 后填」的防闪顺序及其整类 bug 归零)→ DrawBitmap → EndDraw(**HRESULT 必查**,设备丢失唯一上报点)→ Present(交互/动画 `Present(0,0)` 防 vsync 阻塞拖拽;OCCLUDED 停渲染轮询恢复)→ EndPaint。WM_SIZE:`SetTarget(None)`+释放全部引用→`ResizeBuffers`→重建→同步重绘一次。`MakeWindowAssociation(DXGI_MWA_NO_ALT_ENTER)` 必加。设备依赖对象收敛进一个 GpuStack struct 一起生一起死。

### D9. DPI 立场(#79)

现状=系统 DPI aware(window.rs:7079 `SetProcessDPIAware`,等价上游 manifest `dpiAware=true`,c-original/res/voidImageViewer.Manifest:19)——主缩放显示器上 1:1 今天就成立(咨询二「进程 DPI-unaware、1:1 现在是假的」经核验为**误**);残余缺口=无 PMv2→混合 DPI 多屏上异 DPI 屏会 DWM 位图拉伸,D2D 后会伪装成「D2D 不如 GDI 清晰」。升级 PMv2(winresource `set_manifest` 注入,不引 rc.exe)= 对上游的有意偏离,记 Differences;`WM_DPICHANGED` 布局链复用 #78 的子 HWND resize 时序;LOGPIXELS 读取点逐个审计防半迁移。

### D10. 验收基础设施(#80/#81)

分级:L0 字节精确(1:1+identity 颜色,dump 断言)/ L1 与 GDI golden 字节一致(整数倍 Nearest 放大、边界)/ L2 视觉等价(线性档 MAE+无接缝)/ L3 色彩不变量(纯 Rust 单测,离线解析期望值)。**主入口=进程内回读 dump**(`CPU_READ` bitmap→CopyFromRenderTarget→Map→PNG;GDI 栈同通道从 master 实现,两栈 golden 互比),**勿以 PrintWindow 为 D3D 内容契约**(默认 flag 不捕获 flip;`PW_RENDERFULLCONTENT` 行为先 spike 三环境);golden 语料固定 WARP 生成跨机器确定;RGB 状态栏读出升级为断言通道(master 直读)。冒烟脚本存量手段(BitBlt 抓 DC/GetPixel 采样)在 D2D 视口上失效——采集入口随 #80 换,断言逻辑尽量保留。

> **#81 落地后记(2026-09-20)**:上段「golden 语料固定 WARP 生成」在 #81 落地为 **GDI 臂生成**(票面原文「GDI 栈生成→冻结入仓」,`smoke/golden81/`)——L1 正确性由比 golden 更硬的独立 oracle 承担(整数放大 dump 与 `src[x/k,y/k]` 复制模型逐像素相等,与生成臂无关),golden 只做跨 build 漂移检测,且每轮冒烟同时断言 warp==gdi 逐字节(warp-vs-golden 传递成立)。#82 冻结语料时择一而定,并更新本后记。

> **#82 落地后记(2026-09-20)**:上段「#82 冻结语料时择一而定」**未在本票定案**——冻结语料唯一的用途是 #90 的删除前置(删除前必须有一份 GDI 栈产出的语料,否则删了 GDI 就没有生成臂了),而删除判据的三条在开工日两条无证据,故冻结随删除一起**移交 #90**(#90 依赖 #82 + 发布周期 + RDP/Hyper-V 实测留档,见该票)。本票自带的接缝/正确性证据不是 golden 而是**同构建内的对照**(tiled vs untiled、巨图 fit/1:1 的内容断言、边界阶跃对照),这些不依赖语料冻结;`smoke/golden81/` 继续作为跨 build 漂移的既有参考。

> **#90 落地后记(2026-09-21)**:冻结在删除前完成——`smoke/golden90/` 5 帧(1:1 双底色、整数放大 k=2/k=3、旋转 90°+1:1),**GDI 臂生成**,冻结时逐帧验证 gdi dump == warp dump 原始字节相等(语料域=已证字节相等的 NEAREST 域;滤波档不入 golden,维持同构建内 A/B)。生成臂既亡,golden90 自此为跨 build 漂移的冻结参考(smoke81 断言 warp 臂逐字节复现;硬件臂同域复现时一并断言)。两条在案教训:EditRotate90 的 shell 动词会**改写磁盘 fixture**(#43 既定行为),rot90 场景每臂/每次须新造 fixture——且**逐场景独立**:(单 fixture 双臂互污,SHA 证据定位;首轮冻结中 s5 白底帧还因「与 rot90 场景共享同一 fixture 文件、s4 的两次 rotate 已改写之」被污染成 rot180 内容,bbox 与字节 oracle 双双免疫(两臂同读污染文件),最终由 smoke S10 的**内容模型对照**(dump vs HashSource 直立模型)抓获并以干净 fixture 重冻——bbox/字节比较不证内容身份,模型对照才是);golden 为本机生成参考,跨机器字节复现不是契约(矩阵脚本用内容级断言)。

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
