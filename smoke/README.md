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

## smoke126-renderer.ps1(#126 `-renderer` sticky 开关 + dump output_gen)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File smoke\smoke126-renderer.ps1 [-Exe <path>]
```

场景(17 检查;**S0 前置门**:任何非 staged 路径的 riviv 进程在场即
`FATAL foreign riviv running` 退出 3——真窗口会劫持单实例转发,#109 教训;
退出码 0 全绿 / 1 断言失败,失败保留 stage 作证据):

- **S1** force-warp 全链:`-renderer warp` + `-dump-viewport` → stderr 精确
  `riviv: renderer=warp backend=d2d/warp`;PNG 产出且尺寸=视口;
  `riviv: dump-viewport output_gen=1`;`d2d max_bitmap` 行在场。S1b 双向
  override:ini=warp + `-renderer auto` → `renderer=auto backend=d2d/hw`;
  ini=d2d + `-renderer warp` → `renderer=warp backend=d2d/warp`。
- **S2**(一致性表;**#130 起前提翻转**)同 1:1(NEAREST 纯拷贝)场景、
  同校准 900×600 视口,ini=auto(hw)与 `-renderer warp` 两臂 dump
  **整文件 SHA256 不等**——hw 臂过 #130 显示段 ColorManagement 变换、
  warp 臂不过(决策表硬互斥),字节相等反而=显示段静默失效;hw 像素
  **正确性**由 smoke130 S3 的 mscms 参考容差断言承担,本节只钉「分歧
  在场 + 两臂可读」。(#130 之前两臂逐字节相等,是 force-warp 跨机器
  色彩断言的前提;该确定性在 warp 侧由 smoke130 S2c 保留。)
- **S3** 不写 ini:staged `renderer=auto` 跑 `-renderer warp` 会话,关闭后
  ini 仍 `renderer=auto` 且无 `renderer=warp` 泄漏(override 只存内存)。
- **S4** 单实例 handoff=面包屑+忽略:A(无开关)先起,B 以
  `<img> -renderer warp` 起 → B 秒退 0(转发),A stderr 精确
  `riviv: -renderer warp ignored - device already built (d2d/hw)` 且原
  startup 行不变(与 -dump-viewport「照常武装」的关键差异)。
- **S5** 悬空/坏值=不 armed:`-renderer`(尾)/`-renderer oops` → 无 usage
  弹窗(弹窗会阻塞 WM_CLOSE→退出码 -1)、ini 值生效(auto/hw)、无 ignore
  面包屑、退出 0。

工程注:Read-Err 以 FileShare ReadWrite 打开(S4 需在 A 存活时轮询其
stderr 重定向文件——smoke80 只在退出后读);S4 在 B 的 Start-Riv 覆盖
`$script:RawH` 前保存 A 的活句柄、关 A 前还原(Close-Main 从该句柄读
退出码)。

## smoke130-stage.ps1(#130 M8-3 静态 gpu_effect 显示段)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File smoke\smoke130-stage.ps1 [-Exe <path>]
```

场景(11 检查;S0 前置门同 smoke126 退出 3;**机器钉**:本机显示
profile=TPLCD_8BAF_AdobeRGB.icm(Custom 类)才全绿——sRGB 等价 profile
的机器 S1 的 gpu_effect 行不会出现,该脚本诚实 FAIL 而非静默 skip):

- **S1** hw 臂(`-renderer d2d` + dump):stderr
  `riviv: display-stage=gpu_effect profile=<name> backend=hw`(在
  `renderer=` 行之前),profile 名解析存变量并校验存在于
  `%windir%\System32\spool\drivers\color`;PNG=视口尺寸;
  `output_gen=1`。
