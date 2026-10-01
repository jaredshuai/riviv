# ADR 0006: D2D 显示效果链(sharpen 首刀)——非破坏显示效果、挂点继承与 WARP 排除继承

- 日期: 2026-10-01
- 状态: 已接受(六题裁定 O1:A / O2:A / O3:B / O4:A / O5:A / O6:A 由 #182 定向咨询外部裁定,处置收敛评论 5932401006 为定向权威;实现票族 #183①/#②/#③)
- 范围: 用户显示效果链的档位语义、挂点、决策表、WARP 排除继承、值域出处;**不含**像素编辑路径(O1 已裁显示档)、第二效果(blur/invert 等,链架构已泛化,纯增量票)、CLI 开关(首刀非目标)
- 上游权威: #182 裁定+处置、#180 spike(`docs/spikes/s-d2d-effects.md` 证据电池)、#130 viewport-pass 裁定、#127 决策表(硬排除原文)、#156/#141 dump 与读侧合同、ADR 0002(D0 效果链动机)

## 背景

上游 wishlist 两处点名锐化而零实现(viv.c:76「color correction, white balance, sharpening」);ADR 0002 D0 把「效果链」写进 D2D 上马动机。#180 spike 完成证据电池:`CLSID_D2D1Sharpen` 在 windows 0.62.2 原生在绑(65 个内置效果 CLSID 之一,零新依赖零体积预算问题,依赖原则③④⑤不触发)、效果仅两个 FLOAT 属性(SDK 头 d2d1effects_2.h:271-285)、min-OS Win10 1507+(1607 地板之上)。#182 定向咨询裁毕六题,本 ADR 将裁定落为可执行决策。

## 决策

### D1. 档位 = 非破坏显示效果(O1=A)

会话级显示态:master 字节、读侧(状态栏 RGB/剪贴板/copy)、保存路径**永不携带**;只有视口与 dump 携带——dump = 屏上所见(#156 合同:gpu.rs dump_draw_pass 单分派直接调 draw_pass,自动继承),读侧走 master 直读接缝(#141)。关 = 逐位回现状(off = 0)。

### D2. 首刀仅 sharpen(O2=A)

链架构泛化(`intermediate → [user effects] → ColorManagement → target`),后续效果 = 纯增量票(枚举加值 + 表面一行)。

### D3. 挂点继承 #130:post-composition viewport pass,never per-tile

锐化挂 #130 已确立的唯一挂点(合成后的整幅视口上跑,gpu.rs effect_pass 两段式)。per-tile 空间卷积必新增接缝;viewport pass **自身不新增 per-tile 接缝**,但合成自身在案残差(ADR 0002:58,各向异性帧缩小轴块边界 1-2px,巨图+panscan 分轴+滤波档下可达)经高通卷积会被放大观感——「巨图分轴缩放+锐化开」的边界观感列入票③冒烟对照与人工 QA(Codex P2 收窄措辞,#182)。

### D4. 用户链插 ColorManagement 之前(γ 域卷积,跨臂一致)

intermediate 格式随 master content space(Srgb = BGRA8 码值 / F16Srgb = gamma-sRGB f16 / F16P3 同 f16 布局,ADR 0004 D2)——所有臂上卷积输入恒为 γ 类非线性编码域;若插 CM 之后,AC 臂在线性 scRGB 域卷积,暗部锐化观感不同且跨臂不一致。AC 臂 FP16 读回在「链插 CM 前」下的逐字节等价性 = 票③风险 2 回退的触发条件(处置风险表)。

### D5. WARP 排除继承 #127,不解除(O3=B)

#127 的「WARP never runs the effect」原是对 ColorManagement(quality BEST 全色域变换,README 记档 9.61ms/1080p 中位对 8ms 门)的裁定,本 ADR 显式继承至用户链。实现为两层:display_arm 表使效果臂在 WARP 上本就不可达(窄内容 WARP → Direct,宽内容 WARP → WideBlank);`pass_shape` 决策表另立 (Warp, Direct, 链非空) → Direct 一格,把排除写成**可执行表格行 + 单测钉死**(WARP 会话开链 = 内容照画、效果丢弃)。

**解除前置(不锁死,独立后续票)**:WARP 两段式通路(新 intermediate + WARP 效果实例——今日结构性不存在)+ 8ms 计时证据(#137 探针口径)。两者齐备可开票解除,无需改本 ADR 的其余决策。

### D6. 值域出处与刻度(O4=A / O6=A)

- 出处 = MS Learn `D2D1_SHARPEN_PROP` **枚举页** Constants 表:SHARPNESS allowed range 0.0–10.0 default 0.0、THRESHOLD allowed range 0.0–1.0 default 0.0(#182 核验原文;效果页是 84 词 stub,勿引)。
- config 落 **int 键 0..=10**(枚举页 0.0–10.0 域的整数连续刻度,apply_* 宏族零新解析形态),off = 0。
- 属性索引照 #130 头文件转录先例钉测(d2d1effects_2.h:SHARPNESS=0、THRESHOLD=1)。
- THRESHOLD **钉实现内常数 0.0**(= 枚举页默认值,显式 SetValue 不依赖驱动默认),不暴露第二旋钮,待真实用例再开。
- 探针转负控钉测(票③):默认读回 = 0.0、越界 SetValue 行为(docs 已给域与默认,正控探针不再取)。

### D7. 表面(O5=A,最终形态待所有者拍板,默认 A)

View 菜单行 + 热键 + Options 一行;`Cmd` 枚举尾追加 id 123(#178 手法:既有 id 零扰动,冒烟按数字 id 直发 WM_COMMAND);README Differences 记段(beyond-original,记法参照 #68 keep_zoom、#178 undo-delete 先例)。fingerprint 的 policy 项加链身份 → toggle 触发 output_gen 递增(#126/#132 dump 变化信号),随票②接线。

### D8. 零扰动负控(结构保证,票①验收口径)

链空 ⇒ `pass_shape` 逐格回到 pre-#183 分派(全 (backend, arm) 单测钉死);Direct 臂链空不建 intermediate、不 CreateEffect;`EffectGraph::built_for` 携链身份(空链 = 现状四元组语义原样);票①合入 = 零用户可见变化(无键/无命令/无菜单)。票③冒烟补机器面:效果 off 逐字节比对、WARP 链非空输出与现状逐字节同。

## 实现票族

| 票 | 内容 | 状态 |
|---|---|---|
| ① #183 | 本 ADR + `pass_shape(backend, arm, chain)` 决策表纯函数(全单测)+ EffectGraph 链推广(CM-less 形态 + 链插 CM 前 + built_for 链身份)+ 零扰动负控 | 本 PR |
| ② | sharpen 键(0..=10)+ Cmd id 123 + View 行/热键/Options 一行 + fingerprint 链身份项 + README Differences | 待开 |
| ③ | 冒烟矩阵(off 逐字节负控 + hw on/off + dump 合同 + WARP 仅负控 + A4 负控钉测)+ 巨图分轴缩放+锐化边界观感对照 | 待开 |

冒烟分门禁口径(#182 处置):效果 off 沿用既有数值门(smoke130 ≤24);效果 on 只断结构合同(dump 非空、尺寸/格式不变、同输入逐位可复现),观感交人工 QA,不进自动化像素门。
