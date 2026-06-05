# github-devloop 自主开发状态机 — 设计与分阶段实施方案

把 sshx 循环（共识 → 实施 → review → 共识 → merge）固化成跑在 fkst 引擎上的长运行状态机，以 GitHub
issue/PR 为状态载体。本方案经 sshx thinking triplet（minimal/structural/delete）三方收敛。

⟦AI:FKST⟧

## 1. 架构总览

- 新增 GitHub-aware **composed 包 `github-devloop`**（`composed.deps: github-proxy, consensus`），把现有
  `consensus` 引擎编排成 issue → 实施 → PR → merge 的自主开发循环。
- **GitHub/git 是唯一状态源（doctrine）**：
  - issue/PR 的 **label = 当前状态**（`fkst-dev:<state>`，单状态互斥，写前清旧）；
  - 评论 **HTML marker = attempt / 共识结果 / loop 计数 / 分解链接**（沿用 github-proxy 现有 marker 幂等）；
  - **git branch / PR = 实现事实**。
  - 每次 poll 从 GitHub/git **重导**状态，**不在 `<RT>`/cache 存业务状态**；崩溃恢复 = 重新 poll。
- **安全**：opt-in（只处理带 `fkst-dev:enabled` label 的 issue/PR）；每个自动化阶段后是 human-gated 节点；
  GitHub 写默认 dry-run + `FKST_GITHUB_WRITE`；merge 额外 gated（CI + mergeability）；每段 loop 有 budget。

## 2. 状态机（完整转移，已验证闭合）

> 闭合性审查补全了原草图遗漏的转移：**失败路径**（implementing/fixing/merging 失败）、**merge 前 CI/冲突
> 检查失败**、**人工 escape**（label 被移除/改）、**人工 re-entry**（blocked 重开）。每个非终态对其所有可能
> 事件都有出口。

label = `fkst-dev:<state>`（单状态互斥，写前清旧）。终态：`merged`（自动，关 issue）、`blocked`（人工可逆）。
`needs-human` = 尚未实现的 phase 在该状态停下等人工，后续 phase 把它自动化。loop 计数走 GitHub marker（不用
`<RT>`），崩溃后重新 poll 即重导。

### ISSUE 段
```
 (unmanaged) --+fkst-dev:enabled--> intake --raise proposal--> thinking

 thinking --approve----------------> ready
 thinking --reject-----------------> (blocked)
 thinking --unresolved & n<budget--> thinking          # 自环：loop 计数 n+1（marker）
 thinking --unresolved & n>=budget-> stuck
 thinking --codex 失败--------------> thinking           # 可靠投递自动重试，不前进

 stuck --[P1] 停-------------------> needs-human
 stuck --[P2] meta ACTION=implement-> ready
 stuck --[P2] meta ACTION=split-----> (blocked) + 建链接子 issue（各自 intake）
 stuck --[P2] meta ACTION=block-----> (blocked)

 ready --[P1] 停-------------------> needs-human
 ready --[P3] 实施-----------------> implementing

 implementing --ok----------------> pr-open  (P4；P3 先停在 needs-human 等授权)
 implementing --fail--------------> stuck     # codex/测试失败，不留半成品状态
```

### PR 段
```
 pr-open --poll-------------------> reviewing
 pr-open --PR 被关闭--------------> (blocked)

 reviewing --approve--------------> merge-ready
 reviewing --reject---------------> fixing
 reviewing --unresolved & n<budget-> reviewing          # 自环 loop
 reviewing --unresolved & n>=budget> review-meta

 fixing --ok----------------------> reviewing
 fixing --fail--------------------> review-meta

 review-meta --fix----------------> fixing
 review-meta --accept-------------> merge-ready
 review-meta --block--------------> (blocked)

 merge-ready --[P6] CI+mergeable OK-> merging
 merge-ready --CI 红/冲突----------> fixing               # 回去修，不强 merge
 merge-ready --[<P6] 停------------> needs-human

 merging --ok---------------------> (merged) 关 issue
 merging --fail-------------------> fixing                # merge 竞态/钩子失败，回修
```

