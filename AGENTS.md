# AGENTS.md

## Quick Start

Rust 重写项目(裸 Win32 + GDI,单 exe)。验证与构建命令:

```bash
cargo fmt --check                          # 格式检查(提交前必绿)
cargo clippy --all-targets -- -D warnings  # lint,警告即错误
cargo test                                 # 纯逻辑单元测试(fit 数学/像素转换/标题构造)
cargo build --release                      # 产出 target/release/riviv.exe
```

- **提交钩子**(每 clone 一次):`git config core.hooksPath .githooks`。钩子依次跑 fmt --check / clippy -D warnings / test,失败即阻止提交。
- **冒烟测试**(GUI 无法自动验收):`target/release/riviv.exe <图片路径>`。agent 收尾时**必须附 QA 清单**(启动→显示→拖放换图→Ctrl+O→关闭,含预期状态)。人工验收批量进行,**不阻塞 issue 关闭**(2026-09-03 起,仿 yanxue 惯例):issue 关闭标准 = 代码入库 + `cargo test` 绿 + 附 QA 清单。

## 工具使用纪律

遇到任何工具调用未达预期效果(FAIL、BLOCKED、超时、返回空结果、被验证码/权限墙拦截、返回内容不完整),**必须如实报告用户**,禁止:

- 自行用搜索结果或推测填充缺失信息然后宣称"拿到了"
- 用其他渠道的信息"凑"成完整答案后隐瞒工具失败的事实
- 跳过关键步骤直接进入下一阶段

正确做法:明确说明哪个工具、做了什么、卡在哪里、哪些内容缺失,然后给用户选择——换工具、换方法、还是你自己补。

## 技术栈约定

- **`c-original/` 是上游 C 原版只读留档**(voidtools/voidImageViewer @ 02a8acb,MIT)。任何 skill **不得修改、不得作为实现参考直接搬运 C 代码**;行为有疑问时读它做对照,实现一律写进根 `src/` 的 Rust。
- **unsafe 纪律**:Win32 FFI 是 unsafe 密集区。每个 `unsafe` 块必须带 `// SAFETY:` 注释(讲契约:谁保证指针/句柄/生命周期有效),由 clippy `undocumented_unsafe_blocks = "deny"` 强制。
- **错误处理遵循 [ADR 0001](docs/adr/0001-fail-loud.md)**:系统级失败带 GetLastError 上下文显式直报,禁止静默吞错;用户级图片加载失败遵循原版行为(保留旧图、窗口不退出、不弹框)。
- **质量档位(双轨)**:纯逻辑(fit 数学、BGRA 像素转换、标题构造)按产品标准——行为改动必须配套 `#[cfg(test)]` 断言,断言名写业务语义;unsafe 壳(窗口/GDI/消息循环)保持薄,推导逻辑一律下沉到纯函数纳入测试网,不要在 wnd_proc 里堆积可测逻辑。
- **行为对齐以原版为准**:改动交互行为前先查 `c-original/src/viv.c` 对应实现;有意偏离必须写进 README「Differences」并在 commit 正文说明 why。

## 多 agent 协作纪律

多个 agent 会话可能同时操作本仓库:

- **先认领再开工**:做任何 issue 前先 `gh issue edit <n> --add-assignee @me` 认领占位,已被认领的跳过;一个会话只做一个 issue。详见 `docs/agents/issue-tracker.md`「Claim before work」。
- **开工前 `git pull --rebase`**:不在过期 HEAD 上叠提交。
- **交接只写验证过的事实**:跨会话交接/总结里「测试已过」「冒烟已过」等断言必须是本会话亲自验证过的,未验证的明确标注「未验证」;引用文件用路径指针,不复制内容(防双份漂移)。

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues on `jaredshuai/riviv` (via the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-label vocabulary (`needs-triage` / `needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
