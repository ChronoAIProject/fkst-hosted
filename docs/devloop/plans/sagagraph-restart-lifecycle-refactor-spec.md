# fkst SagaGraph 精炼重构 SPEC v9

本文由两层组成：

1. **CONSTITUTION**：唯一规范核心，只陈述不可削弱的 invariants、authority、scope 与 migration partial order。
2. **EXECUTION ANNEX**：对 Constitution 的保守扩展，只能细化、见证或履行核心义务；不得重新定义、削弱核心 invariant，也不得增加核心未授予的 authority。

---

# 第一层：CONSTITUTION

## §1. Scope

本 SPEC 定义 fkst 从 SagaGraph 中应吸收的最小 refactoring residue。它不是 Dapr/C# port，不引入 JSON graph runtime、SagaExpr、universal host、graph registry、governor、Ring model、package-side durable engine 或 package-side router。

本次迁移只改变结构：

- `restart_transition_table` 成为唯一 authored lifecycle model；
- lifecycle legality、CAS、branch、payload、effect authority、coverage、trace、observation 与 temporal obligations 从同一模型派生；
- generic runtime duties继续归 `fkst-substrate`；
- 所有当前可观察行为必须保持不变。

这里的“可观察行为”包括 transition、CAS status、reason、`cas_outcome`、handoff lookup 次数、timeout decision、queue、comment、label、codex dispatch、git push、merge、payload、dedup、terminal WHY 与 package-visible delivery expectation。

任何改变这些行为的 tightening 都不是 refactor，必须在本重构完成后作为独立 R9 behavior-change PR 落地。

## §2. Thesis

fkst 已具备 graph-as-data、saga rows、responsibility contract、resolver-backed liveness、durable crash-only substrate 与机械 conformance。剩余问题不是缺少新 framework，而是 lifecycle authority 仍散落在 canonical rows、`state_graph`、caller-supplied predecessor sets、caller-selected CAS、post-decision overrides 与直接 effect sinks 中。

唯一正确的 transition authority chain 由 §8 唯一规范；本节只引用，不再复述。

Row replay/kickoff authority也必须来自 canonical row 与同一个 owner-bound decider，但其 effect set 与 successor transition effects 严格分离；row replay capability不得被误当成 successor transition authorization。

## §3. Verified Premises

| id | verified fact | source |
|---|---|---|
| P1 | 独立全局 `state_graph` 与 owner-local `restart_transition_table` 同时存在。 | `libraries/devloop/state.lua:14,45`; `libraries/devloop/restart.lua:214-230,383-385` |
| P2 | `transition_status` 的 predecessor authority 当前由 caller-supplied `from_states` 提供。 | `libraries/devloop/state.lua:505-524` |
| P3 | `versioned_transition_status` 先拒绝 older incoming version；`cyclic_transition_status` 明确执行 newer→`pending`、older→`stale`、equal 后再做 source/stage 判断。 | `libraries/devloop/state.lua:527-572` |
| P4 | 多个 consumers 在 shared ordered CAS 后再做 raw/safe version 与 exact-state overlay。 | `review_result/main.lua:181-201`; `fix/main.lua:707-728`; `review_meta/main.lua:200-215`; `merge_executor.lua:357-399`; `reconcile/main.lua:262-273` |
| P5 | `review_pr` 有独立 `version-mismatch` observable outcome，不使用普通 shared CAS。 | `packages/github-devloop-pr/departments/review_pr/main.lua:28-49,85-124` |
| P6 | `implement` 只在初始 acceptance 做一次 direct-ID handoff verification；后续 gates只对保存的 handoff做结构重检，production test断言 lookup count 为一。 | `implement/main.lua:406-505,722-748`; `payloads/predicates.lua:119-130`; `integration_implement_meta_test.lua:765-800` |
| P7 | Timeout truth source并不由 `mode` 唯一决定：现有 rows分别使用 `codex_run:v1`、`live_defer_heartbeat:v1`、`live_defer_epoch:v1`、`child_workflow_wait:v1` 与 `state_entry:v1`。 | `restart_actionable_epoch.lua:322-345,511-517` 及对应 rows |
| P8 | `reviewing` 的 truth source是 `review-converge-round` heartbeat，不是 process-not-running。 | `packages/github-devloop-pr/core/restart/transitions/reviewing.lua:20-62` |
| P9 | Real lifecycle sinks包括 queue/comment/label、`workflow_codex.dispatch`、git push 与 irreversible merge。 | `implement/main.lua:113,228`; `fix/main.lua:466,651`; `review_meta/main.lua:72`; `merge_executor.lua:618` |
| P10 | Row `effects` 表示当前 state 的 replay/kickoff effect；successor transition effects由实际 writer branch产生，两者不是同一 authority。 | `thinking.lua:89`; `consensus_result/main.lua:39-120` |
| P11 | 外部 owners已向 published seam `github-devloop.devloop_execute_request` 发出命令；owner front door会 fresh re-derive 后接受。 | `intake_judge/main.lua:24,84-88`; `workflow_select/main.lua:7`; `execute_start/main.lua:13-20,87-95` |
| P12 | `workflow.registry.build_indexed_array`只保证排序、唯一与 key 对齐，不提供 owner seal。 | `libraries/workflow/registry.lua:59-90` |
| P13 | Issue canonical row modules与 lifecycle provider当前位于 shared `libraries/devloop`，ops经该 provider读取 issue rows/budgets；PR rows由 package-local wiring组装；ops当前只依赖 `github-devloop` event graph，且 observability pipeline丢弃 report return。 | `libraries/devloop/restart/issue_lifecycle.lua:3-36,48-80`; `libraries/devloop/fkst.toml:102-112`; `github-devloop-ops/core/doctor.lua:10,135`; `github-devloop-ops/core/state_gap.lua:5,179-191`; `github-devloop-ops/fkst.toml:8-12`; `observability/main.lua:84-87` |

`versions_equivalent` 的完整 domain semantics仍标记为 **ASSUMED-UNVERIFIED**：虽然当前实现可见于 `libraries/devloop/state.lua:162-170`，但其对所有历史 version forms 的兼容语义必须由 Step 0.1 corpus确认，不能只凭 helper body提升为新规范。

## §4. R1–R11 Constitutional Invariants

### R1. SINGLE AUTHORED LIFECYCLE MODEL

`restart_transition_table` 是唯一 authored lifecycle model。所有 typed edges、legality、pending participation 与 temporal obligations必须从 canonical rows派生；temporal obligation ID与 obligation body直接 authored in row，index与 provider bindings只能从 rows派生，orphan、duplicate或 drifting entry一律失败。

Issue 与 PR canonical rows及其 derived temporal indexes必须 owner-local。Issue 与 PR projections只允许在 conformance 中 union 后与 frozen legacy graph比较，不得形成跨包 production decider；shared library与 ops不得加载 owner rows或 derived temporal index。

Step 0.1 必须同时证明 OLD→NEW 与 NEW→OLD 的 symmetric parity；缺失或多出的 edge都失败。

### R2. OWNER-CONFINED OPAQUE EFFECT AUTHORITY

Production decider、snapshot seal、grant seal 与 builder verifier必须位于 owning package 的非 peer-requireable模块中；shared `libraries/devloop` 只能提供 extractor、pure relation、closed policy 与 normalization machinery。

Opaque grant只约束 **lifecycle-authoritative effects**：state markers、lifecycle labels、lifecycle-advancing queue raises、git push、merge，以及推进 lifecycle 的 codex dispatch。每条 edge必须为这些 effects声明 closed `transition_effect_entitlements`，分别覆盖 `apply` 与 `idempotent`；row replay effects保持独立。

R3 published-seam intents与 R7 anomaly telemetry是显式 grantless、non-authoritative classes；它们不得要求 transition grant，也不得获得 grant-minting authority。任何 lifecycle-authoritative sink若未分类、无 entitlement或无 opaque grant，terminal deletion失败。

### R3. TRUSTED ACCEPTANCE, UNTRUSTED PUBLISHED INTENT

Published-seam command是 untrusted intent，producer不需要 grant，也不得获得 grant-minting authority。Lifecycle owner必须在 acceptance 时 fresh re-derive、验证 intent、解析 canonical edge并自行 mint authority。

Owner产生的 marker、state label、internal lifecycle activation 与 downstream lifecycle-authoritative effects仍全部 grant-gated。Published-seam intent与 R7 telemetry始终 grantless；它们不能要求、携带或 mint transition grant。Decision完成后不得由 caller扩大、缩小或替换 grant。

### R4. ONE FAIL-CLOSED PAYLOAD REGISTRY

Payload token validation与resolution只有一个 registry。Unknown token、unknown strategy或缺失 required evidence必须 fail closed；grant本身永不进入 payload。

### R5. TYPED PATH OBLIGATIONS

L1必须从完整 typed edges生成 edge、edge-pair、family variant、bounded-loop、CAS、pending、generation、entitlement与timeout obligations；不得先退化为 `(from,to)` pairs，也不得用 `assert_covers` 字符串代替真实 witness。

### R6. TRACE CONFORMANCE

L2 actual trace必须逐步等于 table-derived expected trace，并覆盖 decision、effect entitlement、queue、payload、observable write与 terminal WHY。Test harness只执行和记录，不解释 lifecycle semantics，不构造第二个 router。

### R7. OWNER-LOCAL READ-ONLY OBSERVATION

Issue 与 PR owners各自绑定自己的 rows并执行 owner-local read-only legality analysis；evidence不足时必须诚实产出 `ordering-indeterminate` / `cause-indeterminate`，不得猜测。Ops不加载 owner rows、不重做 legality、不 mint grant、不反馈 mutation。

本 refactor中 analyzer与 anomaly schema只以 shadow/conformance pure computation存在：不 emit anomaly event、不新增 queue或 package-visible delivery、不增加 ops event dependency、不启用 ingestion。Anomaly event emission、ops的 `github-devloop-pr` dependency与 ingestion必须等 terminal deletion完成后，作为独立 R9 behavior-change PR并携带 intent manifest落地。

未来 anomaly transport仍是 grantless、non-authoritative telemetry；production 中不得 union issue/PR rows。

### R8. RESOLVER-FAITHFUL LIVENESS

Timeout evidence必须忠于 row声明的 `actionable_epoch.source` 与 resolver。任何较弱 proxy不得替代该 truth source。Positive process-running/not-running只适用于 `codex_run:v1`；heartbeat、durable-clear、dependency release与 child-workflow rows使用各自证据。

### R9. LEGACY-EXACT BEHAVIOR PRESERVATION

所有 CAS、handoff、timeout与effect policies必须以 **LEGACY-EXACT** 形式 bootstrap，完整复现当前 source semantics，即使当前语义不够漂亮。

Refactor PR不得包含 fresh-each-gate handoff lookup、heartbeat process-not-running要求、mismatch tightening或其他行为改变。此类改变只能在 refactor terminal deletion之后，以独立 R9 behavior-change PR和 intent manifest落地。

Checker、comparator、normalizer与 mandatory corpus必须来自 protected merge-base；checker与 checked semantics不得同窗替换。

### R10. CAPABILITY-MATCHED TEMPORAL PROVIDERS

每个 temporal obligation的 ID与 body必须 authored in owning canonical row；owner-local index与 provider binding只从 rows及 closed provider capability matrix派生。Orphan、duplicate、drifting entry或 separately-authored provider binding必须 fail conformance。

每个 temporal obligation必须绑定能实际证明它的 provider：

- response-with-deadline由 R8 liveness/watchdog evidence监控；
- observed structural precedence由 R7 transition history监控；
- absence必须由明确 emission ledger与 declared producer监控。

没有 capable provider的 obligation必须 fail conformance，并记录 `unmonitored`/`indeterminate`；不得把所有 obligations虚假路由到 R7。Issue rows与 R10 index必须位于 `packages/github-devloop/core/restart/`；ops只能消费 owner发布的 legacy-exact read-only observation facts，不得经 shared `libraries/devloop`读取 issue rows或 temporal index。

### R11. FANOUT-ONLY MESSAGE SEMANTICS

