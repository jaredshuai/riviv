# playlist pane / tool window 开题 spike

- 票:#197(本 spike 为其唯一交付;实现票在咨询/拍板后另立)
- 日期:2026-10-02
- 基线:master `6474c2a`(v0.4.0 收官);exe 6,859,776 B(v0.4.0 发布口径,本票零代码变更不重测);test 817 过/0 fail/3 ignored(v0.4.0 发版口径,commit 钩子将亲跑)
- 证据性质:上游锚点/代码锚点均为本会话亲读(grep/Read);**本轮未跑任何新探针**——布局实测类结论(auto-fit 计入面板宽的观感、巨列表滚动性能)留给实现票

## 追记(2026-10-03,用户拍板)

- 用户裁定原文:「我没啥要排版的,保持和原版软件一致就行」。因原版对该面板零实现零规格,可执行读法 = **六题全按推荐项 + 观感对齐原版既有 chrome**(全停靠、系统原生控件、极简表面):O1=A 停靠 pane(右缘;原版 chrome 无浮动窗先例)、O2a=A 快照重建、O2b=A 固定名序(Jump To=原版唯一列表 UI 先例)、O3=A v1 即 owner-data 虚拟化、O4=A v1 仅选中/双击/Enter 跳转、O5=A 开关+宽度两 ini 键(参照原版持久化 toolbar/几何惯例)、O6=A 默认无键、Controls 页可绑(原版键表无 toolbar toggle 键)。
- auto-fit 计入面板宽(窗宽 = 面板 + 图像有效区,图像保持完整可见——与「窗随图缩放」语义一致);fullscreen 藏面板随其余 chrome 退出、恢复时还原(原版 fullscreen=纯图面)。

## 结论一句话

