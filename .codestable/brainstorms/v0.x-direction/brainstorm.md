---
doc_type: brainstorm
slug: v0.x-direction
created: 2026-05-15
status: active
summary: Rust 不到"可用"形态不发版本；首个 release 留给 Rust 可用、面向 B 类自托管用户
tags: [release-strategy, rust-rewrite, target-users, versioning]
---

# Roostery 0.x 版本方向与目标用户

> 创意空间 | 2026-05-15 | 下一步：cs-roadmap

## 项目核心动机（事后补记）

> 2026-05-15 update：本节在 req 起草过程中浮现，是脑暴最初没抓住但确实是 Roostery 存在的首要理由。回写到此供 roadmap 阶段不漏。

Roostery 的最强 "why" **不是**"飞书呈现"、**也不是**"vendor-neutral"——而是**多设备 / 跨窗口 vibecoding 的 agent 上下文同步问题**。

具体场景：

- 开发者越来越多在 vibecoding——跟 agent 对话出活
- agent 跑在某台特定机器上（桌面、远程开发机、云 GPU 实例）
- 开发者本人在多设备间切换（工位桌面、出门笔记本、通勤手机、晚上平板）
- 一旦人不在那台跑 agent 的机器跟前，就失去了对 agent 进度的可见性
- 终端 SSH 在手机上极其难用；Termius / blink shell 等方案要内网穿透 / 公网可达，对绝大多数普通开发者直接劝退

飞书（或任何云同步的 IM / 协作面）天然解决"同一份数据多设备可见"。把 agent 状态推到飞书 = 借云协作面已经做好的同步层做多设备整合，不必自己搭。

这条主动机也回过头解释了：

- **为什么 Feishu-native** 不是某种偏好，是因为飞书的天然跨设备能力是这个项目的物理前提
- **为什么 vendor-neutral / local-first / data-portable** —— 它们不是并列的卖点，是这条主动机下"不要因为引入 Roostery 又造一个新 lock-in"的具体兑现
- **为什么命名为 Roostery 而不是 feishu-xxx** —— 强调中间件主体性，避免被读成"飞书周边工具"

req 层面，这条主动机已写入 `agent-work-in-feishu.md`（pitch + "为什么需要"首段）；vendor-neutral / portable 在 `runtime-neutral.md` + `portable-by-default.md` 各自承接。

---

## 出发点

用户问"项目目标和开发路径清晰了吗"。诚实评估发现：

- **架构红线、定位、错位**清楚（README / CLAUDE.md / ARCHITECTURE.md 已就位）
- **planning/2026-05-15-rust-rewrite.md** 写得扎实，但是 gitignored 本地笔记，外部 contributor 看不到
- **关键缺口**：从"M3.C → M5.A Python baseline"到"open-source MIT product"之间的衔接没说。"行为兼容 Python 版"对从未公开发布过的 baseline 而言不构成 milestone
- **额外盲点**：0.1.0 对外发布的最小可用形态是什么？第一波目标用户是谁？怎么验证 PMF？
- **文档时间线打架**：CLAUDE.md / ARCHITECTURE.md 描述 Python 是 going-forward，但 planning §Phase 0 第一步就是把 Python 挪到 `legacy/`

不脑暴这块直接拆 roadmap 会得到一份只回答"怎么做"不回答"做给谁、做完算成功"的路线图。

## 聊过的方向

### 第一波目标用户的 4 个候选

| 候选 | 描述 | 评估 |
|---|---|---|
| A. 只给作者自己 | "自用 + 学 Rust"，开源是顺手 push | 浪费 vendor-neutral 红线 |
| **B. 飞书生态自托管 agent 用户** | 个人开发者 / 小团队，5 分钟装机 + 至少 CC 一个 runtime 出 task | **选定** |
| C. 公司 / 团队的 agent 平台运维 | 多用户配置、权限隔离、可观测性 | 当前架构是 single-user 假设，空中楼阁 |
| D. OSS 围观者 + dogfood | 代码质量 + 架构文档 + 学习笔记，不主动招用户 | 反直觉但和"学 Rust"事实契合；最后实质并入 Y2 形态 |

### Python → Rust 切换时机的 4 个候选

| 候选 | 描述 | 评估 |
|---|---|---|
| X1. 全等 Rust Phase 5 才发 0.1.0 | planning 文档原样推进 | 6-12 个月对开源仓库是天文数字；跟 B 不对齐 |
| X2. Python 版先发 0.1.0，Rust 是 0.2.0 | 双轨维护 | 工程量大；跟 planning Phase 0 归档冲突 |
| X3. 缩 scope vertical slice | 砍多 runtime / Base / reporting，最小 1500 LOC | 务实但要降级 README 承诺；与 Rust 重写定位冲突 |
| **X4. Rust 重写过程本身是 0.1.x** | 每 phase 对应一个版本，pre-release | **选定**（与 Y2 组合后） |

### Y1/Y2/Y3：B + X4 矛盾的解法

最初 B（5 分钟装机）和 X4（0.1.0 = Phase 0 脚手架）直接矛盾。AI 点出来，给三个解法：