部门/包之间只有两种通信原语：**fanout-shaped 单向消息**（`raise` 发布领域事实或 one-way published-seam intent/command，subscription 决定 acceptance、无 requester-correlated reply）与**直接 library 调用**（同步、返回值）。没有第三种。单向 published-seam intent/command是合法 fanout-shaped entry，不是 request-reply；**request-reply（1:1 对话，如 consensus）的 requester-correlated reply绝不做成消息——它是一次 lib 调用。** 完整 doctrine以 `CLAUDE.md` 的「消息只许 fanout」节为权威准绳。

这里的 fanout-only 是 **provenance-independent acceptance semantics**，适用于所有 queues；它不是 `M.spec.fanout` 字段。后者只是 queue-cardinality contract，不声明 broadcast topic，也不决定 R11 是否适用。

- **doctrine/review lens：requester-provenance noninterference（受众无关性）**。固定 queue/领域事实/消费者状态，只改变/删除 requester·origin·correlation·continuation 来源 R，消费者的**业务效果必须不变**（skip/drop、写哪个领域状态、恢复哪个 continuation、触发哪个 effect、产生哪些后续消息；诊断 log/trace 不算）。反事实 review 测试：删掉「谁发起/哪个 pending 在等/reply address/correlation token」后仍是完整领域事实 → 可能真 fanout-shaped message，否则 request-reply。三类 id 分角色：`event_id`（去重）合法/选调用方非法；`entity ref`（回源实体）合法/冒充 reply correlation 非法；provenance-independent admission后的 same-entity lineage/CAS合法；`requester·correlation·continuation ref` 不得进 public event业务 contract。当前 `consensus_result` 等 `skip-foreign(proposal_id)` origin filters命中最后一类，是 shrink-only MIGRATION DEBT，不是合法 exemplar。该 invariant与反事实目前是 review lens，不是 general mechanical detector。
- **canonical 唯一形态**：`consensus.reach(proposal) -> reached|converge`，source-agnostic workspace library，同步返回，不认 caller/reply-queue；caller 持 saga marker/CAS/retry/re-derive。Request-reply logic的唯一 reuse form是 declared direct `lib_deps` composition；需要包面时用薄包 call library。`[event_deps]` 保持 event-topology composition（Facade/Adapter），只组合 fanout-shaped queues，明确禁止用于 request-reply reuse；两种 composition的职责与语义严格分离。
- **层归属**：引擎只知静态 `raise ⊆ produces ⊆ published_seam` 与 `M.spec.fanout` queue-cardinality contract，看不到 Lua provenance-independent acceptance semantics → 归 **fkst-packages conformance**；不新增引擎 `kind="broadcast"` 自报字段。
- **refactor phase：inventory + no-growth，绝不 zero-surface。** 本 refactor必须 inventory现有 known request-reply message surfaces；`migration/request-reply-message.allowlist` 是 shrink-only no-growth baseline。Allowlist机制结构上只许 shrink，但 R9 要求现有 consensus surface（`consensus.proposal`、`consensus_reached`、`consensus_converge`、reply consumers及相关 `[event_deps]`）在本 refactor中原样保持 allowlisted，所以本阶段实际只有 no-growth、没有 deletion。禁止新增 request-reply surface，但不禁止这份现有 surface；R11在 refactor中不删除任何 queue、delivery或 package surface，R9 legacy-exact parity保持成立。
- **post-terminal phase：单独 behavior change才激活 zero-surface。** Terminal deletion完成后，`consensus` package→library迁移与 zero-surface activation必须在**单独 R9 behavior-change PR**中携带 intent manifest落地，把 known-dialogue inventory ratchet到 zero；不得并入本 refactor。`product-outcome parity`定义为同一 caller输入与 external facts下，迁移前后 caller-observable outcome相等：相同 reply value、相同 saga transitions、相同 markers。Delivery form从 queue→call是 manifest明确授权的变化，不要求 delivery-form equality，也不得借此放宽其余 product outcome。
- **mechanical harness status：`DESIGNED, NOT YET ENFORCED`。** 包拥有的 checker固定为 `scripts/check_repo_fanout_only.py`，shrink-only inventory固定为 `migration/request-reply-message.allowlist`。Checker PR落地、配套 controls通过并接入 §10 repository checks之前，CI尚未强制 R11。当前唯一可声称的 mechanical harness design是上述 known-dialogue inventory ratchet（refactor no-growth、post-terminal zero）；不得声称已有 general detector。
- **general checker是 future work，不从 lifecycle rows臆造。** 当前 `output_obligation`只有 kinds/exits，不声明 requester identity、response-to relation或 admission前后 identity use；same lineage又可合法服务 post-admission CAS，所以不能推出 request-reply。通用 audience-independence checker必须先新增 typed evidence：每条 cross-boundary message声明是否携带 requester provenance，且 producer/consumer contract声明 consumer acceptance与该 provenance无关。这些 evidence当前不存在；`output_obligation completed by same-lineage return`不得作为 detector claim。

## §5. Row Admission + Family-Fanout Rule

Row或edge字段只有在同时具备 named consumer、evidence provider、conformance rule与 deletion path时才能 admission。

必须满足：

- 每条 edge显式声明 closed CAS policy、pending participation、typed cause与 apply/idempotent effect entitlements；
- pending participation不得从 edge kind、state name或 stage rank猜测；
- row replay effects与 successor transition effects不可合并；
- autonomous successors只来自 responsibility successor families；
- entry、operator reentry、timeout、guard、canonicalization与 receiver activation不得伪装成 autonomous successor；
- worker fanout按 postcondition family计数，同一 success family可有多个 variants，但不得增加 unrelated success family；
- idempotent effect只能由 exact idempotent entitlement授权；
- terminal deletion前必须对完整 head tree证明零 unclassified lifecycle-authoritative sink、零 grantless lifecycle-authoritative sink与零 authority bypass；published-seam intents与 R7 telemetry必须显式列为 grantless-by-classification，且不能 mint transition grant。

## §6. REJECT

明确拒绝：

- SagaExpr/CEL；
- JSON graph format、graph registry、universal host、zero-restart dynamic deploy；
- full model checker、P-language、完整 Dwyer suite与 unbounded liveness；
- Ring/governor/trust-ledger/lease constitution；
- Forge-emits-graph；
- package-side inbox/outbox/journal/reminder/durable engine；
- package-side router或 TestRuntime；
- hierarchical-state flattening、JoinLedger、k-of-N machinery；
- independent L3 monitor service与 circuit breaker；
- generic LLM-decider fallback；
- universal effect catalog或 effector registry；
- Python/Rust中的 GitHub lifecycle legality；
- caller-supplied rows、caller `cas_mode`、policy callback或 grant seed；
- post-decision handoff/version override；
- ops-owned cross-package legality。

Per-edge closed effect entitlements不属于被拒绝的 universal effect catalog；它们只收窄既有 lifecycle authority，不扩展 effect surface。

## §7. LOCUS

- **Substrate Issue A**：generic read-only live transition/delivery timeline ledger。仅当 owner-visible facts无法提供 engine-only ordering/provenance时实施；它不是 R7 shadow/conformance analysis的前置。若未来 post-terminal behavior-change PR激活 anomaly transport，它只补 generic ordering/provenance，不改变 owner-local authority。
- **Substrate Issue B**：hermetic run-to-quiescence TestRuntime。仅当现有 `run_department`、`run_graph` 与 `fire_raiser`实证不足时实施。
- **Substrate Issue C（候选·方向已定、层界待对抗辩论）**：generic **lifecycle-row totality + no-silent-swallow escalation** 引擎原语。把 package 声明的 restart-transition-table 提升为**引擎识别的声明式契约**（如 `M.spec`），引擎在 supervise-load **fail-closed** 强制两件事：① 每个非终态携带 `{budget, watchdog-mode, guaranteed-termination target, single-responsibility signature}`；② **任何未显式处理的情况必然 fail-closed、带富事实（`error_class`/`fingerprint`/`source_ref`/WHY）逃逸，不得静默 `return`/`skip`/false-terminal**。二者一起才让「A→执行→B + 有界 + 抛向上级 + 日志迭代写回 case」这套自进化循环**全域成立**：完备性不靠上游穷举、靠下游日志迭代长出来，而静默吞掉的未知**永不进日志、永不被 codify**——故「进料口不可堵」（no-silent-swallow）与 bounded-shape **同为原语契约、缺一不可**（只强制 bounded-shape 不够）。generic：引擎只校验 shape，不认具体状态（`thinking`/`reviewing`）。与 §6「package-side engine」拒绝不冲突——这是**引擎侧**通用强制（与 `raise ⊆ produces ⊆ published_seam` 同类、同属本节 LOCUS 引擎归属），非 package-side engine。
  - **美不美（BEAUTY·对抗待定，不自证）**：正向——引擎已通用强制 `raise ⊆ produces ⊆ published_seam`（事件图），这只是「同一 move 往上一层」到 lifecycle-state totality：有先例、generic、make-illegal-unrepresentable（malformed 生命周期表加载不了）。丑的风险——引擎耦合到 `github-devloop` 具体抽象（leaked abstraction），或它其实该是 ② typed schema 而非 ③ runtime-guard。
  - **值不值（WORTH·proportional-containment·对抗待定，不自证）**：值——SDLC-as-process 方向 = 多 process/repo 会用（通用能力配得上引擎层）、跨所有包不可绕（库 conformance 只在包记得跑时守）。不值的风险——若实证只 `github-devloop` 用，引擎化即过度上提（proportional-containment 违例），应停在 `workflow`/`devloop` 最强库 conformance。
  - **定界规则**：方向（引擎强制、通用、no-swallow escalation totality）判为对；**精确层界（引擎识别声明式 FSM ③ runtime-guard / typed engine schema ② / 最强库 conformance）由 sshx 5 席 + ChatGPT Pro oracle 按证据对抗定，不单方假设、不自证**。排序：核心行不变式（budget + watchdog + guaranteed-termination + responsibility_signature）**已稳定** → 可据它先设计（比 grant 原语更靠前）；typed-edge / responsibility_signature schema 仍在 additive 相收敛 → **先锁稳定核心、随 schema 定型再扩，不 front-run**。engine 改动高 blast radius（重建所有 supervise 加载的 BIN），小步 + 合入后盯 dogfood。

`raise ⊆ produces ⊆ published_seam`继续由 substrate拥有；Lifecycle GraphSemantics、owner decider、CAS、effect entitlement与 anomaly analysis保持 package-side。Substrate Issue C 若落地，只把 lifecycle-row **totality + escalation-non-swallow** 这一 generic 不变式收归引擎强制，不把具体 GraphSemantics/decider/CAS 上移。

## §8. Migration DAG Shape, Authority Chain and Prohibitions

唯一允许的 partial order：

```text
line-budget containment
  → independent OLD observations + full sink inventory
  → shared pure extractor/policies
  → owner-local grant-disabled shadow deciders
  → symmetric edge/CAS/effect/pending parity
  → dependency-cycle cut
  → owner seals + grant-consuming sinks in shadow
  → protected-base R9 active
  → per-family shadow/swap/shrink
  → branch and payload consolidation
  → R5/R6 obligations and traces
  → R10 capable-provider index
  → R7 owner-local shadow analysis + schema conformance
  → terminal deletion
```

Transition authority chain不得改变：

```text
canonical rows
  → typed edges
  → owner-bound decider
  → opaque grant
  → grant-consuming lifecycle effect
```

禁止：

- caller rows；
- caller `cas_mode`；
- post-decision override；
- ops-owned cross-package legality；
- SagaExpr；
- graph registry；
- package-side engine；
- shared-library `make_decider`；
- producer-side grant requirement for published-seam intents；
- production issue/PR row union；
- behavior tightening混入 refactor；
- refactor内删除 known request-reply queue/delivery、激活 R11 zero-surface或执行 consensus package→library迁移；
- refactor内激活 R7 anomaly event、ops PR dependency、ingestion或 delivery；
- terminal deletion时保留 grantless或 unclassified lifecycle-authoritative sink，或未显式分类 published intent / telemetry。

---

# 第二层：EXECUTION ANNEX

## Annex 0. Conservative-Extension Rule

本 Annex只能：

- 给 Constitution invariant提供 schema；
- 给 invariant提供 algorithmic witness；
- 给 migration partial order提供执行 mechanics；
- 给 parity、hash、manifest、trace与 negative control提供可运行定义。

