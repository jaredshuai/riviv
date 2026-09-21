# S1 平台地板(riviv.exe 静态导入表 → min-OS)

- 票:#97(M7-S0)
- 日期:2026-09-21
- 基线 exe:master `559f1cc` `cargo build --release`(cargo 判定 up-to-date,即 #94 会话所建)
  sha256 `13980f50f3c53c82e00c2cb9669885b75f16f83c1b0dab4fdfa22dbd6e8dd4bc`,
  大小 3,099,648 字节(≈2.96 MiB;`lto=thin`+`strip` 生效)。
  原始输出留档:`%TEMP%\riviv-s0\imports.txt`

## 结论一句话

**min-OS = Windows 10 1607(build 14393)**,由唯一的 1607 级静态导入
`user32.dll!GetDpiForSystem`(#79 chrome/对话框 DPI 源)钉死;次高地板仅到
Win8.1(`shcore!GetProcessDpiAwareness`)与 Win8(`WaitOnAddress` 族、
`D3D11CreateDevice`),**零 Win10-1703+/Win11-only 静态导入**;PMv2 效果
(1703+)、ACM(Win11 22H2+)等一律运行时探测,属可选增强不抬地板。

## 证据块

命令(完整路径,MSYS 环境须 `MSYS2_ARG_CONV_EXCL="*"` 防止 `/imports`
被 Git Bash 吃成路径):

```
"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\dumpbin.exe" /imports D:\codespace\riviv\target\release\riviv.exe
```

静态导入 DLL 全集(24 个;**无 delay-load 节**,文件尾直接进 Summary,
`.text`=0x224000):

| DLL | 地板驱动符号 → 引入版本 | riviv 用途 |
|---|---|---|
| **user32.dll** | **`GetDpiForSystem` → Win10 1607** ←绑定地板;其余 ≤Vista/Win7 | 窗口/DPI(#79) |
| api-ms-win-shcore-scaling-l1-1-1.dll | `GetProcessDpiAwareness` → Win8.1 | PMv2 运行时自检(#79) |
| api-ms-win-core-synch-l1-2-0.dll | `WaitOnAddress`/`WakeByAddress*` → Win8 | Rust std 同步原语 |
| d3d11.dll | `D3D11CreateDevice` → Win8(WARP 兜底) | GPU 栈(#80) |
| combase.dll | 仅 `CoTaskMemFree` → Win8 | COM 内存(co-task 分配族经 ole32) |
| d2d1.dll | `D2D1CreateFactory` → Win7 SP1 | 渲染(#80) |
| bcryptprimitives.dll | `ProcessPrng` → Win7 SP1 | Rust std RNG |
| mscms.dll | WCS 变换 API → Vista | ICM(#77) |
| kernel32/ntdll/gdi32/advapi32 | ≤Vista | 核心/GDI chrome |
| shell32/ole32/oleaut32/comctl32/comdlg32 | ≤Vista | shell 谓词/对话框/公共控件 |
| VCRUNTIME140.dll + api-ms-win-crt-\*×6 | VS2015 运行时 + UCRT | MSVC CRT(见下注) |

演算:全部 24 个 DLL 里版本最新的符号是 `GetDpiForSystem`(1607);
没有任何静态导入落在 1607 之后(1703 的 `GetDpiForWindow`/
`GetSystemMetricsForDpi`、1809 的 `CreateDXGIFactory7`、Win11 的 ACM 系
均未出现——版本敏感能力全部经 COM 接口 QI 探测获取,如 #82 的
`IDXGIAdapter3` 查询)。

注:VCRUNTIME140.dll 不是 OS 地板问题(msvc target 动态链 CRT,需 VC++
2015+ 运行库或 app-local 拷贝,Win10 1607 装机普遍满足;UCRT 自 Win10
起 in-box)——但它是**发布面事实**:任何依赖原则预算(见 AGENTS.md
「依赖原则」)测得的 exe 增量都以本基线 3,099,648 字节为参照。

## 决策影响

1. **min-OS 钉死 Win10 1607**(与 #79 在案结论一致,本 spike 用导入表
   实证钉死);Win11 22H2+ 功能(ACM/自动色彩管理)只能以
   运行时探测形态出现,永不静态导入。
2. **版本敏感 API 纪律成文**(进 AGENTS.md「依赖原则」):COM 版本升级
   一律 QI 探测(`IDXGIOutput6`/`IDXGISwapChain4`/`ID2D1DeviceContext5`
   等);**新增 DLL 静态导入必须先复跑本 spike 的 dumpbin 流程出证据并
   写 ADR**,未过此关的 PR 不合入。
3. 对 M7 两候选路线的直接约束:
   - SVG:resvg 路线纯 Rust,不碰导入表;D2D SVG
     (`ID2D1SvgDocument`)是 1703+ COM 能力,若采用必须 QI 探测 +
     无则降级(见 s-svg.md)。
   - AVIF:`avif-native`(dav1d)会引入 C 工具链与潜在 DLL,违反原则
     ①;WIC 路线若真采用将新增 `windowscodecs.dll` 静态导入(须走
     ADR + 本表复跑)——三方咨询已把 WIC 限定为「装机率探测,非依赖
     路径」,见 s-avif.md。

## 降级路径

若未来某依赖要求 >1607 的系统能力:(a) 首选换实现路线(纯 Rust/回退
QI);(b) 确需抬地板则开 ADR 声明新 min-OS + README「Differences」记
录 + 复跑本 spike 的 dumpbin 流程更新本表。本表随任何新增静态导入的
PR 一并维护(导入表变更 = 本文件的 review 检查项)。
