# 项目总览

这份说明给看 riviv 的人，也给下一位从 [AGENTS.md](../AGENTS.md) 进来的 Agent。它不代替 [README](../README.md) 的用法、路线图和 Differences，也不代替 [ADR 0001](adr/0001-fail-loud.md) 与 [ADR 0002](adr/0002-d2d-wcs-render-stack.md)。

三层分开写：

- **设计**：ADR 与 AGENTS 里已经接受的约束，回答「应该怎样」。
- **现状**：写作时读到的代码，回答「当前怎样」。对照提交是 `7befdfe`（`master`）。
- **推断**：读代码后的候选判断。推断不是已接受的设计。

依据不足的地方写在文末「未确认」，不补成事实。

## 主要部分

riviv 是 Windows 上的单 exe 看图程序，[voidImageViewer](https://github.com/voidtools/voidImageViewer) 的非官方 Rust 重写。行为有疑问时可以读 `c-original/` 对照，那份目录只读，实现在根目录 `src/`（AGENTS「技术栈约定」）。

下面只写职责和它们之间的关系。里程碑勾选以 README Roadmap 为准。

### 窗口

**设计。** 视口像素在子窗口 `riviv_view` 上，用 DXGI flip；菜单、状态栏、工具条仍走 GDI。父窗口带 `WS_CLIPCHILDREN`，不再往视口区做 GDI 绘制（ADR 0002 D4）。DPI 决策是 PerMonitorV2，相对上游的 system DPI 是有意偏离（ADR 0002 D9）。系统级失败要带上下文直报；用户给的坏图片不能把程序带崩（ADR 0001）。

**现状。** `window::run` 依赖加载器在用户代码之前套用的嵌入清单；进程若是 DPI-unaware，直接失败退出（`src/window.rs` 7976–8001）。主窗口自己的 `WM_PAINT` 只校验更新区（7287–7295）。像素在子窗口的 `WM_PAINT`（`view_proc` 980–986），由 `paint_view` 选 D2D 或 GDI（1105–1118）。

### 打开、解码、色彩

**设计。** 后台解码只产出内存帧 `PixelFrame`（top-down BGRA）。GDI 面是 UI 线程按需派生的东西，不是真源（ADR 0002 D3）。嵌入的 ICC 在解码期变成 sRGB，发生在与背景合成之前；无 profile 或 profile 就是 sRGB 时走快速路径（ADR 0002 D2 Stage 1）。ADR 把顺序写成：decode(RGBA) → ICM → 合成到背景上 → 再 swizzle 成 BGRA。512 MiB 解码预算保持不变（ADR 0002 D3）。用户级加载失败不弹框、不退出（ADR 0001）。

**现状。** 唯一的后台线程一次只解一张（`src/loadthread.rs` 模块说明，162–164 行附近）。格式按文件内容嗅探，不看扩展名（`src/loader.rs` 208–224）。静态图和 GIF / WebP / APNG 动画都经过 `assemble_frame`（静态臂 653，动画的 `stream_animation` 599；三个分发点是 319、350、451）。有 ICC 变换时，swizzle 折进 `TranslateBitmapBits` 的输出，合成发生在已经是 BGRA 的缓冲上（`assemble_frame` 269–277）。`icm=0`、没有 profile、或不是 RGB ICC v2/v4 时不变换，画面仍显示（`src/icm.rs` `prepare`，349–356）。APNG 能动画是相对上游的明确偏离，README Differences 的 #98 段已写明。色深不能动画、或画布超过动画预算时，改为静态显示，并在 stderr 写下是哪一条（`src/loader.rs` 422–435）。

播放列表收十个扩展名：`bmp`、`gif`、`ico`、`jpeg`、`jpg`、`png`、`tif`、`tiff`、`webp`、`apng`（`src/playlist.rs` 365–366）。名为 `.apng` 的文件经 `is_valid_path`（384–392）进入这张表；`add_filename` 在 689 用它过滤。README 的 #98 段写明：文件夹、多文件拖放、随机、Everything 的 LIST2 入列和跳转列表认这个扩展名；命令行直接打开、单个文件拖放和 `stdin:` 仍按内容解码。Ctrl+O 的图像过滤器同步带上 `*.apng`（`src/text.rs` 80）。把 `.apng` 接进导航过滤是 #108，已经做了。安装器关联表仍是九项，没有改。

### 绘制

**设计。** 上 D2D 是为了高分辨率下交互缩放的吞吐，以及以后的能力空间，不是为了色彩管理（ADR 0002 D0）。色彩管理不依赖渲染栈（D1）。`renderer` 取 `auto`、`d2d`、`warp`、`gdi`。ADR 正文曾写默认 `gdi`；同一篇 #81 后记改为：缺键和认不出的值都落 `auto`。`auto` 是硬件，失败再 WARP，过渡期再 GDI；GDI 删除的判据未满足，删除在 #90（#82 后记）。绘制保持 `WM_PAINT` 驱动，不另跑 Present 循环（D8）。1:1 使用 NEAREST、整数矩形、`B8G8R8A8_UNORM`（D6）。

**现状。** 配置缺键时 `renderer` 是 `auto`（`src/config.rs` 241，以及测试 `missing_renderer_key_defaults_to_auto`）。窗口显示前按这个请求建栈：要 D2D 就 `gpu::create`，否则留在 GDI（`src/window.rs` 8462–8526）。`auto` 先试硬件设备，失败再试 WARP（`src/gpu.rs` 475–488）。有栈时 `paint_d2d` 画视口；没有栈时走 GDI 的 `paint`（`paint_view` 1105–1118）。D2D 一次场景是 BeginDraw、Clear、DrawBitmap、EndDraw（`src/gpu.rs` 989–1081），然后 `Present(0, 0)`（1379）。超过设备位图上限的帧留在 D2D 臂里，用 overview 或分块，不再因此改走 GDI（`paint_view` 1099–1104，与 ADR #82 后记一致）。

**推断。** 一台具体机器第一次画出的是硬件 D2D、WARP 还是 GDI，取决于当时 `gpu::create` 是否成功。这是设计里写过的降级，不是本轮观察到的后端。本轮没有读这台机器的 `renderer=` / `backend=` stderr 行。

### 旁边的能力

设置、菜单与快捷键、幻灯片、剪贴板、外壳动词、文件管理、Everything 搜索、安装与文件关联，README 现状段已经各有句子。它们不另备一套像素真源。

**现状（只核对了入口，没有重读每个命令）。** 导航打开调用 `request_open`（`src/window.rs` 2402）。Everything 随机结果打开也调用 `request_open`（4781）。播放列表、预加载、上一张缓存是 `request_open` 开头的旁路（3145–3174），命中则不再解码。

## 关键关系

用户动作进主窗口。打开请求进那一个解码线程。线程把回复放进这次加载的队列，并用 `WM_APP+1` 叫醒 UI（`REPLY_KICK_MESSAGE`，`src/loadthread.rs` 51；主窗口在 7692–7694 交给 `on_load_replies`）。UI 收下第一帧，改当前显示，再让视口重画。视口从 CPU 帧画出像素。菜单和状态栏不保存这帧的像素。

## 从给出一张图到窗口里看见它

这条流程只写「用户给出磁盘上的一张图」。`stdin:` 和 `clipboard:` 是没有背后文件的同类请求（`request_open_virtual`，3347–3380），解码之后仍走同一条回复和绘制。它们的语义以 README Differences 为准，这里不逐步展开。

### 设计上应该怎样

- 坏路径、解不出、解码预算溢出：不弹框、不退出。第一帧到达之前保留旧图或空窗口（ADR 0001）。第一帧已经上屏之后，同一次加载中途失败会清屏、标题仍留失败文件名——这句是 README Differences 的现状，ADR 0001 只写到「保留旧图或空窗口」。
- 窗口类注册、模块句柄，以及 ADR 0001 原文里的 DIB/DC 创建失败：带 `GetLastError` 直报。ADR 写的当前实现是 fatal 对话框加非零退出码。和代码的分歧见文末。
- 看见的像素来自 CPU 帧，不是工作线程上的 GDI 对象（ADR 0002 D3、D8）。
- `icm` 开着且嵌入的是可用的 RGB ICC 时，先变到 sRGB，再与窗口背景合成（ADR 0002 D2）。字面顺序和 `assemble_frame` 的差别见文末。

### 当前代码怎么走

1. **路径从三条入口进来。**
   - 命令行：安装族开关若处理完就退出、不建窗口（`run` 8044–8053）。否则在单实例门之后建窗口，再 `process_parsed_cl`（8591–8598）。一个文件词走 `open_from_filename`（4501–4503）。没有文件词则不发起加载，窗口是空的（README Usage）。第二个进程在默认单实例下把整行命令交给已在运行的窗口后自己退出（8055–8163）；接收端改到对方的工作目录，再调用同一个 `process_parsed_cl`（4628–4682）。
   - Ctrl+O 或文件菜单的打开：`open_image_via_dialog`。取消则返回。打开（不是添加）先清空播放列表，再 `open_from_filename`（5458–5496）。
   - 往窗口拖一个文件、且没有按 Shift：`WM_DROPFILES` → `on_drop_files` → `apply_drop_files` → `open_from_filename`（1062–1066、6937–7005）。
2. **目录和普通文件在这里分开。** `open_from_filename` 见到目录就收进播放列表再 home；见到普通文件就 `request_open(..., OpenOrigin::Direct)`（3393–3408）。这一步不看扩展名。多个文件或按着 Shift 拖放会先经 `add_filename` 改播放列表，扩展名不在那十个里的文件不会进列表。本轮没有逐行读 `home_open`，所以文件夹打开后第一张是哪一个文件，这里不写死。
3. **排队解码，或直接换上已有的图。** `request_open`（3137–3304）若命中上一张缓存或预加载，换上现成的帧并返回。若路径不存在，不进加载器，状态栏走 “File not found.”，旧画面留着（3177–3223）。否则记下当时的背景色和 `icm`（`decode_env`，3312–3316），`LoadThread::request` 送出 `LoadSource::File`，并打开第一帧绘制握手（3282–3288）。标题在这一刻就改成所请求的文件（3258–3302），不等解码结束。
4. **工作线程产出内存帧。** `decode_to_sink` 成功则以 `Complete` 收尾，用户级错误则 `FailedUser`（121–132）。帧在进回复之前做 ICC（若启用）和背景合成，见上一节的 `assemble_frame`。透明在这一步变成不透明（`composite_over_background_in_place`，`src/pixels.rs` 52–70；BGRA 路径在 80–81 转调它）。工作线程不创建 GDI 对象（`loadthread.rs` 22–24）。
5. **UI 收下第一帧。** 踢消息到达后 `on_load_replies` 排空队列（4819–4824、4850–4864）。帧用 `Surface::from_master` 包起来，这一步只是移交所有权，不建 DIB（`src/surface.rs` 330–340）。`apply_reply` 让第一帧替换当前显示；`frame_gen` 增加，下一轮 D2D 绘制会重新上传（4878–4914）。然后 `repaint` 让视口失效（5176–5177、1986–1990）。
6. **视口把这帧画出来。** 子窗口 `WM_PAINT` 进入 `paint_view`。有 D2D 栈则 `paint_d2d`：BeginPaint 之后准备位图并绘制，Clear 出的颜色就是信箱边（`paint_d2d` 1397–1404，场景体 989–1081，`Present` 1379）。没有栈则 GDI 的 `paint` 从全分辨率面 StretchBlt / BitBlt，再填信箱边（`src/paint.rs` 1–32、51–77）。GDI 面在第一次需要它的绘制里派生；建失败则这帧空白，并打一行 stderr，同一次表面不再重试（`surface.rs` 419–432、472–476）。动画在第一帧画完之后才继续解后面的帧（`stream_animation` 600–610；握手等待见 `loadthread.rs` 120–124）。

用户这时应在视口里看见这张图，四周不足处是背景色。本轮没有启动 `riviv.exe` 目视确认这一点。

### 失败时当前怎样

- 文件不存在：第 3 步即停，旧图还在，状态栏是 “File not found.”。
- 解不出或预算溢出，且第一帧还没换上画面：`FailedUser` 不退出进程。ADR 0001 要求保留旧图。README 同时写了：第一帧已经在屏幕上时，同一次加载的后续失败会清成空窗口，标题仍是失败文件名。
- D2D 初始化失败：降到 GDI，stderr 加一次状态栏提示，不弹致命框（8462–8517）。这和 ADR 0002 D5 的「初始化失败是环境问题，温和降级」一致。
- 运行中的设备丢失：10 秒内累计后再升级到 WARP；WARP 仍在这个窗口里失败，则推迟到绘制借用结束之后 fatal（`gpu_runtime_failure` 1206–1211）。非丢失类的 EndDraw / Present 失败改为整段会话留在 GDI（`is_device_loss` 340–344，`gpu_session_degrade` 1130–1144）。后一种和 ADR 正文的阶梯不完全相同，见下节。

## 设计与现状不一致

只记录，不改 ADR。

1. **DIB/DC 创建失败不再按 ADR 0001 直报。** ADR 0001 把 DIB/DC 创建列为系统级失败：fatal 对话框，非零退出。现状是绘制时 `ensure_face` 失败只把该帧降成空白，stderr 留一行，`face_stuck` 之后不再重试（`src/surface.rs` 419–432、472–476）。`Surface::from_master` 不会失败（335–340），所以回复路径上的 `FatalSystem`（`src/loader.rs` 660–671）接不住这次失败。README Differences 关于 #76 的那一段已经写了同样的后果。ADR 0001 的正文还没有改。
2. **ICC 之后的字节顺序和 ADR 0002 D2 的字面顺序不同。** ADR 写的是合成之后再 swizzle 成 BGRA。`assemble_frame` 在有变换时先得到 BGRA，再合成（269–273）。README Differences 的 #77 段把这记成相对票面步骤的实现偏离，并写明语义相同、少一遍全缓冲。本文件沿用那个记录。
3. **D2D 运行期失败的最后一档，比 ADR 0002 D5 正文多一条去 GDI 的路。** D5 写：运行期丢失则从 master 重传；10 秒内 3 次则 WARP；WARP 也失败才 fatal；绘制路径内一律降级而不是 fatal。代码里，丢失类 HRESULT（含 `DRIVER_INTERNAL_ERROR`）走这条阶梯（`src/gpu.rs` 340–344，`src/window.rs` 1206–1211）。其余失败的 EndDraw / Present 则拆掉栈，本次进程留在 GDI（`gpu_session_degrade` 1130–1144）。README 的 #80 段写了这个加宽。ADR D5 的决策段没有写「非丢失错误整段会话改 GDI」。

下面两项是 ADR 里过时的句子，决策或后记已经把行为说清楚，不算上面那种未改写的冲突：

- D5 开头的「默认 gdi」由同文 #81 后记改成缺键即 `auto`。代码默认值是 `auto`。
- D9 的「现状」句仍写 `SetProcessDPIAware`。决策句是升级到 PerMonitorV2。`run` 已去掉那次调用，并依赖嵌入清单（7976–7980）。

## 未确认

- 没有启动 `riviv.exe`，没有目视确认视口里出现像素，也没有读本机 stderr 上的 `backend=`。
- 多个文件、文件夹或 Shift 拖放之后，`home_open` 最终打开哪一张，本轮没有逐行读。
- 播放列表导航和 Everything 随机打开之外，其余会调用 `request_open` 的位置（`window.rs` 3608、3632、3791、3816、3847）没有逐段读。
- GDI 臂的 `paint` 只读了文件头和 `BeginPaint` 那一段（`src/paint.rs` 1–80）。巨图救济中介和放大重锚的行为以 README Differences 为准，本轮没有沿 StretchBlt 分支逐行核对。
- D2D 巨图的 overview / tile 选择以 ADR #82 后记和 `paint_view` 的注释为准。`tile::detail_level` 的实现本轮没有读。