若 Annex与 Constitution冲突，以 Constitution为准；Annex中的任何 convenience不得增加 authority、放宽 evidence或授权 behavior delta。

## Annex §4. Typed Lifecycle Mechanics

### A4.1 Module Ownership

Shared modules：

```text
libraries/devloop/restart_edges.lua
libraries/devloop/restart_cas_policies.lua
libraries/devloop/restart_analysis.lua
libraries/devloop/restart_effect_entitlements.lua
libraries/devloop/restart_payload_registry.lua
libraries/devloop/restart_obligations.lua
libraries/devloop/restart_transition_anomaly.lua
libraries/devloop/restart_temporal_obligations.lua
```

它们只允许导出 canonical extractor、grantless pure relation、closed policy evaluators、schema、normalization与 conformance helpers；不得导出 `make_decider`、owner/grant seals、minting primitive、private verifier、owner rows/index或 production mutation API。

Owner-local modules：

```text
packages/github-devloop/core/restart/transitions/{index,<row>}.lua
packages/github-devloop/core/restart/temporal_obligations/index.lua
packages/github-devloop/core/restart_{authority,effects,analysis}.lua

packages/github-devloop-pr/core/restart/transitions/{index,<row>}.lua
packages/github-devloop-pr/core/restart/temporal_obligations/index.lua
packages/github-devloop-pr/core/restart_{authority,effects,analysis}.lua
```

Issue rows从 `libraries/devloop/restart/issue/transitions/`迁入 owner-local path；迁移完成时删除 `devloop.restart.issue.transitions.*`、`devloop.restart.issue_lifecycle`及其 `libraries/devloop/fkst.toml` exports，ops doctor/state-gap不得再 require它们。

每个 `restart_authority.lua`：

1. 从本 package-local wiring取得 canonical rows；
2. 创建不可伪造 owner seal并 seal fresh snapshot；
3. 调用 shared pure relation；
4. mint owner-local opaque grant；
5. 只暴露 bound `decide_transition`，并只向同 package effect facade闭包注入 private verifier。

`workflow.registry.build_indexed_array` 的 array identity不等于 owner identity；owner seal必须由 package-local closure建立。

### A4.2 TypedEdge Schema

```lua
{
  id = "<owner>/<row>/<kind>/<variant>",
  owner = "<package-owner>",
  row_id = "<row-id>",

  kind = "autonomous"
       | "entry"
       | "operator_reentry"
       | "timeout"
       | "guard_boundary"
       | "canonicalization",

  source = {
    state = "<state>" | nil,
    boundary = "<published-or-internal-boundary>" | nil,
  },

  target = "<state>",

  cause_evidence = {
    schema_id = "<closed-schema-id>",
    resolver_id = "<closed-resolver-id>",
    required_fields = { ... },
    trust = "<closed-trust-rule>",
  },

  cas_policy_id = "<legacy-exact-closed-policy-id>",

  pending_order = {
    participates = true | false,
    predecessor_state = "<state>" | nil,
  },

  generation_epoch = {
    mode = "preserve" | "bump" | "open",
    keys = { ... },
  },

  lineage_keys = { ... },

  transition_effect_entitlements = {
    apply = {
      {
        id = "<edge-apply-entitlement-id>",
        condition_id = "<closed-condition-id>",
        effect_ids = { ... },
        payload_projection_id = "<projection-id>" | nil,
        marker_replay = "none",
      },
    },

    idempotent = {
      {
        id = "<edge-idempotent-entitlement-id>",
        condition_id = "<closed-completeness-condition-id>",
        effect_ids = { ... },
        payload_projection_id = "<projection-id>" | nil,
        marker_replay = "none" | "exact-target-version",
      },
    },
  },

  timeout_evidence_policy_id = "<resolver-keyed-policy-id>" | nil,

  provenance = {
    owner = "<package>",
    row = "<row-id>",
    field = "<authored-field>",
  },
}
```

约束：

- `apply` 与 `idempotent` keys必须同时存在；允许显式空数组，不允许缺省为 unrestricted。
- `effect_ids` 必须来自 OLD writer observations，不得从期望中的 NEW architecture猜测。
- 同一 source/target但 cause、policy、generation或 effects不同的 edges不得合并。
- `to_states` 不再 authored；只从 autonomous rich successors派生。
- `pending_order` 是 required field，缺失即 conformance failure。
- Timeout edge必须声明与 resolver一致的 policy。
- `review_pr` receiver activation仍表达为 `entry`，不新增 transition kind。

唯一 extractor：

```lua
extract_typed_edges(canonical_rows)
  -> ordered immutable TypedEdge[]
```

Extraction sources：

| kind | canonical source |
|---|---|
| `autonomous` | `responsibility_signature.successors` |
| `entry` | typed `ingress` / `receiver_activations` |
| `operator_reentry` | typed operator entries |
| `timeout` | `on_timeout.on_escalate` |
| `guard_boundary` | `guard_boundaries[].successors` |
| `canonicalization` | typed canonicalization entries |

### A4.3 Row Replay Authority

Canonical row `effects`表示当前 state 的 replay/kickoff capability，不是 successor edge effect set。

Normalization产生独立对象：

```lua
{
  authority_kind = "row-replay",
  owner = "<package>",
  row_id = "<row-id>",
  condition_id = "<row-replay-completeness-condition>",
  effect_ids = { ... },
  payload_projection_id = "<projection-id>",
  lineage_keys = { ... },
}
```

示例：

- `thinking.effects = {"consensus.proposal"}` 是 replay/kickoff effect；
- `thinking → ready|dependency_wait` 的 comments、labels与 downstream effects来自 `consensus_result` transition observations；
- 不得把 `thinking.effects`复制到上述 successor edge；
- row replay也必须经 owner-bound decider取得 opaque grant，但 grant绑定 `authority_kind="row-replay"`，不能用于 successor transition。

### A4.4 Effect ID Grammar and Sink Closure

Effect IDs使用 closed grammar：

```text
queue:<qualified-queue>
comment:<issue|pr>:<semantic-purpose>
label:<issue|pr>:<semantic-purpose>
adapter:<adapter-operation>
codex.dispatch:<role>
git.push:<purpose>
github.merge:verified-pr
```

每个 observed sink必须有 stable ID、callsite与 closed `authority_class`：`lifecycle-authoritative`、`grantless-published-intent`、`grantless-telemetry`或 `grantless-non-lifecycle`。第一类仅包括 state marker、lifecycle label、lifecycle-advancing queue raise、git push、merge与推进 lifecycle 的 codex dispatch；只有它需要 opaque grant，后三类不得要求 grant或接触 minting primitive。

必须覆盖：

| sink | verified examples |
|---|---|
| queue/comment/label | lifecycle request builders与 `consensus_result/main.lua:39-120` |
| codex | `implement/main.lua:228`; `fix/main.lua:466`; `review_meta/main.lua:72` |
| git push | `implement/main.lua:113`; `fix/main.lua:651` |
| merge | `merge_executor.lua:618` |
| row replay kickoff | `thinking.lua:89` |

规则：

1. Owner-side lifecycle builder/facade/queue raise与 lifecycle-advancing codex/git/merge call必须消费同一 in-process opaque grant及 exact effect ID。
2. Grant不得序列化；durable lifecycle queue只序列化已授权 ordinary request，transport不得获得 minting authority。Published intent保持 grantless；post-terminal R7 telemetry遵守 A4.14 ephemeral posture。
3. 一个 grant不得授权未列出的 sibling effect；external lifecycle write前须以 fresh snapshot取得新 grant，E5 legacy handoff只可在同 receiver attempt按 legacy policy复用 evidence。
4. Unclassified或 grantless lifecycle-authoritative sink、unknown alias/wrapper/DI/direct call阻塞 terminal deletion。
5. Published intent与 telemetry必须在 inventory中显式 grantless-by-classification，且无 minting path。

### A4.5 Sealed Snapshot, Intent and DecisionResult

Production snapshot：

```lua
{
  owner = "<package>",
  entity = { kind = "issue|pr", repo = "...", number = "..." },
  proposal_id = "...",

  current = {
    state = "...",
    version = "...",
    stage_rank = 0,
    marker_provenance = { ... },
  },

  claim = { ... },
  head = { ... } | nil,
  base = { ... } | nil,

  snapshot_fingerprint = "...",
  lock_epoch = "...",

  _owner_snapshot_seal = <opaque>,
}
```

Snapshot必须在 entity-scoped `with_lock` 内 fresh re-derive。普通 table、analysis fixture或另一 owner snapshot不能用于 production grant minting。

Intent：

```lua
{
  semantic_variant = "<canonical-variant>",
  source_boundary = "<boundary>" | nil,
  target = "<target>" | nil,
  evidence_refs = { ... },
}
```

Caller不得提供 rows、edge set、`cas_mode`、`cas_policy_id`、resolver、`requested_effect_ids`、grant seed或 policy callback。Effect entitlement由 owner decider依据 canonical authority与 fresh evidence独立选择；caller不存在请求 subset或 superset的通道。

DecisionResult：

```lua
{
  status = "apply" | "idempotent" | "pending" | "stale" | "illegal",

  reason_code = "<closed-reason-code>",
  cas_outcome = "<legacy-exact-observable-outcome>",

  edge_id = "<canonical-edge-id>" | nil,
  row_replay_id = "<row-replay-id>" | nil,
  cas_policy_id = "<closed-policy-id>" | nil,

  evidence = {
    status = "complete" | "incomplete" | "invalid" | "indeterminate",
    resolver_id = "<resolver-id>" | nil,
    refs = { ... },
    facts = { ... },
    preliminary_status = "<status>" | nil,
  },

  current_fingerprint = "<snapshot-fingerprint>" | nil,
  effect_entitlement_id = "<entitlement-id>" | nil,
  granted_effect_ids = { ... },

  grant = <opaque owner-local grant> | nil,
}
```

Result rules：

- Owner decider独立选出的 `granted_effect_ids`必须与 selected entitlement exact相等；任何内部 subset/superset drift均为 `illegal`，caller不能参与选择。
- `apply`：exact canonical authority、legacy-exact policy与 required evidence通过后，按 apply entitlement mint grant。
- `idempotent`：只有 exact idempotent entitlement可 mint；effects已完整时允许 `grant=nil`。
- `pending`：无 grant。
- `stale`：无 grant；`version-mismatch`必须保留 distinct `reason_code`与 `cas_outcome`。
- `illegal`：unknown/ambiguous edge、unknown policy、unsealed snapshot、owner mismatch、malformed intent、forged evidence、unsupported effect或 generation violation；无 grant。
- Analysis relation返回 grantless result，runtime identity不能被 builder接受。

### A4.6 Opaque Grant

Grant绑定：

```text
owner seal
authority_kind
edge_id or row_replay_id
entity
snapshot fingerprint
lock epoch
target/version/generation
decision status
effect entitlement ID
exact effect IDs
```

Grant必须：

- 由 owner-local private closure创建；
- 不能由字段相同的 Lua table伪造；
- 不可序列化；
- 不可跨 entity、snapshot、edge、entitlement、generation或 lock epoch复用；
- 不能由 analysis API产生；
- 不能通过 shared library、ops、DI或 tests取得 minting primitive。

### A4.7 Published-Seam Intent Contract

`github-devloop.devloop_execute_request` 是 untrusted intent seam。

Producer：

- 只需构造符合 published payload schema的 ordinary intent；
- 不持有 lifecycle grant；
- 不持有 owner rows、owner seal或 minting API；
- 无法授权 `thinking` marker、state label或 `consensus.proposal`。

Owner acceptance：

1. `execute_start`验证 payload与 source reference；
2. 在 owner lock内 fresh读取 issue、claim与 current marker；
3. 将普通 intent解析为 canonical entry edge；
4. owner decider决定 accept/pending/stale/illegal；
5. 只有 acceptance成功后，owner grant授权 thinking comment、state label与 proposal activation。

Producer自己的 non-lifecycle intake UI effects按 inventory独立分类；它们不能被误认为 lifecycle acceptance grant。

## Annex §4.8 Legacy-Exact CAS Catalog

### A4.8.1 Base Result Algebra

记号：

