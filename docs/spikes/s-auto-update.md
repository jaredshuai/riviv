# 应用内更新检查(auto-update)开题 spike

- 票:#208(本 spike 为其唯一交付;实现票在拍板后另立)
- 日期:2026-10-06
- 基线:master `d998cb1`(v0.5.0 发布后同步收官态);exe 6,872,576 B;test 828 过/0 fail/3 ignored(v0.5.0 发布口径;本票 docs-only 零代码变更,commit 钩子将亲跑)
- 证据性质:上游锚点/代码锚点/PE 导入/registry feature 均为本会话亲读;GitHub API 本机亲通(curl);横评来源=公开文档/仓库(WebSearch,2026-10-06),**未读横评对象源码**——形态结论可信,行级细节以链接为准;**本轮未跑任何新探针**(docs-only 无行为面)

## 结论一句话

自动更新是**首个无上游锚点的纯用户愿望票**(viv.c 零更新机制,亲证);横评定形态:框架应用(Clash Verge Rev / cc-switch)的自动替换靠框架 updater(Tauri 插件+minisign 签名),搬运不了;**原生 Win32 同形态应用(Everything / Sumatra PDF)的公共形态 = HTTP 查版本 → 提示 → 开页或下载安装器,没有一家做进程内自替换**;与上游同门的 Everything 给出文化对标(Help→Check for Updates… 菜单行、`check_for_updates_on_startup` ini 键默认关);riviv 侧硬约束 = exe 今日零网络 DLL 导入(PE 导入表基线在案),WinHTTP 是唯一合依赖原则的系统通道(windows 0.62 feature 在,零新第三方 crate),但**新 DLL 静态导入 = S1 地板规则 ADR 硬前置**;推荐首刀 = notify-only(Help 菜单手动检查 + 三态对话框 + 有新版开 releases 页),下载/安装/portable 自替换留作后继刀。

## 为什么做

