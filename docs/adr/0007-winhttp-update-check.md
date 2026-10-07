# ADR 0007: 应用内更新检查(notify-only)——WinHTTP 静态导入

- 日期: 2026-10-06
- 状态: 已接受(O1:A / O2:A / O3:A / O4:A / O5:A / O6:A 全按推荐,用户拍板 2026-10-06「全按推荐」;实现票 #210;spike #208 证据电池 `docs/spikes/s-auto-update.md`。**O2 后继刀已落**:启动自动检查经 #214 spike(`docs/spikes/s-startup-update-check.md`)P1-P4 全按推荐,用户拍板 2026-10-07「全按推荐」,实现票 #217,决策 = D6)
- 范围: Help→Check for Updates 的网络栈选型与导入面、回执协议、失败语义;**不含**下载/安装/portable 自替换(O1=B/C 留作后继刀)。启动自动检查原为 O2=B 留作后继刀,**已由 D6 裁定落地**(#214/#217)
- 上游权威: #208 spike(横评/PE 基线/依赖原则核对)、AGENTS「依赖原则」第 6 条(系统能力装机≈100% 且无授权费)、s1-platform-floor(新增 DLL 静态导入须 dumpbin 证据并写 ADR)、ADR 0001(错误处理姿态的边界——本 ADR 明确其**不**裁更新网络失败)

## 背景

viv.c 零更新机制(亲证,#208 R1)——更新检查是 riviv 首个引入网络面的特性。exe 此前零网络 DLL 导入(spike R2 字符串法 + 本 ADR dumpbin 正式基线)。渠道 = GitHub `releases/latest` API(本仓唯一发布面,O3 单渠道钉死)。横评结论(#208):原生 Win32 公共骨架 = HTTP 查版本→提示→开页;notify-only 覆盖「知道有新版」的全部价值且风险面最小。

## 决策

### D1. 网络栈 = WinHTTP(windows crate feature,静态导入 winhttp.dll)

`Win32_Networking_WinHttp` feature(registry windows-0.62.0 Cargo.toml:505 亲核)启用,零新第三方 crate、近零体积。**dumpbin 证据**(本 ADR 留档,VS18 BuildTools 14.51.36231 x64):

- 改前(= v0.5.0 发布 exe,6,872,576 B;#210 双臂复跑 2026-10-06:pre-change 代码态重建字节一致,dumpbin 大小写不敏感去重):24 DLL——advapi32 / api-ms-crt×6+core-synch+shcore / bcryptprimitives / combase / comctl32 / comdlg32 / d2d1 / d3d11 / gdi32 / kernel32 / mscms / ntdll / ole32 / oleaut32 / shell32 / user32 / VCRUNTIME140,无任何网络 DLL(raw dumpbin 行 kernel32 双记共 25 行——初稿误按行计数且 crt 误记 ×5,双臂复跑订正)。
- 改后(6,895,104 B,+22,528 B;cubic #211 五项采纳后的终态,首版实现为 6,888,960 B/+16,384 B):25 DLL,唯一增量 = **winhttp.dll**(双臂 diff 恰一条,首版与终态各亲跑一次 dumpbin 复核通过 2026-10-06)。

地板合规:winhttp.dll 随系统交付(WinHTTP 5.1 自 XP SP1 起;初稿误记 SP3,cubic #211 P3 订正),Win10 1607 地板自带、装机≈100%、无授权费——依赖原则第 6 条合格;TLS 面由系统 WinHTTP 承担(1607 默认 TLS 1.2,GitHub API 要求 1.2+,无沟)。否决项:urlmon `URLDownloadToFile`(COM+IE 缓存怪癖)、第三方 reqwest(**不违**依赖原则字面[禁的是外部工具链],败于 hyper/tokio 依赖树+MB 级体积,#208 O4 评审订正)。

### D2. 回执协议 = 后台线程 + WM_APP+4 纯通知 + 进程内队列

单飞闸(static AtomicBool)挡重入;`Builder::spawn` 失败走用户级(#65 P2 先例,绝不 panic)。结果 `Outcome` 由 worker 放入进程内 `Mutex<VecDeque<Outcome>>` 队列后投递**零载荷**的 `PostMessageW(WM_APP+4)`——指针不跨消息边界,外进程伪造该消息只能弹到空队列被忽略(cubic #211 P2 采纳;初稿盒装指针经 lparam 递送的方案废弃)。单飞保证至多一个生产者,worker 入队前清队(窗口已亡未领取的旧裁决不滞留);投递失败(窗口已亡)仅留未领取条目随进程退出。WM_APP+1/+2/+3 已被 load kick / Everything retry / pane jump 占用,+4 为本特性。四个超时全非零(resolve/connect/send/receive = 5s/5s/5s/10s)——MSDN 契约 0=无限,cubic #211 P2:resolve 若无限,DNS 挂起会让单飞闸整会话卡死且永不弹 Failed。

### D3. 失败语义 = 独立 UX 决定(ADR 0001 不适用,显式不引申)

ADR 0001 的用户级 = 图片加载失败且规定**不弹框**;更新网络失败是新恢复类,ADR 不裁此事(#208 Codex 评审核正)。裁定:手动检查失败 → 消息框告知(用户主动询问,应有回答);stderr 留 GetLastError 面包屑(冒烟/诊断通道,非用户面);窗口不退出、无任何致命路径。HTTP 全程后台线程,UI 永不冻结。

### D4. 解析与比较 = 手写纯函数,严苛失败

不引 serde/semver crate:响应手解析 `"tag_name":"vX.Y.Z"` 单字段(宽容字段序/空白,值形状严格);semver 三元数值比较(**数值比较非字典序**,0.10>0.9 钉测);任何畸形(含非 200 的错误 JSON——其中无 tag_name)→ Failed,永不出假「有新版」。读体上限 64 KB(敌意面纪律);跳过 HTTP 状态码查询 FFI(非 200 体无 tag_name 自然落 Failed,少一处签名面)。

### D5. notify-only 边界

「打开下载页」= ShellExecuteW 开 `https://github.com/jaredshuai/riviv/releases/latest`(canonical 永远指向最新,无需解析 html_url)。riviv **永不下载、永不校验、永不替换**任何文件——GitHub `assets[].digest` 是 integrity-only(与二进制同信任域,#208 R4),称「已验证下载」前须另立信任锚,那是 O1=B 的前置而非本刀的事。

### D6. 启动自动检查(#214/#217;O2=B 后继刀)

复用 D1/D2/D4 全部底座(WinHTTP worker、WM_APP+4 零载荷回执、手解析严苛失败)——本节只裁增量。拍板 2026-10-07「全按推荐」(spike `s-startup-update-check` P1-P4):

- **P1=A 默认开 + 首启跳过**:ini 键 `check_for_updates_on_startup`(BYTE 档)默认 1——双原生同形态样本(Notepad++/Sumatra)默认开,Sumatra `UpdateCheck.cpp` 亲读同构。`check_for_updates_last_day`(int 档)= 本地日序数(`days_from_civil` 纯函数,`GetLocalTime` 薄壳);**≤0 = 从未** → 首启(全新安装或自 #214 前版本升级)照常记今日但**不发请求**(刚装完必是最新版;Sumatra 首启跳过先例——「给隐私敏感用户留出关闭时间」)。
- **P2=A 通知 = 状态栏 temp_text**:启动态仅 Available 可见——`status_set_temp_text`(#47 基建,3 秒闪现);UpToDate/Failed 静默(D3 启动语义:用户未询问;Failed 的 stderr 面包屑是诊断通道)。可见性弱点(3s 即逝/可被 panscan flash 替换/`show_status=0` 或全屏不可见)由**每日闸的幂等重试**补偿:明日启动再提示,直到更新;单次丢失无累积伤害。
- **P3=A 频次闸 = 判定即记**:两个标记分支(MarkOnly/Check)都把今日写进内存 config,WM_DESTROY 常规落盘——**best-effort**(`Config::save` 写失败只记日志,只读安装目录先例;未落盘 = 下次启动重查一次,有界)。网络失败也记账(Sumatra mark-at-start 同构)。跳过分支若不记账会把每次启动都变成首启,自动检查永不发生(cubic #216 P1)。手动检查不走闸。
- **P4=A 升格**:`MANUAL_REQUESTED` 标志在手动入口置位(含被单飞闸挡住的点击),`on_reply` 读清后按手动三态框呈现——用户主动询问必须有答(D3 手动半边)。
- spawn 失败二态分叉:手动 = 直报框(既有);启动 = stderr 静默(cubic #216 P2)。发起点 = run() 泵前、`refresh_status` 后,`IsWindow` 守卫;-install/转发实例在发起点之前已退出,持锁实例独查;`multiple_instances=1` 时各实例各查(幂等)。Options General 页第三行 checkbox(`Kind::Checkbox` 既有机制)编辑开关;日序数键不暴露。

## 验证

纯函数(parse/semver/decision/上限)入单测;loc 三表 + 助记符 U(Help 层内空闲,撞车网按层判重)+ 撞车网/复用网/非空网自动覆盖新键;冒烟 = 菜单行在场 + WM_COMMAND 127 → 三态框端到端(当前版本=已是最新臂;**过期版本探针 exe**(临时 0.4.0 构建)走「有新版 0.5.0」臂,点「否」不开浏览器);README Differences 记段。

D6 增量(#217):闸判定(`should_check` 全输入类)+ 日序数(`days_from_civil` 锚点/闰边界)入单测;config 两键入往返网+拼写钉测;Options 模型经既有 field 往返测;冒烟 = 首启跳过(删 ini 两连启)/当日闸(同日双启单请求)/开关关零请求/Available temp_text 落点(过期版本探针 exe,环境敏感标注意:真实 GitHub API)/手动升格;README Differences #210 段扩句。