- `T`：current state是 target；
- `S`：current state是 declared source；
- `P`：current state可经 frozen pending projection到达 source；
- `O`：其他 current state；
- `Ø`：current state/version缺失；
- `A/I/P/S`：`apply/idempotent/pending/stale`。

#### `cas.base_plain_legacy_v1`

| current class | result |
|---|---|
| `T` | `I` |
| `S` | `A` |
| `P` | `P` |
| `O` | `S` |
| `Ø` | 按 `unmanaged` pending projection计算 |

#### `cas.base_versioned_legacy_v1`

它必须先执行 current source中的 ordered comparison，再执行 plain relation。

| incoming order vs current | `T` | `S` | projection predecessor | unrelated | missing current |
|---|---:|---:|---:|---:|---:|
| older | `S` | `S` | `S` | `S` | 不适用，退回 plain |
| equal | `I` | `A` | `P` | `S` | plain |
| newer | `I` | `A` | `P` | `S` | plain |
| incoming/current不可比较或缺失 | plain | plain | plain | plain | plain |

“newer”不能被改成 cyclic semantics；当前 `versioned_transition_status` 对 newer并不自动返回 `pending`。

#### `cas.base_cyclic_legacy_v1`

执行顺序不可交换：

1. incoming missing → plain；
2. current target且 current version与 `target_version` equivalent → `idempotent`；
3. compare incoming/current；
4. newer → `pending`；
5. older → `stale`；
6. equal时 target→`idempotent`、source→`apply`、target stage rank更高→`apply`，否则 `stale`。

| incoming order | equivalent target | non-equivalent target | source | missing | unrelated |
|---|---:|---:|---:|---:|---:|
| missing | plain | plain | plain | plain | plain |
| newer | `I` | `P` | `P` | `P` | `P` |
| older | `I` | `S` | `S` | `S` | `S` |
| equal | `I` | `I` | `A` | stage fallback | stage fallback |

`versions_equivalent` 的 complete domain semantics保持 **ASSUMED-UNVERIFIED**；Step 0.1必须冻结所有 observed forms后才能认定 parity。

### A4.8.2 Exact Overlay Rule

Raw/safe overlay只能在 ordered base relation之后执行，不能替代 base relation。

```text
base relation
  → preserve base pending/stale/idempotent branches in legacy order
  → exact source/state overlay
  → raw or safe version equality overlay
  → legacy reason/cas_outcome
```

通用 overlay结果：

| base result | exact source? | version equality? | final |
|---|---|---|---|
| `pending` | 任意 | 任意 | 保留 `pending`，除非 explicit legacy handoff policy |
| `stale` | 任意 | 任意 | 保留 `stale` |
| `idempotent` | 按 consumer legacy rules | 按 consumer legacy rules | 保留其 exact legacy idempotent branch |
| `apply` | no | 任意 | `stale` / exact legacy from-state reason |
| `apply` | yes | no | `stale`, `reason_code=version-mismatch`, exact legacy `cas_outcome` |
| `apply` | yes | yes | `apply` |

Safe overlay只使用当前 consumer已使用的 `safe_version_segment`，不得用于 order comparison。

### A4.8.3 Complete Consumer Profiles

每个 profile是 closed policy，不允许 caller在 profile返回后继续 override。

| complete policy ID | consumer | ordered base | legacy overlay/order |
|---|---|---|---|
| `cas.legacy_loop_plain_v1` | issue `loop` | plain | 无 |
| `cas.legacy_consensus_result_v1` | `consensus_result` | versioned | exact-version idempotent completeness repair |
| `cas.legacy_issue_reconcile_v1` | issue reconcile | versioned | existing state/terminal checks保持原顺序 |
| `cas.legacy_timeout_reconcile_v1` | issue/PR timeout reconcile | versioned | fresh instance、due、attempt与 lineage checks先行 |
| `cas.legacy_observe_issue_entry_v1` | issue ingress/replay | versioned | current exact branch order |
| `cas.legacy_awaiting_pr_v1` | awaiting-PR replay | versioned | current parent/child fact checks |
| `cas.legacy_observe_pr_v1` | `observe_pr` | versioned | raw `pr-open` version equality；mismatch保持 distinct |
| `cas.legacy_review_result_v1` | `review_result` | cyclic | safe-segment equality overlay |
| `cas.legacy_fix_v1` | `fix` | cyclic | raw exact state/version overlay |
| `cas.legacy_review_meta_v1` | `review_meta` | cyclic | raw exact state/version overlay |
| `cas.legacy_merge_v1` | merge executor | cyclic | exact admissible-state与 raw version overlay |
| `cas.legacy_pr_fix_reconcile_v1` | PR fix reconcile | versioned | safe-segment overlay |
| `cas.legacy_review_loop_safe_v1` | `review_loop` | dedicated safe equality | 不执行 ordering；exact current semantics |
| `cas.legacy_review_activation_handoff_v1` | `review_pr` | dedicated marker ordering | distinct preliminary `version-mismatch` + direct-ID handoff |
| `cas.legacy_implement_activation_handoff_v1` | `implement` | versioned normal path；cyclic recovery path | initial direct-ID once + later structural rechecks |

每个 consumer fixture必须冻结：

```text
incoming older/equal/newer
× source/target/projection-predecessor/missing/unrelated
× raw-or-safe match/mismatch
× idempotent complete/incomplete
× handoff absent/valid/invalid where applicable
```

只验证 happy path不构成 policy parity。

### A4.8.4 `review_pr` Distinct Version Mismatch

`review_pr` preliminary relation：

| current | comparison | preliminary |
|---|---|---|
| missing state/version | — | `pending` |
| `reviewing`, stripped versions equal | equal | `apply` |
| `reviewing`, stripped versions differ | mismatch | `version-mismatch` |
| canonical order earlier | earlier | `pending` |
| canonical order equal/later but not accepted | later/diverged | `stale` |

Verified handoff可把 preliminary `pending|version-mismatch`转成 `apply`。若 handoff失败：

- preliminary `version-mismatch` → final `status=stale`；
- `reason_code=version-mismatch`；
- `cas_outcome=skip-stale(version-mismatch)`；
- 不得塌缩成 generic advanced/diverged。

### A4.9 Legacy-Exact Handoff Resolvers

#### `handoff.reviewing_direct_id_legacy_v1`

复现当前 `payloads.predicates`：

1. 结构检查 `kind`、proposal、state、event version、marker version、stage rank与 safe comment ID；
2. direct comment-ID GET；
3. decode JSON；
4. returned comment ID若存在则必须相等；当前 response若省略 ID不得在 refactor中新增拒绝；
5. trusted bot author；
6. comment中存在 proposal/state/marker-version/stage-rank匹配的 state marker；
7. current source未要求 whole-body equality或 effects equality时，refactor不得擅自增加。

要求 exact whole-body、mandatory returned ID或 effects equality属于后续 R9 behavior change。

#### `handoff.implement_ready_once_legacy_v1`

Initial acceptance：

- 仅当 base transition为 `pending`；
- `retry_failure == nil`；
- `impl_retry_attempt == nil`；
- direct-ID resolver成功；
- 才保存 `accepted_ready_hand_off`并视为 `apply`。

Pre-spawn与 write-time gates：

- fresh读取 current state；
- 运行普通 current-state checks；
- 仅当 base仍为 `pending`时，对已保存 handoff调用 `is_ready_hand_off`结构检查；
- 不进行第二次 direct-ID lookup；
- lookup count必须保持一；
- retry/reentry不得借用 ready handoff。

`integration_implement_meta_test.lua:798` 的 one-lookup observation进入 mandatory parity corpus。

以下 tightening明确延期：

- every-gate direct-ID re-resolution；
- mandatory returned comment ID；
- exact full marker/body equality；
- extra effects equality；
- mismatch立即 stale；
- handoff TTL。

它们只能作为单独 R9 behavior-change PR。

## Annex §4.10 Pending Projection

Legality view与 pending-order view从相同 edges派生，但不得互相替代。

```lua
derive_pending_projection(edges) -> {
  [predecessor_state] = { target_states... }
}
```

Algorithm：

```text
for each edge:
  require pending_order field
  if pending_order.participates:
    require predecessor_state
    require target is a lifecycle state
    add unique pair(predecessor_state, target)

can_reach(from, to):
  normalize nil from to unmanaged
  identity is reachable
  traverse only pending pairs
```

不得按 `kind`、stage rank、target name或 source presence推断 participation。

Frozen legacy projection：

```text
unmanaged       → thinking
thinking        → dependency_wait, ready, blocked
dependency_wait → dependency_wait, ready, blocked
ready           → dependency_wait, implementing, blocked
implementing    → awaiting-pr, impl-failed
awaiting-pr     → merged, ready, blocked
pr-open         → reviewing, blocked
reviewing       → merge-ready, fixing, review-meta
merge-ready     → merging, blocked
merging         → merged, reviewing, fixing, blocked
fixing          → reviewing, review-meta
review-meta     → fixing, blocked
impl-failed     → implementing
merged          → {}
closed-unmerged → {}
blocked         → {}
```

Issue与 PR owner各自产生 local projection。唯一允许的 union位置是 composed conformance：

```text
union(issue_projection, pr_projection)
  == frozen legacy state_graph
```

该 union不得被 export、不得用于 production decision、不得由 ops组装。

## Annex §4.11 Payload Token Registry

唯一 API：

```lua
payload_registry.validate(token)
payload_registry.resolve(token, context)
```

Token grammar：

```text
marker:<family>.<attr>
source_ref:<derivation>
literal:<registered-value>
dedup:<registered-strategy>
comment_body:<registered-strategy>
typed:<registered-strategy>
```

规则：

- unknown prefix、family、attribute或 strategy：fail closed；
- known token缺 required fact：`missing-evidence`，不产生 partial effect；
- `typed:review-feedback` 与 `typed:ci-failure`必须注册 resolver或从 row删除；
- retained `payload_builder`必须有 stable exception ID与 exact parity witness；
- lifecycle payload projection发生在 grant validation之后；
- grant、owner seal与 snapshot seal绝不序列化；
- content仍只通过 `source_ref`回源，不能进入可靠 payload。

## Annex §4.12 R5 Obligation Schema

```lua
{
  obligation_id = "<stable-id>",
  owner = "<owner>",
  edge_id = "<edge-id>",
  case_kind = "edge"
            | "edge-pair"
            | "family-variant"
            | "bounded-loop"
            | "cas-matrix"
            | "pending"
            | "entitlement"
            | "timeout",

  input_fixture_id = "<fixture-id>",
  expected_decision = { ... },
  expected_effect_ids = { ... },
  expected_payload_obligations = { ... },
  witness_id = "<replay-or-real-trace-id>",
}
```

R5派生：

- typed edges；
- compatible typed edge-pairs；
- family variants；
- entry、activation、operator、timeout、guard与 canonicalization cases；
- CAS matrix cells；
- pending participation；
- generation/epoch；
- advancing facts；
- apply/idempotent entitlements；
- row replay effects；
- resolver-specific timeout cases；
- bounded-loop representative cases。

Wall-clock budget不是 path-unroll bound。Loop只使用独立小型 bound或 representative `self-loop once/release/timeout/stale-lineage` cases。

## Annex §4.13 R6 Trace Schema

```lua
{
  schema = "restart-trace.v1",
  owner = "<owner>",
  fixture_id = "<fixture-id>",
  steps = {
    {
      edge_id = "<edge-id>" | nil,
      row_replay_id = "<row-replay-id>" | nil,
      kind = "<kind>",
      source = { ... },
      target = "<state>" | nil,

      cause_evidence = { ... },
      cas_policy_id = "<policy-id>",
      cas_status = "<status>",
      reason_code = "<reason>",
      cas_outcome = "<observable-outcome>",

      pending_status = "<included|excluded|not-applicable>",
      generation_epoch = { ... },

      grant_fingerprint = "<non-secret-fingerprint>" | nil,
      effect_entitlement_id = "<entitlement-id>" | nil,
      effect_ids = { ... },

      queue = "<queue>" | nil,
      payload_obligations = { ... },
      observable_writes = { ... },
      terminal_why = { ... } | nil,
    },
  },
}
```

Trace不得包含 opaque grant本体。Expected trace来自 R5；testkit只执行 actual caller witness并记录。

