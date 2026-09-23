<!-- lazypack:start block=release-discipline src=DECISIONS.md@0.3.0 gen=bfce1f5fd317b6a0 input=31a85accd0875321 fp=2395aa8f4ba3904f -->
# 发版与提交纪律 (RELEASE)

> 派生自 lazypack-discipline 固定层 DECISIONS.md@0.3.0（依据 lazypack-setup 内置快照编译，来源内容标识: 1c78a216d27b82666c9724701f858f2bafe2b10d；离线事实源查阅 lazypack-setup/references/DECISIONS.md）§5。

## 1. 提交与版本规则（固定段）

### 1.1 提交头 (Conventional Commits 1.0.0)
- 格式：`type(scope)!: 描述`
- 允许的 8 个 Angular type：`build`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `test`，加上 `chore`, `revert`。
- 破坏性改动使用 `!` 或正文注明 `BREAKING CHANGE`。
- 脚注使用 `Closes #n` 或 `Fixes #n` 关联票据。

### 1.2 提交正文 (Google CL 规范)
- 第一行独立说清「改了什么」。
- 正文说清「为什么做此改动」、有哪些未做完或折衷之处、关联的 Issue / Bug 号。

### 1.3 版本号映射 (SemVer 2.0.0)
- `!` 或 `BREAKING CHANGE` -> Major 升级 (`vX.0.0`)
- `feat` -> Minor 升级 (`v0.X.0`)
- `fix` / `perf` -> Patch 升级 (`v0.0.X`)
- 其他 type 不触发版本发版。
- 每次发版必须在 Git 打对应版本标签（如 `v1.2.0`）。
- 变更记录遵循 Keep a Changelog 1.1.0 格式，由提交历史自动编译生成，禁止手写篡改。
<!-- lazypack:end block=release-discipline -->

## 2. 项目打包与发布（项目层）

> 本区位于受管托管块外部，记录**这个项目怎么打包、怎么上传**。现行打包说明以本区为准。

### 产物形态（已存在，未验证发布链）

- **NSIS 安装包**：`installer/nsis/riviv.nsi`（MUI2 端口，`RequestExecutionLevel user` + staged exe `/install` runas 自提升），构建入口 `installer/build-installer.ps1`。本机试打可用（#26 期间多次实测）；**未接线任何云端发布**。
- **裸单 exe**：`cargo build --release` → `target/release/riviv.exe`（纯 Rust 静态产物）。

### 版本与标签方案（已裁定，2026-09-23，用户委托旗舰 AI）

- **旧 4 段式标签 = 上游参考标记**：仓库既有标签 `1.0.0.9`–`1.0.0.15`（7 枚）指向的是上游镜像提交，**不是 riviv 的发布记录**；保持原样，不改名、不重解释。
- **riviv 版本标识 = `0.1.0`**：`Cargo.toml` 与 `installer/nsis/version.nsh` 两处一致，已是 SemVer 形状。
- **首发 = `v0.1.0`**：在通过验收的发布提交上打标签并建 GitHub Release（0.x 阶段如实反映 early development；不提前借 `v0.2.0`/`v1.0.0` 表达更高成熟度）。此后按块内固定层 SemVer 映射执行（`feat`→Minor、`fix`/`perf`→Patch、破坏性→Major），每次发版打对应 `vX.Y.Z` 标签。
- **首发尚未发生**：发版动作（标签/GitHub Release）由维护者决定时机；发布前须——①核对 `Cargo.toml` 与 `version.nsh` 均为 `0.1.0`；②跑齐四门禁 + `cargo build --release` + 安装器构建 + AGENTS.md 要求的 GUI QA 清单；③CHANGELOG 自动编译流程**已建成**（2026-09-23）：git-cliff 2.14.2 + `cliff.toml` 策略（复合类型前缀分类、仅收 feat/fix/perf/refactor、`tag_pattern` 排除上游参考标签、历史起点=首个 riviv 提交 `0b370db`），再生成命令 `git-cliff -o CHANGELOG.md 0b370db^..HEAD`；首发打标签后重跑同一命令即产出 `vX.Y.Z` 分段，禁止手写篡改生成结果。
- 现状：无 CI 发版流水线（CI 仅跑门禁，#93）；标签由人手打。

### 发布命令（未验证）

无已验证的对外发布命令。本机试打（`installer/build-installer.ps1`）不等于发版。
