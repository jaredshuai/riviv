# smoke/ — 可复现冒烟脚本入库

本目录存放**随仓库入库**的冒烟测试脚本(GUI 无法自动验收,见 AGENTS.md 的冒烟
测试纪律)。先例:`installer/smoke26-assoc.ps1`(#26 关联/安装冒烟,PR 处置时
入库)。回归矩阵的其余脚本目前仍散落在 `%TEMP%\riviv-test\`(未纳入版本控
制),待 #80「后台进程显示闸」重访时再评估批量入库。

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
