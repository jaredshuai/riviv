# smoke/ — 可复现冒烟脚本入库

本目录存放**随仓库入库**的冒烟测试脚本(GUI 无法自动验收,见 AGENTS.md 的冒烟
测试纪律)。先例:`installer/smoke26-assoc.ps1`(#26 关联/安装冒烟,PR 处置时
入库)。回归矩阵的其余脚本目前仍散落在 `%TEMP%\riviv-test\`(未纳入版本控
制),待 #80「后台进程显示闸」重访时再评估批量入库。

## smoke81-filters.ps1(#81 滤波映射全表 + mip 退役 + 默认 auto)

驱动 #81 的三面:D2D shrink Linear 档 HIGH_QUALITY_CUBIC、GDI 臂 face 直绘
(mip 链退役后的巨图 relief 路径)、`renderer` 默认翻 auto。运行:

```powershell
powershell -NoProfile -File smoke\smoke81-filters.ps1 [-Exe <path>] [-Regolden]
```

场景(71 检查;golden 冻结于 `smoke/golden81/`,GDI 臂生成,票面原文):

- **S1** 默认翻:missing key→`renderer=auto backend=`;frobnicate→
  `unrecognized`+`using auto`;`renderer=gdi` 逃生键。
- **S2(L1)** 整数放大 2×/3×/4×:`fill_window=1`+子窗精确 k× 校准
  (SetWindowPos 循环);warp/gdi dump 文件字节+解码像素双等,且与
  `src[x/k,y/k]` 复制模型逐像素等;magenta 边距场景边界 ±0px;四 golden
  字节比对(`-Regolden` 重冻结,**失败场次拒写**——预审 3 P3-2)。
- **S3(L2)**:mag=1 LINEAR 平滑区 MAE≤2;shrink=1 CUBIC vs HALFTONE 真实
  差异记录;shrink=0 整数比 2× warp(NEAREST)/gdi(COLORONCOLOR)**字节相等**。
- **S4 巨图**:banner 40000×256 双臂内容断言(warp 上传无 gate);极端
  16777217×1 gdi+warp(warp 侧断 gate 行+GDI 通道内容);边界 census
  4,000,000/2^22/2^23/6,291,456×1(2^22 起走 relief 两级路径,接缝扫描;
  6,291,456 = 非 2 幂倍数点,钉 relief_divisor 离开 2 幂格点)。
- **S5** exit-2:dump 到不存在目录→exit 2+stderr。
- **S6** 帧时间基线(记录档,数字存档见 issue #81)。

S4 的 WriteWidePngGrad 手写 PNG 配方沿 smoke80 的 WriteWidePng(>65535 宽
GDI+ 拒建),渐变列用于内容断言。

## smoke80-d2d.ps1 (#80 D2D renderer + -dump-viewport)

Drives the #80 renderer stack (`renderer = auto|d2d|warp|gdi`) and the
`-dump-viewport <path>` WM_CLOSE readback channel. Run:

```text
powershell -ExecutionPolicy Bypass -File smoke\smoke80-d2d.ps1
powershell -ExecutionPolicy Bypass -File smoke\smoke80-d2d.ps1 -Exe <other build>
```

- ASCII-only PS 5.1 script (smoke78/79 discipline). Every instance runs from
  a staged exe copy in `%TEMP%\riviv-80-smoke`; the staged ini is deleted
  before EVERY launch (WM_CLOSE writes config back, so a leftover renderer
  key would fake cross-scenario regressions). Close is always WM_CLOSE
  (PostMessage to the owner): taskkill would skip the dump entirely.
- The probe is `SetProcessDPIAware` (200% dev machine, smoke79 lesson).
- stderr is captured per launch (`Start-Process -RedirectStandardError`);
  the startup breadcrumb `riviv: renderer=<req> backend=<eff>` is the S1
  assertion channel. Exit codes are read via `GetExitCodeProcess` on a raw
  handle captured while the process is alive: PS 5.1 `Start-Process
  -PassThru` objects lose `.Handle`/`.ExitCode` once the process exits.
- Fixtures are built in-script: the giant-frame PNG (S5) is hand-written at
  2^24+1 px wide because GDI+ refuses >65535 AND the gate threshold is a
  runtime `GetMaximumBitmapSize()` query - 16385 is NOT giant on machines
  where WARP reports 2^23 (measured value on the dev machine). The GIF (S6)
  is hand-built (2 frames, 10 s delays, uncompressed-LZW recipe) because
  this .NET's GDI+ `Encoder` lacks `FrameDelay`.
- Summary line `SMOKE80 RESULT: PASS=N FAIL=M SKIP=K`; `FAIL > 0` exits 1,
  and the stage dir with dump PNGs + stderr captures is kept as evidence.

### Scene table

| Scenario | Assertion | Notes |
| --- | --- | --- |
| S1a (gdi/d2d/warp/auto) | stderr breadcrumb `riviv: renderer=<req> backend=<eff>` per ini value | d2d/auto expect `d2d/hw` where hardware D3D11 exists; `warp` expects `d2d/warp` |
| S1b | `renderer=frobnicate` -> `renderer=auto backend=` + `unrecognized renderer value` + `using auto` hints | invalid string value falls back to the default (auto since #81; gdi pre-#81) |
| S1c | missing key -> `renderer=auto backend=` | the default (auto since #81; gdi pre-#81) |
| S2 | warp dump channel: adopted image + WM_CLOSE -> PNG on disk, exit 0, dims == view client rect | |
| S3a-S3f | L0: the same 1:1 scene dumped through warp AND gdi | file bytes equal; non-white bbox == source rect at source size; every bbox pixel RGBA == source; both arms pixel-exact |
| S4a-S4c | resize chain: after SetWindowPos the dump dims follow the NEW viewport while the 1:1 bbox stays the source size | swapchain ResizeBuffers + target rebuild |
| S5a-S5c | giant gate: `exceeds the D2D max bitmap ... rendering it via gdi` stderr line after the startup breadcrumb; process alive; dump still succeeds, exit 0 | GDI fallback channel (paint gate tears the stack down) |
| S6a-S6d | animation re-upload: dumps before/after `AnimationFrameStep` (cmd 100) differ; frame 0 = red, frame 1 = blue at 1:1 | frame_gen bump re-uploads |
| S7a-S7b | rotation re-upload: after `EditRotate90` (cmd 23) the bbox swaps 120x80 -> 80x120 and the content equals the source rotated 90 CW | rotate bumps frame_gen |
| S8a | minimized-start warp instance dumps the image (content-checked) at WM_CLOSE | the D2D dump renders from the CPU master inside the dump call - no Present, no WM_PAINT dependency |
| S8b | gdi twin of S8 | SKIP by design (pre-existing background-paint gate, master-identical, smoke78 SKIP semantics); observed behavior recorded in the detail line |

Adjudicated (2026-09-19, commit 028abf7): S3b's original failure was the
letterbox ALPHA only - warp wrote A=255 (D2D `Clear`) while gdi left the
DIB-zeroed A=0 (GDI never writes the alpha byte; RGB was pixel-identical).
The dump contract is now the fully opaque viewport BOTH arms render, and
the gdi channel forces A=255 on readback - S3b asserts byte-identical
files and passes.

## smoke79-dpi.ps1(#79 PerMonitorV2 DPI,专项)

验证 DPI manifest 生效与 `WM_DPICHANGED` 布局链。运行方式:

```text
powershell -ExecutionPolicy Bypass -File smoke\smoke79-dpi.ps1
powershell -ExecutionPolicy Bypass -File smoke\smoke79-dpi.ps1 -Exe <其他构建路径>
```

- 被测 exe 复制到 `%TEMP%\riviv-79-smoke` 暂存副本上跑(该目录同时生成
  red512.png 断言素材),退出即清理,ini 永不落在真实构建旁。
- **探针进程自身先 `SetProcessDPIAware()`**:本机 200% 缩放,unaware 探针的
  几何读数会被 DPI 虚拟化折半,断言全错(2026-09-19 diag79 实证)。
- **合成 `WM_DPICHANGED` 必须用 `SendMessage`**:`PostMessage` 对该消息被 OS
  拒收(返回 FALSE,同日实证);SendMessage 跨线程投递由目标线程执行,建议
  矩形指针在调用期天然有效。
- 汇总行 `SMOKE79 RESULT: PASS=N FAIL=M SKIP=K`;`FAIL > 0` 时退出码 1。

### 场景表

| 场景 | 断言 | 说明 |
| --- | --- | --- |
| S1.1–S1.3 | manifest 读回 | exe 资源明文 XML 含 `PerMonitorV2`、`name="riviv"`、SMI/2016 命名空间(防空 manifest 静默回归成 unaware) |
| S1.4 | 窗口出现 | FindWindowW 空标题须 `[NullString]::Value` |
| S2.1 | 活窗上下文 = PMv2 | `GetWindowDpiAwarenessContext` 句柄是不透明值,必须 `AreDpiAwarenessContextsEqual` 比,不能拿 -4 直比 |
| S2.2 | 窗口 DPI = 所在显示器有效 DPI | `GetDpiForWindow` 对 `GetDpiForMonitor`(per-monitor 语义本体) |
| S3.1–S3.5 | 前置状态 | riviv_view/riviv_rebar 子窗存在;图像已渲染(PrintWindow flag2);视口铺满 chrome 之上;条带高 = `controls_height(系统 DPI)` |
| S3.6–S3.11 | 合成 DPI 变更链 | SendMessage 投 144-DPI 建议矩形(1.5x 居中)→ 主窗逐字采纳 → #78 dock 链重跑(视口铺满新客户区、chrome 不变、条带仍系统 DPI)→ 视口仍 z 序最底(**`GW_HWNDNEXT` 才是「下方」,`GW_HWNDPREV` 是「上方」**)→ 图像仍渲染 → 再投 96-DPI 原矩形恢复 |
| S4.1 | 最大化态忽略建议矩形 | SW_MAXIMIZE 后投「最大化矩形×1.5」诱饵 → IsZoomed 保持且 rect 逐值不变(DPI 比例建议会把最大化窗缩离工作区,预审三 P2 的回归位) |
| S4.2 | 全屏态忽略建议矩形 | WM_COMMAND 35 进全屏 → 投任意建议矩形 → rect 恒等于覆盖矩形(monitor 覆盖权威,设计 §3/F3 的真链路背书) |
| S3.3/S3.10/S3.12 | 渲染断言 | 依赖前台激活解除显示闸;闸未解除时输出 SKIP(同 smoke78 的 SKIP 语义),几何断言不受影响硬跑 |

## smoke78-view-child.ps1(#78 视口子 HWND,PR #86 专项)

驱动 `riviv_view` 子窗的结构、消息路由与生命周期。运行方式:

```text
powershell -ExecutionPolicy Bypass -File smoke\smoke78-view-child.ps1
powershell -ExecutionPolicy Bypass -File smoke\smoke78-view-child.ps1 -Exe <其他构建路径>
```

- 需 Windows PowerShell 5.1(脚本自身 ASCII/注释编码兼容 5.1 的 ANSI 读取)。
- 真输入场景(键鼠注入、前台像素采样)只需普通前台权限,与管理员无关。
- 被测 exe 默认取 `target\release\riviv.exe`(经 `-Exe` 换构建);实例一律
  运行在 `%TEMP%\riviv-78-smoke` 的暂存副本上,ini 分阶段写入,退出即清理,
  强杀进程(不走 WM_CLOSE 回写配置)。
- 汇总行 `SMOKE78 DONE pass=N fail=M`;`fail > 0` 时退出码 1。

### 场景表

| 场景 | 断言 | 说明 |
| --- | --- | --- |
| S1a | `riviv_view` 子窗存在 | 枚举主窗子窗,按类名定位 |
| S1b | 锚在客户区原点、宽度铺满 | GetWindowRect 对比客户区原点 |
| S1c | 子窗底边与 chrome 顶边无缝 | 视口 → rebar → 状态栏自下而上堆叠 |
| S2 | 无 chrome 变体 | `show_menu/show_status/show_controls` 全关:唯一子窗即 `riviv_view`,仍锚原点 |
| S3 | 右键菜单经子窗产出 | `WM_RBUTTONUP` 直投子窗 → DefWindowProc 产 `WM_CONTEXTMENU` → 弹出菜单;并验证可关闭(轮询 + 真实 ESC 兜底) |
| S4 | 拖放转发 | 手造 HDROP,`WM_DROPFILES` 直投子窗 → owner 采纳请求(标题换为 blue.png) |
| S5a/S5b | 双击进出全屏 | 双击子窗:子窗覆盖全屏;再双击:缩回窗口 rect |
| S7 | 最小化/恢复后内容存活 | 恢复后采样视口内像素仍为 blue |
| S6 | 滚轮直投子窗缩放 | 角落 letterbox → 图像;依赖 S7 的显示存活 |
| S8a–S8f | 子窗路由输入扫盲 | L/M/R/X 按钮族直投子窗;点击动作暂改为导航(4/2),效果经标题观测,免受显示闸干扰:S8a 左键下一张、S8b 中键隐/显光标、S8c 右键上一张、S8d 右双键导航、S8e action 2 下 `WM_RBUTTONUP` 无菜单、S8f XBUTTON1 导航 |

### SKIP 语义

S6/S7 依赖**显示存活**(视口里真的画出了图)。进程从未被前台激活时,Windows
存在「后台进程显示闸」——不为它产出 WM_PAINT,像素采样读到旧表面。该闸在
master 上同形预存(probe78-bg.ps1 已验证与 master 行为一致),非 #78 引入;
闸未解除时 S6/S7 输出 **SKIP,属预期而非失败**。#78 相关的结构/路由场景
(S1–S5、S8)全部硬断言,不经过该闸。

### 坐标基准

#78 的核心恒等式:chrome 仅底部停靠,子窗锚定主窗客户区原点,因此**子窗本地
坐标 ≡ 主窗客户区坐标**——发给子窗的鼠标消息 lParam 无需任何换算,直接等值
于主窗客户坐标(脚本中的 `lpClient`/`lpDown` 均按此构造)。屏幕采样点由
`ClientToScreen(主窗)` 推出。