## Annex §4.14 R7 Anomaly Schema and Posture

Shared analyzer API：

```lua
analyze_observed_transition_history(
  canonical_rows,
  marker_history,
  observed_evidence
) -> restart_transition_anomaly_v1[]
```

Owner-local placement：

```text
packages/github-devloop/core/restart_analysis.lua
packages/github-devloop-pr/core/restart_analysis.lua
```

Anomaly schema：

```lua
{
  schema = "restart-transition-anomaly.v1",
  owner = "<owner>",
  entity = { ... },

  observed_from = "<state>" | nil,
  observed_target = "<state>" | nil,

  edge_kind = "<kind>" | nil,
  edge_id = "<edge-id>" | nil,
  cas_policy_id = "<policy-id>" | nil,

  observed_generation = "...",
  observed_epoch = "...",

  decision_status = "<status>",
  reason_code = "<reason>",
  cause_status = "complete|incomplete|invalid|indeterminate",
  ordering_status = "complete|indeterminate",
  evidence_refs = { ... },

  disposition = "illegal-transition"
              | "ordering-indeterminate"
              | "cause-indeterminate"
              | "malformed-evidence",
}
```

Refactor posture固定为 **SHADOW / conformance-only**：analyzer只做 owner-local pure read-only computation；result只供 parity/fixtures/conformance，不进入 production mutation或 delivery path；owner/ops specs不新增 anomaly `produces`/`consumes`，ops event deps保持 current，不 emit、不 ingest、不 delivery。Evidence不足返回 indeterminate，不得搬到 ops或猜测。

Event activation只能在 Step 8后以独立 R9 behavior-change PR实施，且固定为 **ephemeral / level-triggered**：owners每 pass重算，ops以 `M.spec.ephemeral`消费两个 qualified queues，at-most-once，无 durable identity、`source_ref`、`dedup_key`或 rehydration；同 parent的多个 anomalies只是本 pass的独立 records。Transport是 `grantless-telemetry`，不得要求/携带/mint grant；new queues、ops PR dependency、ingestion与 delivery由 manifest逐项授权。

Issue A不阻塞 shadow或后续 ephemeral activation；它只补 engine-only ordering/provenance evidence。

## Annex §4.15 Resolver-Keyed Timeout Policies

Policy dispatch key：

```text
actionable_epoch.source
  + declared resolver
  + row lineage
```

不得只按 `liveness_contract.mode`选择。

| policy ID | source/resolver | truth source | legacy-exact behavior |
|---|---|---|---|
| `timeout.state_entry_legacy_v1` | `state_entry:v1` | state entry/actionable epoch | row-budget behavior与 progress rules按 OLD observations保持 |
| `timeout.codex_run_legacy_v1` | `codex_run:v1` | `fkst.codex_runs` | running→defer；not-live→actionable；indeterminate在 row budget内 defer、耗尽后 actionable |
| `timeout.heartbeat_legacy_v1` | `live_defer_heartbeat:v1` | declared heartbeat marker | fresh→defer；stale/missing按现有 heartbeat epoch与 redrive/escalate规则 |
| `timeout.durable_clear_legacy_v1` | `live_defer_epoch:v1` | hold/observed/clear facts | fresh hold→defer；clear/stale marker打开 exact actionable epoch；证据缺失→contract invalid |
| `timeout.child_workflow_legacy_v1` | `child_workflow_wait:v1` | delegated child state | nonterminal child→defer；terminal/absent按 current resolver进入 actionable/wait/escalate |
| `timeout.instance_cas_legacy_v1` | all timeout edges | scheduled/current lineage | mismatch→stale timeout noop；matching lineage才继续 |

Matrices：

| resolver result | before declared deadline/budget | at/after deadline/budget |
|---|---|---|
| codex running | defer，无 timeout grant | defer，无 timeout grant |
| codex not running | legacy redrive/actionable path | legacy escalation path |
| codex indeterminate | defer | actionable/escalation |
| heartbeat fresh | defer | 仍按 heartbeat truth，不要求 process state |
| heartbeat stale/missing | legacy redrive/wait | legacy escalation |
| dependency hold fresh | defer | 按 durable-clear resolver，不要求 process state |
| dependency clear visible | fresh actionable epoch | legacy escalation only when its budget expires |
| child nonterminal | defer | defer |
| child terminal/absent | legacy wait/actionable | legacy escalation |
| lineage mismatch | stale no-op | stale no-op |

Only `codex_run:v1` may use positive process liveness.以下均是 behavior changes，refactor中禁止：

- 对 heartbeat row要求 positive process-not-running；
- 把 dependency或 child state降级成 marker age；
- resolver mismatch直接 stale的新规则；
- 修改 redrive/attempt顺序；
- 修改 actionable epoch起点。

## Annex §4.16 R10 Temporal Schema and Capable Providers

Temporal obligations直接 authored在 canonical row：

```lua
temporal_obligations = {
  {
    id = "<owner-local-id>",
    pattern = "absence"
            | "precedence.structural"
            | "precedence.gate-fact"
            | "response-with-deadline",
    scope = { ... },
    antecedent = { ... },
    consequent = { ... },
    deadline_source = "<row-liveness-field>" | nil,
  },
}
```

`owner`与 `row_id`从 containing row派生；`verification`由 `derive_temporal_index(canonical_rows, provider_capabilities)`派生。Owner-local indexes只放在：

```text
packages/github-devloop/core/restart/temporal_obligations/index.lua
packages/github-devloop-pr/core/restart/temporal_obligations/index.lua
```

`index.lua`只调用 derivation，不列 ID、row或 override；derived key set须与 row-authored IDs exact相等，duplicate/orphan/missing/drift fail。

Issue rows同时迁入 `packages/github-devloop/core/restart/transitions/{index,<row>}.lua`，删除 shared row/provider modules及 exports。Ops doctor/state-gap不得读取 owner rows、R10 index或 provider bindings；issue owner只发布从 rows派生的 legacy-exact narrow projection：

```lua
{
  schema = "restart-owner-observation-facts.v1",
  owner = "github-devloop",
  source_rows_fingerprint = "...",
  states = {
    ["<state>"] = {
      from_state = "<state>", terminal = true | false,
      driving_queue = "<queue-or-none>", budget = { minutes = 0 } | nil,
    },
  },
}
```

该 owner-generated、schema-checked、grantless read-only value seam只能暴露 doctor/state-gap当前消费字段；不得新增 event/queue/delivery/marker/write，也不得成为 legality authority。Ops切换后以 frozen fixtures证明 doctor与 state-gap outputs逐字段等于 OLD；fingerprint mismatch fail closed，不回退 shared rows。

Provider capability matrix：

| pattern | capable monitored provider | incapable substitute |
|---|---|---|
| `precedence.structural` | R7 owner transition-history analyzer | timeout evidence |
| `precedence.gate-fact` | R7 owner analyzer with observed gate facts | stage names alone |
| `response-with-deadline` | R8 resolver-keyed watchdog/timeout evidence | R7 edge legality alone |
| `absence` | explicit emission ledger + declared producer + bounded observation window | missing marker、R7 transition relation或 silent logs |

Admission：obligation ID必须 owner-local unique；index/provider binding必须完全可重建且 owner/scope匹配；incapable provider记录 `unmonitored`/`indeterminate`并 fail，不得降级 warning，也不得新增 parallel timer/evaluator。

## Annex §5. Row Admission Mechanics

每条 row/edge必须通过：

1. owner/provenance；
2. unique ID；
3. explicit CAS policy；
4. explicit pending participation；
5. typed evidence resolver；
6. apply entitlement；
7. idempotent entitlement；
8. row replay separation；
9. resolver-keyed timeout policy；
10. named R5/R6 witness；
11. R9 normalization；
12. terminal deletion path。

Autonomous normalization：

```text
responsibility_signature.successors
  → autonomous rich edges
  → derived to_states
```

`output_obligation.exits`必须覆盖每个 autonomous successor。额外 hand-authored `to_states`、missing exit或 duplicate successor均失败。

Fanout按：

```lua
family = {
  class = "success" | "failure",
  id = "<postcondition-family>",
}
```

计数。

`thinking` normalization必须表达：

- `ready`：`success/issue-consensus` variant；
- `dependency_wait`：同一个 `success/issue-consensus` family的另一 variant；
- `blocked`：failure family。

这修复 row缺失 `thinking → dependency_wait` 的结构差异，但只有 OLD branch observation证明后才能 admission。

## Annex §8. Migration Mechanics

### Step 0.0 — Bootstrap Prerequisites

#### Step 0.0.0 — Line-Budget Containment

以下 current files已验证达到 soft split threshold：

| file | verified lines |
|---|---:|
| `libraries/devloop/replayer.lua` | 990 |
| `packages/github-devloop-ops/tests/integration_observability_test.lua` | 989 |
| `packages/github-devloop-pr/departments/fix/main.lua` | 930 |
| `packages/github-devloop-pr/core/merge_executor.lua` | 904 |
| `scripts/run.sh` | 980 |

按稳定职责拆分，所有行为测试保持不变。该 PR不得引入 semantic migration。

#### Step 0.0.1 — Independent OLD Observation and Full Inventory

新增：

```text
scripts/check_repo_restart_lifecycle.py
migration/restart-lifecycle.inventory.json
migration/restart-lifecycle.allowlist
```

Observation schema：

```lua
{
  schema = "restart-old-behavior-observation.v2",
  observation_id = "<stable-site-case-id>",

  owner = "<package>",
  site = {
    path = "<path>",
    symbol = "<symbol>",
    ordinal = "<stable-callsite-id>",
  },

  boundary = "writer"
           | "effect_sink"
           | "receiver_activation"
           | "entry_acceptor"
           | "published_intent_producer"
           | "row_replay"
           | "shared_row_export"
           | "owner_observation_fact"
           | "observation_fact_reader",

  typed_intent = {
    kind = "<observed-kind>",
    source_state = "<state>" | nil,
    source_boundary = "<boundary>" | nil,
    target = "<state>" | nil,
    cause_schema_id = "<old-observed-cause>",
    generation_epoch = { ... },
    lineage = { ... },
  },

  old_inputs = {
    current_fact = { ... },
    caller_from_states = { ... },
    incoming_version = "...",
    target_version = "...",
    handoff_reference = { ... } | nil,
  },

  old_outcome = {
    status = "<status>",
    reason_code = "<reason>",
    cas_outcome = "<observable-outcome>",

    emitted_effects = {
      {
        effect_id = "<stable-effect-id>",
        sink_kind = "<queue|comment|label|adapter|codex|git|merge>",
        authority_class = "<lifecycle-authoritative|grantless-published-intent|grantless-telemetry|grantless-non-lifecycle>",
        ordinal = 1,
      },
    },

    observable_writes = { ... },
    handoff_direct_lookup_count = 0,
    timeout_evidence_source = "<source>" | nil,
  },

  evidence_refs = { ... },
}
```

独立性：

- OLD producer不得 import NEW extractor、NEW policy、NEW entitlement或 NEW edge map；
- record不含 NEW `edge_id`；
- actual old execution必须捕获 outputs与 sink calls；
- `thinking → dependency_wait` identity来自 existing branch，不从 NEW row反推；
- public seam producer被记录为 untrusted intent，不要求 grant；
- R7 telemetry被记录为 grantless且在 refactor中无 emitted sink；
- 每个 lifecycle-authoritative sink必须 observed并映射 entitlement/grant；每个 grantless sink必须显式分类，否则标 `unobserved` blocker；
- shared issue row exports、ops row readers与 owner observation fact consumers必须全部入 inventory。

Generated inventory包含：

```text
schema/version
source_tree
old_behavior_observations
old_pending_projection
production_writer_sites
effect_sink_sites
row_replay_sites
published_intent_sites
receiver_activation_acceptors
consumer_entry_acceptors
direct_constructor_sites
shared_issue_row_exports
ops_issue_row_reader_sites
owner_observation_fact_sites
grantless_sink_sites
unobserved_sites
watched_files
artifact_sha256
```

#### Step 0.0.2 — Shared Pure Machinery

新增：

