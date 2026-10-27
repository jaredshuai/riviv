# ADR 0009: 更新「下载+验签+安装」信任锚——ed25519-compact 域分离验签 + 双形态安装

- 日期: 2026-10-27
- 状态: 已接受(P1-P4 全按推荐,用户拍板 2026-10-26「全按推荐」;实现票 #237;spike #235 证据电池 `docs/spikes/s-trusted-download.md`,PR #236 评审 17 findings 全处置)
- 范围: O1=B(下载+验签+跑安装器)+ O1=C(portable 自替换)同刀;信任锚选型、签名形态与仪式、下载通道、双形态安装语义、失败语义、riviv-sign 工具、触发 UX。**不含**: Authenticode 级链式信任、私钥泄漏后的根治(见 D7 预案分层)
- 上游权威: spike #235(CNG 双证据否决/横评带数字/下载通道实测/端到端仪式演示)、ADR 0007 D5(notify-only 边界与「称『已验证下载』前须另立信任锚」的动因)、ADR 0001(显式不裁更新失败面——同 ADR 0007 D3 先例)、依赖原则③(体积;本刀双臂实录 +72 KB 级、零新 DLL,**无需破例 ADR**)、s1-platform-floor(新 DLL 导入纪律)

## 背景

ADR 0007 D5 钉死 notify-only:「riviv 永不下载、永不校验、永不替换」——因为 GitHub `assets[].digest` 是 integrity-only(与二进制同信任域,s-auto-update R4 亲证),不防投毒。本刀把 D5 预留的信任锚立起来:用户在 Available 框点「是」,riviv 走「下载 → 域分离验签 → 按形态安装」的全自动通道,信任根 = 逐版本发布 exe 内嵌的 Ed25519 公钥,独立于 GitHub 账号/CDN。

## 决策

### D1. 信任锚 = Ed25519 pinned-key,`ed25519-compact = 2.6.0 + opt_size`

系统 CNG(bcrypt.dll Ed25519)路线**双一手证据否决**:官方「CNG Algorithm Identifiers」算法表(2026-05-30 版)整页亲读无 ED25519 条目;本机 Win11 26300 `BCryptOpenAlgorithmProvider(L"ED25519")` 返回 `0xC0000225`——最新构建都没有,1607 地板更无从谈起(WebSearch 转述层曾两次给出相反结论,按工具纪律弃用,只认一手证据)。纯 Rust 两候选横评(spike E2,探针口径):**compact 2.6.0 + `opt_size`** +30,208 B、零依赖单叶、MIT、验签 25.67 ms @8.5 MB 实测——速度对单次更新无感,推荐并拍板;dalek 2.1.0(+63 KB、依赖树 ×10、BSD-3)备选,跨实现互操作已亲证(compact 签→dalek `verify_strict` 过,换 crate 无格式迁移成本)。依赖原则逐项全绿(spike 表):纯 Rust 静态链零外部工具链、落纯逻辑层(`src/sig.rs` 纯函数入测试网)、体积距 400 KB 线余量巨大、装机≈100% 构造性成立(纯算术零 OS API,零新 DLL 导入——dumpbin 双臂照 S1 纪律复核,见验证)。精确锁 `=2.6.0`(oxideav 先例),升级走独立票。公钥 32 B 以 hex 常量嵌 `src/sig.rs`(keyid/轮换逻辑首刀不做:换钥=发新 exe,新 exe 本身由旧用户手动过渡——见 D7)。

### D2. 签名形态 = per-asset raw `<asset>.sig` 64 B ×4,签名消息域分离

每个发布资产配一个 raw 64 B `.sig`(四资产 ×4,零解析面、免客户端哈希实现)。**被签消息 ≠ 裸资产字节**(cubic #236 P1#3 采纳:raw sig 只认证字节不认证身份,投毒渠道可重放旧签名资产做降级/错包替换):

```text
"riviv-update-v1\n" + <tag> + "\n" + <资产文件名> + "\n" + <资产字节>
```

tag 与文件名都是发版仪式已知值,客户端从检查得来的 tag 与自己构造的资产名得到同一对,验签前构造同前缀缓冲(纯函数)。效果:旧资产+旧 sig 在新 tag 下失配(降级重放闭死)、同版本异资产失配(错包替换闭死)、格式演进有 magic 可版本化。残余面如实:①`tag_name` 值与资产名都含 LF 时存在字段走私碰撞,但 tag 上游经 `parse_semver` 数字门、资产名恒为自构 ASCII 名,实践不可达(测试明文留档该已知残余);②**保障边界 = 安装版以下、非渠道全量**(cubic #238 P1 处置):无钥攻击者只能重放「严格新于安装版」的**真实签名发版**(decide() 数值门拒绝一切降级),`releases/latest` 响应被替换为陈旧真实发版时用户最多拿到「新于己、旧于最新」的真版本——miss-not-corrupt;「单调最低版本闸」类增强经 spike 评估**记录不推荐**(每发版改硬编码的仪式负担换冗余防线,对持钥对手可伪造任意「≤闸」消息无效),持钥者的任意伪造属 D7 泄漏预案面。**signer 与 client 的消息构造必须同一公式**(codex #236 P1:signer 签裸字节而 client 验域分离缓冲=每个正确发版都验签失败)——两份实现有意分离(见 D8),以「跨实现测试向量」双侧钉死锁合(`src/sig.rs` 与 riviv-sign 各内嵌同一向量,tag 与资产名故意异版防字段对调),发版回验环节是端到端哨兵。

### D3. 下载通道 = 复用 ADR 0007 WinHTTP 底座,流式写盘 + 128 MB 预算

E3 实测:默认重定向策略(跟随同方案 302)走通 github.com → release-assets CDN 链,零新重定向代码;下载 URL 由 tag+资产名直接构造(不经 API 二次解析——D2 的域分离使任何 URL 混淆都过不了验签,构造即安全)。`winhttp_get` 从 #210 的 `fetch_latest_body` 参数化抽出(host/path/sink),检查与下载共用一条传输壳;壳内统一 **HTTP 200 门**(cubic #238 P2:非 200 资产体落盘会死在验签、弹「渠道投毒」框——错分失败类)+ **每次读分配上限 64 KB**(`avail` 系服务器口径,#237 重构曾把 #210 的「先查预算再分配」退化成「先分配后查」——codex #238 抓回)。资产**流式写盘不进内存**(8.5 MB 量级,预算 128 MB 另立=敌意流防线,`asset_fits` 纯函数与 D4 的 64 KB JSON 预算同构);sig 走内存 sink 且必须**恰好 64 B**(404 的 HTML 体超预算自然落失败)。验签在盘上完整字节流落地后进行(读回内存一次,≤128 MB 上限内),绝不流中验签。**staging 位置分形态**(codex #236 P2 采纳):portable = exe 同目录旁置(`riviv-update-<tag>-<pid>.exe`,pid 后缀=per-flight 唯一——`multiple_instances=1` 双实例并发接同一更新时共享确定性名字会让一方的截断落进另一方的读/验签/spawn,最坏把半成品交进 helper(codex #238);清扫前缀不受影响;USB 盘的 `%TEMP%` 跨卷,`MoveFileEx` 跨卷要么退化 copy+delete 非原子要么直接失败);installed = `%TEMP%`(标准用户进程写不进 Program Files,Setup 自提升)。

### D4. 安装版路径 = Setup relay,先启后退,`/installdir` 传当前目录

安装版判定 = **Uninstall.exe 邻接**(安装器 staged `/install` 本来就复制它,`install_start_menu_shortcuts` 已用同款判定;codex #236 P2:注册表 uninstall 键/ini 都构不成判别式——nsi 只读从不写、appdata 是正交选项)。验签通过 → 一框「即将关闭 riviv 并运行安装程序」(OK/CANCEL)→ **先 `ShellExecuteW` Setup 再退出自身**(codex #236 P2:先退再启=无人调 ShellExecuteW);覆盖在跑文件(R7)由安装器既有握手解(`assoc.rs` staged `/install` 的 `close_existing_process` + `CopyFileW` 直覆盖)。**Setup 启动传当前安装目录**:nsi `.onInit` 接 `/installdir` 参覆盖 `$INSTDIR`(其 uninstall 键目录恢复现行脚本只读从不写——无此参时自定义目录更新会静默装出第二份而旧安装原地滞留)。边界如实收窄(codex #236 P1):保障止于「Setup 启动前」;安装器 `CopyFileW` 直覆盖无备份,中途 I/O 失败可破安装——assoc 先例,本刀记录不承诺。

### D5. portable 自替换 = 同文件 helper + 父进程句柄 + ReplaceFileW 三段

Sumatra helper 模式对映(codex #236 四 findings 订正后的形态):

- 下载物/验签对象 = `<exe 目录>\riviv-update-<tag>.exe`;helper = **同一下载物**,由旧实例以 `-update-self-to <原路径>` 启动;
- 等待对象 = **父进程句柄**(spawn 时 STARTF_USESTDHANDLES + hStdInput 继承传递,子端 GetStdHandle 读回;`multiple_instances=1` 时 RIVIV mutex 根本不创建,等 mutex 会立即通过对在跑映像动手——codex #236 P1);60 s 上限=挂死守卫(超时放弃,旧版无恙,残留由清扫收)。**straggler 实例残余如实**(cubic #238 P2 处置):同目录多实例下,helper 只等「发起方」退出;替换瞬间的另一在跑旧实例不受锁害(rename 型替换容忍运行中映像,其继续从旧字节运行),最坏情形 ReplaceFileW 失败或 `.old` 删除失败 → 走 fail-soft 框、旧版完整在位、下次更新重试——退化不损坏,「确保全部实例退出再替换」的进程间协调对 `multiple_instances=1` 的边缘并发不成比例,记录不采;
- **`-update-self-to` 短路在单实例 mutex 创建之前**(`run()` 内、install pass 后;#109 载重交互:helper 启动时旧实例仍持锁,走到转发逻辑会被当第二实例静默丢弃)——纯解析函数钉测;
- 替换序列:helper 读自身盘上字节(运行中可读)→ 写 `<orig>.tmp` → **`ReplaceFileW(orig, tmp, backup=orig.old)`**(三步 rename 舞的单调用形态:旧版字节完整在位直到替换落地、备份内建、失败即败不半换;第二步失败 rename-back 的回滚语义由该 API 合同承担)→ 启动新 exe → 删 `<orig>.old`(父进程已退;滞留旧映像的 straggler 实例使删除失败=无害,下次清扫重试)→ helper 退出;
- helper 自身映像运行中不可删 → **每次启动旁置清扫** `riviv-update-*.exe` 前缀、`<exe 名>.old` 备份与 `<exe 名>.tmp`(写 tmp 后、ReplaceFileW 前崩溃滞留的半成品;评审 #238 补 .tmp 盲区;exe 目录 + `%TEMP%` 两处,staged-ini 清理纪律同款;删除失败=仍被占用,fail-soft 下次再扫)。多实例残余:他实例的启动清扫撞上在飞下载时,open 句柄(无 FILE_SHARE_DELETE)使删除自然失败;下载已闭、验签前的窄窗被删则该 flight 退化为 fail-soft 重试(cubic #238 P2 处置记录,无安全面——被删物无法通过验签);
- 原子性如实限定(cubic #236 P2 二轮采纳):同卷双 rename 在 NTFS 原子,FAT/exFAT 家族替换语义弱——「旧版字节完整在位直到替换成功 + 失败可回滚」成立,两段之间的中断窗口随文件系统而异,留档不掩盖。

### D6. 失败语义 = 独立 UX 表(ADR 0001 不裁,同 0007 D3 先例)

| 失败类 | 用户面 | stderr 面 | 旧版保护 |
|---|---|---|---|
| 网络/超时/下载中断 | 框「下载失败,当前版本未受影响」 | 各 stage GetLastError 面包屑(fetch 同型) | 删半成品;旧版不动 |
| **验签失败** | 框「签名校验失败,已取消安装,请勿安装此更新」 | 记录资产名/tag/期望公钥前缀 | **删下载物;绝不执行;无自动降级;不引导去 releases 页**(codex #236 P1:验签失败本身可能意味渠道正被投毒,递梯子=把安全边界变点击旁路;人肉通道用户自担) |
| 磁盘写失败/空间不足 | 同网络类 | io 错误原样 | 旧版不动 |
| 替换/安装器失败 | fail-soft 框「更新失败,当前版本未受影响」 | 面包屑 | portable:替换成功前旧字节完整在位、失败可回滚(两段 rename 间中断窗口随 FS 而异,D5);installed:保障止于 Setup 启动前(D4) |

全部后台线程,UI 永不冻结;单飞闸(INSTALL_RUNNING)挡重入。

### D7. 密钥仪式与事故预案(便利层/安全层分离)

私钥 = 32 B seed 文件,**离线双拷、永不入库、不进构建**;公钥 hex 嵌 `src/sig.rs`(#237 实现时 keygen 生成真钥;spike 探针固定种子钥仅测试用、明文留档视同已烧)。发版仪式增补见 RELEASE.md(四资产各签、上传 4+4=8、回验升格「下载 exe+sig → riviv-sign verify → SHA 对拍」)。事故预案(codex #236 P1 订正后形态):**丢失**(无对手)→ 发布带新公钥的版本,老版本内置更新断供,用户手动过渡一次,无安全问题;**疑似泄漏** → 双钥过渡**不构成安全恢复**(旧钥持有者可对任意字节造合法签名,「过渡版验任一」=该窗口内完全可伪造;pinned-key 模型下旧钥无法真正吊销是固有残留,同 Tauri/minisign 形态)。便利层=非泄漏场景平滑换钥(过渡版双签→只验新钥,攻击面随存量升级单调收敛,如实留档);安全层=确认泄漏时**带外公告**(OSV/README)用户停止内置更新、手动从可信渠道取带新钥版本。根治=Authenticode 级链式信任,超出本刀。可选缓解「exe 内嵌最低安全版本单调闸」记录不推荐(仪式负担换冗余防线,且对持钥对手可伪造任意「≤闸版本」消息无效)。

### D8. riviv-sign = 仓内独立小工程(不做主 crate 第二 [[bin]])

`tools/riviv-sign/`(自有 Cargo.toml,不进主构建图):`keygen`(ProcessPrng OS 随机,仅工具侧 feature)/`sign`/`verify` 三命令。**不得**做主 crate 第二 `[[bin]]`——同一构建图内 cargo feature 合并会把签名侧 feature 并进 ship 构建污染最小面(spike 载重结论)。签名消息构造公式与 `src/sig.rs` **两份实现有意重复**(工程隔离的代价),以 D2 的跨实现向量+发版回验哨兵锁合。

### D9. 触发 UX = 三钮 Available(P4=A),启动 flash 维持 notify-only

手动 Available 框升 `MB_YESNOCANCEL`:**是**=下载验签安装全自动通道(主动询问者得全自动)、**否**=仅开 releases 页(notify-only 老路)、**取消**=关框;启动 flash 维持 notify-only(被动通知不自动碰盘)。portable 的「是」直达自替换(三钮之「是」已是明确授权,无二框);installed 多一框 D4 的安装器预告(OK/CANCEL)。

## 验证

- 纯函数入测试网:域分离消息构造字节级钉测+sign/verify 往返+跨实现向量+篡改/重放/错包/错钥五向拒绝(sig.rs 7 测试);资产名对拍真实 v0.8.0 四资产名、staging/URL 构造、128 MB 预算边界、Uninstall.exe 邻接判定(update_install.rs 5 测试);helper 行解析全拼写/非 helper 行/悬空参、清扫谓词、清扫只删残留(self_replace.rs 5 测试);Outcome::Available 改带原始 tag 的 decide 钉测更新。
- loc 三表:新 4 串(安装预告/下载失败/验签失败/替换失败)+ 三钮措辞改写;非空网/撞车网自动覆盖。
- 机械面(双臂亲录 2026-10-27,VS BuildTools dumpbin 14.51.36231):arm A = `ba46b39` 重建 8,523,776 B(与 v0.8.0 发布字节一致,基线复现);arm B = 评审处置终态 8,598,016 B,**库内净增 +74,240 B**(初版 8,596,480 → 孪生收敛 8,595,968 → 状态码门+pid 后缀等评审修复 8,598,016;发版锚以全量干净重构建为准——v0.7.0 勘误先例。探针 +30,208 B 系 verify-only 口径:s-avif 教训重演,探针差分不含本刀其余新代码面 update_install/self_replace/loc 等;量级距任何阈值余量充分,无需破例);dumpbin 双臂各 25 唯一 DLL、**diff 恰零条=零新 DLL 导入**(纯算术零 OS API,GetStdHandle 在既有 kernel32 面;WinHttpQueryHeaders 在既有 winhttp 面)。
- 冒烟四臂(smoke237,子代理串行):验签通过(工具级;应用内全链待首个带 `.sig` 发版)/篡改 1 B 失败/网络失败/磁盘失败 + 资产选择(三语名=单测对拍真实 v0.8.0 资产名;运行时臂=本机语言+双形态的 stderr 面包屑断言——UI 语言按 D4 同款检测即一切,单值不可注入)+ `-update-self-to` 端到端(含持锁真窗口在场证明短路不被转发);1607 真机走既有观察项通道。
- 兼容面:v0.8.0 存量只认 notify-only(无 `.sig` 资产时点「是」=下载失败框,不炸),**首个带签名资产的发版起仪式生效**。
