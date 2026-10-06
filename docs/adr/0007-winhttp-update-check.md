# ADR 0007: 应用内更新检查(notify-only)——WinHTTP 静态导入

- 日期: 2026-10-06
- 状态: 已接受(O1:A / O2:A / O3:A / O4:A / O5:A / O6:A 全按推荐,用户拍板 2026-10-06「全按推荐」;实现票 #210;spike #208 证据电池 `docs/spikes/s-auto-update.md`)
- 范围: Help→Check for Updates 的网络栈选型与导入面、回执协议、失败语义;**不含**下载/安装/portable 自替换(O1=B/C 留作后继刀)、启动自动检查(O2=B 留作后继刀)
- 上游权威: #208 spike(横评/PE 基线/依赖原则核对)、AGENTS「依赖原则」第 6 条(系统能力装机≈100% 且无授权费)、s1-platform-floor(新增 DLL 静态导入须 dumpbin 证据并写 ADR)、ADR 0001(错误处理姿态的边界——本 ADR 明确其**不**裁更新网络失败)

## 背景

viv.c 零更新机制(亲证,#208 R1)——更新检查是 riviv 首个引入网络面的特性。exe 此前零网络 DLL 导入(spike R2 字符串法 + 本 ADR dumpbin 正式基线)。渠道 = GitHub `releases/latest` API(本仓唯一发布面,O3 单渠道钉死)。横评结论(#208):原生 Win32 公共骨架 = HTTP 查版本→提示→开页;notify-only 覆盖「知道有新版」的全部价值且风险面最小。

## 决策

### D1. 网络栈 = WinHTTP(windows crate feature,静态导入 winhttp.dll)

`Win32_Networking_WinHttp` feature(registry windows-0.62.0 Cargo.toml:505 亲核)启用,零新第三方 crate、近零体积。**dumpbin 证据**(本 ADR 留档,VS18 BuildTools 14.51.36231 x64):

- 改前(= v0.5.0 发布 exe,6,872,576 B;#210 双臂复跑 2026-10-06:pre-change 代码态重建字节一致,dumpbin 大小写不敏感去重):24 DLL——advapi32 / api-ms-crt×6+core-synch+shcore / bcryptprimitives / combase / comctl32 / comdlg32 / d2d1 / d3d11 / gdi32 / kernel32 / mscms / ntdll / ole32 / oleaut32 / shell32 / user32 / VCRUNTIME140,无任何网络 DLL(raw dumpbin 行 kernel32 双记共 25 行——初稿误按行计数且 crt 误记 ×5,双臂复跑订正)。
- 改后(6,888,960 B,+16,384 B):25 DLL,唯一增量 = **winhttp.dll**(双臂 diff 恰一条,PR 亲跑 dumpbin 复核通过 2026-10-06)。

地板合规:winhttp.dll 随系统交付(XP SP3 起),Win10 1607 地板自带、装机≈100%、无授权费——依赖原则第 6 条合格;TLS 面由系统 WinHTTP 承担(1607 默认 TLS 1.2,GitHub API 要求 1.2+,无沟)。否决项:urlmon `URLDownloadToFile`(COM+IE 缓存怪癖)、第三方 reqwest(**不违**依赖原则字面[禁的是外部工具链],败于 hyper/tokio 依赖树+MB 级体积,#208 O4 评审订正)。

### D2. 回执协议 = 后台线程 + WM_APP+4 盒装回执

单飞闸(static AtomicBool)挡重入;`Builder::spawn` 失败走用户级(#65 P2 先例,绝不 panic)。结果 `Box<Outcome>` 经 `PostMessageW(WM_APP+4)` 递送(wparam 弃用/lparam=指针);投递失败(窗口已亡)由 worker 收回盒。WM_APP+1/+2/+3 已被 load kick / Everything retry / pane jump 占用,+4 为本特性。

### D3. 失败语义 = 独立 UX 决定(ADR 0001 不适用,显式不引申)

ADR 0001 的用户级 = 图片加载失败且规定**不弹框**;更新网络失败是新恢复类,ADR 不裁此事(#208 Codex 评审核正)。裁定:手动检查失败 → 消息框告知(用户主动询问,应有回答);stderr 留 GetLastError 面包屑(冒烟/诊断通道,非用户面);窗口不退出、无任何致命路径。HTTP 全程后台线程,UI 永不冻结。

### D4. 解析与比较 = 手写纯函数,严苛失败

不引 serde/semver crate:响应手解析 `"tag_name":"vX.Y.Z"` 单字段(宽容字段序/空白,值形状严格);semver 三元数值比较(**数值比较非字典序**,0.10>0.9 钉测);任何畸形(含非 200 的错误 JSON——其中无 tag_name)→ Failed,永不出假「有新版」。读体上限 64 KB(敌意面纪律);跳过 HTTP 状态码查询 FFI(非 200 体无 tag_name 自然落 Failed,少一处签名面)。

### D5. notify-only 边界

「打开下载页」= ShellExecuteW 开 `https://github.com/jaredshuai/riviv/releases/latest`(canonical 永远指向最新,无需解析 html_url)。riviv **永不下载、永不校验、永不替换**任何文件——GitHub `assets[].digest` 是 integrity-only(与二进制同信任域,#208 R4),称「已验证下载」前须另立信任锚,那是 O1=B 的前置而非本刀的事。

## 验证

纯函数(parse/semver/decision/上限)入单测;loc 三表 + 助记符 U(Help 层内空闲,撞车网按层判重)+ 撞车网/复用网/非空网自动覆盖新键;冒烟 = 菜单行在场 + WM_COMMAND 127 → 三态框端到端(当前版本=已是最新臂;**过期版本探针 exe**(临时 0.4.0 构建)走「有新版 0.5.0」臂,点「否」不开浏览器);README Differences 记段。
