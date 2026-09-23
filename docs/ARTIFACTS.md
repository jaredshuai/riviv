<!-- lazypack:start block=artifacts-register src=DECISIONS.md@0.3.0 gen=bfce1f5fd317b6a0 input=a4942594f8f48dc9 fp=3c27e06bf86347b5 -->
# 产物登记册 (ARTIFACTS)

> 派生自 lazypack-discipline 固定层 DECISIONS.md@0.3.0（依据 lazypack-setup 内置快照编译，来源内容标识: 1c78a216d27b82666c9724701f858f2bafe2b10d；离线事实源查阅 lazypack-setup/references/DECISIONS.md）§4。
> 原则：一个东西只有一个家；能推导出来的不手写；有生命周期的写清何时死。

## 1. 产物状态词与流转规则

| 状态词 | 含义 | 流转约束 |
|---|---|---|
| `current` | 当前唯一的现行有效基准 | **在同一主题/用途的权威范围内全局唯一**（不同 feature 规格或多项有效 ADR 可各自主管对应主题的 current）。经明确确认存在替代关系的新产物标为 `current` 时，被替代旧产物方可标记为 `superseded` 并互指；未确认替代关系的不自动假定废弃 |
| `reference` | 外部素材、外部规范、参考设计 | 永久作为参照依据 |
| `exploration` | 探索方案、对比调研 | 仅供比对，不作为实现基准 |
| `superseded` | 已废弃或被取代的旧产物 | 状态变化本身不构成文件移动或删除授权；按材料类型、保留目的、有效引用及四步关卡（确认替代、核查引用、安全封存、登记更新）处理；setup 首期保持原位保留（preserve-existing），不执行自动 `git mv` 物理迁移；登记行同步更新互指新产物 |
| `pipeline` | 由源文件生成的派生品（图标、数据） | 登记源与生成器命令；严禁手工修改派生文件，重新运行 pipeline 生成 |
| `wip` | 正在编写或设计中的未决草案 | 完成后裁决为 `current` 或归档 |

> [!IMPORTANT]
> **未登记产物视为未决**：册上查不到的产物，先向维护者核实，严禁按文件名或创建日期猜测新旧！

## 2. 现存产物登记表

| 产物相对路径 | 类别 | 状态 | 来源/对应票/ADR | 说明 |
|---|---|---|---|---|
| docs/agents/issue-tracker.md | 任务跟踪 | current | 外部前置(Matt 约定) | GitHub Issues + gh CLI 约定;前置双检通过(有实质正文) |
| RELEASE.md | 发版规范 | current | lazypack 固定层 §5 | 块内固定发版纪律;块外=项目层打包与版本方案(SemVer/首发 v0.1.0,2026-09-23 裁定) |
| docs/agents/roles.md | 角色映射 | current | lazypack 固定层 §3 | 六角色防撞车边界+Skill 映射+留存策略 ideas-pool |
<!-- lazypack:end block=artifacts-register -->

## 3. 项目自选材料与存量文档登记（非受管协作区）

> 本区由团队与 Agent 协同维护，位于受管托管区外部。遵循保留既有模式（preserve-existing），登记已存在各逻辑区域的实际原件、探索性想法池、访谈纪要归档或外部参考素材。八大逻辑区域（方向、需求与验收、设计与决策、规则与术语、工作与进度、现状与使用说明、验证与观察、来源材料）为逻辑导航，不强制预建八个物理目录。支持 path#anchor 细粒度定位；setup 重跑保持本区已有手工排版与字节不变：

| 产物相对路径 / 锚点 | 逻辑区域 | 状态 | 维护者 / 更新触发 | 来源 / 替代关系 / 依据说明 |
|---|---|---|---|---|
<!-- 示例：| README.md#快速开始 | 现状与使用说明 | current | 维护者与各角色 / 上手流程或命令变化时 | 根目录快速上手指引 | -->
<!-- 示例：| specs/pay-v2.md | 需求与验收 | current | 规划者 / 支付网关变更时 | 现行支付网关重构规格；supersedes specs/pay-v1.md | -->
<!-- 示例：| specs/pay-v1.md | 需求与验收 | superseded | 规划者 / 历史保留 | 初版支付规格；superseded by specs/pay-v2.md；原位保留 | -->
<!-- 示例：| docs/ideas/inbox.md | 来源材料 | exploration | 团队与规划者 / 讨论产生新想法时 | 规划研讨碎片想法池，非现行基准 | -->
| docs/overview.md | 现状与使用说明 | current | 书记员/架构变化时 | 项目总览与「给图→上屏」流程(#111 起) |
| CONTEXT.md | 规则与术语 | current | 规划者/术语或决策变化时 | 单一上下文领域文档(docs/agents/domain.md 约定) |
| docs/adr/0001-fail-loud.md | 设计与决策 | current | 规划者/错误处理策略变化时 | ADR:系统级失败显式直报 |
| docs/adr/0002-d2d-wcs-render-stack.md | 设计与决策 | current | 规划者/渲染栈变化时 | ADR:D2D/WCS 渲染栈(M6 基石) |
| docs/spikes/s1-platform-floor.md | 验证与观察 | current | 调查者/平台地板再评估时 | min-OS 钉死 1607 的导入表证据(#97) |
| docs/spikes/s-svg.md | 验证与观察 | current | 调查者/SVG 复议时 | SVG=resvg-full 体积推迟结论(#97) |
| docs/spikes/s-avif.md | 验证与观察 | current | 调查者/M8 启动时 | AVIF 三路线推迟 M8 结论(#97) |
| docs/agents/triage-labels.md | 规则与术语 | current | 规划者/标签词汇变化时 | 五标签分诊词汇 |
| docs/agents/domain.md | 规则与术语 | current | 规划者/文档架构变化时 | 单上下文+ADR 约定 |
| docs/plan-26-association-nsis.md | 需求与验收 | reference | 书记员/历史留档 | #26 安装族历史规格;特性已交付,原位保留 |
| docs/ideas/inbox.md | 来源材料 | exploration | 规划者/新想法产生时 | 碎片想法池(非现行基准) |
| CHANGELOG.md | 工作与进度 | pipeline | 书记员/每次发版 | git 提交历史经 git-cliff 编译(cliff.toml 为策略);严禁手工修改派生文件;再生成命令见 RELEASE.md 项目层 |
