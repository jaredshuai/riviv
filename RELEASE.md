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

### 版本与标签现状（未裁决）

- 仓库既有标签 `1.0.0.9`–`1.0.0.15`（7 枚，4 段式，沿用上游 voidImageViewer 版式，均为人手打的 README 类提交标签）。
- **张力待裁决**：块内固定段的 SemVer 映射（`feat`→Minor 等）与仓库现行 4 段式 `1.0.0.N` 不一致。采用哪套、由谁转换，**未定**——需要项目维护者显式裁定后在本文档块外改写本节。
- 现状：**不发版不打标签**；标签由人手打；无 CI 发版流水线（CI 仅跑门禁，#93）。

### 发布命令（未验证）

无已验证的对外发布命令。本机试打（`installer/build-installer.ps1`）不等于发版。
