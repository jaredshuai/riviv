# D2D 显示效果链(sharpen 先行)开题 spike

- 票:#180(本 spike 为其唯一交付;实现票在咨询/拍板后另立)
- 日期:2026-10-01
- 基线:master `2832f80`;exe 6,829,568 B(交接档口径,本票零代码变更不重测);SDK 10.0.26100.0;windows crate 锁定 0.62.2(Cargo.lock 亲核)
- 证据性质:代码锚点/头文件/registry 符号均为本会话亲读;docs 页两处外部事实已抓原页;**本轮未跑任何新探针**——凡标「待探针」的结论留给实现票

## 结论一句话

sharpen 用 D2D 内置 `CLSID_D2D1Sharpen`(windows 0.62.2 原生在绑,零新依赖、零体积预算问题),挂 #130 已确立的「post-composition viewport pass,never per-tile」同一挂点(推广 `EffectGraph` 为可选链、插在 ColorManagement 之前),巨图 tile 接缝问题结构性不存在;语义定「非破坏显示效果」——master 读侧(状态栏 RGB/剪贴板/copy)不带效果,dump=屏上所见带效果(#156 先例);新决策集中在两处(Direct 臂新增两段式形态、WARP 是否继承 #127 硬排除),连同 O1-O6 开放问题交定向咨询。

## 为什么做(上游与已接受设计)

1. 上游 wishlist 两处点名、零实现:viv.c:76「color correction, white balance, sharpening」;viv.c:66 只是想外挂 ImageMagick 管道。
2. riviv 侧不是新方向:ADR 0002 D0 原文把「效果链」写进 D2D 上马动机(「未来能力空间(HDR/效果链)」「D2D 路线同时解锁 Stage 2 GPU 变换与效果链」);README Roadmap Unscheduled wishlist 第一条「D2D effect chain (sharpen etc.)」。
3. beyond-original 表面(上游无参照)→ README Differences 记段,记法参照 #68 keep_zoom、#178 undo-delete 两个 riviv-authored 先例。

## API 面证据(全部本地可复核)

| # | 事实 | 证据 |
|---|---|---|
| A1 | `CLSID_D2D1Sharpen` 原生在 windows 0.62.2 | Cargo.lock `windows=0.62.2`;registry 源 `Direct2D/mod.rs` 符号亲核(60 个内置效果 CLSID 全在绑;备选 GaussianBlur/ArithmeticComposite/ConvolveMatrix 亦在) |
| A2 | 效果仅两个 FLOAT 属性 | SDK d2d1effects_2.h:271-285:`D2D1_SHARPEN_PROP_SHARPNESS=0`、`_THRESHOLD=1`,无枚举无结构体;GUID 在同文件 :36 |
| A3 | min-OS | docs 页 Requirements:Minimum supported client = **Windows 10**(即 1507+ 已含)→ 1607 地板(S1)之上;仍照 #130 手法带运行时探测兜底(CreateEffect 未注册 → 会话 latch off + stderr 面包屑) |
| A4 | 取值域/默认值未钉 | docs 页是 84 词 stub,无 range/默认值原文;示例代码用了 SHARPNESS=1.0/THRESHOLD=0.5(仅示例非合同)→ 域与默认**待实现票探针钉**(默认读回值、越界 SetValue 行为,负控必测) |
| A5 | 属性索引转录 | 照 #130「docs 页不可靠 → 头文件转录 + 钉测」先例,两个 u32 常量落 gpu.rs 旁并钉测(值 0/1,风险≈0) |

## 挂点证据(代码锚点,全部本会话亲读)