playlist pane 是 viv.c:36 的一句愿望(`[HIGH] playlist pane or tool window`),上游零实现、零规格(死菜单枚举 `_VIV_MENU_NAVIGATE_PLAYLIST` 证之;上游无应用级 .rc,菜单全代码建表,无 UI 规可对);riviv 侧模型与列表构建**全部现成**——playlist.rs(#6/#39)承载全部播放列表语义,jumpto_dlg.rs:98-132 的 `entries().to_vec()` → `nav_compare` 排序 → `filename_part` 展示就是面板的列表构建链,故本特性 ≈ **把 Jump To 对话框做成持久停靠/浮动面板 + 导航实时高亮同步**;新工作集中在 UI 壳、三个布局侵入面(#80 viewport 几何 / auto-fit 窗口缩放 / fullscreen)、大列表虚拟化、同步策略;beyond-upstream 表面照 #68 keep_zoom、#178 undo-delete 先例记 README Differences。

## 为什么做(上游与 wishlist)

1. 上游 wishlist 点名、零实现:viv.c:36——仅此一句,无任何规格。上游 DONE 段的「*check current item in playlist」(viv.c:167)说明上游当年用跳转类既有 UI 满足过「标当前项」子诉求,面板本体从未动工。
2. README Roadmap Unscheduled wishlist 余项之一(README.md:110,效果链三联清空后与 I18N beyond en/zh-CN 并列)。
3. beyond-original 表面(上游无参照)→ README Differences 记段,记法参照 #68、#178 两个 riviv-authored 先例。

## 上游面证据(全部本地可复核)

| # | 事实 | 证据 |
|---|---|---|
| A1 | 愿望原文一句、无规格 | viv.c:36 `// [HIGH] playlist pane or tool window` |
| A2 | 零实现:菜单枚举死位 | `_VIV_MENU_NAVIGATE_PLAYLIST`(viv.c:332)全文件仅 1 次出现——定义处,从未被任何建表/填充代码引用;viv.c:5015 另有一条同名词面注释「playlist mode. (navigate the playlist)」(命令行单/多文件语义,不含该标识符) |
| A3 | 无上游 UI 规可对 | 上游 src/ 无应用级 .rc(仅 libwebp 内部 .rc);菜单由 `_viv_commands[]` 代码建表(viv.c:3399/12332 一带),字符串走 localization_en_us.h / localization_zh_cn.h |
| A4 | 底层模型早已完整(riviv 已镜像) | 双向链表、全路径存 cFileName、插入 id、导航不重排(每次按键重扫 compare 取严格后继)、shuffle 惰性索引数组;操作面 clearall / add_current_if_empty / add(fd) / add_path / add_filename / delete(fd) / rename(viv.c:514-520 声明区) |
| A5 | 「标当前项」子诉求上游已由既有 UI 满足 | viv.c:167(DONE 段)`*check current item in playlist` |

## riviv 复用面证据(代码锚点,全部本会话亲读)

1. **模型零工作**:`src/playlist.rs`(1984 行,#6/#39)——PlaylistEntry{path,modified,created,size,id}、SortMode 五式、shuffle、导航数学,纯半已单测。面板不碰模型。
2. **列表构建链现成**:`src/jumpto_dlg.rs:98-132`——`state.playlist.entries().to_vec()`(空则目录扫描兜底,`playlist::is_valid_path` / `modified_ticks` / `created_ticks` 补元数据)→ `items.sort_by(playlist::nav_compare)` → `playlist::filename_part()` 展示(560)。面板 = 同一构建 + 持久显示 + 实时同步(呈现序是否沿用该固定名序是显式决策,见 O2b)。
3. **菜单面有预留**:`src/menu.rs:699` 注明 File/View/Navigate/Help 内死行等特性——View 行有位;命令表 ENTRIES 按 upstream 序尾追加(#178 手法,冒烟按数字 id 直发 WM_COMMAND 零扰动)。
4. **布局数学可测先例**:toolbar.rs 模块头「Everything decidable is pure: heights, layout rects」——面板条宽/边侧/最小宽同法下沉纯函数纳入测试网。
5. **本地化机制现成**:loc.rs 双语 Id(en/zh-CN)。
6. **右键命令族现成,但目标是当前图**:`state.nav_current`(window.rs:232)是唯一行动目标——Delete/Rename/Copy Filename 族全部作用于**当前显示图**,不支持任意选中行。对非当前行执行 = 先跳转(选中行→跳转→命令)或新增 entry 目标化路径,非纯接线(O4 的成本项)。

## 侵入面与风险(实现票主战场)

1. **#80 viewport 几何**:视口子窗口是渲染器路由目标(WM_SIZE 钩子 `gpu_view_resized`,window.rs:2219);停靠面板收缩视口 = 主窗 client 布局再分一条竖条,WM_SIZE 链上多一个兄弟窗口。
2. **auto-fit 窗口缩放**:窗随图缩放(Alt+4 家族)须决定是否计入面板宽——计入=图像有效区变小,不计入=窗宽=面板+图宽,观感差异留给实现票实测。
3. **fullscreen**:面板藏或留(mpc-hc 先例=留),须裁。
4. **大列表**:上游 playlist 无上限(拖放整棵树同步扫描);ListView owner-data 虚拟模式是自然解,条目数即 playlist count 量级。
5. **unsafe 壳增长**:新子窗口类 + wnd_proc——按质量档位保持薄,快照/差分/布局数学下沉纯函数单测(jumpto 已证可提取)。
6. **多窗/单实例**:面板状态窗口本地,无跨窗问题(上游多窗竞态是既有语义,不新增)。

## 热键余量(双侧键表亲核)

- riviv Ctrl+字母已占(keys.rs:100-355 默认表):B C E K L O P Q R S T V W X Z;Ctrl+0(zoom reset);bare J=Jump To、bare 1/2/3=presets、Alt+1..4=窗尺寸;Ctrl+Shift+O/E/C(FileAddFile/FileAddEverythingSearch/EditCopyFilename,keys.rs:106-141)——Add Folder 上游有、riviv 未实现,Ctrl+Shift+B 在 riviv 侧空闲。
- 上游键表(viv.c:972-1039 亲读):Ctrl+O/B/E/P/W/Q/X/C/V/T/R/0、Ctrl+Shift+O/B/E/C、Ctrl+Enter、Ctrl+Alt+0;**Ctrl+D 是被注释掉的壁纸预留**(viv.c:981,「needs a confirmation dialog」;987 是 EDIT_CUT/Ctrl+X)。
- **双侧全空**:Ctrl+A D F G H I J M N U Y + Ctrl+1..9。

## 语义与边界(设计要点,待咨询/拍板确认)

- **只显示与跳转,不改播放列表的构建语义**(边界钉死,见「边界划清」;O4 若裁 B,右键命令是调用既有命令,非新增列表语义)。
- **呈现序是显式决策(O2b),不是推导**:`nav_compare`(playlist.rs:512)是 Jump To 的固定序——文件名 collation + 插入 id 决胜,**恒名序、与 config sort 无关**(上游 `_viv_nav_compare` 本义 = `_viv_fd_compare_name`,viv.c:13324);Next/Prev 走的则是 `fd_compare`(config sort 参数化,默认 mtime 降序)或 shuffle 序——两者今日就不同,面板选哪边见 O2b。
- 同步策略:导航换图→高亮跟随 + 滚动到可见;播放列表变更(拖放扫描 / CLI 解析 / 单实例转发 / 删除 / 重命名 / undo-delete 重接)→列表刷新。快照式(每次重建,jumpto 同法)vs 差分更新,大列表下差分才有意义(O2a)。

## 表面(面向用户)

- 命令:`Cmd` 枚举尾追加下一 id;View 菜单行 + checkmark(死行预留位)。
- config:面板开关 / 宽度(或边侧)ini 键,参照 keep_zoom(#68)riviv-authored 记法。
- loc:en/zh-CN 双语 Id 若干。
- CLI:非目标。

## 开放问题(附推荐项;O 编号沿 #182 咨询票格式)

- **O1 形态**:A(推荐)停靠 pane(client 区竖条;三侵入面全吃,与主窗一体感强)vs B 浮动 tool window(owned 窗;零主布局侵入,多一个窗口管理面:z 序/最小化随主窗)。愿望原文两者皆可,首刀裁一。
- **O2 同步与呈现序**:(a) 刷新策略——A(推荐)快照式重建(jumpto 同法;简单,变更点全走单一重建入口)vs B 差分更新(须 playlist 变更埋点 + 行稳定性论证,复杂;仅巨列表下有收益);(b) 呈现序——A(推荐)沿 Jump To 先例固定名序(`nav_compare`;稳定可寻、与 Jump To 行为一致)vs B 跟随导航序(`fd_compare` 随 config sort/shuffle——「面板=导航地图」直观,但序随设置翻动、shuffle 下=会话态)。
- **O3 大列表**:A(推荐)v1 即 ListView owner-data 虚拟模式(上游 playlist 无上限,拖放整棵树即巨列表)vs B 先全量控件、巨列表后补(两段路)。
- **O4 交互面**:A(推荐)v1 = 单击选中 / 双击或 Enter 跳转 vs B v1 即附带右键命令族(命令语义现成但目标=当前图:对非当前行须先跳转再作用,或新增 entry 目标化路径 + 状态;表面积×冒烟面同步增大)。
- **O5 持久化**:A(推荐)开关 + 宽度两键 vs B 只开关(宽度固定)vs C 边侧也可配。
- **O6 热键**:A(推荐)默认无键、Controls 页可绑(参照上游键表 viv.c:972-1039 无 toolbar toggle 键的惯例,View 类开关轻表面)vs B 默认键(候选 = 双侧全空的 Ctrl+D/H/Y 等)。

## 边界划清(不进本票)

相邻上游愿望各自成票:`/add` 与 500ms 重启追加(viv.c:33/35)、playlist 文件格式 / album(viv.c:78)、m3u/efu 载入(viv.c:102)、Play All Instances(viv.c:88)、Everything randomize 键(viv.c:89)。本票(及其实现票)只做**显示与跳转**,不改播放列表的**构建语义**(add/clear/排序如何形成列表——那是上述相邻愿望的事);O4 若裁 B,右键命令只是**调用既有命令**(删除/重命名语义与主界面同一),不是本特性新增的播放列表变更,但同步/undo/错误面须随实现票明写。

## 决策影响

- 本票交付 = 本文档 + ARTIFACTS 登记;零代码变更。四门禁照常(commit 钩子),test 基线 = master 817 过/0 fail/3 ignored(v0.4.0 发版口径)。
- 拍板路径:用户跑外部三 AI 咨询(提示词随 PR 附)或直接拍板 O1-O6 → 实现票拆分候选:①纯层快照/同步/布局数学 + 单测 ②子窗口 + 停靠/浮动接线(含三侵入面)③交互 + loc + ini + Differences + 冒烟矩阵。
- ADR:本特性不新增渲染栈/依赖面不变量,预计无需新 ADR;若 O1 裁 B(浮动窗)且引入多窗管理语义翻面,或 O3 裁 B 引入两段路,再议。

## 验收

- 文档入库(PR 合并);commit 钩子 fmt/clippy/test 绿。
- QA 清单:N/A(docs-only,无行为变更)。
