# Common-Controls v6 chrome 主题化 开题 spike

- 票:#227(本 spike 为其唯一交付;实现票在拍板后另立)
- 日期:2026-10-24
- 基线:master `ae979ba`(v0.7.0 发布后收官态);**arm A** = 发版态 exe 8,523,264 B(manifest 无 v6 依赖,comctl32 5.82 活跃);**arm B** = `ae979ba` + manifest v6 依赖块(`%TEMP%\riviv-s227` worktree 探针臂),8,523,776 B(+512 B 纯资源增量);双臂唯一源差 = `res/riviv.manifest` 一个 `<dependency>` 块(镜像上游 `c-original/res/voidImageViewer.Manifest:5-9` 逐字)
- 证据性质:激活探针/dumpbin/视觉矩阵/度量/行为全为本会话亲跑(单子代理严格串行执行,主会话复核 summary+PNG+像素统计);**B 臂 PNG 的 AI 目验通道失败(400 三连,CDN 侧疑损,A 臂同形 URL 成功)——如实记录;B 臂「v6 主题化」定性改由激活证据 + 像素差分 + 色板统计 + A 臂独立目验三角支撑**

## 结论一句话

v6 激活 = **manifest-only 一刀**:`InitCommonControlsEx` 早在 #79 之前就以上游同款旗标在场(window.rs:9986-9993,`ICC_STANDARD_CLASSES|ICC_BAR_CLASSES|ICC_WIN95_CLASSES` = viv.c:5241 一字不差),comctl32.dll 本就在静态导入表 → 加依赖块即激活,零代码零新 DLL 静态导入(dumpbin 双臂 26 原始导入行逐行 diff 零差异)+512 B;视觉差异面 = comctl32 拥有的全部控件面(工具栏/状态栏/playlist ListView/五个手造对话框之四 = Options/JumpTo/CustomRate/Everything——Rename 未入矩阵,边界 7——加 About 按钮,10 面窗寸/布局逐项全同、纯换皮——唯一例外 = playlist 行高 25→28px,控件内项度量,窗布局不动),OS 自绘面(菜单栏/上下文菜单/通用文件对话框)逐字节不变;31 项度量唯一漂移即该项;行为探针双臂全同;**合成直发探针(阴性观察):WM_DPICHANGED 直发子控件双臂零反应,但该探针不能证真亦不能证伪真实迁移行为 → 「v6 自带 chrome per-monitor 缩放」收益不计入采纳理由,v6 按纯视觉决策票裁**。实测口径 = 本机 Windows 11 26100、200% 单屏(地板覆盖见边界 6)。

## 为什么做

