# smoke/ — 可复现冒烟脚本入库

本目录存放**随仓库入库**的冒烟测试脚本(GUI 无法自动验收,见 AGENTS.md 的冒烟
测试纪律)。先例:`installer/smoke26-assoc.ps1`(#26 关联/安装冒烟,PR 处置时
入库)。回归矩阵的其余脚本目前仍散落在 `%TEMP%\riviv-test\`(未纳入版本控
制),待 #80「后台进程显示闸」重访时再评估批量入库。

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