```text
libraries/devloop/restart_edges.lua
libraries/devloop/restart_cas_policies.lua
libraries/devloop/restart_analysis.lua
libraries/devloop/restart_effect_entitlements.lua
libraries/devloop/restart_payload_registry.lua
```

此阶段：

- extractor可 report missing fields；
- closed legacy-exact policies有 total unit tests；
- pending projection可 shadow生成；
- effect inventory可生成 report；
- analysis relation不 mint grant；
- 不替换 production caller；
- 不改变 behavior。

不得新增 `libraries/devloop/restart_decider.lua`。

#### Step 0.0.3 — Owner-Local Grant-Disabled Shadow Deciders

新增：

```text
packages/github-devloop/core/restart_authority.lua
packages/github-devloop-pr/core/restart_authority.lua
```

Shadow mode：

- package-local wiring绑定 canonical rows；
- owner module创建 private seal；
- 完整运行 NEW decision；
- `grant=nil`；
- 记录 shadow result；
- 不构造 lifecycle effect；
- 不改变 live behavior。

### Step 0.1 — Symmetric Edge/CAS/Effect/Pending Parity

定义：

- `A`：independent OLD observations；
- `B`：NEW extractor、legacy-exact policies与 grant-disabled owner shadow deciders。

比较前不得从 B取得 `edge_id`补写 A。

必须满足：

```text
semantic(A) - semantic(B) = ∅
semantic(B) - semantic(A) = ∅
```

Semantic key至少包括：

```text
owner
kind
source state/boundary
target
cause schema
generation/epoch
lineage
ordered CAS behavior
status/reason/cas_outcome
apply/idempotent effect IDs
row replay effect IDs
observable effect order
handoff lookup count
timeout evidence source
pending participation
```

本步完成：

- `thinking → dependency_wait` normalization；
- source-less ingress；
- receiver activations；
- operator reentry；
- timeout edges；
- guard boundaries；
- canonicalizations；
- apply/idempotent entitlements；
- row replay separation；
- explicit pending participation；
- issue/PR conformance-only projection union。

Parity gates：

```text
union(issue_pending_projection, pr_pending_projection)
  == frozen OLD state_graph
```

每个 CAS consumer运行完整 older/equal/newer matrix。`review_pr`的 `version-mismatch`与 implement one-lookup handoff均为 equality boundary。

Intentional delta不得进入 bootstrap allowlist。

### Step 0.2 — Cycle Cut

新增：

```text
libraries/devloop/restart_metadata.lua
```

把 labels、orders、stage ranks与 pure version metadata从 `devloop.state`剥离。

要求：

- extractor与 policies读取 metadata；
- owner row assembly不反向依赖 lifecycle builders；
- OLD authority仍运行；
- parity持续为零 diff。

### Step 0.3 — Owner Seals and Grant-Consuming Sink Shadowing

新增：

```text
packages/github-devloop/core/restart_effects.lua
packages/github-devloop-pr/core/restart_effects.lua
```

执行：

- owner-local grant seal启用；
- owner-local private builder verifier注入 effect facade；
- raw serializers移到 private construction path后；
- every sink instrumentation进入 report/shrink mode并记录 authority classification；
- old writers继续运行；
- NEW lifecycle-authoritative builders无 grant时拒绝；
- published intent与 telemetry保持 grantless且无法 mint；
- shared analysis仍无法 mint；
- valid OLD path与 shadow NEW effect set保持 equality。

不得用未来 R9 artifact为本步自证。

### Step 0.4 — Protected-Base R9 Active

在以下动作前启用 Annex §9：

- 替换 production caller；
- 删除 OLD authority；
- 修改 row semantics；
- 修改 CAS或 timeout policy；
- 修改 handoff lookup behavior；
- 修改 payload resolution；
- 修改 effect entitlement；
- 启用 grant-consuming effect；
- 删除 duplicate graph/helper/DI export。

### Step 1 — Per-Family Shadow → Swap → Shrink

一次迁移一个 row family：

1. owner lock内 fresh re-derive；
2. seal snapshot；
3. 构造 typed intent；
4. 调用 bound owner decider；
5. lifecycle-authoritative effect facade只消费 exact grant；published intent与 telemetry不进入该 facade；
6. external write gate fresh读取 current facts；
7. implement legacy handoff只做 initial direct-ID一次，后续结构重检；
8. R9 exact equality；
9. shrink writer、CAS、constructor、sink、DI与 bypass debt。

建议顺序：

```text
ready/dependency_wait
→ thinking
→ implementing/impl-failed
→ awaiting-pr
→ pr-open/reviewing
→ fixing/review-meta
→ merge-ready/merging/merged
→ timeout/operator/canonicalization
```

### Step 2 — Branch Routing

Branch code只选择 typed semantic variant。CAS policy与 entitlement由 edge决定。

删除：

- caller predecessor authority；
- caller policy选择；
- post-decision version override；
- post-decision handoff override；
- direct lifecycle-authoritative effect construction。

### Step 3 — Typed Guard Consolidation

仅在 2–3 个重复 branch shape被 Step 2证明后引入 minimal typed guard table。其余保留 named pure capabilities。

不得引入 expression evaluator。

### Step 4 — Payload Consolidation

- 启用 single registry；
- unknown/missing fail closed；
- 迁移 expressible builders；
- retained builders提供 exact parity witness；
- grant validation先于 lifecycle payload construction；
- content继续 source rehydration。

### Step 5 — R5/R6

- typed obligations；
- complete CAS matrices；
- effect entitlement cases；
- resolver-specific timeout cases；
- actual-vs-expected trace；
- 若现有 harness实证不足，file Issue B。

### Step 6 — R10

- Issue rows迁入 owner-local path并切换 wiring；canonical issue/PR rows author temporal obligation IDs/bodies；
- owner-local index/provider links只从 rows与 capability matrix派生，orphan/drift fail；
- owner派生 legacy-exact narrow observation facts，ops doctor/state-gap切换后 outputs逐字段等于 OLD；
- 删除 shared issue rows/provider exports、manifest entries及 ops readers/tests；
- unsupported absence fail；不增加 timer、event、delivery或 second monitor。

### Step 7 — R7

- Issue/PR analyzers各自绑定 own rows，只运行 shadow/conformance pure read-only computation；
- 覆盖 illegal、malformed与 honest indeterminate；no mutation、no production row union；
- refactor内不 emit、不增加 ops PR dependency、不 ingest、不 delivery，board/ops behavior保持 OLD exact；
- Issue A只补 engine-only provenance；ephemeral transport推迟到 Step 8后的独立 behavior-change PR。

### Step 8 — Terminal Deletion

全部满足才允许删除 OLD authority：

- inventory零 `unobserved`/`unmapped`，每个 writer、acceptor、activation、row replay与 sink唯一分类；
- 每个 edge有 closed apply/idempotent entitlement，row replay与 successor effects不混淆；
- 每个 lifecycle-authoritative marker/comment、label、queue、codex、git、merge sink消费 exact grant；intent/telemetry显式 grantless且不能 mint；
- whole-head lifecycle-authority bypass为零；production/DI不暴露 seal、grant factory、private verifier、raw serializer或 OLD authorities；shared library无 decider factory；
- shared issue rows/provider exports与 ops row/index readers为零；owner observation facts保持 legacy-exact output parity；
- edge/CAS/effect、handoff lookup、timeout source及 pending-union parity全部通过；
- R7 shadow/schema通过且 head无 anomaly event/new ops PR dependency/ingestion/delivery；
- R10 obligations row-authored，owner-local derived index无 orphan/drift且 monitored provider capable；
- R11 known-dialogue inventory相对 protected baseline零增长；现有 consensus message surface仍在 shrink-only allowlist中，terminal deletion本身不删除其 queue/delivery、不激活 zero-surface；
- mandatory corpus/controls通过；behavior-change manifest不得豁免 refactor diff。

### Rollback

只支持 reverse-topological rollback bundle：

1. 恢复被删除的 OLD exports/helpers/authority；
2. 逆序恢复 dependent caller swaps与 allowlist entries；
3. 恢复 prior owner wiring；
4. restart/re-derive，让 durable与 external facts驱动恢复。

禁止：

- 手改 marker、durable或 runtime program-state；
- isolated revert一个依赖已删除 helper的历史 PR；
- 用 temporary caller policy或 grant bypass止血；
- 把后续 behavior-change rollback混入 refactor rollback。

## Annex §9. R9 Semantic Oracle

### 9.1 Scope

R9覆盖：

- lifecycle caller swap；
- receiver acceptance swap；
- row/extractor/CAS/pending changes；
- lifecycle-authoritative effect entitlement与 grant path；
- grantless intent/telemetry classification；
- row replay normalization；
- timeout resolver policy；
- handoff behavior；
- payload semantics；
- duplicate authority deletion；
- anomaly shadow/temporal semantic mapping；
- issue-row owner-local migration、shared export removal与 owner observation fact output parity。

纯 file split、report-only inventory与 unused grant-disabled helper可免。

R7 anomaly event emission、ops PR dependency、ingestion与 delivery不在 refactor equality内；它们必须在 terminal deletion之后以独立 behavior-change manifest授权。Refactor head中这些新增 observables必须为零。

### 9.2 Protected Inputs

Checker-owned inputs：

```text
scripts/check_repo_intent_bounded_replay.py
scripts/intent_bounded_replay/corpus_manifest.json
scripts/intent_bounded_replay/compare.py
scripts/intent_bounded_replay/normalize.py
```

另包括：

- protected merge-base inventory；
- OLD observation corpus；
- mandatory fixtures；
- whole-head-tree scan rules；
- canonical hash implementation。

Semantic PR必须使用 protected merge-base版本。修改 checker、normalizer、comparator、mandatory corpus或 hashing必须先走独立 precursor PR，不得与 lifecycle semantics同窗。

### 9.3 Canonical JSON and Hashing

```text
canonical_artifact_hash_v1(artifact):
  remove exactly artifact's own self-hash field
  canonicalize remaining content as deterministic UTF-8 JSON
  SHA-256
```

Canonical JSON：

- object keys按 bytes排序；
- arrays保序；
- UTF-8；
- no insignificant whitespace；
- no duplicate keys；
- normalized JSON numbers；
- schema/version included。

Self-hash fields：

```text
artifact_sha256
manifest_sha256
attestation_sha256
```

计算时只省略当前 artifact自己的 self-hash field。引用其他 artifact的 hashes不得省略。

该规则适用于：

- lifecycle inventory；
- OLD observation corpus；
- fixture manifest；
- normalized trace；
- behavior-change manifest；
- CI attestation。

### 9.4 OLD-vs-NEW Equality

OLD：

- protected merge-base semantics；
- independent observations；
- protected comparator/normalizer/corpus；
- actual accepted boundaries与 sinks；
- OLD `state_graph`仅供 pending projection。

NEW：

- PR head semantics；
- canonical extractor；
- legacy-exact policies；
- owner-bound decider；
- grant-consuming effects；
- same protected comparator/normalizer/corpus。

Envelope：

```text
base_sha
observed_head_sha
inventory_sha256
old_observation_sha256
corpus_manifest_sha256
fixture_set_sha256
comparator_sha256
normalizer_sha256
old_trace_sha256
new_trace_sha256
behavior_diff_sha256
```

Equality boundary：

```text
typed edge identity
ordered CAS matrix
status/reason/cas_outcome
pending projection
generation/epoch
handoff lookup count
timeout resolver/source/decision
apply/idempotent entitlement
row replay effects
effect order
queue/payload/dedup
marker/label/comment writes
codex/git/merge calls
terminal WHY
package-visible delivery expectations
anomaly event/ops dependency/ingestion/delivery absence during refactor
known request-reply message inventory no-growth during refactor
```

Opaque grant本身是新内部结构；比较的是 grant授权的 existing lifecycle-authoritative effects是否 exact，不允许借 grant增加或删除行为。Published intent与 R7 telemetry以 grantless classification校验。R7 shadow result可在非 observable conformance channel比较，但 OLD/NEW production都必须保持零 anomaly event raise、零新增 ops PR dependency、零 ingestion与零 package-visible anomaly delivery。

### 9.5 Separate Behavior-Change Manifest

后续 intentional behavior change使用：

```text
migration/intent-diffs/<pr-number>.json
```

Refactor PR不得使用它。