1. README Differences(:154)在案 known difference + follow-up candidate:上游 manifest 依赖 v6,riviv 刻意 DPI-only(#79 零差验收);v5.82-vs-v6 分歧挂账至今。
2. 上游锚点两件齐:manifest v6 依赖块(上游 res/voidImageViewer.Manifest:5-9)+ `InitCommonControlsEx` 调用(riviv 已有,旗标一致)→ 采纳后与上游观感对齐的是上游 20 年的默认态。
3. 零依赖成本:系统 DLL、XP 起可用(riviv 地板 1607 无沟)、零新 crate、零新工具链——依赖原则五条全不触发。

## 机械面证据(全亲测)

| # | 事实 | 证据 |
|---|---|---|
| M1 | arm B 进程只加载 v6 程序集 → 激活成立 | 模块枚举:arm B 唯一 comctl32 = WinSxS `...common-controls_..._6.0.26100.9711_...`;arm A 双载:默认 5.82(`..._5.82.26100.8328_...`,riviv 控件所绑)+ 一个来源不明的惰性 6.0 加载(候选:shell/公共对话框组件自建激活上下文;**活跃皮肤仍 5.82 由视觉矩阵证实**) |
| M2 | **版本串判别不可靠** | 26100 构建的 6.0 SxS comctl32 磁盘 FileVersion 报 `5.82 (WinBuild.160101.0800)`、ProductVersion `10.0.26100.8117`(亲查)——**判别式必须用 SxS 目录名 flavor 位**(5.82/6.0),FileVersion 会骗人 |
| M3 | 零新 DLL 静态导入,S1 地板零扰动,无 ADR 硬前置 | dumpbin -IMPORTS 双臂 26 原始行(25 唯一 DLL,与 ADR 0007/0008 记载同一导入面)逐行 diff = IMPORTS-IDENTICAL;comctl32.dll 既有行在列 |
| M4 | 体积 +512 B | manifest 资源段增量;远低于任何阈值,不涉 ADR |
| M5 | 地板零沟 | v6 自 XP;riviv min-OS 1607(AGENTS S1);uxtheme.dll 双臂均已加载 |

## 视觉矩阵(13 面,Windows 11 26100 本机构建、200% 单屏,PrintWindow PW_RENDERFULLCONTENT,探针 DPI-aware;地板 1607 面未测,边界 6)

| surface | hash | pixel-diff | 观察 |
|---|---|---|---|
| menubar | **same** | 0% | OS 自绘,不经 comctl 激活上下文(本构建实测;XP 时代 v6 曾主题化菜单,Win11 26100 实测不变) |
| context-menu | **same** | 0% | 同上(屏采 best-effort,防遮挡注意在案) |
| filedlg | **same** | 0% | 现代通用文件对话框自绘主题,与宿主激活上下文无关 |
| toolbar | DIFF | 12.23% | A 经典 3D(B 主会话经视觉模型独立确认 Windows 9x/2000 风);B v6 扁平+浅蓝选中高亮+点状 gripper;色板 4→12 色(主会话像素统计佐证) |
| statusbar | DIFF | 10.19% | A 立体分格;B 扁平+点状 resize gripper |
| playlist-pane | DIFF | 2.59% | 同尺寸同列形;B 主题化选中/焦点渲染,行高 25→28px |
| options / jumpto / customrate / everything | DIFF ×4 | 4.3–12.1% | 尺寸/布局/树/页全同;按钮/组合框经典 3D→v6 扁平 |
| about-dlg | DIFF | 2.37% | 文本逐字同(控件 dump 验证);仅 OK 按钮换皮 |
| main-window / playlist-pane-full | DIFF ×2 | 0.29%/0.62% | 差异全收敛于 chrome 条带,图像区逐字节同 |

- 运行时菜单枚举(GetMenuItemInfoW 全走查)双臂 145 行 id/label 全同;7 个探针命令 id 活菜单核对(1/3/69/92/119/121/126 与 menu.rs 静态表吻合)。

## 度量(31 项)

唯一漂移:`playlist-pane/lvm_item0_h` **25px → 28px**(v6 ListView 主题行度量)。其余 30 项全同,含:窗 1728x1080、菜单栏 39、toolbar 高 52 / TB_GETBUTTONSIZE 139x38、statusbar 高 42 / SB_GETRECT 1378x40、五对话框窗寸与按钮高 35 / 组合框 32、JumpTo 列表项高 24。**含义:现行 system-DPI 布局数学在 v6 下全稳,无一处公式须改。**

## 行为面(零 FAIL 零 BLOCKED)

| 探针 | arm A | arm B |
|---|---|---|
| P3.1 WM_DPICHANGED 合成直发子控件(toolbar/statusbar/listview)前后高度 | 无反应 | **无反应** |
| P3.2 playlist 键盘路径(3 行计数/↓ 移选/Return 跳转标题 img→img2/↑ 回) | PASS | PASS |
| P3.3 toolbar 项路径(8 按钮,TB_GETITEMRECT 40x38) | PASS | PASS |
| P3.4 About BM_CLICK 关闭 | PASS | PASS |

- **P3.1 是合成探针阴性观察,不是裁决**:「v6 控件自带 WM_DPICHANGED 处理 → 收掉 chrome 恒系统 DPI 比例 seam」的预期在合成直发下未见高度变化(双臂一致),但该观察**不可归因亦不可外推**——①合成直发 ≠ 真实跨 DPI 迁移的完整消息序(真实路径由窗口管理器逐子走查 + 主题度量刷新);②chrome 几何本由 app 自持(window.rs:8956 `controls_height(logical_dpi())` 定条带高,toolbar.rs:300 同源),高度不变分不清「控件无处理器」与「合成投递未触达控件侧缩放」。处置 = **收益不计入采纳理由(O1 按纯视觉裁);真实混合 DPI 硬件迁移 = 人工 QA 矩阵项(机器不可达),验证后再裁是否另立自家 WM_DPICHANGED 布局链票**。
- P3.2 技术附注(探针工艺,非回归):ListView 箭头键须 SendMessage 直发(PostMessage 的 WM_KEYDOWN 不驱动默认键处理);Enter 走 pane subclass 路径 posted 即可达;标题跟随跳转而非选中 = app 设计(README:118)。

## 开放问题(拍板项,全附推荐)

- **O1 是否采纳 v6(纯视觉决策票)**:**A = 采纳(推荐)**——零功能回归、窗布局度量逐项全同(playlist 行高 25→28 唯一漂移,O2 接受;Rename 未测面见边界 7)、零依赖成本(M3/M4/M5)、对齐上游 20 年默认观感、撤掉 README:154 在案 known difference;代价 = chrome 全控件面视觉一次性全变(发布说明须带观感变化条目;地板 1607 机视觉面未测,边界 6)/ **B = 保持 5.82**——唯一理由是极简审美偏好,但 known difference 继续挂账。
- **O2 行高漂移处置(若 O1=A)**:**接受 28px 系统主题原生值(推荐)**,不手工钉 25px 旧值(钉旧值 = 与主题打架,后续每主题变一次修一次);`playlist_pane_width` 宽度面不受影响。
- **O3 chrome per-monitor 缩放与 v6 解耦**:P3.1 仅证明合成直发未改变子控件高度,不能裁定真实 DPI 迁移时 v6 的缩放行为;不将该收益计入 v6 采纳理由,真实迁移(人工 QA 矩阵项,机器不可达)验证后再决定是否另立自家 WM_DPICHANGED 布局链改造票。
- **O4 发布与 QA 面(若 O1=A)**:说明稿带观感变化条目 + 截图更新;QA 清单人工面加「混合 DPI 双屏真机迁移」项(机器不可达,P3.1 已证合成探针测不到该面)。

## 边界与风险(实现票前须知)

1. **视觉验收是人工拍板项**:机器矩阵已证窗布局零漂移(playlist 行高 25→28 例外在案,系控件内项度量),但「v6 好不好看」只有用户能裁;实现票前 O1 须表态。
2. 冒烟回归面:既有 smoke 的 chrome 度量断言(78/79 族)在本矩阵实测全同 → 预期零红;实现票仍须全量复跑冒烟矩阵(纪律)。golden81/golden90 为图像区字节断言,不涉 chrome。
3. context-menu 零差结论的取证面是屏采(best-effort,遮挡敏感):矩阵跑时无人值守,结论可信;复跑时保持无遮挡环境。
4. 探针产物留档:`%TEMP%\riviv-s227\probe\out\`(双臂 13 面 PNG、metrics.csv、behavior.log、summary.md、menu-A/B.txt、探针脚本 5 只);worktree `%TEMP%\riviv-s227`(实现票落地后清)。
5. 会话卫生注记:两个前会话遗留 zombie riviv 进程(PID 53376/66156,~32K,taskkill 拒绝访问,无窗口未参与任何探针)——已在 summary.md hygiene 节留档,不阻塞本票。
6. **地板覆盖未测**:机器矩阵实测口径 = 本机 Windows 11 26100、200% 单屏;Win10 1607 地板机的 v6 视觉/度量面未入矩阵(实现票 QA 面:地板机或最接近环境过一遍);「菜单零差」结论限本构建(v6 在 XP 时代曾主题化菜单,OS 版本敏感性注记)。
7. **Rename 对话框未入矩阵**:riviv 第五个手造对话框(rename_dlg.rs,与四 Dialog 同族 button/edit 控件)未实测——完整性口径以「四实测 + 一未测」记,实现票冒烟补几何/行为 + 人工 QA。

## 拍板路径

用户对 O1 表态(开工词即裁决)→ A 则立实现票:manifest 依赖块(抄 worktree 补丁转正式注释)+ README:154 段改写(known difference 撤销改记实测差异面)+ 全量冒烟复跑 + Rename 对话框冒烟(边界 7)+ QA 清单(含混合 DPI 真机与地板机人工项,边界 6/O4);量级 = 小实现票(单 PR)。
