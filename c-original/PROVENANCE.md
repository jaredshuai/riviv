# c-original — 上游原版留档(只读参考)

- 来源: https://github.com/voidtools/voidImageViewer (MIT, © voidtools / David Carpenter)
- 基点: 本仓库 fork 时的上游 master,commit `02a8acb`(2026-09 本地 clone 时 HEAD)
- 内容: `src/` 主逻辑(viv.c 等 17 模块)、`libwebp/` vendored 解码库、`nsis/` 安装脚本、`res/` 资源、`vs2005|2019|2026/` VS 工程文件、`Changes.txt`、原 README.md(即本目录 README.md,未改动)

## 用途与纪律

本目录是 Rust 重写的行为对照参考:**勿在此目录做任何修改**;实现一律写仓库根 `src/`。

- 行为有疑问先读本目录源码(主逻辑在 `src/viv.c`,约 15,300 行);
- 渲染手感的关键参照:`_viv_get_render_size`(尺寸钳制,只缩不放)、`_viv_proc` WM_PAINT(HALFTONE/COLORONCOLOR + SetBrushOrgEx)、`_viv_update_title`(标题格式);
- 有意偏离原版行为,须记入根 README「Differences from upstream」并在 commit 正文说明 why(见 AGENTS.md 技术栈约定)。