Manifest：

```json
{
  "schema": "fkst.intent-diff.v2",
  "intent": "behavior-change",
  "pr_number": 1234,
  "base_sha": "<protected-merge-base-sha>",

  "semantic_tree_sha256": "<canonical-head-semantic-tree-hash>",
  "semantic_diff_sha256": "<canonical-base-to-head-semantic-diff-hash>",

  "changed_row_ids": ["..."],
  "changed_edge_ids": ["..."],
  "changed_policy_ids": ["..."],

  "old_trace_sha256": "<...>",
  "new_trace_sha256": "<...>",
  "behavior_diff_sha256": "<...>",

  "cause": "<bounded-reason>",
  "review_reference": "<review-reference>",
  "one_use_identity": "<pr/base/semantic-hashes>",

  "manifest_sha256": "<hash-with-this-field-omitted>"
}
```

Manifest不得包含 authoritative exact `head_sha`或 whole-diff self-binding。

Fixed exclusions：

```text
migration/intent-diffs/<actual-pr-number>.json
generated CI attestations outside tracked tree
```

`semantic_tree_sha256`：

1. enumerate tracked head paths；
2. fixed exclusions；
3. length-framed path、mode、blob SHA-256；
4. sort by path bytes；
5. domain separator `fkst-semantic-tree-v1`；
6. hash record stream。

`semantic_diff_sha256`：

1. enumerate base→head tracked changes；
2. fixed exclusions；
3. rename→delete+add；
4. record status、old/new paths、modes、blob hashes；
5. canonical tuple sort；
6. domain separator `fkst-semantic-diff-v1`；
7. hash stream。

R7 anomaly transport activation是保留的 post-terminal behavior change。其独立 manifest必须逐项列出新 qualified queues、ops `github-devloop-pr` dependency、ephemeral `consumes`、ingestion与预期 package-visible delivery delta，并证明：Step 8已完成；`M.spec.ephemeral`覆盖两个 anomaly queues；无 `source_ref`/`dedup_key`/durable identity；transport为 `grantless-telemetry`且无 minting path。该 PR不得与 refactor或其他 behavior change合并。

R11 zero-surface activation同样是保留的 post-terminal behavior change。其独立 manifest必须证明：Step 8已完成；protected-base与 pre-migration known-dialogue inventory一致；`consensus` package→library迁移删除列明的 request-reply queues/deliveries并把 `migration/request-reply-message.allowlist` ratchet到 zero；`product-outcome parity`逐 fixture成立，即迁移前后 reply value、saga transitions与 markers相同。Queue→call delivery form是该 manifest唯一授权的 communication-form delta；该 PR不得与 lifecycle refactor或其他 behavior change合并。

### 9.6 CI Attestation

```json
{
  "schema": "fkst.intent-diff-attestation.v1",
  "pr_number": 1234,
  "base_sha": "<actual-protected-base>",
  "head_sha": "<actual-ci-head>",

  "manifest_path": "migration/intent-diffs/1234.json",
  "manifest_blob_sha256": "<exact-blob-sha256>",
  "manifest_sha256": "<validated-self-hash>",

  "semantic_tree_sha256": "<recomputed>",
  "semantic_diff_sha256": "<recomputed>",
  "old_trace_sha256": "<recomputed>",
  "new_trace_sha256": "<recomputed>",
  "behavior_diff_sha256": "<recomputed>",

  "result": "approved",
  "attestation_sha256": "<hash-with-this-field-omitted>"
}
```

CI从 actual PR context取得 PR/head并重新计算全部 hashes。任一 mismatch fail closed；head变化使旧 attestation失效。

### 9.7 Preflight

Protected-base scanner扫描完整 head tree，检测：

- new/renamed writer；
- new effect sink；
- new acceptor；
- raw constructor；
- policy；
- lifecycle queue；
- grant factory exposure；
- owner seal exposure；
- tracked attestation；
- exclusion abuse；
- checker与 semantic co-change；
- unlisted caller；
- new shared `make_decider`；
- published intent producer grant requirement；
- telemetry grant requirement或 minting path；
- refactor内 anomaly event/ops PR dependency/ingestion/delivery activation；
- shared issue row/index/provider export；
- ops direct或 library-mediated issue row/R10 index reader；
- production cross-owner row union。

## Annex §10. Global Conformance

Public lifecycle ratchet：

```text
scripts/check_repo_restart_lifecycle.py
migration/restart-lifecycle.inventory.json
migration/restart-lifecycle.allowlist
```

职责：

- independent OLD observation coverage；
- typed symmetric parity；
- ordered CAS matrices；
- result algebra；
- pending union parity；
- writer/acceptor/sink inventory；
- apply/idempotent entitlements；
- row replay separation；
- lifecycle-authoritative owner grant mapping与 grantless intent/telemetry classification；
- whole-head bypass；
- owner seal boundaries；
- published intent trust boundary；
- payload registry；
- timeout resolver mapping；
- R5/R6；
- R7 owner-local shadow analysis、honest indeterminacy与 deferred-activation absence；
- R10 row-authored obligations、owner-local derived indexes、shared-export/ops-reader removal、legacy-exact observation facts与 capable providers；
- terminal deletion。

Behavior oracle：

```text
scripts/check_repo_intent_bounded_replay.py
migration/intent-bounded-replay.allowlist
migration/intent-diffs/
```

R11 known-dialogue ratchet（**`DESIGNED, NOT YET ENFORCED`**）：

```text
scripts/check_repo_fanout_only.py
migration/request-reply-message.allowlist
```

职责：

- inventory已知 request-reply message surfaces，包括 request queue、reply queues、reply consumers与仅为该 dialogue存在的 `[event_deps]`；
- refactor phase以 protected baseline校验 no-growth：allowlist机制结构上只许 shrink，但 R9要求现有 consensus entries原样保留，故本阶段不删除 queue/delivery；
- post-terminal仅接受独立 R9 behavior-change manifest + product-outcome parity，随后把 inventory与 allowlist ratchet到 zero并禁止再引入；
- checker controls必须命中 `origin-keyed benign skip` positive fixture，同时不得把 schema rejection、`event_id` dedup、entity rehydration或 legal same-entity post-admission CAS误报为 request-reply；
- 不从 `output_obligation`、lineage或字段名推导 audience semantics，不宣称 general requester-provenance detector；
- checker PR必须独立于本 doctrine PR落地，并在落地时配套 tests、接入 `scripts/check_repo_runner.py`，从 `scripts/check_repo.py`与 `scripts/run.sh check`可达。上述 wiring完成前，R11不属于现行 CI enforcement。

每个已落地 checker，以及 R11 checker的独立 checker PR，必须：

- wired into `scripts/check_repo_runner.py`；
- reachable from `scripts/check_repo.py`；
- reachable from `scripts/run.sh check`；
- paired with tests；
- fail closed；
- 遵守 line budget；
- 不实现第二份 Lua legality、writer list、effect list或 pending graph。

---

# APPENDIX A. Generated Lifecycle Migration Inventory

## A.1 Normative Artifact

唯一 authoritative inventory：

```text
migration/restart-lifecycle.inventory.json
```

每个 lifecycle writer、acceptor、activation、row replay与 effect sink记录：

```text
source location
old_observation_id
old typed intent
canonical edge_id or row_replay_id
edge kind
CAS policy
pending participation
apply entitlement
idempotent entitlement
effect IDs
authority classification
timeout evidence policy
grant consumer (`none` for classified grantless intent/telemetry)
fixture
migration status
bypass status
rollback node
```

Terminal deletion只查询该 artifact，不查询 prose。

## A.2 Temporary Bootstrap Seed

Generated artifact提交后必须删除本 seed。Seed存在期间 terminal deletion禁止。

| site | required observation |
|---|---|
| `consensus_result/main.lua:39-120,169-207` | thinking successors；ordered versioned CAS；comment/label effects；idempotent repair |
| `requests/lifecycle.lua:33-67` | result comment内 state marker；grant-consuming construction |
| `thinking.lua:77-92` | row replay `consensus.proposal`，不得复制到 successor |
| `ready_split.lua:69-85,238-299` | dependency hold/release/canonicalization |
| `execute_start/main.lua:13-20,50-68` | published intent acceptance与 owner-emitted thinking effects |
| `intake_judge/main.lua:21-29,66-89` | grantless untrusted seam producer |
| `workflow_select/main.lua:4-13`; `workflow_select.lua:475-499` | grantless untrusted seam producer |
| `implement/transitions.lua:34-55` | versioned normal path与 cyclic recovery |
| `implement/main.lua:406-505,700-748` | initial direct-ID once；later structural rechecks |
| `implement/main.lua:228` | codex dispatch sink |
| `implement/main.lua:113` | git push sink |
| `implement/main.lua:605` | ready→dependency_wait |
| `observe_issue/main.lua:602,648-694,744-775` | awaiting-pr replay、canonicalization、entry |
| `pr_delegation.lua:120,160` | PR delegation/awaiting-pr writers |
| `awaiting_pr_replayer.lua:123-151,173-224,274-300` | parent-child transitions与 row replay |
| issue `reconcile/main.lua:118-151,183-281` | convergence/timeout to blocked |
| issue `loop/main.lua:75-80` | plain CAS |
| `replay_thinking_convergence.lua:43-51` | thinking replay |
| `observe_pr/main.lua:190-258,397-431` | operator rereview与 base self-heal |
| `observe_pr/main.lua:435-485,550-613` | pr-open blocked/reviewing；versioned+raw overlay |
| `pr_review_replayer.lua:614-745` | PR-local canonical replay |
| `review_pr/main.lua:28-49,85-124,150-173` | distinct version-mismatch；direct-ID review handoff；proposal effect |
| `review_loop/main.lua:65-83,141-204` | safe equality without order |
| `review_result/main.lua:175-227` | cyclic then safe overlay |
| `fix/main.lua:466,651,707-750` | codex、push、cyclic then raw overlay |
| `review_meta/main.lua:64-86,200-215` | codex、cyclic then raw overlay |
| PR `reconcile/main.lua:176-207,249-281,330-445` | versioned then safe/raw overlays；resolver-keyed timeout |
| `merge_executor.lua:91-163,284-319,357-399,618` | merge guard、ordered CAS、irreversible merge sink |
| `reviewing.lua:20-62` | heartbeat resolver，不得替换为 process liveness |
| `dependency_wait.lua:13-45` | durable hold/release resolver |
| `awaiting_pr.lua:13-56` | child-workflow resolver |
| `restart_actionable_epoch.lua:181-345,348-517` | resolver dispatch与 legacy decisions |
| `state.lua:14,45,483-572,788` | duplicate graph/CAS exports；terminal deletion |
| `workflow/registry.lua:59-90` | no owner seal |
| owner `core/devloop_wiring.lua` files | owner-local canonical assembly；issue wiring切离 shared `issue_lifecycle` |
| `libraries/devloop/restart/issue_lifecycle.lua:3-80`; `libraries/devloop/fkst.toml:102-112` | shared issue row/provider exports迁移并删除 |
| `github-devloop-ops/core/doctor.lua:10,135`; `core/state_gap.lua:5,179-191` | shared row readers切换为 legacy-exact owner observation facts，outputs不变 |
| `github-devloop-ops/fkst.toml:8-12` | refactor中保持 current deps；PR dependency推迟到 post-terminal behavior-change |
| `observability/main.lua:84-87` | current discarded return保持；refactor不得替换为 anomaly event transport |
| all lifecycle request builders/replayers | payload parity、raw serializer与 grant inventory |

Blanket `operator`、`replay`、`observability`或 `legacy` exemptions禁止。

---

# APPENDIX B. Substrate Issue Bodies

## Issue A

**Title:** Expose a generic read-only live transition and delivery timeline ledger

本 refactor中 Issue/PR owners只运行 R7 shadow analysis，不发布 anomaly events。Terminal deletion后的独立 behavior-change PR可激活 ephemeral owner anomaly transport；完整 runtime provenance仍可能包含只有 substrate持有的 durable delivery、consumer与 ordering facts。

Requirements：

- generic observe JSON/API；
- stable entity/correlation、owner、queue、consumer、delivery、timestamp、outcome与 lineage；
- producer/schema identity；
- crash-only与 at-least-once semantics；
- read-only；
- project-agnostic；
- 可供 package owner关联其 own anomaly；
- 可作为未来 explicit emission ledger provider。