- **S2** warp 臂互斥:`display-stage=none profile=<同名> backend=warp`
  + gen=1;**S2c warp 确定性**:两次 warp dump 整文件 SHA256 相等
  (#126 时代的跨后端确定性在 warp 侧的保留)。
- **S3**(P4 容差断言,正确性核心)**warp dump(S2,未变换 sRGB 原像)
  整幅过 mscms CPU 参考**(C# P/Invoke `OpenColorProfileW`
  (MEMBUFFER)+`CreateMultiProfileTransform`(RelativeColorimetric+Best)
  +`TranslateBitmapBits`,BM_xRGBQUADS 双侧零 swizzle,常量与
  src/icm.rs 所链一致)得参考图;hw dump 逐像素 max 通道差 ≤ **24**
  (首标定实测 **10**)+ 差异像素数 > 0(防双臂恒等假通过)。
- **S4** 棘轮契约(形态断言,不强制触发):hw stderr **无**
  `display effect failure`(防「构建失败静默降级直绘、gen 照打」假绿;
  触发路径由单测+面包屑契约覆盖)。

工程注:S3 的 profile 文件路径从 S1 的 stderr 行解析(与 riviv 判据
同源);参考变换对整幅 dump(含 letterbox 边距)做——边距也过变换,
整幅比对即端到端。#130 顺带前提修复:smoke82 品红边距换黑(中性轴
在相对色码下不变,S2a/S2b 检测恢复精确),smoke98 颜色窗/parity 臂改
warp(断言语义渲染器无关)。

## smoke132-stage.ps1(#132 M8-4 profile 热重载:新鲜度通道幂等)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File smoke\smoke132-stage.ps1 [-Exe <path>]
```

场景(5 检查;S0 前置门同 smoke130 退出 3;机器钉同 smoke130——本机
Custom profile 才有 hw 臂 gpu_effect 行)。**热重载的「真切换」路径不
冒烟**(改机器显示 profile 侵入性大):其证据=issue #132 设计评论的
P5 探针(getter 即时翻转 + 写侧零广播),人工验收走 QA 清单;本脚本
钉的是**判据输入不变时新鲜度通道必须零动作**(恰好一条 display-stage
面包屑 + `output_gen=1` + 棘轮无噪音):

- **S1** hw 臂:会话中向主窗投递合成 `WM_DISPLAYCHANGE`(0x7E)→ 面包屑
  恰 1 条、gen=1(事件臂对未变名字 no-op)。
- **S2** hw 臂闲置 5s(≥2 个 2000ms 计时器周期)→ 同上 + 无
  `display effect failure`(计时器臂不刷屏、不喂棘轮)。
- **S3** hw 臂同屏平移(`SetWindowPos(+80,+80)`,SWP_NOSIZE|
  NOZORDER|NOACTIVATE)→ 窗口实测位移 +80(投递证)且 **ini 落
  x=140 y=140**(WM_MOVE 臂确实跑了)但面包屑仍 1 条、gen=1(同屏
  监视器比对不重建立)。
- **S4** warp 臂复跑 S1(`display-stage=none` 前提)。

工程注:S1/S4 的合成消息走 PostMessage(handler 异步在消息泵消费,
固定等 1.5s);S3 用 ini 落盘证 WM_MOVE 投递而非仅凭窗口位移(config
回写在 WM_CLOSE,场景间 ini 重写防串染)。

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
powershell -NoProfile -File smoke\smoke82-tiles.ps1 [-Exe <path>]
```

