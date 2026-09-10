# riviv #26 实现计划 — 文件关联 + NSIS 安装器

日期:2026-09-10;基于 master d38ca14(228 tests green)。认领人:本会话(issue #26 assignee @me 已设)。

## 三视角审查处置(2026-09-10,工程+行为两代理;技术代理 provider 限流未出——其范围被另两份+实测覆盖,winresource/NSIS3-InstallOptions 留构建期实证)

**P1-1 解析器无 "skip first parameter"**(双代理同报):viv.c:4489 的 `// skip first parameter.` 是从 `_viv_process_command_line`(4801 同款)复制来的陈旧注释;4490-4619 的单循环就是主解析器,**只跳过 exe 名,其后每个 word 都处理**。证据:NSIS 以 `/install` 为首参调用(installer.nsi:409)、Options re-exec 首参 `/appdata`(viv.c:8808)、runas re-exec 首参 `/isrunas`(4650)。→ 解析器消费 argv[0] 后处理**全部**剩余 word;钉测试 `parse(["riviv.exe","/png"]) → install_flags==1<<5, handled==true`。比较全部大小写不敏感(string_icompare_lowercase_ascii)。

**P1-2 RegDeleteKey 有子键即失败**(行为代理本机实测:HKCU 带子键返 ERROR_ACCESS_DENIED=5,什么都不删):上游 8977-8980 对必带 DefaultIcon+shell\open\command 子键的 progid 键调 RegDeleteKey 且忽略返回——**上游卸载从不真删 progid 树,静默泄漏**(Vista+)。issue 验收明文"卸载后无残留"→ **采用 RegDeleteTreeW,入册 README Differences(有意偏离,上游 bug)**;冒烟断言卸载后 progid 树不存在。

**P1-3 CLI 接线**(工程代理):main.rs 现把过滤后的 file list 传 run()——install 解析器必须拿**原始 GetCommandLineW**(① 引号语义:上游 was_quote 使引号开关被忽略,args_os 看不到引号;② `/isrunas` re-exec 需转发 exe 名之后的原始尾串 cl_start)。→ run() 增参或 run 内自取(先例:#21 handoff 已在 window.rs:3478-3487 读 GetCommandLineW);插在 `Config::load()`(3430)与 mutex(3442)之间(上游 WinMain 5261→5270 同序);handled → return Ok(())(该点 LoadThread/mutex 均未建、COM 无需 teardown,资源干净——计划明示)。

**P2-4 Options 表手术**(工程代理):新 Kind::AssocCheckbox/StartMenuCheckbox 绕开 OptionsModel(注册表现状非 ini 态,上游忠实),但必须同步:① `every_field_round_trips_through_exactly_one_accessor_family` 缩域到 model-backed kinds;② `every_model_field_has_exactly_one_control` 计数断言更新;③ `every_row_sits_on_its_own_line` 页高 214 dlu 断言更新;④ Check All/None 按钮放 ctrl_id 网格外 id 块(仿键编辑器 250-254 先例,用 260-263);⑤ on_clicked 分发加臂。

**P2-5 图标加载**(工程代理):~~LoadIconW(hinstance, IDI_APPLICATION)~~ 是系统预定义图标,资源永远不会出现。→ `LoadImageW(hinstance, MAKEINTRESOURCEW(1), IMAGE_ICON, SM_CXICON/SM_CXSMICON, LR_DEFAULTCOLOR)`(上游 viv.c:5346-5350 同法);资源 id 钉 1。

**P2-6 提交序列**:提交 1(独立 assoc.rs)过不了钩子(dead_code=-D warnings)→ 提交 1+2 折叠为"纯逻辑+壳+CLI 接线"一笔。

**P2-7 错误策略归口**(工程代理,ADR 0001 分类):注册表读写/.lnk Save 失败 = 用户级 fail-soft eprintln(先例 config.rs:240-242 save 的措辞;eprintln 在 windows_subsystem 下本就不可见,与 options_dlg.rs:244 现状一致);无完全静默分支;runas re-exec 的 ShellExecute 失败 = 系统级直报(装不了就明说)。

**P2-8 测试补钉**(工程代理):① 位序数字钉死 `/png→1<<5`、`/nopng→uninstall_flags 1<<5`;② 未知开关 `/slideshow` `/foo` → **handled==false**(防止把手尾 config 开关变成进程退出——README"开关忽略"承诺的守卫);③ `/isrunas` re-exec 参数构造纯函数化测试(含带空格 exe 路径);④ shell_execute wait 契约 = NOCLOSEPROCESS + WaitForSingleObject + CloseHandle(SAFETY 注释明示);⑤ 组合序测试(/uninstall 清 install_path、置 0xffffffff、startmenu=-1、双 admin 旗标)。

**P3-9 模块**:assoc.rs 单文件两半(pure 段零 windows-crate 类型,keys.rs:32-33 模式;壳段 unsafe+SAFETY),模块 doc 明示分界。

**P3-10 依赖**:winresource 钉版本(=0.1.x 查 crates 后定);windows features 仅 +Win32_System_Registry(IShellLinkW 在 Win32_UI_Shell、IPersistFile 在 Win32_System_Com、ShellExecuteExW 在 Win32_UI_Shell——均已有;PropertiesSystem 不需要)。

**P3-11 README 措辞**:"no build-time resource compiler" 句限域到 dialog 模板(winresource 自写资源对象,无外部编译器);新增 Differences:RegDeleteTree 偏离、start-menu 目录/lnk 名(riviv)、注册表 progid/riviv.Backup 命名空间、Changes.txt 不拷(riviv 无此文件)、MUI.nsh→MUI2.nsh 移植选择、上游 MUI_PAGE_FINISH 保留。

**行为代理 CONFIRMED 要点**(实现照做):Backup 可为**存在的空串值**(install 时 .ext 无默认值则 Backup=""),uninstall 恢复它 → 卸载后 .ext 默认值="";Backup 值被删;真正无 Backup 值时 uninstall **不碰** .ext 默认值;os_create_shell_link 目标不存在则**跳过建链**(os.c:1038)——Uninstall.lnk 仅当旁边有 Uninstall.exe;Options OK 次序 = 关联直装/卸(8693)→ View/Controls 读(8711+)→ 菜单重建(8783)→ admin re-exec(8801)→ config_save(8812 最后);is_association 不查 DefaultIcon。

## 上游考古结论(已核对;两处勘误见审查处置 P1-1/P1-2)

1. **关联表**(viv.c:1136-1189):9 扩展 bmp/gif/ico/jpeg/jpg/png/tif/tiff/webp;描述串 = "Bitmap Image"/"Animated GIF Image"/"Icon File"/"JPEG Image"×2/"PNG Image"/"TIFF Image"×2/"WebP Image"(en/zh 同串);icon_locations 仅 ico=%1 其余 NULL(exe",0")。CLI 位序 = 表序(install_flags bit i)。
2. **注册表布局**(viv.c:8831-9053):全 HKCU `SOFTWARE\Classes` 下:
   - progid = `voidImageViewer` + `.ext`(riviv 用 `riviv` + `.ext`,与上游错开——README namespace 约定)
   - `<progid>\DefaultIcon` ← icon_location 或 `exe,0`
   - `<progid>` ← 描述
   - `<progid>\shell\open\command` ← `"exe" "%1"`
   - `.ext` 默认值 ← progid;**备份机制**:`voidImageViewer.Backup` 值存旧默认值(仅当不存在时写,uninstall 恢复)
   - uninstall:恢复 `.ext` 的 Backup 值(有则恢复+删 Backup 值),删 progid 键(RegDeleteKey 不递归——单层删除)
   - is_association:`.ext` 默认值 == progid **且** open command == 当前 exe 的 `"exe" "%1"`(两查 ret==2)
3. **install CLI**(viv.c:4454-4742,WinMain 在 config_load 之后、mutex 之前调,return 1 = 处理过即退出):
   - 解析:跳 exe 名后循环 word(**勘误:无 skip-first-parameter,见 P1-1**);开关判定 = 非引号 + '/'|'-' 前缀 + 整词无点(同 riviv 现有 is_switch;引号语义需原始命令行,见 P1-3);比较大小写不敏感
   - 开关:`install <path>`(拷 exe+Uninstall.exe 到 path,清 uninstall_path)、`install-options <opt>`(拷完后用新 exe 执行 opt)、`uninstall [path]`(无 path 用 exe 路径;清 appdata 目录+安装目录;uninstall_flags=0xffffffff 全清关联+startmenu=-1)、`appdata`/`noappdata`(翻转 config_appdata + 双存盘)、`startmenu`/`nostartmenu`、`isrunas`(re-exec 递归哨兵)、扩展名/`no<ext>`(关联 install/uninstall flags)
   - 顺序:先(非 isrunas)标准用户装/卸关联 → is_admin_install 且非 admin 且非 isrunas → `/isrunas <剩余原 cl>` re-exec "runas" 后 return;admin 或 isrunas 继续:appdata 翻转+双存盘 → startmenu → install_path 拷贝(先 _viv_close_existing_process 杀运行中实例)→ install_options → uninstall_path 清理
   - return (is_admin_install || is_standard_user_install) — 有 install 系开关才 return 1 退出
4. **Options General 页**(viv.c:7947-8000/8490-8534/8626-8813):
   - 勾选态:init 时 `_viv_is_association` 扫;startmenu 勾选 = `_viv_is_start_menu_shortcuts`
   - Check All/None 按钮(7982-8000):9 复选全置
   - **IDOK 直接调 install/uninstall_association_by_extension(标准用户特权,HKCU 无需 admin)**;仅 appdata/startmenu 变化走 `_viv_append_admin_param` 收集 params,OK 末尾 `os_shell_execute(0, exe, 1, NULL, params)` re-exec(runas 由 is_admin_install 分支决定);**关联勾选不算 need_admin(代码被注释掉,8498-8516)**
   - BCM_SETSHIELD 盾牌 = appdata/startmenu 变化 && !admin(riviv #24 已知:直切 appdata 无盾)
5. **start menu**(viv.c:12708-12791):CSIDL_COMMON_PROGRAMS(全用户!)下 `riviv` 目录 + `riviv.lnk` + `Uninstall.lnk`(COM IShellLinkW+IPersistFile);is = 目录存在;uninstall 删两个 lnk + RemoveDirectory
6. **NSIS**(c-original/nsis/installer.nsi):MUI2;License→Directory→InstallOptions(appdata 二选一)→InstallOptions2(startmenu+9 关联)→InstFiles;Section 里把 exe 发到 $pluginsdir、WriteUninstaller,然后 `ExecWait '"exe" /install "$INSTDIR" /install-options "<admin选项>"'` + `ExecWait '"$INSTDIR\exe" <user 选项>'`;uninstall section 拷 exe 到 $Temp 执行 `/uninstall "$INSTDIR"`。RequestExecutionLevel user(靠 exe 自己 UAC 提权)
7. **os_is_admin**:Vista+ IsUserAnAdmin(shell32);**os_shell_execute**:SEE_MASK_INVOKEIDLIST,verb "runas" 即 UAC

## riviv 现状

- main.rs:`file_args` 已有 is_switch(与上游开关判定一致);无任何 install 处理;run() 里 config load → mutex → worker
- options_dlg:GENERAL 表仅 appdata/multiple_instances 两行;Field/Kind/Ctrl 声明表驱动;on_ok 现有 commit→effects→keys→save 流程
- 无注册表代码;Cargo.toml 无 Win32_System_Registry feature;无 .ico/winresource
- README 偏离条目:line 58(图标)、line 68(General 页缺 startmenu/关联+appdata 直切偏离说明)、line 70(CLI 开关忽略)

## 设计

### 模块:src/assoc.rs(新,纯逻辑 + 薄 Win32 壳)

纯逻辑(测试网):
- `EXTENSIONS: &[&str]`(9 项表序)+ `DESCRIPTIONS: &[loc::Id]`/直接 str(上游 en/zh 同串→硬编码)+ `icon_location(ext) -> Option<&str>`(ico=Some("%1"))
- `progid(ext) -> String` = `"riviv." + ext`;`dot_key(ext)`/`progid_key(ext)`/`command_key(ext)`/`default_icon_key(ext)` 路径构造(SOFTWARE\Classes 前缀)
- `open_command(exe) -> String` = `"\"{exe}\" \"%1\""`;`icon_command(exe)` = `"{exe},0"`
- `backup_value_name()` = "riviv.Backup"(上游用 "voidImageViewer.Backup";**命名空间错开**——与 progid 一致,防两 viewer 互踩备份)
- `parse_install_options(words) -> InstallPlan{...}`(**只跳 argv[0] exe 名**,逐字移植 viv.c:4481-4619 语义:开关判定同 is_switch;install 清 uninstall_path;uninstall 无 path 补 exe 路径需 IO → 解析层留 None 由壳补)
- plan 归约:`needs_admin(plan)`(is_admin_install)、`handled(plan)`(任一 install 系开关出现 = 进程退出)

Win32 壳(unsafe,SAFETY 注释):
- `is_association(ext) -> bool`(双查 ret==2)
- `install_association_by_extension(ext)`(uninstall→DefaultIcon→描述→command→.ext 默认+Backup)
- `uninstall_association_by_extension(ext)`
- `install_association(flags)`/`uninstall_association(flags)`(循环表)
- start menu:`is_start_menu_shortcuts()`/install/uninstall(CSIDL_COMMON_PROGRAMS;IShellLinkW COM,CoCreateInstance 已有先例;失败 fail-soft eprintln——上游 Save 失败静默)
- `is_admin()`(IsUserAnAdmin)
- re-exec `shell_execute_runas(params)`(ShellExecuteExW verb=runas,SEE_MASK_NOCLOSEPROCESS wait=true 对齐 os_shell_execute wait=1)

### CLI 入口:main.rs

- `file_args` 之前:`std::env::args_os` 全量(含 exe 名)→ `assoc::process_install_command_line()`;handled=true → exit(0)(上游 _viv_kill return 0,成功路径)
- 上游时序 = config_load 之后、mutex 之前 → riviv 同位:run() 内 config load 后插(需要在 run 里,因为要读 config.appdata 翻转+存盘)→ `run` 返回特殊值/直接在 run 内 process+return Ok(())
- kill existing process:`FindWindowA("riviv")` 循环 SendMessage WM_CLOSE + WaitForSingleObject(上游 _viv_close_existing_process;riviv 类名一致可用)

### Options General 页

- GENERAL 表 + 11 行:Start menu shortcuts 勾选(读 is_start_menu_shortcuts,OK 时变更 → admin param 收集)+ Associations groupbox(9 复选,init 读 is_association,OK 直接装/卸)+ Check All/None 按钮
- Field 扩展:StartMenu(bool)、Bmp..WebP(9 个)——但**不走 OptionsModel**(它们不进 config/ini!上游就是注册表现状直读直写)→ 处理法:Kind 加 `AssocCheckbox(ext)`/`StartMenuCheckbox` 新 kind,shell init 读注册表、OK 读勾选执行;OptionsModel 不动(加 9 bool 字段会污染 commit→ini 映射)
- admin params 收集:on_ok 里对比 startmenu 勾选 vs 现状 → `/startmenu`|`/nostartmenu` append;关联勾选差异**直接装/卸**(上游 8693-8709);params 非空 → `shell_execute(NULL, exe, wait=1, NULL, params)`(无 verb——不自提权,上游 runas 分支在 CLI 的 is_admin_install 里;标准用户跑 `/startmenu` 会再 re-exec runas)
- appdata:保持 #24 直切(riviv 已入册偏离;#26 后 CLI 开关可用,可在 PR 说明"现在 -appdata 系开关已支持,Options 直切仍偏离"——**或**顺手接上 admin param 路径?→ 保守:保持现状,README 更新偏离措辞)

### 图标

- 生成 riviv.ico(多尺寸 256/48/32/16,32bit ARGB)——**程序化生成**(不搬上游美术):简单"眼睛/图片"主题或首字母 r 图形;PNG-compressed 256 + BMP 小尺寸;用 Rust 脚本(image crate 已有)或 PowerShell System.Drawing 生成
- winresource crate(build-dependency;winres 不维护)嵌 .ico + 版本信息;WNDCLASSEXW hIcon/hIconSm = LoadImageW(hinstance, MAKEINTRESOURCEW(1), IMAGE_ICON, SM_CXICON/SM_CXSMICON, LR_DEFAULTCOLOR)(P2-5);README 勾销 line 58
- 图标同样给 NSIS MUI_ICON 和 DefaultIcon 用

### NSIS

- `installer/nsis/riviv.nsi` + version.nsh + InstallOptions*.ini(汉/英)从上游移植改名;OutFile `riviv-Setup.exe`;!include MUI2.nsh(NSIS3 自带);x64 InstallDir $PROGRAMFILES64\riviv
- 构建脚本 `installer/build-installer.ps1`(cargo build --release → makensis);本机 winget 装 NSIS 3.12 验证
- 上游 InstallOptions.nsh 是 NSIS2 老 API 但 NSIS3 兼容;保持移植忠实
- license 文本:MIT + 上游 license 文本改写(riviv 是重写,写自己的 MIT 文本)

### Cargo/依赖变化

- windows features += Win32_System_Registry, Win32_UI_Shell_PropertiesSystem(IShellLinkW? 查:IShellLinkW 在 Win32_UI_Shell;IPersistFile 在 Win32_System_Com)、Win32_UI_Shell(已有)
- build-dependencies += winresource(仅 Windows target)

## 范围决策(写进 PR)

1. **config 系 CLI 开关(/slideshow /fullscreen /sort 等)不在本 issue**——上游在 `_viv_process_command_line`(第二个解析器)且量大;issue 文说"至少落地 install 系";README 偏离条目改写为"config 系开关仍忽略,install 系已支持"
2. **usage dialog(未知开关弹用法)不做**——上游 viv.c 4850+ 的 usage 对话框属 config 开关家族,随上面一起延后
3. **盾牌 BCM_SETSHIELD 不做**——appdata 直切偏离保持;关联是 HKCU 无需盾
4. Uninstall.exe(自拷贝卸载器)= NSIS 的 WriteUninstaller 产物,riviv 的 /install 拷 Uninstall.exe 文件本身即可,无需自己生成
5. /install-options 执行的语义 = 新 exe + 给定参数 wait——上游如此(给 NSIS 传 /appdata 等)

## 测试面

纯逻辑单测(assoc.rs #[cfg(test)]):
- 表完整性:EXTENSIONS 9 项序对(bmp..webp)、icon_location 仅 ico=%1
- 路径构造:progid/dot/command/default_icon 各 key 精确串
- open_command/icon_command 格式(引号、%1、,0)
- parse_install_options:每开关一测(install 带参/不带的 path 捕获、uninstall 清 install_path、uninstall 无 path→None 壳补、appdata/noappdata/startmenu/nostartmenu、isrunas、单 ext、no<ext>、组合序、dotted 当文件、quoted 跳过——注:args_os 看不到引号,上游同样限制已在 is_switch 记录)
- needs_admin/handled 归约
- backup value name = "riviv.Backup"

## 冒烟面(PowerShell,%TEMP%\riviv-test\smoke26-*.ps1)

- S1 CLI 关联往返:`riviv.exe /png` → HKCU 查 .png 默认值/progid 三键 + Backup=空串值;`riviv.exe /nopng` → .png 默认值恢复为 ""(Backup 为空串)、Backup 值删除、**progid 树已删(RegDeleteTree 偏离,上游会残留)**
- S2 CLI startmenu:`/startmenu` → 目录+lnk 存在(Uninstall.lnk 需旁边有 Uninstall.exe——os_create_shell_link 目标不存在则跳链,冒烟素材放个占位 Uninstall.exe);`/nostartmenu` → 清
- S3 /appdata 翻转:exe 副本目录跑 `/appdata` → ini 双写检查(需 admin?appdata 开关上游标 is_admin_install=1 → 非 admin re-exec runas;**本机 jared 是管理员组**,ShellExecute runas 会弹 UAC 或静默——冒烟可遇 UAC;若弹则标 SKIP 如实报)
- S4 /install 拷贝:目标临时目录,exe+Uninstall.exe 出现(Uninstall.exe 由测试者先放旁边)
- S5 /uninstall:清目录+appdata+关联全清(0xffffffff)
- S6 Options 勾选:PNG 勾上 OK → 注册表;去勾 OK → 清(BM_CLICK 合成;OK 按钮 BM_CLICK)
- S7 关联启动联动:装 png 关联 → explorer.exe 双击模拟困难 → 用 `Start-Process shell:C:\...\test.png`?(Win11 关联)或 Invoke-Item → riviv 进程起+标题=文件名(**UAC/前台限制注意;失败按 #8 惯例 SKIP**)
- S8 NSIS:构建 → 静默装 `/S`(装到 %TEMP% 目录覆盖 InstallDir)→ 装后检查 → 静默卸载 → 清净
- 冒烟用 exe 副本放场景目录隔离 ini(既有惯例)

## 提交/PR 计划

分支 `m3-26-association-nsis`;提交序列(**1+2 折叠,见 P2-6;每笔独立过钩子**):
1. feat: assoc.rs 纯逻辑 + 注册表壳 + CLI install 系接线(run 内时序)——一笔
2. feat: 图标(ico+winresource,LoadImageW 资源 id 1)
3. feat: Options General 页关联区(含三不变量测试缩域/更新)
4. feat: NSIS 安装器 + 构建脚本 + README 勾销/新增偏离
(可按实际折叠;最后统一跑质量门)
PR 正文:范围决策、处置表预留、QA 清单链接;merge 后 issue #26 关闭条件 = 代码入库+测试绿+QA 清单(AGENTS.md 惯例)

## 风险

- HKLM 不涉(全 HKCU)——admin 需求主要来自 Program Files 写入与 CSIDL_COMMON_PROGRAMS;**本机 admin 用户跑 UAC re-exec 冒烟可能弹窗**——预设 SKIP 出口
- RegCreateKeyEx on uninstall(上游 uninstall 也用 Create 打开 .ext 键!)→ Rust 用 RegOpenKeyEx 或保持 Create(忠实);失败静默(上游 debug_printf only)
- ShellExecuteExW "runas" 在无人值守冒烟会卡 UAC → 冒烟脚本须跳过或标注
- ICO 生成质量:程序化图标审美一般——README/PR 说明"占位美术,功能验证多尺寸"