1. **挂点已被 #130 裁定**:"the one post-composition viewport pass(AI1's ruling — never per-tile)"(gpu.rs:514-516 注释原文)。锐化是空间卷积,per-tile 必接缝;viewport pass 在合成后的整幅视口上跑,**接缝结构性不存在,免费继承**。
2. **两段式 pass 现成**:`effect_pass`(gpu.rs:1967-2029)阶段 1 SetTarget(intermediate)+ `draw_scene`(码值 Clear),阶段 2 SetTarget(swapchain)+ DrawImage(effect_image,NEAREST,1:1)。锐化 = `EffectGraph`(gpu.rs:556-573)推广为链:**`intermediate → [user effects] → ColorManagement → target`,插在 ColorManagement 之前**——intermediate 格式随 master content space(Srgb=BGRA8 码值 / F16Srgb=gamma-sRGB f16 / F16P3=f16 同布局,gpu.rs:559-565),即锐化输入在所有臂上恒为 γ 类非线性编码域(F16P3 的 TRC=sRGB 形曲线,ADR 0004 P-A 探针),跨臂语义一致;若插 CM 之后,AC 臂在 scRGB 线性光域卷积,暗部锐化观感不同且跨臂不一致。
3. **Direct 臂是唯一新形态**:`draw_pass` 按 DisplayArm 分派(gpu.rs:1847-1908),Direct/WideBlank 现走 `direct_pass` 无 intermediate。锐化开启且臂=Direct 时也要两段式(场景→intermediate→锐化→target,无 CM)——「pass 形态」与「color arm」解耦为两个正交维度,决策落 transform_stage 纯函数(如 `pass_shape(backend, arm, chain)`),全单测(#127 先例:决策表先行的落地序)。
4. **意图同步入口现成**:`sync_display_intent`(gpu.rs:1438,每 paint + 每 dump 前调)——链身份沿同一入口进;`built_for:(w,h,ContentSpace,DisplayArm)`(gpu.rs:572)追加链项,resize 惰性重建机制原样。
5. **零扰动负控(合同)**:效果关闭(默认)时 Direct 臂不建 intermediate、不 CreateEffect,像素与 stderr 逐字节不变;blank(plan=None)在纯用户链下可恒 direct_pass(常数场卷积=自身)。#130/#172 的负控纪律直接套用。
6. **读侧分流与 dump 合同**:#156 已立「a transformed screen dumps transformed, or the dump channel would diverge from what the screen shows」(gpu.rs:2251-2253)——用户显示效果属屏上所见,**dump 带效果**;状态栏 RGB/剪贴板/copy filename 走 master 直读接缝(#141),**不带效果**(它们回答「图像是什么」,不是「屏幕是什么」)。
7. **可观测合同**:fingerprint_for(transform_stage.rs:671)的 policy 项加链身份 → toggle 触发 output_gen 递增(#126/#132 的 dump 变化信号),smoke 以此断言。

## 语义与边界(设计要点,待咨询确认)

- **档位=非破坏显示效果**(O1 推荐):会话级显示态、不写回 master、不进像素编辑族(非 rotate 类)。随时可关,关=逐位回到现状。
- **#127 硬排除「WARP never runs the effect」须显式重裁**(O3):该排除是 #127 咨询对 ColorManagement(quality BEST 全色域变换)的裁定,不是性能测量结论;锐化是小核卷积、viewport-res 成本数量级更低,但裁定继承与否是档位问题不是推导问题。实现票带 WARP 计时冒烟(8 ms 门参照,#137 探针口径)。
- **动画**:每帧 phase1+链重跑,成本=viewport-res 有界;#163 SVG 重栅与显示段正交(换 master 像素,不碰 pass)。
- **失败姿态**:用户链失败沿 DisplaySegment.failure → 棘轮既有机制(#130/#156),不 fatal、不进 ladder。
- **依赖/体积**:零新 crate(D2D 内置 + windows-rs 已在绑符号)、纯代码增量,依赖原则 ③④⑤ 均不触发——**这是本票不破任何预算的核心理由**。

## 表面(面向用户)

- config:`sharpen` ini 键(apply_* 宏族,参照 keep_zoom 的 riviv-authored key 记法);值语义(连续/档位)在 O4。
- 命令:`Cmd` 枚举尾追加下一 id(**123**,#178 手法:既有 id 零扰动,冒烟按数字 id 直发 WM_COMMAND);热键进 keys 表、Controls 页可绑。
- 菜单/Options:View 页 + 菜单行(轻,O5 推荐)vs 独立 Effects 菜单(重)。CLI 开关首刀不做(非目标)。

## 开放问题(定向咨询输入,推荐项已标)

- **O1 档位**:显示效果(推荐)vs 像素编辑(destructive;dump/copy/保存语义全部翻面,巨图链路复杂化)。
- **O2 第一刀范围**:仅 sharpen(推荐)vs 同链附带 blur/invert/grayscale 等(链架构已泛化,后续效果是纯增量票;首刀带多个 = 表面×验证面乘法)。
- **O3 WARP**:继承 #127 排除(安全)vs 放行 + 计时门(推荐,理由见上)。
- **O4 值域/默认**:docs 无 range(A4)→ 探针钉域后选「0..N 连续值」vs「离散档位」(上游无参照;JPEGView 5 档先例)。off=0。
- **O5 UI 表面**:View 菜单行 + 热键 + Options 一行(推荐)vs Effects 菜单 + 滑杆。
- **O6 Threshold**:钉实现内常数(推荐,docs 默认读不到)vs 暴露为第二键。

## 决策影响

- 本票交付 = 本文档 + ARTIFACTS 登记;零代码变更。四门禁照常(commit 钩子),test 基线 = master 797(2026-10-01 交接档口径)。
- 拍板路径:用户跑外部三 AI 咨询(提示词随 PR 附)或直接拍板 → 实现票拆分候选:①决策表纯函数 + pass 形态推广 + 负控 ②键/命令/菜单表面 + Differences ③冒烟矩阵(hw/warp × on/off × dump 合同)+ A4 探针。
- ADR:挂点/语义沿 ADR 0002 D0、#130 viewport-pass 裁定、#156 dump 合同推导而来,无新不变量;若 O3 重裁 WARP 排除或 O1 选像素编辑 → 补 ADR(0002 后记或新 ADR)再动代码。
