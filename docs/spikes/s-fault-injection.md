# 故障注入手段选型(device-loss ladder / 三层棘轮 / 后端迁移的活体验证)

- 票:#172(本 spike 为其 S0;#165 第五节待表态项,2026-09-30 用户拍板)
- 日期:2026-09-30
- 基线 exe:master `b302fec` 所建 `target/release/riviv.exe`(v0.2.0 同字节 6,801,920 B)
- 原始输出留档:`%TEMP%\riviv-faultinj-spike\`(driver.ps1 / probe2.ps1 / stderr*.log / RESULT.txt / evtlog-probe.ps1)

## 结论一句话

**可自动化的注入手段 = 应用内故障 seam(env `RIVIV_FAULT`,在分类边界把「真实成功」合成「失败」,下游 ladder/棘轮/重派生/重绘全真跑);真实 OS→HRESULT 链的确认留一项人工 runbook(物理按 Win+Ctrl+Shift+B)。** 真实触发在开发机上不可合成、不可无提权达成;VM 舞台需用户侧解锁,留作可选轨。

## 待验证面(为什么需要注入)

五条运行时容错路径的纯逻辑已穷举钉测(`failure_window`/`display_arm`/决策表),接线面(失败检测→分类→drain→重建/重派生→stderr 证据行)无自然触发环境,从未活体验证:

| # | 路径 | 代码锚点 |
|---|---|---|
| 1 | 单次设备损失→同型重建 | gpu.rs `enddraw_outcome`/`present` → window.rs `gpu_runtime_failure`(design §7 第一档) |
| 2 | 10s 内 3 次→永久 WARP(后端迁移) | window.rs:2050 `failure_window` Escalate;宽图走 `rederive_wide_masters` |
| 3 | `ac_surface_latched`(运行时 AcFace + 创建期两触发面) | window.rs:1384(创建)/1964(运行时 drain) |
| 4 | `wide_effect_latched`(legacy 宽 effect 连续 3 失败) | window.rs:1984 + `RederiveWide` |
| 5 | prepare 连续上传失败→空白帧+喂 ladder | gpu.rs:2531-2575 |

另:PR #159 QA7 引用的 `backend migrated to WARP …` stderr 行文已不存在于现行代码;迁移的可观测合同现为 `riviv: EndDraw failed (…) — device loss ladder` + 迁移后 `display-stage=… backend=warp` 行,票面已重锚定。

## 证据块(手段排除过程)

仪器化方法:stage 副本 exe + 宽 ICC fixture(p3.icc 由 `RIVIV_S156_ICC=… cargo test smoke156_write_p3_profile -- --ignored` 写出,6680 B;C# 写 iCCP PNG,抄 smoke156),stderr 重定向文件轮询,WM_CLOSE 关闭读退出码。每次启动基线健康:`display-stage=p3_to_scrgb … backend=hw` + `output-surface=ac-scrgb`(AC 臂活)。

1. **keybd_event VK 弦(Win+Ctrl+Shift+B,1+3 次)**:riviv 零反应(无损失行/无重建行);System 事件日志场景 B 窗口(19:00:07-13)零事件——场景 A 时刻(18:59:49)恰逢一次「退出新型待机」转换,系机器自身待机节律与和弦时刻巧合,不能作为触发证据。阳性对照(keybd_event 'C' 后 GetAsyncKeyState)=True,合成本身进了输入流。
2. **SendInput 扫描码弦**:初版 INPUT 结构体 32 B(x64 实为 40 B,union 含 MOUSEINPUT)致 SendInput 拒收(返回 0/4);修正 padding 后 **4/4 注入成功,riviv 仍零反应,System 事件日志窗口(19:03:50-19:04:30)为空**——该 shell 保留热键只认物理键盘,两条合成路线均被忽略(与社区同报一致)。
3. **TDR 框(蓄意挂 GPU 触发真重置)**:单发可行,但路径 2(3-in-10s)需 ~8s 内 3 次 TDR,逼近 Windows TDR-storm 防护(60s 内 5 次 → bugcheck 0x116),自主操作用户开发机有蓝屏尾部风险,弃。
4. **PnP 禁/启显示适配器**:需提权(会话实测非 admin,`Get-VM` 亦无权限)。
5. **Hyper-V VM 舞台**:vmms/vmcompute Running、HypervisorPresent=True,但会话不在 Hyper-V Administrators 组,`Get-VM` 拒绝;需用户侧解锁,留作可选轨(场景 choreography:VM 内跑 riviv + PowerShell Direct 从宿主 pnputil 禁/启适配器)。

## seam 设计要点(票 #172 实现)

- `src/fault.rs` 纯逻辑解析器:`RIVIV_FAULT=device=3,effect=4,ac_create=1,prepare=5`(逗号分隔 kind=count,容忍非法 token),启动 `init_from_env()` 读一次进 AtomicU8 余量;env 未设=全零=行为逐位不变(零扰动负控必测)。
- 注入点全在既有失败边界旁的「成功侧」:device→draw_pass EndDraw 成功后重分类 DEVICE_REMOVED(count=3 时三次重排毫秒级自然落进 10s 窗,无需计时);effect→effect_pass 按当前臂合成失败(AcFace 一次 latch,折回 legacy 宽后继续消耗即 WideLegacy×3,一次 env 钉完整三层棘轮);ac_create→`gpu::create` AC 臂 SetColorspace1 前合成失败;prepare→prepare 合成上传失败。
- **落地偏差注记(2026-09-30 外部评审 P3 处置,实现时相对本节前移)**:device 实际落在 **draw_pass 入口**(BeginDraw 之前)——比「EndDraw 成功后」更优:无 mid-bracket 状态需恢复,分类路径(enddraw_outcome)同一;ac_create 实际落在 **QI(cast IDXGISwapChain3)之前**——比「SetColorSpace1 前」更早一层,同走创建期 latch;prepare 落在 **stamp(frame_space 写入)之后的失败区头部**(评审 P2:真实失败必然已 stamp,合成失败必须等价,否则陈旧 stamp 会喂出虚假 wiring-bug 拒绝——S7s 场景双向钉)。
- 语义边界:seam 不测 OS→HRESULT plumbing(DEVICE_REMOVED/RESET/DRIVER_INTERNAL_ERROR 三码由文档+`is_device_loss` 分类表覆盖);测的是我们自己的全部接线与恢复行为。

## 决策影响

- #172 以 seam 为自动化验收手段;真实链确认留人工 runbook 项(物理按弦,~1 分钟,不阻塞)。
- 本机(jared 开发机)结论:合成输入无法触发保留热键;此后任何需要「真实驱动重置」的验证一律走人工项或 VM 轨,不再重复合成尝试。