Package consumers：

- issue owner analyzer；
- PR owner analyzer；
- ops aggregator；
- board renderer；
- R10 absence provider，仅在 ledger capability实际存在后。

Non-goals：

- no GitHub lifecycle semantics in substrate；
- no AssertLegal in Rust/Python；
- no owner row assembly；
- no circuit breaker；
- no mutation；
- no graph registry；
- no activation of anomaly transport inside the refactor；
- no replacement of post-terminal owner anomaly seams。

Acceptance：

- owner anomalies可关联 generic timeline；
- ordering不足时仍诚实 `indeterminate`；
- engine ledger crash不影响 data path；
- absence provider只有在 declared producer与 bounded ledger query均存在时才可标 capable。

## Issue B

**Title:** Provide a hermetic run-to-quiescence TestRuntime for package trace conformance

仅当 R6证明现有 harness不足时实施。

Requirements：

- load real package graph；
- hermetic approximation of router/durable semantics；
- bounded run-to-quiescence；
- fake external ports；
- trace delivered queue、consumer、raised effect、retry/DLQ与 terminal；
- deterministic fixtures；
- package-neutral API。

Non-goals：

- no package-side router；
- no production monitor；
- no graph registry；
- no GitHub lifecycle semantics；
- no grant minting；
- no owner row union for production。

Acceptance：

- expected trace来自 R5；
- TestRuntime只执行与记录；
- actual trace可逐步比较；
- grant fingerprint可记录但 grant不可伪造。

---

# APPENDIX C. Frozen Corpus and Negative Controls

## C.1 Mandatory Base Corpus

### Lifecycle and CAS

- `normal_issue_path`
- `dependency_wait_path`
- `thinking_dependency_wait_old_observation`
- `execute_start_untrusted_intent`
- `execute_start_owner_acceptance_grant`
- `unmanaged_entry_thinking`
- `pr_open_blocked`
- `pr_open_reviewing`
- `pr_approve_merge_ready`
- `pr_reject_fixing`
- `fixing_reviewing`
- `review_meta_fix`
- `review_meta_block`
- `merge_ready_merging`
- `merged_finalization`
- `terminal_with_WHY`
- `stale_lineage`
- per-consumer `older/equal/newer × source/target/missing/unrelated`
- `versioned_newer_preserves_plain_relation`
- `cyclic_newer_pending`
- `cyclic_older_stale`
- `cyclic_equal_stage_fallback`
- `review_result_safe_overlay_after_cyclic`
- `fix_raw_overlay_after_cyclic`
- `review_meta_raw_overlay_after_cyclic`
- `merge_raw_overlay_after_cyclic`
- `observe_pr_raw_overlay_after_versioned`
- `review_pr_version_mismatch_distinct`
- `safe_segment_equality_without_ordering`
- `versions_equivalent_observed_forms`

### Handoff

- `reviewing_activation_direct_marker`
- `reviewing_activation_verified_handoff`
- `reviewing_activation_version_mismatch`
- `implement_ready_verified_handoff`
- `implement_handoff_direct_lookup_once`
- `implement_handoff_structural_pre_spawn`
- `implement_handoff_structural_write_gate`
- `implement_handoff_rejected_for_retry`
- `implement_handoff_missing_returned_id_legacy_behavior`
- `implement_handoff_alternate_effects_legacy_behavior`

### Effect Authority

- `row_replay_thinking_consensus_proposal`
- `transition_effects_consensus_result`
- `row_replay_not_successor_entitlement`
- `apply_comment_label_queue_entitlements`
- `idempotent_missing_result_repair`
- `idempotent_complete_result_noop`
- `idempotent_exact_target_version_marker_replay`
- `codex_dispatch_implement_grant`
- `codex_dispatch_fix_grant`
- `codex_dispatch_review_meta_grant`
- `git_push_implement_grant`
- `git_push_fix_grant`
- `verified_merge_grant`
- `published_intent_requires_no_producer_grant`

### Timeout and Liveness

- `ready_state_entry_budget`
- `merge_ready_state_entry_budget`
- `merging_state_entry_budget`
- `codex_run_live_defers`
- `codex_run_not_live_redrive`
- `codex_run_indeterminate_under_budget`
- `codex_run_indeterminate_cap`
- `heartbeat_fresh_defers`
- `heartbeat_stale_redrive`
- `heartbeat_missing_legacy_decision`
- `dependency_hold_defers`
- `dependency_release_opens_epoch`
- `dependency_evidence_missing_invalid`
- `child_workflow_nonterminal_defers`
- `child_workflow_terminal_actionable`
- `timeout_lineage_mismatch_noop`

### Pending and R7/R10

- `pending_projection_old_graph_exact`
- `issue_pr_projection_union_only_in_conformance`
- `legal_edge_excluded_from_pending_projection`
- `duplicate_marker_history`
- `out_of_order_marker_history`
- `cause_indeterminate`
- `ordering_indeterminate`
- `issue_owner_anomaly_shadow`
- `pr_owner_anomaly_shadow`
- `r7_shadow_no_transport_and_r10_owner_local_output_parity`
- `structural_precedence_uses_r7`
- `response_deadline_uses_r8`
- `absence_uses_emission_ledger`
- `absence_without_ledger_fails`

### R11 Fanout-Only Controls (`DESIGNED, NOT YET ENFORCED`)

- positive fixture：`origin-keyed benign skip`必须进入 known-dialogue inventory并被 no-growth/zero-surface ratchet命中；
- negative control：schema rejection不得命中；
- negative control：`event_id` dedup不得命中；
- negative control：entity rehydration不得命中；
- negative control：legal same-entity post-admission CAS不得命中。

Bounded loops：

- `self-loop once`
- `release`
- `timeout`
- `stale-lineage`

## C.2 Negative Controls

| ID | negative control | required failure |
|---|---|---|
| `NC-01` | 删除 OLD-observed edge | `missing-edge` |
| `NC-02` | 新增无 OLD witness的 edge | `extra-table-edge` |
| `NC-03` | table-only edge扩大 lifecycle cage | `table-only-edge` |
| `NC-04` | 接受 undeclared successor | `illegal-successor-acceptance` |
| `NC-05` | checker与 checked semantics同窗替换 | `checker-checked-cochange` |
| `NC-06` | inventory省略 writer/acceptor/sink | `omitted-boundary` |
| `NC-07` | checker与 unlisted caller同改 | `checker-plus-unlisted-caller` |
| `NC-08` | 改 mandatory fixture掩盖 failure | `fixture-co-change` |
| `NC-09` | manifest semantic hash mismatch | `manifest-semantic-hash-mismatch` |
| `NC-10` | historical manifest跨 PR复用 | `manifest-pr-reuse` |
| `NC-11` | committed manifest自绑定 exact head | `self-referential-head-binding` |
| `NC-12` | CI head变化复用 attestation | `stale-head-attestation` |
| `NC-13` | manifest blob hash mismatch | `manifest-blob-mismatch` |
| `NC-14` | self-hash未省略 own field | `artifact-self-hash-cycle` |
| `NC-15` | caller向 production decider提供 rows | `caller-rows-forbidden` |
| `NC-16` | caller提供 `cas_mode`/policy | `caller-policy-forbidden` |
| `NC-17` | non-owner构造 production decider | `owner-decider-boundary` |
| `NC-18` | analysis relation mint grant | `analysis-grant-forbidden` |
| `NC-19` | plain table伪造 grant | `forged-grant` |
| `NC-20` | grantless lifecycle sink | `grantless-lifecycle-effect` |
| `NC-21` | new head file绕过 watched set | `whole-head-bypass` |
| `NC-22` | post-decision override为 apply | `post-decision-override` |
| `NC-23` | unknown CAS policy | `unknown-cas-policy` |
| `NC-24` | intent匹配多个 edges | `ambiguous-edge` |
| `NC-25` | malformed evidence降级成 apply/idempotent | `illegal-result-required` |
| `NC-26` | review handoff未按 legacy direct-ID验证 | `unverified-review-handoff` |
| `NC-27` | retry/reentry借用 ready handoff | `handoff-scope-violation` |
| `NC-28` | implement later gate执行第二次 direct-ID lookup | `handoff-lookup-count-drift` |
| `NC-29` | OLD observation import NEW extractor或 edge ID | `old-observation-circularity` |
| `NC-30` | A identity从 B反填 | `parity-identity-not-independent` |
| `NC-31` | legal operator/timeout/guard edge按 kind自动进入 pending | `pending-projection-expanded` |
| `NC-32` | missing pending field获得默认 | `pending-order-missing` |
| `NC-33` | conformance union与 OLD graph不等 | `pending-projection-drift` |
| `NC-34` | timeout policy只按 mode选择 | `timeout-mode-only-dispatch` |
| `NC-35` | codex running时获得 timeout grant | `live-codex-timeout` |
| `NC-36` | heartbeat row要求 process-not-running | `heartbeat-process-proxy` |
| `NC-37` | heartbeat fresh仍 timeout | `heartbeat-fresh-timeout` |
| `NC-38` | dependency resolver被 marker age替代 | `dependency-proxy-timeout` |
| `NC-39` | child workflow resolver被 process state替代 | `child-workflow-proxy-timeout` |
| `NC-40` | idempotent repair无 entitlement | `idempotent-entitlement-missing` |
| `NC-41` | idempotent marker replay改 target/version | `idempotent-marker-rewrite` |
| `NC-42` | idempotent missing effects未重放 | `idempotent-replay-lost` |
| `NC-43` | ops require owner internals | `r7-cross-package-require` |
| `NC-44` | ops重做 legality或组装 rows | `r7-second-authority` |
| `NC-45` | anomaly telemetry要求/可 mint transition grant，或进入 mutation | `r7-telemetry-authority-leak` |
| `NC-46` | worker增加 unrelated success family | `unrelated-success-family` |
| `NC-47` | autonomous successor缺 output exit | `successor-missing-output-exit` |
| `NC-48` | hand-authored `to_states`漂移 | `hand-authored-to-states` |
| `NC-49` | unknown payload token被省略 | `unknown-payload-token` |
| `NC-50` | retained builder无 parity witness | `payload-builder-without-parity-witness` |
| `NC-51` | apply edge无 closed entitlement | `apply-entitlement-missing` |
| `NC-52` | row replay effects复制成 successor effects | `row-replay-transition-conflation` |
| `NC-53` | codex dispatch无 grant | `grantless-codex-dispatch` |
| `NC-54` | git push无 grant | `grantless-git-push` |
| `NC-55` | merge无 grant | `grantless-merge` |
| `NC-56` | `make_decider`位于 shared `libraries/devloop` | `shared-decider-forbidden` |
| `NC-57` | registry table identity冒充 owner seal | `owner-seal-missing` |
| `NC-58` | published intent producer被要求持有 grant | `published-intent-grant-leak` |
| `NC-59` | published intent producer能 mint grant | `published-intent-minting-leak` |
| `NC-60` | exact overlay取代 ordered CAS | `ordered-cas-erased` |
| `NC-61` | `review_pr` version mismatch塌缩为 generic stale | `version-mismatch-observability-lost` |
| `NC-62` | refactor增加 every-gate handoff lookup | `behavior-change-in-refactor` |
| `NC-63` | 所有 temporal obligations映射到 R7 | `incapable-temporal-provider` |
| `NC-64` | absence无 emission ledger仍标 monitored | `absence-provider-missing` |
| `NC-65` | refactor新增 anomaly event、ops PR dependency、ingestion或 delivery | `r7-behavior-change-in-refactor` |
| `NC-66` | production decider union issue与 PR rows | `production-cross-owner-union` |
| `NC-67` | behavior-change manifest豁免 refactor parity | `refactor-intent-exemption-forbidden` |
| `NC-68` | operator/replay/legacy blanket exemption | `blanket-exemption-forbidden` |
| `NC-69` | R11 checker漏报 `origin-keyed benign skip` positive fixture，或误报 schema rejection、`event_id` dedup、entity rehydration、legal same-entity post-admission CAS任一 negative control | `request-reply-message-control-drift` |

⟦AI:FKST⟧
