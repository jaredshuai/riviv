# S-TRUSTED-DOWNLOAD:更新「下载+安装」信任锚 spike(三路线带数字)

- 票:#235(O1=B/C 的前置证据电池;ADR 0007 D5 点名「称『已验证下载』前须另立信任锚」)
- 日期:2026-10-26;机器:Win11 build 26300,rustc/cargo 1.98.0(与 #215 重审同机)
- 全量证据:探针工程与原始输出 `%TEMP%\riviv-s235\`(勿入库);关键数字下文引录。
- 取舍原则:信任锚独立于 GitHub 账号/CDN(防投毒,D5 的动因)> 依赖原则全绿
  > 体积 > 验签速度(单次更新非热路径)。
- 边界:**spike ≠ 开工**——证据收敛后用户拍板(P1-P4,见文末),再另立实现票。

## 结论一句话

**系统 CNG(bcrypt.dll Ed25519)路线双证据否决**(官方算法表无此条目 + 本机
Win11 26300 `BCryptOpenAlgorithmProvider(L"ED25519")` 返回
`0xC0000225 STATUS_NOT_FOUND`);纯 Rust 两候选全过依赖原则,横评推荐
**ed25519-compact =2.6.0 + `opt_size`(零依赖单叶 crate,+30,208 B 探针口径,
MIT)**,备选 ed25519-dalek(+63 KB,BSD-3,依赖树 ~10 crate);WinHTTP 默认重
定向策略实测走通 GitHub 资产 CDN(8,523,776 B 逐字节同源回验);端到端仪式演示
(真实 v0.8.0 portable 资产 → 固定 seed 离线签名 → verify-only 双配置验签通过,
25.7/8.1 ms)全链贯通。**建议拍板开实现票(O1=B+C 同刀)**。

## 证据块

### E1 系统 CNG 路线:否决(双一手证据)

| 证据 | 来源 | 结果 |
|---|---|---|
| 官方算法表无条目 | learn.microsoft.com「CNG Algorithm Identifiers」整页亲读(2026-05-30 更新版) | 表列 3DES→XTS-AES 全家族(含 Win11 24H2 新增 ML-DSA/SHA3/KMAC),**无 ED25519** |
| 本机运行时拒绝 | 探针 `%TEMP%\riviv-s235\probe-cng\`(windows 0.62.2 `BCryptOpenAlgorithmProvider(w!("ED25519"))`) | Win11 26300(最新构建)返回 `0xC0000225 STATUS_NOT_FOUND`——最新版都没有,1607 地板更无从谈起 |

注:WebSearch 转述层曾两次给出相反结论(一次称「1607+ 支持」,一次称「不支持」);
按工具纪律以一手证据为准,转述弃用。附带否决面:即使未来 CNG 加入 Ed25519,
未文档化/地板不明的 API 也过不了 S1 地板纪律,不构成路线。

### E2 纯 Rust 候选横评(探针口径 = 双 bin 差分;五工程同 harness、同
`[profile.release]`(lto=thin+strip,riviv 同款)、消息走运行时文件读防 const-fold)

| 路线 | 体积增量(vs base 139,776 B) | 依赖树 | 许可 | 验签耗时 @8,523,776 B(真实资产) | 备注 |
|---|---|---|---|---|---|
| **compact 2.6.0 裸档**(`default-features=false`) | +205,824 B | **0 crate(单叶)** | MIT | 未实测(opt_size 仅 inline 策略差,计算量同阶;速度面非决策因素——cubic #236 P3 计时归属订正:裸档未上大文件) | 全 `inline(always)` 展开致大 |
| **compact 2.6.0 + `opt_size`**(推荐) | **+30,208 B** | **0 crate(单叶)** | MIT | 25.67 ms(big.bin 复测 26.98 ms) | `opt_size`=算术层 `inline(never)`(源码亲核),舍速度换体积;速度对单次更新无感 |
| dalek 2.1.0(`default-features=false`,`verify_strict`) | +62,976 B | ~10 crate(curve25519-dalek+sha2+subtle+digest 族+proc-macro 链,全纯 Rust、零 `*-sys`/cc/bindgen) | BSD-3-Clause | 8.14 ms(big.bin 复测 9.09 ms) | 2.1.0 = 长稳线(2024-01 起) |
| dalek 3.0.0(同上) | +65,024 B | 同上 | BSD-3-Clause(MSRV 1.85) | 8.79 ms | 3.0.0 刚发布(registry 亲核),churn 风险 |
| ring / libsodium-sys | 未探针 | C/asm 经 cc 编译 | — | — | 依赖原则①否决(非纯 Rust),同 s-avif 对 reqwest 的处理 |

- 元数据来源:`cargo info --registry crates-io` 亲跑(license/版本/feature 面);
  探针体积 = 本机实测,全部探针 `cargo build --release` 零外部工具链零网络。
- **跨实现互操作亲证**:compact 固定 seed 签出的签名,dalek `verify_strict`
  (拒绝非规范/可锻造格式)验签通过——两候选同构标准 Ed25519,换 crate 无格式
  迁移成本。
- 探针口径警示(s-avif 先例):上述 Δ 含探针 harness,库内真实增量以实现票
  双臂实录为准;量级(数十 KB)距任何阈值都有充分余量。

### E3 下载通道(真实资产实测)

探针 `probe-dl`:对 v0.8.0 真实 portable 资产
`github.com/jaredshuai/riviv/releases/download/v0.8.0/riviv-0.8.0-portable.exe`
单 GET,**不设任何重定向策略**(默认策略 = 跟随同方案重定向):

- 最终状态 `200`(WinHTTP 自动走完 github.com → release-assets CDN 的 302 链),
  完整体 8,523,776 B;
- 落盘 SHA-256 `0eb087c1fc807e4bd5228bd61be7bfc4c44d332d9da0d23716cb5987aea8ca8e`
  ——与 v0.8.0 发版四资产下载回验留档的 SHA 逐字节同源。

结论:现有 WinHTTP 底座(ADR 0007 D1)不需要为新重定向面加任何代码;资产
流式下载 = 现有 `fetch_latest_body` 读环的放大版(写盘代替内存累积,另立预算,
见失败语义)。

### E4 端到端仪式演示(信任锚全链)

固定 seed `000102…1f`(探针专用,公钥 `03a107bf…31b8`)→ `probe-keygen`
对 E3 下载的真实资产离线签名 → `probe-time`(compact+opt_size 与 dalek
**2.1.0** 双配置)验签:双 `true`,25.67 ms / 8.14 ms(dalek 3.0.0 单独工程
复测 8.79 ms)。**下载(过 CDN)→ 验签 → 通过,即产品形态的最小闭环,探针级
全部走通。**

## 依赖原则逐项(ed25519-compact + opt_size 路线)

| 原则 | 判定 | 依据 |
|---|---|---|
| ① 纯 Rust+静态链+零外部工具链 | **过(实测)** | E2 探针;依赖树 = 自身一个 crate,零 `*-sys`/cc/bindgen |
| ② 新依赖落纯逻辑层 | 过(设计) | 验签决策纯函数(bytes+sig+pk→verdict)入测试网;unsafe 壳(网络/文件/进程)不加码 |
| ③ 体积 | **过且余量巨大**:+30,208 B 探针口径(本刀非解码格式,400 KB 线仅作参照;无需 ADR) | E2 |
| ④ 同管线 | N/A(非解码器);对应纪律 = 验签决策/资产选择(locale→asset 名)纯函数化 | 设计 |
| ⑤ 优先 image 同族 | N/A(image 生态无 Ed25519;网络面先例 = 系统能力,本刀系纯逻辑新面) | — |
| ⑥ 装机≈100% 无授权费 | 过(构造性) | 代码随行、MIT,行为不随装机变;纯算术零 OS API,无 1607 地板面、零新 DLL 导入(实现票仍按 S1 跑 dumpbin 双臂复核) |

## RELEASE.md 仪式接法(推演,细节实现票定)

1. **密钥**:一次性 `riviv-sign keygen` 生成(seed = 32 B 私钥文件,**永不入库
   不进构建**,离线双拷);对应公钥 32 B 以 hex 常量嵌 `src/`。
2. **签名工具 = 仓内独立小工程 `tools/riviv-sign/`**(不进主 crate/主
   workspace):**不能**做成主 crate 的第二 `[[bin]]`——同一构建图内 cargo 的
   feature 合一会把签名侧 feature(随机数等)并进 ship 构建污染最小面。命令面
   `keygen / sign / verify`(sign = `sk.sign(asset_bytes, None)` 确定性签名,
   与探针同 API)。
3. **发版新步骤**(插在四资产构建后、`gh release create` 前):每资产
   `riviv-sign sign` → `<asset>.sig`(raw 64 B,4 个);上传资产 4+4=8 个文件;
   现有「下载回验 SHA」环节升格为「下载 exe+sig → `riviv-sign verify` → SHA
   对拍」(机械面不变量 +1)。
4. **签名消息域分离(cubic #236 P1#3 采纳:raw sig 只认证字节,不认证版本/
   资产身份——被投毒渠道可重放旧签名资产做降级/错包替换,须绑定)**:被签消息
   ≠ 裸资产字节,而是
   `"riviv-update-v1\n" + <tag> + "\n" + <asset-文件名> + "\n" + <资产字节>`
   (tag 与文件名都是发版仪式已知值,客户端从 API `tag_name` 比较与
   `assets[].name` 选择得到同一对,验签前构造同前缀缓冲——纯函数)。效果:
   旧资产+旧 sig 在新 tag 下失配(降级重放闭死)、同版本异资产失配(错包替换
   闭死)、未来格式演进有 magic 可版本化;`.sig` 仍 raw 64 B 零解析面。
   资产字节整体进内存一次(≤ 128 MB 预算同内存上限,8.5 MB 量级无压力)。
5. **私钥事故预案**:丢失 → 老公钥无法再签,发布带新公钥的版本,老版本内置
   更新断供(用户手动下载一次过渡版,无安全问题);疑似泄漏 → **换钥 + 过渡
   版双钥各签(资产 `.sig`/`-v2.sig` 并挂),过渡版验任一;过渡版之后的版本
   一律只验新钥**——「验任一」只存在于过渡版存续期,且攻击者需同时持旧私钥
   与渠道投毒能力才构成攻击面,该面随存量升级单调收敛(cubic #236 P1#2
   采纳后的形态:旧钥在 pinned-key 模型下**无法真正吊销**是固有残留——只内嵌
   旧公钥的老 exe 最多信任到过渡版为止,同 Tauri/minisign 形态,接受并留档;
   根治 = Authenticode 级链式信任,超出本刀)。可选缓解「exe 内嵌最低安全版本
   单调闸」记录不推荐(每发版改硬编码,仪式负担换冗余防线)。

## portable 自替换(O1=C,推演;Sumatra helper 模式对映,s-auto-update O1=C 先例)

- 安装版(O1=B):下载对应语言 Setup 资产 → 验签 → 框「将关闭 riviv 并运行
  安装程序」→ 自身退出 → `ShellExecuteW` Setup(安装器既有 staged exe
  `/install` runas 自提升先例;R7 的「覆盖在跑文件」由先退出解掉)。
- portable 版:下载新 portable 资产 → 验签 → 存 `%TEMP%` → 以
  `-update-self-to <原路径>` 启动 helper(新 exe)→ helper 等旧进程退出
  (等单实例 mutex 消失)→ `MoveFileEx` 替换原位 → 启动新 exe → helper 退出。
  **与 #109 单实例转发的交互是载重点**:helper 启动会被 mutex 转发拦成「第二
  实例」——`cli.rs` 必须在 mutex 创建**之前**识别 `-update-self-to` 短路
  (installer `/install` 同型先例),实现票钉测。
- 资产选择纯函数:安装版三语 Setup(zh-CN/zh-TW/en-US,loc 已检测系统语言)
  / portable 版 portable 资产;「本实例是安装版还是 portable」的判定(注册表
  uninstall 键/ini 区分)实现票定。

## 失败语义(推演;ADR 0001 不裁此面——同 ADR 0007 D3 先例,更新类失败独立 UX)

| 失败类 | 用户面 | stderr 面 | 旧版保护 |
|---|---|---|---|
| 网络/超时/下载中断 | 框「下载失败」 | 各 stage GetLastError 面包屑(fetch 同型) | 删半成品;旧版不动 |
| **验签失败** | 框「签名校验失败,已取消安装」 | 记录资产名/期望公钥 | **删除下载物;绝不执行——D5 不变式的硬边界,无任何自动降级**(人工通道:提示可去 releases 页) |
| 磁盘写失败/空间不足 | 同网络类 | io 错误原样 | 旧版不动 |
| 替换/安装器失败 | fail-soft 框 | 面包屑 | 旧版保持可运行 |

预算纪律:JSON 元数据维持 64 KB(D4 不变);资产流式**写盘**(不进内存),
上限另立(建议 128 MB 量级,敌意流防线);验签在盘上完整字节流完成后进行。

## 风险与缓解

1. **compact 单主维护者**(jedisct1):零依赖叶 + 精确锁 `=2.6.0`(oxideav
   先例)+ 实际取用面极小(from_seed/sign/new/verify 四符号);E2 互操作亲证
   换 dalek 是机械替换,无格式迁移。
2. **API churn**:2.x 面稳定(2.6.0 与本 spike 探针逐一编译验证);升级走独立票。
3. **私钥管理是人的纪律**:RELEASE.md 提醒项 + 离线双拷;事故预案见仪式节。
4. **未验证面(实现票 QA 项)**:慢网中断臂、磁盘满臂、验签失败臂(篡改 1 B
   重签走 false)、三语资产选择臂、`-update-self-to` helper 端到端;1607 真机
   仍走既有观察项通道(纯算术零 OS 面,构造性无风险)。

## 拍板项(全附推荐;拍板后开实现票,预计 = ADR 0007 扩 D7 或新 ADR + 下载半边
+ cli helper 分支 + loc 三表 + riviv-sign 工具 + 冒烟四臂 + RELEASE.md 仪式增补)

- **P1 crate**:A = ed25519-compact `=2.6.0` + `opt_size`(**推荐**:零依赖、
  +30 KB、MIT、够快)/ B = ed25519-dalek 2.1.0(更快 3×、组织维护,但 +63 KB
  且依赖树 ×10)。
- **P2 签名资产形态**:A = per-asset raw `<asset>.sig` 64 B ×4,**签名消息按
  仪式节 4 域分离绑定 tag+资产名**(**推荐**:零解析面、免客户端哈希、降级
  重放与错包替换闭死)/ B = 单一 signed manifest(少 3 个资产,但引入
  manifest 格式+SHA-256 依赖+解析面,且防重放同样要绑版本——域分离的裸 sig
  已覆盖其安全收益)。
- **P3 首刀范围**:A = O1=B+C 同刀(**推荐**:下载/验签/失败语义底座完全共用,
  portable 半边缺了则 portable 用户无法用内置更新)/ B = 先 B(仅安装版)后 C。
- **P4 触发 UX**:A = 手动 Available 框升三钮 MB_YESNOCANCEL
  (是=下载验签安装 / 否=仅开 releases 页 / 取消),启动 flash 维持 notify-only
  (**推荐**:主动询问者得全自动通道,被动通知不自动碰盘)/ B = 双通道都装。