1. 用户发起(2026-10-06「自动更新的功能做了吗」),并点名横评成熟社区应用(Clash Verge / DeepSeek desktop / cc-switch)——本 spike 第 2 节即该横评。
2. 上游锚点:**无**。`c-original/` grep 全库仅 os.c 的 OS 版本探测(为 shell 集成服务),Help 菜单(viv.c:958-965)有 Help/website/donate 行、无任何 update 行——做了即 riviv-authored,README「Differences」记段(#68/#178/#199 先例)。
3. 发布节奏已稳定(v0.1.0→v0.5.0 五连),用户侧获知新版的唯一通道 = 手动看 GitHub;一个轻量检查面与发布节奏匹配。

## 横评(用户点名 + 同形态类比;来源见文末)

| 应用 | 形态 | 更新实现 | 对 riviv 的可搬运性 |
|---|---|---|---|
| **Clash Verge Rev** | Tauri 2 框架 | tauri-plugin-updater(v2.9.0):latest.json 清单 + minisign 签名验证 + 双通道(stable/alpha)+ CDN 回退;资产在 GitHub Releases | 框架自带,**不可搬运**;可学:签名验证思想、清单与渠道分离 |
| **cc-switch** | Tauri 桌面应用 | .msi 经 GitHub Releases 分发,Tauri 式更新流 | 同上,不可搬运 |
| **DeepSeek desktop** | —(无官方 Windows desktop) | 官方 dsh CLI **零更新检查**(2026-08 仍有 open idea issue 求加);npx 每次重拉 = 事实上的更新 | 反例数据点:大热工具也可以没有更新面 |
| **Everything**(voidtools,与上游同门) | 原生 Win32,无框架 | Help→Check for Updates…;ini `check_for_updates_on_startup`(静默安装默认关);发现新版走 voidtools.com 下载 | **文化对标最强**:菜单位置、ini 键名、默认关,三个决定可照抄 |
| **Sumatra PDF** | 原生 Win32 C++ | 设置勾选「自动检查更新」→ 查官方站版本 → 提供下载安装器 | 同形态先例:HTTP 查版本+下载安装器,无框架 |

**横评归纳**:①原生 Win32 应用的公共形态 = 查版本→提示→开页/下载安装器,无一家做进程内自替换;②全自动替换专属框架应用(Electron autoUpdater / Tauri updater),代价是签名体系+清单服务;③voidtools 家族惯例 = 手动菜单 + 启动检查开关默认关。

## riviv 侧证据(全部本地可复核)

| # | 事实 | 证据 |
|---|---|---|
| R1 | 上游零更新机制 | c-original grep:仅 os.c:111-112 `os_major_version`(shell 集成用);viv.c:958-965 Help 菜单无 update 行;全库无 version check/下载器 |
| R2 | exe 今日零网络 DLL 导入 | `grep -aoE '[a-z0-9_-]+\.dll' target/release/riviv.exe` 全量 = advapi32/bcryptprimitives/combase/comctl32/comdlg32/d2d1/d3d11/dbghelp/gdi32/kernel32/mscms/ntdll/ole32/oleaut32/shell32/user32 + api-ms-*;无 winhttp/wininet/ws2_32/urlmon。**注:字符串法是本 spike 的便宜证据;实现票 ADR 前须 dumpbin /imports 正式留档**(S1 地板规则先例 #156) |
| R3 | WinHTTP 走 windows 0.62 feature 即得,零新第三方 crate | registry windows-0.62.* Cargo.toml:505 `Win32_Networking_WinHttp = ["Win32_Networking"]`;riviv Cargo.toml 现未启用任何 Networking feature |
| R4 | GitHub API 渠道现成且带免费校验面 | `GET api.github.com/repos/jaredshuai/riviv/releases/latest` 本机亲通(curl,2026-10-06);响应含 `tag_name` 与 `assets[].digest`(sha256,v0.4.0/v0.5.0 资产亲见);未认证限流 60 req/h/IP——手动检查绰绰有余 |
| R5 | 版本比较有纯函数落点 | tag `vX.Y.Z` vs `CARGO_PKG_VERSION`:手写 parse+semver 三元比较(不引 semver crate/serde;单字段 JSON 手解析),入测试网(质量档位:纯逻辑产品标准) |
| R6 | Help 菜单/助记符余量 | Help 层现两行:Command Line Options(C)/About(A),三表一致(loc.rs:696/831、1036/1170、1376/1510);**U 三表全空闲**——Check for &Updates / 检查更新(&U) / 檢查更新(&U) 可用;新行推荐落 Command Line Options 与 About 之间(Everything 惯例:检查更新在 About 附近) |
| R7 | 安装器替换面风险(后继刀的坑) | 运行中实例锁 exe:NSIS 安装器覆盖在跑文件会失败,「下载+安装」须先关实例(单实例 mutex 转发语义 #109 也在场,交互在实现票核);**首刀 notify-only 不碰此面** |
| R8 | TLS/地板 | Win10 1607 的 WinHTTP 默认启 TLS 1.2;GitHub API 要求 TLS 1.2+ ⇒ 地板上无沟 |

## 开放问题(拍板项;全附推荐)

- **O1 首刀范围**:**A = notify-only**(WinHTTP 查 `releases/latest`,semver 比较,三态对话框:已是最新 / 有新版 vX.Y.Z→「打开下载页?」(ShellExecute `html_url`) / 网络失败;**推荐**——横评原生公共形态,网络只读不下载,风险面最小,覆盖「知道有新版」的全部价值)/ B = 下载+sha256 校验+跑安装器(digest 免费拿,但引入 R7 替换坑+安装器语言三选一设计题,留二刀)/ C = portable 自替换(running-exe rename 技巧,最重,最远)。
- **O2 触发**:**A = 仅 Help 菜单手动**(推荐,Everything/Sumatra 的默认姿势;零新 ini 键,零启动时序面)/ B = +启动自动检查(后台线程+节流;Everything 键名 `check_for_updates_on_startup` 默认 0[负控纪律];留二刀)。
- **O3 渠道**:**GitHub `releases/latest` 单渠道钉死**(推荐;本仓无自有服务器,Latest 标记即发布语义;不设镜像/备源)。
- **O4 网络栈**:**A = WinHTTP**(windows feature 启用,推荐;系统组件、无 COM、显式超时/UA;**ADR 硬前置**=新 DLL 静态导入+R2 基线对照)/ B = urlmon `URLDownloadToFile`(一行下载但 COM+IE 缓存怪癖,劣)/ C = 第三方 reqwest(违零外部依赖原则,否决)。
- **O5 UI/语言面**:Help 菜单新行(三表同步,~6 新串:菜单行+三态文案+按钮)+ 消息框复用现有 MessageBox 通道;**无新默认热键**(菜单行即可);助记符 U(R6);新 key 入撞车网净面(grandfather 钉精确计数的既有机制自动护住)。
- **O6 失败语义**:手动检查失败 = 用户级消息框、窗口不退出(ADR 0001 用户级姿态);HTTP 全程后台线程,回执走 WM_APP 族(worker 线程先例),UI 永不冻结;(二刀若做)启动检查失败 = 静默 + stderr breadcrumb。**非选择项,随 O1=A 即生效,列出供否决**。

## 边界与风险(实现票前须知)

1. **ADR 必写**(O4=A 时):新 DLL 静态导入(winhttp.dll)+ dumpbin 证据 + R2 基线对照 + 失败语义;参照 ADR 0005(SVG resvg 破例)格式。
2. **在线冒烟的环境敏感面**:真实 API 依赖网络+GitHub 可达性——纯函数(parse/semver/决策)入单元测试网;WinHTTP 壳保持薄;在线路径冒烟标环境敏感(先例:smoke78 S7/S8b 文档化);失败路径探针=假域名/断网。
3. **JSON 手解析的健壮性**:只取 `tag_name`/`html_url` 两字段,宽容解析(API 加字段不炸),钉测固定响应样本。
4. **体积预算**:feature 绑定+代码预计 <10 KB 级,远低于 +400 KB 单解码阈值;ADR 为导入面而非体积。
5. **横评对象的行级细节未读源码**:本 spike 只引公开文档;实现票若采纳某具体机制(如 minisign 思想),届时读对象源码再核。

## 拍板路径

用户直接逐项拍 O1-O6(或「全按推荐」),实现票随后另立(预计 = ADR + loc 三表 + menu 行 + net 模块纯逻辑层 + UI 线程协议 + 冒烟)。

## 横评来源(WebSearch 2026-10-06)

- Clash Verge Rev:[repo](https://github.com/Clash-Verge-rev/clash-verge-rev) / [DeepWiki Update Mechanism](https://deepwiki.com/clash-verge-rev/clash-verge-rev/10.2-update-mechanism) / [Tauri v2 updater](https://v2.tauri.app/plugin/updater)
- cc-switch:[repo](https://github.com/farion1231/cc-switch) / [docs](https://ccswitch.io)
- DeepSeek dsh 无更新检查:GitHub idea issue「Auto-update / update notification for the official dsh」(2026-08-16,WebSearch 摘要,issue 精确链接未逐字核)
- Everything:voidtools.com FAQ/CLI 文档(Help→Check for Updates、`check_for_updates_on_startup`、静默安装默认不查)
- Sumatra PDF:[repo](https://github.com/sumatrapdfreader/sumatrapdf)(设置内自动检查更新,UpdateCheck 实现)
