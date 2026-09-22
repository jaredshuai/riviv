# smoke/ — 可复现冒烟脚本入库

本目录存放**随仓库入库**的冒烟测试脚本(GUI 无法自动验收,见 AGENTS.md 的冒烟
测试纪律)。先例:`installer/smoke26-assoc.ps1`(#26 关联/安装冒烟,PR 处置时
入库)。回归矩阵的其余脚本仍散落在 `%TEMP%\riviv-test\`(未纳入版本控制);
**#94 裁决(2026-09-21)**:smoke9 timing 族(timing/timing2/firstframe/ab
四套)判 **wontfix 退役**——它们是 #9 mip 决策期的一次性 A/B 设计评估工具,
对照臂指向已删除的旧 worktree(`riviv-nomip` / `riviv-master-ref`),作为
回归门无对照可造(重建 #9 时代基线跑今天的 master,任何差异都不可归因),
其保证已被库内 golden/oracle 断言(smoke80 S3、smoke81 S2、golden90 语料)
更强地覆盖;「降级分片」故障注入判 **wontfix**——GDI 臂 512px 分片降级路径
(#81)本无可测注入通道,且随 #90 GDI 删除而消亡。其余 %TEMP% 脚本维持
不入库,不作为回归门。

## smoke98-apng.ps1(#98 APNG 动画接入)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File smoke\smoke98-apng.ps1 [-Exe <path>]
```

场景(18+ 检查;R2 起退出码三分:**0 全绿 / 1 断言失败 / 2 必需门未跑完**,
失败与门不全都保留 stage 作证据):

- **S0** fixture 自检:chunk 结构自走查(acTL-before-IDAT/fcTL 门每个图像/
  共享序列号 0-4)+ GDI+ 独立解码首帧(红)。
- **S1** 文件打开 → 状态栏帧计数 part 存活(len==5,即 "n / 3";跨进程
  SB_GETTEXT 本机被 UIPI 拦返空,#40 坑,长度法=SB_GETTEXTLENGTH 于
  SB_GETPARTS count-2)+ WM_CLOSE 退出 0。
- **S2** 二实例转发恶意 fcTL(首 fcTL 宽度越界+CRC 重算,构造期失败=
  任何帧前)→ 失败标题采纳/旧动画保留/失败文案差分断言(好载后 main
  part=0 基线)/无弹窗/退出 0。**转发前 sleep 900ms 越
  add_command_line_timeout=500ms 窗口**(#21 坑:窗口内落 ADD 模式)。
- **S3** `stdin:` cmd type 管道 → 独立窗口动画;**按 exe Path 选靶**
  (`Where-Object Path -eq $RunExe` 且恰好一个,开发者自用 riviv 豁免)。
- **S4(必需门)** 静态 PNG 三对 `-dump-viewport` 与 **master parity exe
  字节相等**:a) 纯 RGBA 1:1;b) 带 iCCP(#77 adobe-like profile,base64
  内嵌)且 ini icm=1——盖住 prepare_transform 面;c) palette+tRNS
  1200×1200——盖住 alpha 合成+缩小滤波档面。**parity exe 缺失 = exit 2
  非静默 skip**(R2-3):构建配方
  `git worktree add <tmp> <pre-#98 master sha>; cd <tmp>; cargo build --release;
  copy target\release\riviv.exe %TEMP%\riviv-98-parity\riviv-master.exe`。
- **S9** teardown:无条件清 stage ini(场景 WM_CLOSE 会回写)+ 残留
  staged 进程计入失败(先抓后杀)。

fixture 双源说明(R2-4):单测用 Rust `png` crate Encoder(dev-dep),
冒烟用 C# 手写字节(zlib 0x78 0x01+adler32+CRC32,smoke81 配方;含
APNG chunk 序列与 iCCP/PLTE/tRNS)——两套独立实现互为交叉验证,改动
APNG 解码断言语义时两处都要对齐。

## smoke82-tiles.ps1(#82 巨图 overview + LRU tile + VRAM 字节预算)

```powershell
powershell -NoProfile -File smokesmoke82-tiles.ps1 [-Exe <path>]
```

场景(46 检查;全部 D2D 断言附带「breadcrumb 在场」前置,防空 stderr 假绿):

- **S0** 夹具:900×600 渐变(内容公式可复算)、40000×256 带红带 banner、
  16777217×1 手写宽条、900×600 硬边(两条 1px 全高黑列:源 x=256 恰在
  `-tile 256` 网格边界、x=300 在块内)。
- **S1** `-tile` 诊断面:带参跑打统计行(level=0/tiles>0/uploads>0),无参跑
  无统计行(普通帧)。
- **S2 零接缝核心**:S2a 真 1:1(视口精确 1200×900 + WM_COMMAND 45)tiled
  vs untiled **整文件哈希相等** + 斜坡内容公式 ±2;S2b 缩小档 maxDelta≤1 +
  边界阶跃 tiled≤untiled+1(实测参考 0.299 vs 0.299);S2b-d 硬边黑列屏幕
  位置两 dump **逐值相等**(丢列/重列会移位或消失);S2b-e 硬边整帧
  maxDelta≤21(**高对比档在案数首次有断言看守**)。
- **S3** 硬件巨图 40000×256:fit→overview(level≥1/tiles=0/base>0/
  mip_builds≥1,红带 438..535 对理论 ±0);1:1→tile(level=0/tiles>0/
  base=0/uploads≥1,红带铺满)。
- **S4** WARP 2^24+1:overview 路径 + 统计行解析(gpu≤cap)+ dump 按夹具
  公式复算(±3)。