### 横切 escape（任何状态，fail-closed）
```
 任何状态 --fkst-dev:enabled 被移除--------------> (unmanaged) 停止处理
 任何状态 --状态 label 被人改成非法/多个--------> 跳过 + 评论 warn，不自动动作
 (blocked) --人工移除 blocked + 重加 enabled-----> intake          # 人工 re-entry
```

## 3. 包布局（复用 > 扩展 > 新建）

- **consensus**（复用，不改）：source-agnostic 共识引擎。两段共识都用它。
- **github-proxy**（扩展，保持薄 I/O）：bounded issue/PR snapshot（labels + 解析的 marker）、label 读写请求、
  marker 评论；后续加 issue-create / PR-create / PR-merge 请求。
- **autochrono / github-autochrono**（不改）：保持简单 reply 流，不塞 devloop 逻辑。
- **github-devloop**（新 composed）：状态机本体 —— 状态↔label 映射、loop/stuck 计数、meta-escalation、
  worktree 实施、PR 生命周期。

## 4. 分阶段（每阶段独立可 ship + 可测）

**Phase 0（基础质量，已在做）**：consensus parser 特殊符号 label `⟦FKST:VERDICT⟧`/`⟦FKST:REPLY⟧` + 中和；
autochrono proposal_id lossless。状态机核心是 consensus，先确保它稳。

**Phase 1（最小可恢复闭环）**：issue → design consensus → GitHub 状态/结果回写 + no-consensus loop/stuck。
- 先给 `consensus` 加 bounded `consensus_unresolved` 事件（今天 no-consensus 静默，loop 无法驱动）。
- github-proxy 加：`github_entity_snapshot`（issue + labels + markers）、`github_label_request`、marker 评论。
- github-devloop 部门：`observe_issue`（opt-in snapshot → `consensus.proposal`）、`consensus_result`
  （`consensus.consensus_reached` → label `fkst-dev:ready|blocked` + 结果评论 marker）。
- loop：无共识 marker 计数重试；超 budget → `fkst-dev:stuck`（停，Phase 2 接管）。
- 测试：opt-in 过滤、approve→ready、reject→blocked、retry、budget→stuck、dry-run 不写外部。

**Phase 2**：stuck → meta-escalation（结构化 `ACTION: implement|split|block`；split → `gh issue create` 建链接子 issue，仅评论建议）。
**Phase 3**：ready → implementing（`setup_worktree` + `spawn_codex` 实施 + branch/worktree marker；**先不开 PR**）。
**Phase 4**：人工授权 → `gh pr create` + linkage marker；PR poll → reviewing。
**Phase 5**：PR diff review consensus + fix loop + review meta-escalation。
**Phase 6**：gated merge（`FKST_GITHUB_WRITE` + CI + mergeability 检查）→ merged → issue done/close。

## 5. 关键风险 / doctrine 约束

- no-consensus 今天**静默** → 必须先给 consensus 加 bounded `consensus_unresolved` 事件，否则只能 poll-timeout（有竞态）。
- loop/stuck 计数**只能用 GitHub marker**（不用 `<RT>`/cache）。
- PR diff / issue body 可能超 **64 KiB payload** → 用 `source_ref` 回源 + bounded snapshot。
- 自动 child-issue / PR / merge 有 **runaway + 权限**风险 → human-gated + dry-run + 严格 budget。
- label 可被人改 → 单-phase label 校验 **fail-closed**。
- merge **不绕过** branch protection / CI。

## 6. 待定（开放点）

- opt-in label 名：`fkst-dev:enabled`？还是沿用你已有的 GitHub label 体系。
- stuck 用 `fkst-dev:stuck`（Phase 2 接管）已采纳，便于前向兼容。