- Y1：实际是 D 不是 B，承认不招用户
- **Y2：长期 B，但版本号 0.1.x 是内部 phase 里程碑，0.2.0 才是 B 验收** ← **选定**
- Y3：让 AI 重新分诊

## 当前倾向

**B（长期目标用户）+ Rust 可用前不发任何版本**

最初讨论收敛在 Y2（0.1.x = phase 里程碑 pre-release / 0.2.0 = B 验收）。用户进一步补充："版本发布要等到 rust 可用再说"——推翻了"每 phase 对应一个 0.1.x release"的机制。

更新后的版本策略：

- **Rust 重写期间**：仓库保持 v0.0.0（或不打任何 release tag），Phase 0-N 之间只走 commit + 内部 phase tag（自己回退用，不算 release）。**没有 release note、没有发版仪式、没有 pre-release 1.x 版本**
- **首个 release（0.1.0）**：Rust 形态达到"可用"才发。"可用"如何定义见下方遗留问题
- **0.1.0 之后的版本演进**：等首个 release 形态稳定再讨论，本轮脑暴不预设
- **时间预算**：不急，占位好好做。Rust 学习节奏自由，无外部 deadline

## 已敲定的点

### 已确认

- **目标用户**：B（飞书 + agent runtime 自托管开发者，个人开发者 / 小团队）
- **Rust 可用前不发任何 release**：仓库保持 v0.0.0，重写期间只走 commit 和内部 phase tag（自用回退，非 release）
- **首个 release（0.1.0）= Rust 可用 + B 验收形态**
- **`legacy/python/` 是临时归档**，Phase 7 删
- **Python 当前角色**：baseline 参考，不发布、不维护、不双轨
- **CI 平台**：GitHub Actions（planning §2 已倾向）
- **npm 名 `roostery`**：保留占位，删 `index.js`（planning §8 倾向已采纳）

### 倾向（推迟决定）

- **LLM provider 默认家**：推迟到 Phase 6 / 0.2.0 前夜
- **crates.io 发布时机**：推迟到 0.2.0 前夜；0.1.x 不上 crates.io

## 遗留问题 & 下一步

### 给 cs-roadmap 的输入

1. **路线图主轴 = Rust 重写 Phase 0-7**，已在 `planning/2026-05-15-rust-rewrite.md`（gitignored）写过详细任务清单和学习目标，roadmap 把它正式化进 `.codestable/roadmap/`，外部 contributor 看得到
2. **没有 per-phase release**——每个 Phase 是内部里程碑，不发版本、不写对外 release note。Phase 验收以"代码 + 测试 + 可选学习笔记（私人产出）"为准
3. **0.1.0 = "Rust 可用"触发点**作为独立 milestone 节点。"可用"如何定义见下方"待验证的假设"，Phase 5 临近时再细化成 0.1.0 acceptance criteria
4. **planning §8 推迟项**（LLM provider / crates.io）作为 roadmap 的 open question 挂着，到 0.1.0 前夜再决

### 文档清理（roadmap 之外的独立工作）

- **CLAUDE.md / ARCHITECTURE.md 时间线不一致**：现在描述 Python 是 going-forward，但 Phase 0 第一步就是归档。两种做法：
  - 现在改 → "Rust 重写期 / Python 是 baseline 参考"双轨叙述
  - 等 Phase 0 落地再改 → 接受文档暂时落后
- **README 改写**（**升级为显式任务**）—— 两个独立动因合并到一次改写：
  1. 定位明确 "Rust 重写中，仓库未发版，请关注 commits / phase tags 跟踪进度，0.1.0 = Rust 可用形态再说"
  2. **leading with user-why, not tech stack**：现在的 README "核心定位" 段四条全是技术属性词（vendor-neutral / Feishu-native / local-first / developer-priority），上来不抓"读者为何要看下去"。改写时把**多设备 vibecoding 痛点**提到 hero 段（参 `agent-work-in-feishu.md` "为什么需要" 首段），技术属性下移到后续段落
  - 作为 Phase 0 任务的一部分或独立轻量 feature 都行，按 roadmap 排期

### 待验证的假设

- **"Rust 可用"如何定义**——这是触发首个 release 的判据，本轮没敲死。三个候选：
  - **可用 = Phase 3 完成**：`roostery init` 跑通就算，bot bridge / dispatcher 可缺。但这只是装机不出活，B 用户拿不到价值
  - **可用 = Phase 5 完成**：bot bridge 通 + 至少 CC runtime 出 task，对应原 B 验收。**最合理**
  - **可用 = Phase 7 完成**：全 port 完，对标 Python 完整功能。最保守
  - 倾向 Phase 5，但等 Phase 4-5 临近再敲死
- **"陌生开发者 5 分钟装机"的具体场景** 在 Phase 5 临近时需要细化成 0.1.0 acceptance criteria（user story + smoke 标准）

### 愿景落 requirement 的建议

用户故事 / 痛点 / 边界已经比较清楚（B 类用户的痛点是 vendor lock-in + 数据假于人；边界是 pre-1.0 不承诺 multi-runtime full coverage）。可以触发 `cs-req draft` 把愿景落成 requirement，后续 roadmap 和 design 都有稳定对齐基准。
