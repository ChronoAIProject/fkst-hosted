# github-devloop 自主开发状态机 — 设计与分阶段实施方案

把 sshx 循环（共识 → 实施 → review → 共识 → merge）固化成跑在 fkst 引擎上的长运行状态机，以 GitHub
issue/PR 为状态载体。本方案经 sshx thinking triplet（minimal/structural/delete）三方收敛。

⟦AI:FKST⟧

## 1. 架构总览

- 新增 GitHub-aware **composed 包 `github-devloop`**（`composed.deps: github-proxy, consensus`），把现有
  `consensus` 引擎编排成 issue → 实施 → PR → merge 的自主开发循环。
- **GitHub/git 是唯一状态源（doctrine）**：
  - GitHub 是 **eventually-consistent authenticated fact source**，不是 strong-consistency KV；
  - 评论 **`fkst:github-devloop:state:v1` HTML marker = 当前状态事实**；只信本 bot 作者（`FKST_GITHUB_BOT_LOGIN`）写出的 marker，普通用户伪造 marker 一律忽略；
  - state marker 同时携带 `version="<dedup>"`；转移只在最新可信 state 属于 `from_states` 且 incoming event version >= current marker version 时应用，旧事件晚到时按 stale skip；
  - issue/PR 的 **`fkst-dev:<state>` label = best-effort UI hint**（每次转移 set-exclusive 写目标状态、清其他状态，但 correctness 不依赖 label）；
  - 其他评论 **HTML marker = attempt / 共识结果 / loop 计数 / 分解链接**（读作事实时同样只信本 bot 作者，沿用 github-proxy 现有 marker 幂等）；
  - **git branch / PR = 实现事实**。
  - 每次 poll 从 GitHub/git **重导**状态，**不在 `<RT>`/cache 存业务状态**；崩溃恢复 = 重新 poll。
  - GitHub 没有 atomic compare-and-append；同 issue 的所有 department transition 使用同一个 `with_lock` key 序列化本进程内转移，marker 写入按 dedup 幂等，每次可靠投递都会回源重导并自愈 label/comment。读-CAS 到异步 marker 写之间仍有小 race window，但旧事件不会覆盖新版 marker，系统按 eventually-consistent 语义收敛。
  - meta-escalation 运行非确定性 `codex exec`；在 `with_lock` 内会先重导并检查同 version 的 meta result marker，若已存在则跳过，不重跑 codex、不写第二个结果。仍接受一个残余窗口：GitHub read-after-write lag 下，两次同 version `devloop_stuck` delivery 都可能在第一个 meta marker 可见前各自运行并写入相互矛盾的结果；这是当前模型的 eventual-consistency 限制，因为 GitHub 没有针对非确定性结果的 atomic check-and-write。若残余窗口发生，同 `(updated_at, loop_n)` state marker 用确定性 tie-break 收敛到保守终态。
- **安全**：opt-in（只处理带 `fkst-dev:enabled` label 的 issue/PR）；`FKST_GITHUB_WRITE` 是唯一姿态开关，默认 dry-run，设为 `1` 时直接自治真实写入；merge 仍由确定性 gate 保护（可信 marker、独立 `review-result:v1 approve`、head-bound、CI/mergeability、`--match-head-commit`、branch protection 服务端强制）；每段 loop 有 budget。

## 2. 状态机（完整转移，已验证闭合）

> 闭合性审查补全了目标转移：**失败路径**（implementing/fixing/merging 失败）、**merge 前 CI/冲突
> 检查失败**、**人工 escape**（label 被移除/改）、**人工 re-entry**（blocked 重开）。当前已实现的 issue 段只通过
> observe intake 执行 `nil -> thinking`，其他 escape / re-entry 仍是目标设计。

state marker = `<!-- fkst:github-devloop:state:v1 proposal="<id>" state="<S>" version="<dedup>" -->`。终态：
`impl-failed`、`blocked`、`merged`。`fkst-dev:<state>` label 只作为可自愈 UI hint。`needs-human`
= 尚未实现的 phase 在该状态停下等人工，后续 phase 把它自动化。loop 计数走 GitHub marker（不用 `<RT>`），崩溃后重新 poll 即重导。

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
 ready --[P3] 实施-----------------> implementing        # no push / no PR is currently prompt-level only

 implementing --ok----------------> pr-open
 implementing --fail--------------> impl-failed [needs-human terminal]
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

 merge-ready --CI+mergeable OK + review approve-> merging
 merge-ready --CI 红/冲突----------> fixing               # 回去修，不强 merge
 merge-ready --缺写开关/CI pending--> merge-ready          # dry-run，不推进