- **S5** 预算:每条捕获统计行 gpu/peak_gpu≤cap(≥12 行)。
- **S6**(记录项)`renderer=gdi` banner 走 GDI 巨图支(本票未删的逃生舱)。
- **S7** 证据纯度:15 条 D2D stderr 全扫,禁 `trying the gdi channel`。
- **S8** 单实例变焦 churn(1:1→fit→1:1):uploads>0 且 gpu/peak≤cap。
- **S9** 真压力:`-tile 4` 强制 33750 块 ≈624 MB 对 268 MB cap →
  evictions>0、peak 距 cap 10 KB(**强制诊断帧有意允许半覆盖**,脚本内
  注明;自然路径的不半覆盖由单测钉)。

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
  16777217×1 gdi+warp(自 #82 起 warp 侧断言与旧 gate 相反:D2D 自绘、无
  gate 行、无 gdi 交接,统计行报形态 S4j/S4j2);边界 census
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
  2^24+1 px wide because GDI+ refuses >65535 AND the device bound is a
  runtime `GetMaximumBitmapSize()` query - 16385 is NOT giant on machines
  where WARP reports 2^23 (measured value on the dev machine; since #82 the
  bound routes the frame to the overview/tile path, it no longer gates). The GIF (S6)
  is hand-built (2 frames, 10 s delays, uncompressed-LZW recipe) because
  this .NET's GDI+ `Encoder` lacks `FrameDelay`.
- Summary line `SMOKE80 RESULT: PASS=N FAIL=M SKIP=K`; `FAIL > 0` exits 1,
  and the stage dir with dump PNGs + stderr captures is kept as evidence.

### Scene table

| Scenario | Assertion | Notes |
| --- | --- | --- |
| S1a (gdi/d2d/warp/auto) | stderr breadcrumb `riviv: renderer=<req> backend=<eff>` per ini value | #94: exact-string disjunctions of the documented driver ladder — `warp` -> `d2d/warp` everywhere; `d2d` -> `d2d/hw`, or on a no-hardware host `backend=gdi` WITH its `falling back to gdi` line (a failed d2d goes straight to gdi, never warp); `auto` -> `d2d/hw` or `d2d/warp` (its retry ladder). No SKIP: both host shapes keep every wrong-backend regression failing |
| S1b | `renderer=frobnicate` -> `renderer=auto backend=` + `unrecognized renderer value` + `using auto` hints | invalid string value falls back to the default (auto since #81; gdi pre-#81) |
| S1c | missing key -> `renderer=auto backend=` | the default (auto since #81; gdi pre-#81) |
| S2 | warp dump channel: adopted image + WM_CLOSE -> PNG on disk, exit 0, dims == view client rect | |
| S3a-S3f | L0: the same 1:1 scene dumped through warp AND gdi | file bytes equal; non-white bbox == source rect at source size; every bbox pixel RGBA == source; both arms pixel-exact |
| S4a-S4c | resize chain: after SetWindowPos the dump dims follow the NEW viewport while the 1:1 bbox stays the source size | swapchain ResizeBuffers + target rebuild |
| S5a-S5d | giant path (#82 contract): NO gate line, NO gdi hand-off, NO gdi dump fallback; process alive; dump succeeds out of the D2D channel, exit 0; close-time stats line names the form (level>=1 or tiles>=1) | the D2D arm draws giants itself (overview level or tiles) — the #80 gate is retired |
| S6a-S6d | animation re-upload: dumps before/after `AnimationFrameStep` (cmd 100) differ; frame 0 = red, frame 1 = blue at 1:1 | frame_gen bump re-uploads |
| S7a-S7b | rotation re-upload: after `EditRotate90` (cmd 23) the bbox swaps 120x80 -> 80x120 and the content equals the source rotated 90 CW | rotate bumps frame_gen |
| S8a | minimized-start warp instance (the 0x0 iconic viewport - the documented dump-refusal shape - is restored via SW_SHOWNOACTIVATE before the dump) dumps the image (content-checked) at WM_CLOSE | #94 naming clarification: NOT a pure iconic dump. The subject is the dump channel's independence from the display pipeline - no Present, no WM_PAINT; whether Windows' initial iconic activation handed the instance the foreground is host-dependent and stays recorded in the evidence note, not asserted (fgHeldByRiviv=True observed on the dev machine) |
| S8b | gdi twin of S8 | SKIP by design (pre-existing background-paint gate, master-identical, smoke78 SKIP semantics); observed behavior recorded in the detail line |

Adjudicated (2026-09-19, commit 028abf7): S3b's original failure was the
letterbox ALPHA only - warp wrote A=255 (D2D `Clear`) while gdi left the
DIB-zeroed A=0 (GDI never writes the alpha byte; RGB was pixel-identical).
The dump contract is now the fully opaque viewport BOTH arms render, and
the gdi channel forces A=255 on readback - S3b asserts byte-identical
files and passes.

Adjudicated (2026-09-21, #94): S1a's `d2d/hw` expectations are now the
exact-string disjunctions of the documented driver ladder (scene table
above) - the former hard hw-only string false-FAILed correct behavior on
no-hardware hosts (RDP/VM: an explicit `d2d` degrades to gdi with its
init-failed stderr line, `auto` retries to warp), and a SKIP would have
discarded the arms that stay assertable there; neither SKIP nor a hard
string, the ladder itself is the assertion. S8a's label was clarified to
say what it actually pins (a minimized-start + no-activate-restore dump,
not a pure iconic dump; the foreground observation stays recorded - the
dev machine's run showed fgHeldByRiviv=True). The smoke9
timing family and the degradation-shard injection item are adjudicated in
the intro section above (both wontfix, with reasons). Post-#90 note: the
d2d->gdi arm of the S1a disjunction goes dead when #90 lands (a failed d2d
init fataled instead of degrading) - drop the arm from Test-S1Arms at
merge time; the auto->warp and d2d/hw arms stay.

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
