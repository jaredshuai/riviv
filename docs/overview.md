# 项目总览

这份说明给看 riviv 的人，也给下一位从 [AGENTS.md](../AGENTS.md) 进来的 Agent。它不代替 [README](../README.md) 的用法、路线图和 Differences，也不代替 [ADR 0001](adr/0001-fail-loud.md) 与 [ADR 0002](adr/0002-d2d-wcs-render-stack.md)。

三层分开写：

- **设计**：ADR 与 AGENTS 里已经接受的约束，回答「应该怎样」。
- **现状**：写作时读到的代码，回答「当前怎样」。对照提交是 `e9015a2`（`master`；2026-09-29 文档卫生盘点刷新，此后代码变更以提交历史为准）。
- **推断**：读代码后的候选判断。推断不是已接受的设计。

依据不足的地方写在文末「未确认」，不补成事实。

## 主要部分

riviv 是 Windows 上的单 exe 看图程序，[voidImageViewer](https://github.com/voidtools/voidImageViewer) 的非官方 Rust 重写。行为有疑问时可以读 `c-original/` 对照，那份目录只读，实现在根目录 `src/`（AGENTS「技术栈约定」）。产物是静态链接的单 exe：当前 release 体积基线 6,800,896 B（#163 后；#152 落地时 6,770,688 B；测量日 2026-09-29，口径见 AGENTS「依赖原则」）。

下面只写职责和它们之间的关系。里程碑勾选以 README Roadmap 为准。

### 窗口

**设计。** 视口像素在子窗口 `riviv_view` 上，用 DXGI flip；菜单、状态栏、工具条仍走 GDI。父窗口带 `WS_CLIPCHILDREN`，不再往视口区做 GDI 绘制（ADR 0002 D4）。DPI 决策是 PerMonitorV2，相对上游的 system DPI 是有意偏离（ADR 0002 D9）。系统级失败要带上下文直报；用户给的坏图片不能把程序带崩（ADR 0001）。

**现状。** `window::run` 依赖加载器在用户代码之前套用的嵌入清单；进程若是 DPI-unaware，直接失败退出（`src/window.rs` 9253–9282）。主窗口自己的 `WM_PAINT` 只校验更新区（8529–8540）。像素在子窗口的 `WM_PAINT`（`view_proc` 1090–1097），交给 `paint_view`（1216）——#90 后只有 D2D 一条路径，不再有 GDI 臂可选。

### 打开、解码、色彩

**设计。** 后台解码只产出内存帧 `PixelFrame`（top-down BGRA）。GDI 面是 UI 线程按需派生的东西，不是真源（ADR 0002 D3）。嵌入的 ICC 在解码期变成 sRGB，发生在与背景合成之前；无 profile 或 profile 就是 sRGB 时走快速路径（ADR 0002 D2 Stage 1）。ADR 把顺序写成：decode(RGBA) → ICM → 合成到背景上 → 再 swizzle 成 BGRA。512 MiB 解码预算保持不变（ADR 0002 D3）。用户级加载失败不弹框、不退出（ADR 0001）。

**现状。** 唯一的后台线程按提交顺序串行干活：一次只解一张，交互重栅（#163）也排在同一条队列上（`src/loadthread.rs` 194–197、199–201）。格式按文件内容嗅探，不看扩展名——SVG 的形状闸先于 image crate 跑（`src/loader.rs` 269–289），其余格式仍走 `with_guessed_format`（296–298）。静态图和 GIF / WebP / APNG 动画都经过 `assemble_frame`（`src/loader.rs` 438；静态 `sink_static` 1015，动画 `stream_animation` 891，三个动画分发点是 648、679、792）。有 ICC 变换时，8-bit 臂的合成发生在已经是 BGRA 的变换输出上（470–477），16-bit 臂的合成则发生在 f16 halves 域（463–466）。`icm=0`、没有 profile、或不是 RGB ICC v2/v4 时不变换，画面仍显示（`src/icm.rs` `prepare`，801–828；v2/v4 闸在 852–854）。APNG 能动画是相对上游的明确偏离，README Differences 的 #98 段已写明。色深不能动画、或画布超过动画预算时，改为静态显示，并在 stderr 写下是哪一条（`src/loader.rs` 763–779）。

SVG 是 #152 落地的解码臂：`svg::sniff` 只认「文本序言 + `<svg` 根」的形状，svgz（gzip 封装）刻意不放行（`src/svg.rs` 57、22–25）；命中后 parse 出 `usvg::Tree`、按视口决策光栅面（fit≥1 钉自然尺寸，缩小方向保 1024 中间档、压帧字节预算；`src/svg.rs` 124–146）、tiny-skia 光栅化，产物直乘 RGBA 进共享帧管线（`src/svg.rs` 208、225；`src/loader.rs` 348–372）。解析出的 Tree 以 `Arc` 随帧携带——`PixelFrame::svg`，只在 SVG 臂的帧上非空（`src/pixels.rs` 541–547；`SvgSource` 在 `src/svg.rs` 170–173）。SVG 无 ICC 可言，icm 链结构性缺席。

#142 起（ADR 0003）帧带内容空间：`ContentSpace` 的 `Srgb` / `F16Srgb` / `F16P3` 三态（`src/transform_stage.rs` 500–511）。L1 的门是 `master_content_space`：源深于 8 bit、或真的做了 ICC 变换，该帧才铸成 FP16 gamma-sRGB master（每像素 8 字节 f16 halves），两条都不沾的 8-bit 多数仍是 BGRA（`src/transform_stage.rs` 521–530；`src/pixels.rs` 519–527、629–631；f16 臂在 `src/loader.rs` 446–467）。

播放列表收十一个扩展名：`bmp`、`gif`、`ico`、`jpeg`、`jpg`、`png`、`tif`、`tiff`、`webp`、`apng`、`svg`（`src/playlist.rs` 372–374；`apng` 与 `svg` 是 beyond-original 的追加，#108 与 #152）。`is_valid_path` 按「最后一个 `.` 之后的尾段匹配」进这张表（391–401），`add_filename` 在 692 用它过滤；Everything 的搜索前缀也从同一张表派生（`src/everything.rs` 159、932、943），所以 svg 同样进查询。README 的 #98 段写明：文件夹、多文件拖放、随机、Everything 的 LIST2 入列和跳转列表认这个扩展名；命令行直接打开、单个文件拖放和 `stdin:` 仍按内容解码。Ctrl+O 的图像过滤器同步带上 `*.apng` 和 `*.svg`（`src/text.rs` 78–82）。安装器关联表仍是九项，没有改（`src/assoc.rs` 23–27）。

### 绘制

**设计。** 上 D2D 是为了高分辨率下交互缩放的吞吐，以及以后的能力空间，不是为了色彩管理（ADR 0002 D0）。色彩管理不依赖渲染栈（D1）。`renderer` 取 `auto`、`d2d`、`warp`、`gdi`。ADR 正文曾写默认 `gdi`；同一篇 #81 后记改为：缺键和认不出的值都落 `auto`。`auto` 是硬件，失败再 WARP，过渡期再 GDI；GDI 删除的判据未满足，删除在 #90（#82 后记）。绘制保持 `WM_PAINT` 驱动，不另跑 Present 循环（D8）。1:1 使用 NEAREST、整数矩形、`B8G8R8A8_UNORM`（D6）。

**现状。** 配置缺键时 `renderer` 是 `auto`（`src/config.rs` 236，以及测试 `missing_renderer_key_defaults_to_auto` 851）；`gdi` 键已除名、认不出的值都落 `auto`（413–432）。窗口显示前按这个请求建栈：`auto` 先试硬件设备，失败再试 WARP（`src/gpu.rs` `create` 817–838）。建栈失败自 #90 起是系统级 fatal——没有 GDI 臂可退，`auto` 已在 `create` 内试完硬件和 WARP（`src/window.rs` 9765–9803）。此后视口只有 D2D 一条绘制路径：`paint_view` 先做输出面对账，再进 `paint_d2d`（`src/window.rs` 1216–1240；`src/gpu.rs` 2417；没有栈的绘制只消费更新区并放行首帧握手，`src/gpu.rs` 2503–2517）。D2D 一次场景是 BeginDraw、Clear、DrawBitmap、EndDraw（`src/gpu.rs` 1866–1955），然后 `Present(0, 0)`（2387–2391）。超过设备位图上限的巨图留在 D2D 臂里，用 overview 或分块救济（`src/gpu.rs` 344、1047、1262–1269）。

L2（ADR 0004）的 AC 会话画在 FP16 scRGB 输出面上：swapchain 与画进它的一切都是 `R16G16B16A16_FLOAT`（`AC_SWAPCHAIN_FORMAT`，`src/gpu.rs` 467；遗留面仍是冻结的 `B8G8R8A8_UNORM`，465），`SetColorSpace1(scRGB)` 经 `SwapChain3` QI 声明，两步都 fail loud（898–905），随后 stderr 记 `output-surface=ac-scrgb` 面包屑（910）——遗留面在这里不打印，窄会话的 stderr 保持逐字节不变。`F16P3` 宽 master 只经 `from_f16_halves_wide` 一道生产闸铸出（`src/pixels.rs` 633–640；loader 侧入口在 `src/loader.rs` 453），#156 的 `wide_allowed` 会话闩关掉后新解码不再铸（`src/icm.rs` 794–800）。

**推断。** 一台具体机器第一次画出的是硬件 D2D 还是 WARP，取决于 `gpu::create` 里硬件尝试是否成功；GDI 已不在可能集里（#90 删除了渲染主线）。这是设计里写过的降级，不是本轮观察到的后端。本轮没有读这台机器的 `renderer=` / `backend=` stderr 行。

### 旁边的能力

设置、菜单与快捷键、幻灯片、剪贴板、外壳动词、文件管理、Everything 搜索、安装与文件关联，README 现状段已经各有句子。它们不另备一套像素真源。

**现状（只核对了入口，没有重读每个命令）。** 导航打开调用 `request_open`（`src/window.rs` home 4827、next/prev 5010、5035、5066）。Everything 随机结果打开也调用 `request_open`（`on_random_reply` 5980 → 6017）。播放列表、预加载、上一张缓存是 `request_open` 开头的旁路（4324–4359），命中则不再解码。

## 关键关系

用户动作进主窗口。打开请求进那一个解码线程——交互重栅（#163）也排在这同一条队列上，解码与重栅永不并发持帧（`src/loadthread.rs` 194–197、280–303）。线程把回复放进这次加载的队列，并用 `WM_APP+1` 叫醒 UI（`REPLY_KICK_MESSAGE`，`src/loadthread.rs` 52；主窗口在 8966–8971 交给 `on_load_replies`，同一脚踢也放行重栅回执 `drain_reraster_replies`）。UI 收下第一帧，改当前显示，再让视口重画。视口从 CPU 帧画出像素。菜单和状态栏不保存这帧的像素。

## 从给出一张图到窗口里看见它

这条流程只写「用户给出磁盘上的一张图」。`stdin:` 和 `clipboard:` 是没有背后文件的同类请求（`request_open_virtual`，4566–4600），解码之后仍走同一条回复和绘制。它们的语义以 README Differences 为准，这里不逐步展开。

### 设计上应该怎样

- 坏路径、解不出、解码预算溢出：不弹框、不退出。第一帧到达之前保留旧图或空窗口（ADR 0001）。第一帧已经上屏之后，同一次加载中途失败会清屏、标题仍留失败文件名——这句是 README Differences 的现状，ADR 0001 只写到「保留旧图或空窗口」。
- 窗口类注册、模块句柄，以及 ADR 0001 原文里的 DIB/DC 创建失败：带 `GetLastError` 直报。ADR 写的当前实现是 fatal 对话框加非零退出码。和代码的分歧见文末。
- 看见的像素来自 CPU 帧，不是工作线程上的 GDI 对象（ADR 0002 D3、D8）。
- `icm` 开着且嵌入的是可用的 RGB ICC 时，先变到 sRGB，再与窗口背景合成（ADR 0002 D2）。字面顺序和 `assemble_frame` 的差别见文末。

### 当前代码怎么走

1. **路径从三条入口进来。**
   - 命令行：安装族开关若处理完就退出、不建窗口（`run` 9321–9332）。否则在单实例门之后建窗口，再 `process_parsed_cl`（9919）。一个文件词走 `open_from_filename`（5721–5723）。没有文件词则不发起加载，窗口是空的（README Usage）。第二个进程在默认单实例下把整行命令交给已在运行的窗口后自己退出（9334–9444）；接收端改到对方的工作目录，再调用同一个 `process_parsed_cl`（5897–5918）。
   - Ctrl+O 或文件菜单的打开：`open_image_via_dialog`。取消则返回。打开（不是添加）先清空播放列表，再 `open_from_filename`（6700–6745）。
   - 往窗口拖一个文件、且没有按 Shift：`WM_DROPFILES` → `on_drop_files` → `apply_drop_files` → `open_from_filename`（8834–8838、8175–8190、8243；子窗口也把 HDROP 转给 owner，1174–1177）。
2. **目录和普通文件在这里分开。** `open_from_filename` 见到目录就收进播放列表再 home；见到普通文件就 `request_open(..., OpenOrigin::Direct)`（4612–4645）。这一步不看扩展名。多个文件或按着 Shift 拖放会先经 `add_filename` 改播放列表，扩展名不在那十一个里的文件不会进列表。本轮没有逐行读 `home_open`，所以文件夹打开后第一张是哪一个文件，这里不写死。
3. **排队解码，或直接换上已有的图。** `request_open`（4321–4489）若命中上一张缓存或预加载，换上现成的帧并返回（4324–4359）。若路径不存在，不进加载器，状态栏走 “File not found.”，旧画面留着（4361–4408）。否则记下当时的背景色和 `icm`（`decode_env`，4459；定义 4502），`LoadThread::request` 送出 `LoadSource::File`，并打开第一帧绘制握手（4466–4471）。标题在这一刻就改成所请求的文件（4474–4487），不等解码结束。
4. **工作线程产出内存帧。** `decode_to_sink` 成功则以 `Complete` 收尾，用户级错误则 `FailedUser`（`src/loader.rs` 166–178）。帧在进回复之前做 ICC（若启用）和背景合成，见上一节的 `assemble_frame`。透明在这一步变成不透明（`composite_over_background_in_place`，`src/pixels.rs` 61–80；BGRA 路径在 89–91 转调它）。工作线程不创建 GDI 对象（`loadthread.rs` 22–24）。
5. **UI 收下第一帧。** 踢消息到达后 `on_load_replies` 排空队列（`src/window.rs` 6060；drain 6086–6087）。帧用 `Surface::from_master` 包起来，这一步只是移交所有权，不建 DIB（`src/surface.rs` 99；调用点 6099–6100）。`apply_reply`（纯函数在 `src/loader.rs` 1460）让第一帧替换当前显示；`frame_gen` 增加，下一轮 D2D 绘制会重新上传（6131、6150）。然后 `repaint` 让视口失效（6414；定义 2860）。
6. **视口把这帧画出来。** 子窗口 `WM_PAINT` 进入 `paint_view`（1090–1097、1216），对账输出面之后 `paint_d2d`：BeginPaint 之后按 draw_plan 准备位图并绘制，Clear 出的颜色就是信箱边（`src/gpu.rs` 2417、344、1866–1955），`Present(0, 0)` 收尾（2390）。没有 GDI 臂——`src/paint.rs` 只剩共享视图数学 `scene_rect` 和 dump 通道的 PNG 尾（模块文档 1–11）。动画在第一帧画完之后才继续解后面的帧（`src/loader.rs` 995–1002；等待原语在 `src/loadthread.rs` 150–155）。
7. **SVG 放大过阈值后，后台换上更清晰的面（#163）。** 每次显示面变化的尾部（缩放步进、fit/1:1、全屏、resize、panscan 尺寸步）都会 `consider_reraster`（`src/window.rs` 3032–3086）：先推进视图纪元 `reraster_seq`（414–420），显示面把光栅面任一轴上采样超过 2× 才动手（`needs_reraster`，`src/svg.rs` 160–163）；目标面按 panscan 因子反除回渲染面、过加载期同一道纪律闸（`interactive_target`，`src/svg.rs` 246–261），重栅任务送进同一个 load worker（`request_reraster`，`src/loadthread.rs` 280–303）。回执由 `drain_reraster_replies` 消费：纪元对不上就丢弃，对得上则换掉帧像素并采纳 1:1 视图——换面瞬间屏幕矩形逐像素不动（3095–3148；采纳在 3132，向量句柄在 3136–3137 转挂到新帧，供下一次缩放继续重栅）。重栅失败不产生回执，画面保持原样（`src/loadthread.rs` 116–120）；旋转这类像素域编辑会丢掉句柄（`src/pixels.rs` 541–546）。

用户这时应在视口里看见这张图，四周不足处是背景色。本轮没有启动 `riviv.exe` 目视确认这一点。

### 失败时当前怎样

- 文件不存在：第 3 步即停，旧图还在，状态栏是 “File not found.”。
- 解不出或预算溢出，且第一帧还没换上画面：`FailedUser` 不退出进程。ADR 0001 要求保留旧图。README 同时写了：第一帧已经在屏幕上时，同一次加载的后续失败会清成空窗口，标题仍是失败文件名。
- D2D 初始化失败：自 #90 起没有 GDI 臂可降——`auto` 在 `create` 内已试完硬件与 WARP，仍失败则 fatal 直报（`src/window.rs` 9765–9803）。这偏离了 ADR 0002 D5 正文的「初始化失败温和降级」，README Differences 的 #90 段已记录。
- 运行中的设备丢失：10 秒内累计后再升级到 WARP；WARP 仍在这个窗口里失败，则推迟到绘制借用结束之后 fatal（`gpu_runtime_failure`，`src/window.rs` 2023；`is_device_loss`，`src/gpu.rs` 449）。非丢失类的 EndDraw / Present 失败自 #90 起按不可恢复处置——拆栈并推迟 fatal，不再有「整段会话留在 GDI」这条路（`src/gpu.rs` 1977–1988；`src/window.rs` 1260–1275）。后一种和 ADR 正文的阶梯不同，见下节。

## 设计与现状不一致

只记录，不改 ADR。

1. **DIB/DC 创建失败不再按 ADR 0001 直报。** ADR 0001 把 DIB/DC 创建列为系统级失败：fatal 对话框，非零退出。现状是绘制时 `ensure_face` 失败只把该帧降成空白，stderr 留一行，`face_stuck` 之后不再重试（`src/surface.rs` 419–432、472–476）。`Surface::from_master` 不会失败（335–340），所以回复路径上的 `FatalSystem`（`src/loader.rs` 660–671）接不住这次失败。README Differences 关于 #76 的那一段已经写了同样的后果。ADR 0001 的正文还没有改。

   追记（2026-09-29）：本条描述的代码前提已随 #76/#90 消失——`ensure_face`/`face_stuck` 在 `src/surface.rs` 已无此符号（GDI 渲染主线 #90 删除；`Surface::from_master` 现为不失败的纯所有权移交，`src/surface.rs` 99；该模块余下的 DIB 分配原语只剩剪贴板 CF_BITMAP 拷贝这一个消费者，同文件模块文档 1–17）。正文保留不改，本条自此按历史记录理解。
2. **ICC 之后的字节顺序和 ADR 0002 D2 的字面顺序不同。** ADR 写的是合成之后再 swizzle 成 BGRA。`assemble_frame` 在有变换时先得到 BGRA，再合成（470–477）。README Differences 的 #77 段把这记成相对票面步骤的实现偏离，并写明语义相同、少一遍全缓冲。本文件沿用那个记录。
3. **D2D 运行期失败的处置，比 ADR 0002 D5 正文又加宽了两档。** D5 写：运行期丢失则从 master 重传；10 秒内 3 次则 WARP；WARP 也失败才 fatal；绘制路径内一律降级而不是 fatal。代码里，丢失类 HRESULT（含 `DRIVER_INTERNAL_ERROR`）走这条阶梯（`src/gpu.rs` 449，`src/window.rs` 2023）。其余失败的 EndDraw / Present 自 #90 起不再有去 GDI 的退路，按不可恢复处置：拆栈并推迟 fatal（`src/gpu.rs` 1977–1988，`src/window.rs` 1260–1275）。README 的 #80 与 #90 段记录了这两次加宽。ADR D5 的决策段两者都没有写。

下面两项是 ADR 里过时的句子，决策或后记已经把行为说清楚，不算上面那种未改写的冲突：

- D5 开头的「默认 gdi」由同文 #81 后记改成缺键即 `auto`。代码默认值是 `auto`。
- D9 的「现状」句仍写 `SetProcessDPIAware`。决策句是升级到 PerMonitorV2。`run` 已去掉那次调用，并依赖嵌入清单（7976–7980）。

## 未确认

- 没有启动 `riviv.exe`，没有目视确认视口里出现像素，也没有读本机 stderr 上的 `backend=`。
- 多个文件、文件夹或 Shift 拖放之后，`home_open` 最终打开哪一张，本轮没有逐行读。
- 播放列表导航和 Everything 随机打开之外，其余会调用 `request_open` 的位置没有逐段读。
- 巨图救济（overview / 分块的选择）与放大重锚的行为以 README Differences 为准，本轮只核到入口（`draw_plan` 与 mip 档位文档），没有沿绘制计划逐行核对。
- D2D 巨图的 overview / tile 选择以 ADR #82 后记和 `paint_view` 的注释为准。`tile::detail_level` 的实现本轮没有读。
