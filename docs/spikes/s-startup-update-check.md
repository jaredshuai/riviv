# 启动自动检查更新(O2 后继刀)开题 spike

- 票:#214(本 spike 为其唯一交付;实现票拍板后另立)。前史:#208 spike(`s-auto-update.md`)O2 裁定 A=仅手动(首刀范围控制,#210 已落 v0.6.0),O2=B 启动检查**留作本刀**;ADR 0007 范围显式排除项 = 本刀的正当前史
- 日期:2026-10-07
- 基线:master `8d0d2ec`(v0.6.0 发版同步收官态);本票 docs-only 零代码变更
- 证据性质:riviv 代码锚点本会话亲读(update.rs 全文、window.rs run() 启动链与消息臂、options.rs/config.rs/keys.rs/loc.rs/status.rs);Sumatra `UpdateCheck.cpp` = raw GitHub 本会话亲读(WebFetch,引文逐字);Notepad++/Everything/VS Code = 公开文档+官方社区检索(2026-10-07),**未读其源码**;Everything 交互安装默认值两轮检索不可核,如实标「未核」不硬填(工具纪律);本轮未跑新探针(docs-only 无行为面)

## 结论一句话

七核查面全闭合,复用面极大——#210 的骨架(WinHTTP worker + 零载荷 WM_APP+4 + 队列 + 单飞闸)原样承载启动检查,通知基建(`status_set_temp_text`)、Options 控件机制(`Kind::Checkbox`)、ini 往返钉测网(`every_written_key_reads_back`)全已在库;原生同形态双样本(Notepad++/Sumatra)启动检查**默认开**,且 Sumatra 的落点与 riviv 设计几乎同构(每日闸、时间戳记于检查开始、自动失败静默、仅新版出声、**首启跳过**——注释原文「给隐私敏感用户留出关闭时间」);推荐 = 默认开+首启跳过、发现新版仅状态栏 temp_text 出声(每日重试补偿可见性弱点)、闸落 ini 本地日序数键、静默飞行中手动点击升格为手动呈现。

## 核查面 1:默认值矩阵