场景(43 检查;全部 D2D 断言附带「breadcrumb 在场」前置,防空 stderr 假绿;
#90 起关机统计行为 12 字段——`display=` 记账字段随 GDI 臂删除):

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
- **S6** 已随 **#90 GDI 渲染主线删除**退役:`renderer=gdi` 现映射 auto,
  巨图 relief 路径不存在;编号保留,S7/S8/S9 编号不变。
- **S7** 证据纯度:15 条 D2D stderr 全扫,禁 `trying the gdi channel`
  (#90 后为恒真负向守卫,保留)。
- **S8** 单实例变焦 churn(1:1→fit→1:1):uploads>0 且 gpu/peak≤cap。
- **S9** 真压力:`-tile 4` 强制 33750 块 ≈624 MB 对 268 MB cap →
  evictions>0、peak 距 cap 10 KB(**强制诊断帧有意允许半覆盖**,脚本内
  注明;自然路径的不半覆盖由单测钉)。

## smoke81-filters.ps1(#81 滤波映射全表 + mip 退役 + 默认 auto)

驱动 #81 的三面:D2D shrink Linear 档 HIGH_QUALITY_CUBIC、`renderer` 默认翻
auto。#90 删除 GDI 渲染主线后,原 warp-vs-gdi 双臂对照全部改判:字节域
(NEAREST 域)对照冻结 golden 语料,滤波域改 warp 双跑确定性断言。运行:

```powershell
powershell -NoProfile -File smoke\smoke81-filters.ps1 [-Exe <path>] [-Regolden] [-Regolden90]
```

场景(54 检查;golden81 冻结于 `smoke/golden81/`,golden90 见下段):

- **S1** renderer 键:missing key→`renderer=auto backend=`;frobnicate→
  `unrecognized`+`using auto`;`renderer=gdi`→**#90 迁移断言**:exit 0,
  stderr 含 `renderer=gdi was removed, using auto` 且含
  `renderer=auto backend=d2d/`,且永不出现 `backend=gdi`。
- **S2(L1)** 整数放大 2×/3×/4×:`fill_window=1`+子窗精确 k× 校准
  (SetWindowPos 循环);仅 warp 臂(gdi 双臂随 #90 删),dump 与
  `src[x/k,y/k]` 复制模型逐像素等,并与 golden81 **字节比对**
  (`-Regolden` 重冻结,**失败场次拒写**——预审 3 P3-2);magenta 边距
  场景边界 ±0px。含 padded(192×128 源默认 fit)变体。
- **S3(L2)滤波域**(非字节域,gdi 交叉 oracle 随臂删除):mag=1 LINEAR、
  shrink=1 CUBIC、shrink=0 NEAREST 各改 **warp 双跑字节相等**(确定性是
  滤波域仅存守卫),旧跨臂 MAE/相位统计降为记录档;shrink=1 保留无爆点
  断言。
- **S4 巨图**:banner 40000×256 仅 warp 内容断言(warp 上传无 gate、条带
  均值/单调/接缝);极端 16777217×1 warp 侧断言无 gate、无 gdi 交接、
  统计行报形态(S4j/S4j2)+ 渐变内容。原 `renderer=gdi` 巨图内容跑与
  2^22 relief 边界 census(S4g–S4i、S4m/n/p/q)随 GDI 臂退役(#90:GiantRelief
  与 relief 路径已删,mag ≥2^22 KNOWN GAP 一并消失)。
- **S5** exit-2:dump 到不存在目录→exit 2+stderr。
- **S6** 帧时间基线(记录档,仅 warp,gdi 行随臂删除)。
- **S10 golden90 语料**:见下段。

### golden90 语料(#90)

`smoke/golden90/` 是 **#90 删除前从 GDI 臂冻结的 5 张参考 dump**
(冻结提交 5996944;域 = 双臂已证字节相等的 NEAREST 域:1:1 精确、
整数放大 k=2/k=3、两档底色 letterbox、旋转 90° 后 1:1)。冻结时每场景
gdi dump == warp dump 逐字节,故删除后 **warp 臂 dump 必须逐字节复现
golden** = 「删除未改变输出」的跨臂 oracle:S10 对 5 场景各跑 warp 一枪硬断
言;同场景再跑 `renderer=auto`(硬件)一枪,5/5 全中则升级为硬断言,有差异
则保持记录档(报告字节差数,不强行绿)。**rot90 场景的 EditRotate90 走
shell 动词会改写磁盘上的夹具文件**,所以 S10 每一枪前都重新生成夹具
(HashSource+SaveRgba)——跨枪共用一个夹具会毒化第二枪。**历史(已闭环)**
:s5-one2one 首轮冻结时夹具被更早场景的两次 rotate 动词转了 180°(freeze
脚本共享夹具路径的残留;bbox 对称免疫、双臂同读污染文件使字节 oracle 免疫),
master `be5097b` 以干净夹具重冻后 S10b-s5 通过、硬件臂 5/5 升级为硬断言。
S10 的 rot180 自诊断分支保留为**未来冻结事故的守卫**(dump == 正立模型、
golden == rot180 模型→该帧语料被污染、重新冻结即可,渲染器无错)。
`-Regolden90` 从 warp 臂重冻结(失败场次拒写;文件名保留 `-gdi` 后缀 =
语料身份,GDI 生成臂已亡、重冻内容自 warp——诚实记录在此)。`-Regolden`
(golden81)的失败门只盖到 S2 段尾,S3/S4/S10 后失败仍可能改写 golden81
——已知局限在案,勿在非绿跑上带此开关。

S4 的 WriteWidePngGrad 手写 PNG 配方沿 smoke80 的 WriteWidePng(>65535 宽
GDI+ 拒建),渐变列用于内容断言。

## smoke80-d2d.ps1 (#80 D2D renderer + -dump-viewport)

Drives the #80 renderer stack (`renderer = auto|d2d|warp`; since #90
`renderer=gdi` maps to auto with a migration note on stderr - the GDI render
arm is gone) and the `-dump-viewport <path>` WM_CLOSE readback channel. Run:

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
| S1a (d2d/warp/auto) | stderr breadcrumb `riviv: renderer=<req> backend=<eff>` per ini value | d2d/auto expect `d2d/hw` where hardware D3D11 exists; `warp` expects `d2d/warp` |
| S1a-gdi | `renderer=gdi` -> exit 0, `riviv: renderer=gdi was removed, using auto` AND `riviv: renderer=auto backend=d2d/`, and `backend=gdi` never appears | the #90 migration (not the "unrecognized value" fallback - the word is legal, its arm is gone) |
| S1b | `renderer=frobnicate` -> `renderer=auto backend=` + `unrecognized renderer value` + `using auto` hints | invalid string value falls back to the default (auto since #81; gdi pre-#81) |
| S1c | missing key -> `renderer=auto backend=` | the default (auto since #81; gdi pre-#81) |
| S2 | warp dump channel: adopted image + WM_CLOSE -> PNG on disk, exit 0, dims == view client rect | |
| S3a-S3e | L0 byte-exactness vs the frozen golden90 corpus (#90 form): the golden90 s1 1:1 scene dumped through warp must BYTE-EQUAL `smoke/golden90/s1-one2one-gdi.png` (the GDI-arm reference, frozen where warp == gdi was proven); calibrated viewport exactly 256x192; letterbox margins pure magenta; image box pixel-exact vs the 96x64 source at (80,64) | the old warp-vs-gdi twin dump died with the arm; the frozen golden is the cross-arm oracle now |
| S4a-S4c | resize chain: after SetWindowPos the dump dims follow the NEW viewport while the 1:1 bbox stays the source size | swapchain ResizeBuffers + target rebuild |
| S5a-S5d | giant path (#82 contract): NO gate line, NO gdi hand-off, NO gdi dump fallback, NO `backend=gdi`; process alive; dump succeeds out of the D2D channel, exit 0; close-time stats line names the form (level>=1 or tiles>=1) | the D2D arm draws giants itself (overview level or tiles) — the #80 gate is retired |
| S6a-S6d | animation re-upload: dumps before/after `AnimationFrameStep` (cmd 100) differ; frame 0 = red, frame 1 = blue at 1:1 | frame_gen bump re-uploads |
| S7a-S7b | rotation re-upload: after `EditRotate90` (cmd 23) the bbox swaps 120x80 -> 80x120 and the content equals the source rotated 90 CW | rotate bumps frame_gen |
| S8a | minimized-start warp instance dumps the image (content-checked) at WM_CLOSE | the D2D dump renders from the CPU master inside the dump call - no Present, no WM_PAINT dependency; the former gdi twin is retired with the arm (#90) |

Historical note (2026-09-19, commit 028abf7): the pre-#90 S3 compared the
warp and gdi dumps byte-for-byte; its one adjudicated failure was letterbox
ALPHA only (warp wrote A=255 via the D2D `Clear`, gdi left the DIB-zeroed
A=0; the gdi channel forced A=255 on readback). With the GDI arm deleted
(#90) that twin comparison is replaced by the frozen golden90 byte oracle
above.

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
