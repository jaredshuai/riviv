# I18N beyond en/zh-CN 开题 spike

- 票:#201(本 spike 为其唯一交付;实现票在咨询/拍板后另立)
- 日期:2026-10-03
- 基线:master `45bcee4`;exe 6,862,336 B(#199 收官口径,本票零代码变更不重测);test 824 过/0 fail/3 ignored(#199 收官口径,commit 钩子将亲跑)
- 证据性质:上游锚点/代码锚点均为本会话亲读(grep/Read/sed);NSIS 语言包 = 本机 `Contrib/Language files` 亲核;**本轮未跑任何新探针**——docs-only 无行为面,新语言实测观感留给实现票

## 结论一句话

I18N beyond en/zh-CN 是 viv.c:30 的一句愿望(`[HIGH] language_id needs to go in translation. we don't want to check for it in code.. has to be a list too, to support multiple language ids`),上游从未动工——机制面(语言数组 + `get_string` 二维索引 + 启动一次 `GetUserDefaultUILanguage` 检测)riviv 已全量镜像且结构可扩,加一门语言 = 新 `Language` 变体 + 新表 + 三处 match 臂 + 测试缝收拢;两处载荷不变量先钉死(**英语表 = ini 命令名 wire format,永不本地化**;**检测即一切、无运行时切换 = 上游语义**);真功夫在**翻译供给**(293 串/语言 100% riviv-authored,上游无第三语言表可搬)与**每语言逐菜单层助记符重分配**;zh-Hant 是最便宜首刀(修上游 zh-TW/zh-HK→简体表的已知粗映射、简繁转换供给最易、字体同路、NSIS 自带 TradChinese 包)。

## 为什么做(上游与 wishlist)

1. 上游 wishlist 点名、零实现:viv.c:30——愿望说的是「language_id 进翻译、列表化以支持多语言 id」,即**数据驱动的语言表**,不是运行时切换;上游最终只 ship 了 en_us/zh_cn 两表。
2. README Roadmap Unscheduled wishlist 最后余项(README.md:110,playlist pane #199 落后仅剩此一项)。
3. beyond-original 表面(上游无第三语言参照)→ README Differences 记段,记法参照 #68/#178/#199 先例。

## 上游面证据(全部本地可复核)

| # | 事实 | 证据 |
|---|---|---|
| A1 | 愿望原文 | viv.c:30 `[HIGH] language_id needs to go in translation. we don't want to check for it in code.. has to be a list too, to support multiple language ids` |
| A2 | 语言面 = 两表 + 数组 | localization.h:30-32(`LOCALIZATION_LANGUAGE_ENGLISH=0/CHINESE_SIMPLIFIED=1/COUNT=2`);localization.c:28-32 `_localization_language_array[COUNT]` |
| A3 | 表规模 | localization.h ID 枚举 247 项(grep 亲计,含 COUNT 哨兵;loc.rs 头注同口径);en_us.h 297 行 / zh_cn.h 298 行 |
| A4 | 取串 = 二维索引 | `localization_get_string` = `array[lang][id]`(localization.c:36-48);debug 版 id 越界 Fatal |
| A5 | 检测即一切,无运行时切换 | `localization_init` = `GetUserDefaultUILanguage`;3 个中文 langid(0x0804 zh-CN / 0x0404 zh-TW / 0x0C04 zh-HK)→ zh 表,否则英语(localization.c:55-73);全库无 language setter |
| A6 | 繁中 langid 被粗映射到简体表 | localization.c:64-71 注释自认(Check if it's Chinese (Simplified or Traditional))——zh-TW/zh-HK 系统用户看到简体 UI |
| A7 | **英语表兼任 ini wire format** | localization.c:50-53 `localization_get_en_us_string`;viv.c:12104/12139 用它构造 ini 命令名;viv.c:12127 注释原文「make sure this is english only.」 |
| A8 | 上游无第三语言表可搬 | `c-original/src/` 语言头仅 localization_en_us.h / localization_zh_cn.h 两个(ls 亲核) |

## riviv 复用面证据(代码锚点,全部本会话亲读)

1. **机制全量镜像**:`src/loc.rs`——`Id` 枚举 293 变体(已超上游 247:效果链三刀刻度、undo-delete、playlist pane、usage 正文等 riviv-authored 串随消费 issue 生长,既定方针);`EN_US`/`ZH_CN` 两张 `[&str; Id::COUNT]` 长度编译锁;`language_from_langid` 纯函数 + 钉测;`init()` 于 window.rs:9957 一次(UI 前);无运行时切换(镜像上游)。
2. **加一门语言的机械面**:新 `Language` 变体 + 新表 const + 三处 match 臂(`table()`/`language_from_langid`/`current_language`)——数组类型长度锁自动覆盖新表,漏串即编译错。
3. **测试缝现状 = 显式两语言枚举**:menu.rs:2593-2594、options.rs:1182-1199、loc.rs 内部电池,均 `get_for(English)`/`get_for(ChineseSimplified)` 成对硬编码——第三表前先收拢为 `Language::ALL` 常量数组遍历,否则每缝逐臂加、漏缝即测试静默不全。
4. **英语表 wire 语义已镜像**:keys.rs:511/527 `get_for(Language::English, …)` 构造 ini 命令名——新语言永不触碰英语表 = 持久化兼容不变量。
5. **消费面单点**:loc::get 65 处 × 9 文件(菜单/对话框/状态栏/标题),全部经 `get_for` 单点,无散落表索引。
6. **字体无缝**:全库 `GetStockObject(DEFAULT_GUI_FONT)`、零 `CreateFont`/零 charset 硬编码(custom_rate_dlg/everything/jumpto_dlg/options_dlg/rename_dlg/pane 六处亲核,#199 新落的 pane.rs 同路)——新文字系统走系统 font linking,与今日 zh-CN 同路。
7. **热键余量**(本会话复验 keys.rs 默认表):Ctrl 无修饰已占 B C E K L O P Q R S T V W X Z(15 个);**余 A/D/F/G/H/I/J/M/N/U/Y + Ctrl+1..9 全空**——仅当拍板要运行时切换命令时才消耗(O3 裁 B/C 时)。

## 安装器面(本机亲核)

- riviv.nsi:`!define LANG`(默认 Chinese,上游默认);`!if` 分支选 license txt + `InstallOptions{,2}` ini 对 + LANG_CODE + `MUI_LANGUAGE`(SimpChinese/English)(riviv.nsi:19-56/99-102);输出名 `riviv-<ver>.x64.<lang>-Setup.exe`。
- build-installer.ps1:`-Lang` 白名单 Chinese|English(build-installer.ps1:12-15)。
- 本机 NSIS `Contrib/Language files` 亲核:English/SimpChinese/**TradChinese** 三语言包齐——zh-Hant 首刀的安装器面零新依赖。
- 安装器语言 ≠ 应用语言:应用运行时自检(上游同,A5),安装器语言只管安装向导本身;单语言单 exe 语义不变(双语照旧跑两遍)。

## 侵入面与风险(实现票主战场)

1. **翻译供给 = 最大成本项**:293 串/语言 100% riviv-authored(A8);含菜单助记符逐层重分配、Options 三页全控件、usage 正文(~35 行)、安装器 ini 对 + license txt。
2. **助记符冲突面**:每语言每菜单层独立分配——先例三条:OptionsFillWindow 去 `&F`(同对话框撞)、MenuSharpen 无助记符(View 层 S 已属 Slideshow)、MenuPlaylistPane 取 L(View 层 P 属 Preset);新表逐层审,钉测可机械化(单串至多一个 `&`、同菜单层/同对话框不撞)。
3. **测试缝收拢重构**:`Language::ALL` 遍历化(一次把「每语言非空」收进单循环)+ 新表逐串钉测电池(同 loc.rs `menu_strings` 先例)。
4. **langid 映射变更面**:0x0004 中性 langid 仍英语(上游行为,钉测在);zh-Hant 表落位后 0x0404/0x0C04 改映射新表——行为变更(今日繁中系统用户从「简体 UI」变「繁体 UI」),README Differences 记段。
5. **unsafe 壳零增长**:纯数据 + 纯函数,不触窗口/GDI/COM 面。
6. **体积**:纯数据,293 串 × 每语言几十 KB 内,零新依赖,不触 ADR 体积阈值。

## 语义与边界(设计要点,待咨询/拍板确认)

- **英语表 = ini wire format,永不本地化**(A7 + 复用面 4)——一切新语言只加自己的表,不动英语表。
- **检测即一切**(上游 A5):进程启动一次 `GetUserDefaultUILanguage`,进程内不变;若 O3 裁 B/C(Options 下拉 / CLI 开关),是 beyond-upstream 表面,README Differences 记段。
- **enum 随消费 issue 生长的既定方针不回填**:上游 247 表中 riviv 未携带的死 ID 不在本票补。
- 文字转换(若 O1=A zh-Hant):一次性离线转换产物入库,转换工具不进构建链(零依赖原则);转换后仍须人眼终审(术语/助记符非机械面)。

## 表面(面向用户)

- 代码:`Language` 枚举 + 新表 + langid 映射 + 测试缝(若 O2=A:`Language::ALL`)。
- loc:新表 293 串。
- config:无新 ini 键(若 O3=A)。
- 安装器(若 O5=A):license txt + ini 对 + nsi 分支 + ps1 白名单 + MUI_LANGUAGE。
- README:Differences 记段(beyond-upstream 表面 + zh-Hant 映射变更)。

## 开放问题(附推荐项;O 编号沿 #180/#197 咨询票格式)

- **O1 语言清单**:A(推荐)首刀 zh-Hant 一门——修 A6 粗映射、供给最易(简繁转换 + 人眼终审)、字体同路、NSIS 自带 TradChinese 包 vs B 多门齐上(供给 ×N、评审面同乘)vs C 用户点名清单(任意语言,翻译供给另议)。
- **O2 语言表形状**:A(推荐)`Language` 枚举 + `Language::ALL` 常量数组 + 测试缝全语言遍历(viv.c:30「has to be a list」的精神,不引注册表机制)vs B 枚举 + match 逐缝加臂(省一行注册,漏缝风险 = 测试静默不全)vs C 完整注册表结构(name/langids/table 元组数组;对 ≤4 语言规模过度)。
- **O3 选择机制**:A(推荐)检测即一切、无运行时切换、无 language ini 键(镜像上游 A5;O1=A 时 zh-TW/zh-HK 系统用户自动落新表)vs B Options General 加 language 下拉 + ini 键(beyond-upstream;触 config 面 + 冒烟面)vs C CLI `/lang` 开关(beyond-upstream;触 CLI 面)。
- **O4 翻译供给**:A(推荐)agent 起草 + 用户逐表终审(钉测随起草入库;终审 = 用户母语眼)vs B 用户自译(最准,用户工时 293 串)vs C 留坑等社区 PR(表结构就绪,内容后补)。
- **O5 安装器**:A(推荐)同步加该语言 license + ini 对 + MUI_LANGUAGE(NSIS 现成包,O1=A 时 TradChinese 齐备)vs B 只发 zip/exe 不动安装器(新语言用户拿英语安装器,不对称)。
- **O6 助记符**:非选择项,每语言必做功——A(推荐)逐层手工分配 + 机械化钉测(单串至多一个 `&`、同菜单层/同对话框不撞),沿 #199 L 先例。

## 边界划清(不进本票)

相邻面各自成票或明示不做:运行时热切换(若 O3=A 则不存在)、RTL 镜像布局(无 RTL 语言进清单才议)、README/文档多语言化、上游死 ID 回填、安装器多语言单 exe(NSIS 单语言语义不变)。本票(及其实现票)只做**静态语言表扩展**——语言在启动时检测后定死,进程内不变。

## 决策影响

- 本票交付 = 本文档 + ARTIFACTS 登记;零代码变更。四门禁照常(commit 钩子),test 基线 = master 824 过/0 fail/3 ignored(#199 收官口径)。
- 拍板路径:用户跑外部三 AI 咨询(提示词随 PR 附)或直接拍板 O1-O6 → 实现票拆分候选:①`Language::ALL` 重构 + 测试缝收拢 ②zh-Hant 表 293 串 + 助记符 + 钉测 ③安装器面 + README + 冒烟。
- ADR:纯数据 + 零新依赖,不触体积阈值/平台地板,预计无需新 ADR;若 O1=C 引入非 CJK 语言且 font linking 实测缺字,再议。

## 验收

- 文档入库(PR 合并);commit 钩子 fmt/clippy/test 绿。
- QA 清单:N/A(docs-only,无行为变更)。