| 应用 | 形态 | 启动检查默认 | 证据 |
|---|---|---|---|
| **Notepad++** | 原生 Win32 | **开**(安装版) | Settings→Preferences→MISC→「Enable Notepad++ auto-updater」;官方社区与第三方指引一致口径「默认启用,启动时查,有新版弹框」;portable 版不含 updater(该项灰)。新模式下拉 Disabled/Notify only/Notify+silent install/Silent |
| **SumatraPDF** | 原生 Win32 C++ | **开** | #209 官方隐私政策明文(默认开+每日至多一次)+ 本轮 `UpdateCheck.cpp` 亲读:`gSettings->checkForUpdates` 持久设置;**首次启动跳过**(逐字:`// don't check for updates at the first start, so that privacy sensitive users can disable the update check in time`,never 哨兵对比) |
| **Everything**(voidtools,同门) | 原生 Win32 | **未核** | 官方 ini 文档([voidtools.com/support/everything/ini](https://www.voidtools.com/support/everything/ini/))只述功能不载默认值;论坛用户 ini 样本 `check_for_updates_on_startup=1` 非权威;静默安装不查 = #209 在案。闭源,无源码可读 |
| **VS Code**(框架对照) | Electron | **开** | `update.mode` 默认 `"default"` = 后台自动下载+安装(比本刀激进一步);官方 FAQ/企业更新文档 |

**样本范围声明**(上次评审教训:不外推):完全核实默认值的原生同形态样本 n=2(Notepad++/Sumatra),均默认开;Everything 默认值本轮不可核。框架生态两种都常见(Tauri updater 族显式清单 opt-in,#208 在案)——横评**不构成**「原生默认开」的通律,默认值仍由 riviv 自拍(#208 归纳③纪律)。支持默认开的riviv 侧理由:发布节奏活跃而用户获知新版唯一通道=手动看 GitHub,启动检查是「知道有新版」的主通道(票面开题语)。

## 核查面 2:通知形态横评

| 形态 | 打扰度 | 可见性 | 新面成本 | 样本形态 |
|---|---|---|---|---|
| **A 状态栏 temp_text**(`status_set_temp_text`,window.rs:774,#47 已有基建,3s 闪现) | 零(不打断任何操作) | 弱:3s 即逝;可被后至 flash 替换(启动期首图 fit 的 pos_zoom flash 是真实竞争,`status_set_temp_text` 语义=替换);`show_status=0` / 全屏(状态栏随 chrome 隐藏)时不可见 | **零新面** | —(riviv-authored) |
| B 模态弹框(复用手动 YESNO 框,update.rs:230) | 高(启动即抢焦点,违背票面「不打扰原则」) | 强 | 零 | **横评多数形态**:Notepad++ 弹框、Sumatra 安装版 TaskDialog(「Don't install / Install and relaunch」) |
| C 任务栏角标(ITaskbarList3 overlay) | 低 | 强(常态可见) | 高:新 COM 面(S1 地板规则 QI/运行时探测 + 可能 ADR)+ 角标图标设计 + 清除时机,与 notify-only 轻量性不成比例 | 浏览器族(后台更新+角标) |
| D 菜单行标记(Help→Check for Updates 行加「●」类前缀) | 零 | 中(仅开 Help 菜组时可见) | 菜单状态刷新链(WM_INITMENU)添新面;记为备选 | — |

**A 的可见性弱点处置——每日闸 = 幂等重试**:notify-only 下「有新版」提示是可重复的(明日启动再提示,直到用户更新),单次丢失(被 flash 覆盖/状态栏关/全屏)**无累积伤害**;手动菜单永在(其 YESNO 框直达 releases 页)。全屏面补充:fullscreen 非持久态(config.rs 无会话 fullscreen 键,亲证)→ 启动时窗口必非全屏,仅 `-slideshow` 启动线可能在回复到达前进全屏(window.rs:5274 slideshow 先入全屏)——同补偿论证覆盖。

**推荐 A**。B 直接违背票面裁决原则;C 成本与收益不成比例;D 可见性依赖用户主动开菜单。

## 核查面 3:频次闸(每日一次)

- **Sumatra 同构(亲读)**:`constexpr int kSecondsInDay = 60 * 60 * 24`;`timeOfLastUpdateCheck`(FILETIME)**在检查开始时写入**(`GetSystemTimeAsFileTime(&gSettings->timeOfLastUpdateCheck);`)并 Flush;release 版闸 = 至少隔一天。
- **riviv 落点推荐**:ini 键 `check_for_updates_last_day` = **本地日序数**(1970-01-01 起算的本地日,int;纯函数入测试网)。写时机 = **判定即记**:首启跳过分支与 spawn 成功分支都把今日写进内存 config(cubic #216 P1——跳过分支若不记账,每次启动都被当作首启,自动检查永不发生),正常退出经 WM_DESTROY 落盘(window.rs:9680,WM_ENDSESSION 9699 兜底)——当日标记 best-effort(见下);**网络失败也记账**(同 Sumatra mark-at-start 语义;离线机每日至多一次静默失败+stderr 面包屑,fetch_latest_body 已有 eprintln)。
- 跨会话天然继承 ini 双位置语义(appdata 覆盖,config.rs:311/346);时区旅行当日双查无害(本地日粒度)。
- **手动检查不走闸**(用户驱动即时应答,同 Sumatra 手动路径);开关关掉时不写标记,重开后按陈旧标记立即补查。

## 核查面 4:启动时序

- **发起点推荐**:run() 泵前、`refresh_status(hwnd)`(window.rs:10671)之后、`let mut msg`(10673)之前——此时窗口已建已显(10658-10664)、首图 load 已 kick(process_parsed_cl 10654,异步 loadthread)、GPU 栈已立(10522);网络全在 worker 线程(ADR 0007 D2 复用),**启动路径零阻塞**。
- 窗口必活:此处 hwnd 已同步创建成功;补一道 `IsWindow` 守卫(process_parsed_cl 理论上存在即退路径时不白跑一次请求)。
- 单实例语义:转发实例(window.rs:10056-10120 return)与 `-install` 命令行实例(10009-10011 return)都在发起点**之前**退出——只有持锁建窗实例检查;`multiple_instances=1` 时每实例各自检查(各自进程,标记并发写=last-writer-wins,幂等无害)。

## 核查面 5:失败语义

ADR 0007 D3 类比已定方向(用户未主动询问→不应答),本轮核对**无残余 UX 决策点**:

- 启动三态中**仅 Available 可见**(temp_text,面 2);UpToDate/Failed 全静默(Failed 的 stderr 面包屑已在 fetch_latest_body);无弹框、无退出路径、无新致命面。
- 唯一新 UX 面 = **手动点击落在静默检查飞行中**:单飞闸(CHECK_RUNNING)会 no-op 掉点击——违背「用户主动询问必须有答」(D3 手动语义)。处置:**升格机制**——`begin` 增 manual/startup 二态,manual 调用无论是否被闸挡都置 `manual_requested` 标志;on_reply 读并清标志,置位则按手动语义呈现三态框。备选 = no-op(点击无声丢弃,不推荐)。
- **spawn 失败分支二态分叉(cubic #216 P2)**:现有 `begin` 的 `Builder::spawn` 失败会同步弹 `UpdateFailedText` 框(update.rs:167-174)——startup 模式**静默**(stderr 面包屑,与「仅 Available 可见」约定一致;启动时弹错误框=未请求即打扰);manual 模式保留现有直报框。

## 核查面 6:ini/Options 面

- **开关键** `check_for_updates_on_startup`(Everything 同名,同门文化对标;与热键区 `help_check_for_updates_keys`(keys.rs:1156,`*_keys` 行)不同键不同区,无撞)+ **时间戳键** `check_for_updates_last_day`(同前缀成组;Options 不暴露)。
- **Options**:General 页 append 第三行 `Kind::Checkbox`(机制已在,options.rs:466-469,BS_AUTOCHECKBOX;现两行 Appdata y=0 / MultipleInstances y=18,新行 y=36 续 18-du 间距;append-only ctrl_id 纪律,#185/#191/#193 同款注释先例)。
- **钉测网**:config.rs `every_written_key_reads_back`(716)自动覆盖新键(键计数注释随改);拼写另加显式钉测(仿 keys.rs:1156 先例);闸判定纯函数 `should_check(enabled, last_day, today)` 入单测(质量档位:纯逻辑产品标准)。
- **loc 新串**:checkbox 标签 + 启动 temp_text 文案(含 `%s` 版本位,loc.rs:1803-1827 钉测网自动覆盖)×3 表 ≈ 6 条。
- 安装器面:无(不预置 ini;首启无 ini = P1 默认生效)。

## 核查面 7:边界核实

- **启动检查完成时主窗口在场**:worker 持有的 hwnd 即建窗线程的窗口;用户在网络窗口(至多 ~25s:5/5/5/10 超时)内关窗 → post 失败 → 队列留未领条目随进程退出回收(ADR 0007 D2 已裁,零悬空);WM_ENDSESSION 提前杀同理。
- **-install / 转发实例**:发起点之前已 return,零请求发出(面 4)。
- **当日标记时序**:判定即记写内存,正常退出必经 WM_DESTROY save 调用,但**持久化 best-effort(cubic #216 P2)**——`Config::save` 的写入失败只记日志不阻止退出(config.rs 契约:只读安装目录先例),未落盘则下次启动重查:每启动至多一次静默 GET,有界无害(与 Sumatra 的 appdata 路径不可解析时静默跳过保存同类)。崩溃同此结果。
- **首启跳过与升级路径统一**:老版本升级上来(ini 无 last_day)同「首启」处理——刚装完必是最新版,跳过近零损失,纯隐私增益。

## 开放问题(拍板项;全附推荐)

- **P1 默认值**:**A = 开 + 首启跳过**(**推荐**——last_day 缺失=首启:照常把今日写进 last_day(含跳过分支,面 3「判定即记」)但本次不发请求,次日起按闸;先例=Sumatra 首启跳过注释原文;Notepad++/Sumatra 双原生样本+VS Code 框架样本默认开;隐私面最软:用户有一天时间在 Options/ini 关掉它)/ B = 开(首启即查;损失对齐「刚装完必最新」论证为零增益)/ C = 关(负控纪律;但横评无原生默认关样本[Everything 未核不计],且启动检查正是本刀价值主通道)。
- **P2 通知形态**:**A = 状态栏 temp_text**(**推荐**——零新面+不打扰;可见性弱由每日重试补偿)/ B = 模态弹框(横评多数形态,但违背票面不打扰原则)/ C = 任务栏角标(新 COM 面+S1 ADR 成本,不成比例)/ D = 菜单行标记(备选)。
- **P3 频次闸键面**:**A = 本地日序数 `check_for_updates_last_day`,spawn 成功即记、失败也记**(**推荐**——Sumatra mark-at-start 同构,粒度即日,纯函数可测)/ B = FILETIME UTC 字面同构 Sumatra(精度对本闸无用)。
- **P4 飞行中手动点击**:**A = 升格为手动呈现**(**推荐**——用户主动询问必须有答,D3 手动语义)/ B = no-op 丢弃(点击无声,不推荐)。

## 边界与风险(实现票前须知)

1. 改动面清单:loc 三表两串、OptionsModel/Field/Ctrl append 一行、config 两键(Default+apply+to_pairs+键计数注释)、run() 发起点一段、update.rs 增二态与 manual_requested——预计 <5 KB 增量,零新 crate、零新 DLL 导入、**无 ADR 触发面**(WM_APP+4 协议在 ADR 0007 D2 已裁,本刀仅扩其使用方式)。
2. 冒烟面:首启跳过(删 ini 两连启:首启零请求、次启一次请求)、开关关=零请求、当日闸=同日双启单请求、manual 升格、temp_text 落点(status bar 主部件);**网络路径标环境敏感**(真实 GitHub API,先例 smoke78 S7/S8b;失败路径探针=假 host/断网);纯函数(日序数/should_check/semver 复用)入单测。
3. 探针观测点:请求发出的可观测信号——UA 串 `riviv/<ver>` 打 api.github.com(探针可代理/hosts 重定向,或以 stderr 面包屑为零请求/有请求的判据)。
4. README「Differences」#210 段扩一句(启动检查默认开+可关)。
5. 上游锚点:无(viv.c 零更新机制,#208 R1)——riviv-authored 延续。

## 横评来源(WebSearch/WebFetch 2026-10-07)

- Notepad++:[官方社区 topic/16905(禁用路径)](https://community.notepad-plus-plus.org/topic/16905) + Trishtech 指引(默认开/模式下拉口径)
- SumatraPDF:[UpdateCheck.cpp(raw,亲读)](https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/UpdateCheck.cpp) + [Update check doesn't work(官方文档)](https://www.sumatrapdfreader.org/docs/Update-check-doesnt-work)
- Everything:[ini 设置官方文档](https://www.voidtools.com/support/everything/ini/)(不载默认值)+ 论坛用户 ini 样本 + Total Commander 论坛(GUI↔ini 对应);**交互安装默认=未核**
- VS Code:[FAQ(默认自动更新)](https://code.visualstudio.com/docs/supporting/faq) + [企业更新管理](https://code.visualstudio.com/docs/enterprise/updates)
