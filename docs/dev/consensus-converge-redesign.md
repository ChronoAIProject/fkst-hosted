# Consensus 收敛重设计：从「盲循环 + 拆分」到「meta-judge 收敛 + 真停滞调和」

> **状态（2026-06）：本重设计的 Phase 1 + Phase 2a 已全部实现并合并（PR #44–#49）。** 下文「当前（错）」一节描述的是重设计**之前**的状态；当前态权威见 `README.md`（consensus / github-devloop 段）与 `CLAUDE.md`。Phase 2b（共识透明化，把三角度 + meta-judge 决定 post 成评论，覆盖 #11/#15）见下文分期。

## 动机

dogfood 暴露 + maintainer 指出：当前 no-consensus 处理「拆分太多、不正确」。参考已长期运行的 `consensus-rnd`（sshx / codex-refactor-loop 的源设计），其 **meta-judge 收敛** 模型才是正解：分歧只收敛到固定出口（达成 / 收敛中 / 真停滞），真停滞才进 meta-layer 调和，**绝不把 proposal 拆成子 proposal，绝不直接升级人**。

### 当前（错）

- `packages/consensus/decide`：3 角度并发 + **unanimity 聚合**（全 approve/全 reject→`consensus_reached`，否则 `consensus_unresolved`）。**没有第 4 个 meta-judge**，不保留三角度输出给 arbiter，不收窄问题。
- `github-devloop/review_loop`、`loop`：收到 `consensus_unresolved` 后**盲重跑**——只改 dedup `/loop/N`、重建~同一个 proposal，**不收窄问题**；到 `loop_budget=3` 进 `devloop_stuck` / `devloop_review_meta`。
- 失败终局：thinking 侧 `meta` 单 codex 选 implement/**split**/block（`split`=「拆分」核心，落 `blocked`+"Suggested split"）；PR 侧 `review_meta` 单 codex 选 fix/accept/block。

### 目标（consensus-rnd 模型）

```
proposal(round R, [narrowed_question])
  → 3 角度并发(peer-invisible，看到 narrowed_question 但看不到彼此输出)
  → meta-judge(第4 codex，读3角度) 判：
       reached:<framing>           → consensus_reached（达成即决定）
       converge:<narrowed_question> → consensus_converge（带收窄问题 + bounded 角度 digest）
  → 消费者(router) 收到 converge：以 round R+1 + narrowed_question 重发 proposal，回到 consensus
  → 直到 reached，或 router 判 true-stall（round ≥ 3 且角度立场连续无收窄/无变化）
  → true-stall → reconcile：drop(no-actionable-framing) / re-design(有具体新 directive) / re-cluster
                 —— 不 split，不直接 label 人
```

**职责划分**（保持 consensus 为 source-agnostic flat package）：
- `packages/consensus`：每个 proposal 跑「角度 → meta-judge」，只出 `consensus_reached` 或 `consensus_converge`。无状态、不持轮次。
- `github-devloop`（composed router）：拥有收敛**轮次**、真停滞**判据**、**reconcile**。轮次/digest 记在 GitHub trusted-bot marker（marker-as-fact + version-CAS）。

## 事件契约变更（packages/consensus，flat 包，改契约就改完整）

- `proposal`（consumed）：新增 `round`（默认 0）、可选 `convergence_question`（本轮收窄问题）、可选 `prior_round_digests`（bounded 上轮角度摘要，受 64KiB payload 界约束，只 verdict + 短 reply + digest，**不暴露上轮 peer 全文**保持 peer-invisibility）。
- `consensus_reached`（produced）：不变，唯一「达成」出口。
- **`consensus_unresolved` → 替换为 `consensus_converge`**（produced）：payload `proposal_id`、`round`、`narrowed_question`、bounded `angle_digests`、`source_ref`、`dedup_key`。（改契约：删 `consensus_unresolved`，旧形态从当前态删除。）

## 部门变更

### packages/consensus
- `decide/main.lua`：3 角度并发后**不再直接 unanimity 聚合**；新增（或拆 `judge/main.lua`）**meta-judge 第 4 codex**（prompt `prompts/meta_judge.lua`），读三角度输出 → 输出 `reached:<framing>` 或 `converge:<narrowed_question>`，分别 raise `consensus_reached` / `consensus_converge`。meta-judge **看不到下一轮**，只生成收窄问题摘要。

### github-devloop
- `review_loop`、`loop`：从「盲 budget 重跑」改为**消费 `consensus_converge` → 以 `round+1` + `narrowed_question` 重发 `proposal`**，并写 converge-round marker（`proposal`/`round`/`dedup`/`question`/角度 digest）。
- **true-stall 判据**（router 式）：读 trusted marker，绑定同 proposal/source_ref/version/head，`round ≥ 3` 且连续无收窄/角度文本无实质变化 → raise `devloop_review_reconcile` / `devloop_reconcile`。**round 1/2 不可能 stalled**。
- **新增 reconciler dept**（消费 reconcile，第 4 codex 或确定性判）：出 `drop` / `re-design` / `re-cluster`。`drop` → 终态 `blocked`（无 actionable framing），但语义是「放弃这个框架」非「拆分」。

## 删除（改契约就删干净）

- 删 `meta` 的 `split` action；**放弃 #31/#35「让 meta split 执行」方向**——这是错的「拆分」。`meta`（thinking 侧 stuck）从 implement/split/block 改为对接 true-stall → reconciler（implement 保留作 reached 出口的执行）。
- 删 `consensus_unresolved` 队列 + `review_loop`/`loop` 的盲 budget 重跑路径 + 相关常量/helper/测试。
- PR 侧 `review_meta accept` 不作 converge 替代。**保留** merge gate 必须独立可信 `review-result approve` / `merge-ready` fact（accept 不足以 merge）。

## 分期（每期 sshx 全流程：thinking→meta→impl→review→PR→CI→merge）

- **Phase 1 — consensus 引擎**：`decide` 加 meta-judge（角度→judge→reached|converge）；`consensus_unresolved`→`consensus_converge`（带 narrowed_question + bounded digest）；`proposal` schema 加 round/convergence_question/prior_digests；consensus 单测 + conformance。下游暂兼容（github-devloop 先把 `consensus_converge` 当旧 unresolved 触发既有 loop，行为不变）以便分期。
- **Phase 2 — github-devloop 收敛 wiring**：`review_loop`/`loop` 消费 `consensus_converge` → 带 narrowed_question 重发；converge-round marker；true-stall → reconcile；新增 reconciler dept；**删 `meta` split + 盲 budget 重跑**。
- **Phase 3 — 清理**：删死码/旧形态/#31/#35 痕迹；更新 `docs/dev/devloop-design.md` 与相关 memory。

## 风险 / 约束（sshx 三角度指出）

- **peer-invisibility 必须保**：meta-judge 不能把上轮原始 peer 全文喂下轮角度，只能给它生成的**收窄问题摘要**。
- **64KiB payload 界**：`prior_round_digests` 只带 verdict + 短 reply + digest。
- **PR-diff 的「收窄问题」要定义清楚**：不是「再审整个 diff」，而是「只判上轮分歧点 X 是否阻断 merge / 是否需 fix」——否则仍是盲循环。
- **无硬轮次 cap，但要成本 guard**：guard 只触发 true-stall/reconciler，不直接 block/split。
- 大 issue **decompose** 是单独刻意机制（如 consensus-rnd #403：epic→子设计 issue，gated），**不是**共识自动拆分；本重设计不引入自动 decompose。

⟦AI:FKST⟧