merging --ok---------------------> (merged) 关 issue
merging --fail-------------------> retry                 # merge 竞态/命令失败走可靠投递重试
```

### 横切 escape（任何状态，fail-closed）
```
 任何状态 --fkst-dev:enabled 被移除--------------> (unmanaged) 停止处理
 任何状态 --状态 label 被人改成非法/多个--------> 下次有效 state marker 转移时 set-exclusive 自愈
 (blocked) --人工移除 blocked + 重加 enabled-----> intake          # 人工 re-entry
```

## 3. 包布局（复用 > 扩展 > 新建）

- **consensus**（复用，不改）：source-agnostic 共识引擎。两段共识都用它。
- **github-proxy**（扩展，保持薄 I/O）：bounded issue/PR snapshot（labels + 解析的 marker）、label 读写请求、
  marker 评论；label request 不做状态 precondition，只执行 best-effort UI hint；后续加 issue-create / PR-create / PR-merge 请求。
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
  （`consensus.consensus_reached` → `ready|blocked` state marker + 结果评论 marker + label hint）。
- loop：无共识 marker 计数重试；超 budget → `stuck` state marker（停，Phase 2 接管）。
- 测试：opt-in 过滤、approve→ready、reject→blocked、retry、budget→stuck、dry-run 不写外部。

**Phase 2**：stuck → meta-escalation（结构化 `ACTION: implement|split|block`；split → `gh issue create` 建链接子 issue，仅评论建议）。
**Phase 3**：ready-CAS gates the attempt（`setup_worktree` + `spawn_codex` 实施；失败或无变更 → `impl-failed` state marker；有变更 → `implementing` state marker + branch/worktree marker；**先不开 PR**）。
**Phase 4**：`FKST_GITHUB_WRITE=1` → `gh pr create` + linkage marker；dry-run 只记录 would-open；PR poll → reviewing。
**Phase 5a**：PR diff review consensus 的 decision-only 切片：`observe_pr` 进入 `reviewing` 时产生 `devloop_reviewing`；`review_pr` 回源确认 issue canonical state 后，用独立预算保留 bounded PR diff，再附加 bounded issue context，中和为带 reviewed `head_sha` 的 `github-devloop/pr-review/.../<head_sha>` `consensus.proposal`；`review_result` 重新读取 PR trusted backpointer 和当前 head，要求当前 head 仍等于 reviewed `head_sha`，并用 issue state marker CAS 把 `approve` 写成 `merge-ready`、`reject` 写成 `fixing`，同时写 issue-versioned state marker、`review-result:v1` marker、`merge-ready:v1` fact marker 与 set-exclusive label。`approve` 产生 `devloop_merge_ready`，`reject` 产生 `devloop_fixing`；不 push、不 merge。
**Phase 5b（已实现）**：fix loop + review meta-escalation。`review_result` 的 `reject` 产生 `devloop_fixing`；`fix`
回源确认 canonical `fixing` marker、reject review marker、open same-repo PR、trusted PR origin 与 deterministic branch/head
都匹配后，在 deterministic branch worktree 中运行 codex 修复并提交。更新 PR 分支只由 `FKST_GITHUB_WRITE=1`
从 dry-run 切到真实写入，写前重导 issue/PR/head，非 force `git push origin <branch>`，推送后验证 PR head 等于
new head；成功写新 `reviewing` marker（version = `core.next_fix_version` 生成的 new-head fix-round canonical version）并重新产生 `devloop_reviewing`。缺写开关
不推进；无变更进入 `review-meta`。pr-review `consensus_unresolved` 由 `review_loop` 用独立 `review-loop:v1` /
`review-meta-trigger:v1` marker 计数，预算内重审同一 head，预算耗尽进入 `review-meta`；`review_meta` 只接受
`⟦FKST:ACTION⟧ fix|accept|block` + `⟦FKST:REASON⟧ ...`，分别推进 `fixing|merge-ready|blocked`；`accept` 产生 `devloop_merge_ready`。
**Phase 6（已实现）**：`merge` 消费 `devloop_merge_ready`，写前重新回源校验 canonical issue state 仍是同版本 `merge-ready` 或失败重试中的 `merging`、可信 head-bound `merge-ready:v1` comment-stream review-approval fact 与事件字段完全匹配、`review_proposal_id` 解析后仍指向同一 repo / PR / version 派生链 / reviewed `head_sha`、`FKST_GITHUB_WRITE=1`、可信 `review-result:v1 decision="approve"` marker 绑定同一 `review_proposal_id` / `review_dedup_key` / issue proposal / reviewed `head_sha` / version，PR current head open / same-repo / head branch 与 reviewed `head_sha` 未变、`gh pr view --json statusCheckRollup` green、`mergeable` / `mergeStateStatus` 可合并。`review_meta accept` 是预算耗尽后的较弱保守 override，只能产生 `merge-ready`，不能满足 merge 的 `review-result:v1 approve` backstop。`github-devloop` merge 不使用 GitHub `reviewDecision` / `latestReviews` / `addPullRequestReview`，也不生成 merge-time codex。全部满足才先由本 bot 直接写可信 `merging:v1` marker，再执行普通 `gh pr merge --merge --match-head-commit`，不使用 admin override、不绕过 branch protection；随后写 `merged` state marker、`merged:v1` marker、set-exclusive `fkst-dev:merged`，并 `gh issue close`。GitHub branch protection 的 required status checks 是真实运行的必需 repo-ops 前提，bot 账号不得具备 bypass/admin override；Lua 的 `statusCheckRollup` 只是早期/诊断 backstop，真正不可绕过的 gate 是 GitHub 在 `gh pr merge` 时服务端强制的 branch protection。若重试时 PR 仍 open / same head / not merged，会重新推导全部 gate 并再次执行 merge；若重试时 PR 已是 MERGED，只有匹配当前 PR/head 的本 bot `merging:v1` marker 或 canonical `merging` state 已可见才允许 finalize；外部 merge 不会被 devloop 自动关闭 issue 或写 terminal marker。缺可信 `review-result:v1 approve`、缺可信 `merge-ready:v1` approval fact、缺写开关、CI pending 或 mergeability 未定只 dry-run 或 retry 不推进；CI red、明确不可合并或 PR head 在写前重导时前进会写 `merge-gate:v1` marker 后回 `fixing`；merge/close 命令失败 error retry。独立性来自 codex context / proposal / head-bound diff / deterministic checks，不来自 GitHub 账号身份。

## 5. 关键风险 / doctrine 约束

- no-consensus 今天**静默** → 必须先给 consensus 加 bounded `consensus_unresolved` 事件，否则只能 poll-timeout（有竞态）。
- 状态转移**只能用最新 state marker CAS**；label 不区分 stale replay 与合法移除，只能做 UI hint。
- loop/stuck 计数**只能用 GitHub marker**（不用 `<RT>`/cache）。
- 同一 issue 的 version 排序是 `(updated_at ISO, loop round N, stage_rank)`；同 timestamp 下较大的 `/loop/N` 胜过无 loop 或较小 loop，即使后者阶段更靠后。meta 的同 version 终态冲突按确定性保守 tie-break 收敛，避免 GitHub 评论返回顺序影响当前态。
- PR diff / issue body 可能超 **64 KiB payload** → 用 `source_ref` 回源 + bounded snapshot。
- 自动 child-issue / PR / merge 有 **runaway + 权限**风险 → 只能用 `FKST_GITHUB_WRITE` 在 dry-run 与真实自治之间切换，并保留严格 budget 与 merge deterministic backstop。
- Phase 3 的 implement no-push/no-PR 约束目前由 prompt 表达；host-level sandbox 是后续 hardening。
- label 可被人改 → 下次转移 set-exclusive 自愈；状态事实仍以最新 state marker 为准。
- merge **不绕过** branch protection / CI。merge 要求可信 head-bound `merge-ready:v1` + 独立可信 `review-result:v1 approve` + `FKST_GITHUB_WRITE=1` + CI/mergeability/head gate；`review_meta accept` 不能合并。仓库必须配置 branch protection required status checks，bot 不能有 bypass/admin override；package 不查询也不配置 branch protection。
- 真实 supervisor 应从 pinned engine/package revision 启动，不从 mutable dev HEAD 启动；坏的自动 merge 会影响未来 repo 状态，但不会改变正在运行的实例代码。
- 残余风险：bot 账号被攻破可伪造可信 marker；LLM 独立 review 是 bot 派生判断，不是客观证明；branch protection 是 ops 配置，Lua 不能强制；sshx 不授权 commit/push/merge。

## 6. 待定（开放点）

- opt-in label 名：`fkst-dev:enabled`？还是沿用你已有的 GitHub label 体系。
- stuck 用 `fkst-dev:stuck`（no-consensus budget 后 Phase 2 接管）已采纳；实现失败用独立终态 `fkst-dev:impl-failed`，不进入 meta-escalation。
