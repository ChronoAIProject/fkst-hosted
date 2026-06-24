# Decompose github-devloop into per-lifecycle packages (PR split first; 4-package target)

Status: DESIGN — awaiting operator approval before any implementation.
Date: 2026-06-20
Author: operator (out-of-band, dogfood), via `sshx` adversarial exploration + operator review.

> Revision note: the first draft proposed one package with two conformance-scanned
> transition tables. Operator review (user-as-oracle) upgraded it to **two
> packages**: the partition belongs at the package boundary (level ④
> capability-restriction, PREVENT) rather than a conformance scan (level ①,
> DETECT). See §3, §4 and §16. The delegation/return/liveness contracts (§5–§9,
> §11–§12) are unchanged — only *where the wall sits* (§3, §4, §10, §13) changed.
>
> Revision note 2 (sshx clustering triplet, 2026-06-20): a second adversarial round
> (minimal/structural/delete codex workers) on "how many packages, where the seams"
> converged on a **4-package target** — `github-devloop-intake` · `…-issue` ·
> `…-pr` · `…-integration` — but kept **this spec's scope at exactly Ratchet 1:
> extract `github-devloop-pr` only** (the desync fix). intake and integration are
> real but *separate* seams, extracted as later independent ratchets; bundling them
> here would be a big-bang. The workers verified the operator's split instinct was
> right but the *first* seam is PR (not intake): `intake_judge` currently writes
> issue state directly (a wide seam), while branch-promotion is the cleaner third
> cut. See §4 (target vs scope) and §16.

## 1. Problem (the root cause, verified at source)

`github-devloop` today runs **one flat state machine per issue** whose `state:v1`
marker tracks *every* phase — including the PR phases `pr-open, reviewing, fixing,
review-meta, merge-ready, merging`. Verified at source:
`packages/github-devloop/core/restart/transitions/` is **one** table holding all of
`thinking, dependency_wait, ready, implementing, pr-open, reviewing, fixing,
review-meta, merge-ready, merging, merged, impl-failed, blocked`.

Separately, the entity-local-PR work (#7) moved the PR-phase *facts* onto the **PR
entity's** comment stream. So PR phase is now written in **two** places — the
issue stream and the PR stream — which is **two mutable authorities for one
phase ⇒ desync**. The production incident: an issue sat at `pr-open` while its PR
had already reached `merge-ready`; the observability read the issue stream and
falsely reported "review never kicked off."

The current mitigation is a symptom-patch: `entity.lua:97
issue_authoritative_linked_state(issue_state, linked_state)` special-cases "issue
says `pr-open` AND linked PR says `reviewing` at the same version → trust the PR."
It covers exactly one phase-pair and does not stop the divergent writes.

### The desync is a symptom of an over-merged god-package

`github-devloop` has grown to **~30 departments and ~60 core modules**. They
cluster cleanly into two groups that change for different reasons:

- **issue-side**: `intake_scan/probe/judge`, `consensus_result`, `observe_issue`,
  `decompose`, `implement`, `loop`, `reconcile`, dependency gate.
- **PR-side**: `open_pr`, `observe_pr`, `review_pr/loop/meta/result`, `fix`,
  `merge`, `pr_freshness_scan`.

One package, two sources (issue entity vs PR entity), two trust domains (issue
stream vs PR stream — already separated by #7), two liveness families, fused into
one state machine. That fusion is the over-merge; the desync is what leaks out of
it. The fix is to make the two sagas two packages whose state namespaces are
disjoint by construction.

### What this design fixes vs. does not

- **Fixes**: the desync *class* — structurally, by giving each saga its own
  package, table, and single-root conformance. The issue package cannot *name* a
  PR phase because it is not in its namespace.
- **Does not fix (out of scope, tracked separately)**: the merge-ready-not-merging
  gate bug. This design makes it diagnosable (issue `awaiting-pr` + PR
  `merge-ready` = "PR stuck at the merge gate," unambiguous) and supplies the
  head-bound-merge-ready invariant that prevents the head-nudge variant, but the
  merge-step root cause is its own fix.

## 2. Harness (prior art this is anchored to)

The standard **parent workflow + child workflow with an explicit await/join**,
applied to two GitHub comment streams.

- **Temporal child workflows.** A parent records a child handle and waits for the
  child's *terminal* result; the child owns its own history, retries, budgets.
  Deviation: two comment streams + reliable delivery emulate the handles/join;
  there is no shared transaction. Temporal makes parent and child **separate
  workflows** despite their coupling — the coupling is a narrow start/await/return
  contract, which is exactly the case for separation (high cohesion, low surface).
- **Saga composition (Garcia-Molina & Salem, 1987).** A child saga's terminal
  resumes the parent. Retries are *forward generations*, never undo edges.
- **Harel statecharts / orthogonal regions.** Issue and PR are separate regions;
  only the PR region owns PR phase. A package boundary is the **strongest** form of
  region separation — disjoint namespaces, not a shared table with a fence.
- **PR-merge bots (bors / homu / Mergify).** Merge/review/check state is PR-local;
  issue links are metadata, not lifecycle authority.
- **Repo precedent.** `github-ratchet-migration-slicer` was extracted *out of*
  `github-devloop` per prefer-out-of-package; this is the same move for the PR saga.

Operative framing (the cross-model oracle): **the bug was duplicated authority,
not missing hierarchy.** So this is not a generic hierarchical-saga *engine* — it
is single authority + the smallest join primitive, `await_child(pr)`, with the two
sagas living in two packages.

## 3. Design principle

> The issue owns issue progress. The PR owns PR progress. The issue holds only a
> **pointer** ("awaiting PR child X") and resumes **only** from a **terminal** PR
> fact for X. Each saga is its **own package** with its **own** transition table
> and single-root conformance; their state namespaces are disjoint.

Four consequences drive every decision below:

1. **Single authority.** Exactly one comment stream is authoritative per phase.
   PR phase ⇒ PR stream; issue phase ⇒ issue stream. No overlap.
2. **Explicit join.** A formal `awaiting-pr → (resume on PR terminal)` edge,
   carried by durable `pr-delegation` plus level-triggered polling of the child PR
   state marker. Without that explicit pointer + poll boundary, resume is ad hoc and
   the race re-grows.
3. **Structural partition, not a scan (PREVENT > DETECT).** Two packages make
   `ISSUE_STATES ∩ PR_PHASE_STATES = ∅` true *by construction* — the issue
   package's code cannot reference a PR-phase state because it is not in its
   namespace/table. This is level ④ capability-restriction. A conformance scan
   (`state_marker(issue,"merge-ready")` → CI-red) is level ① DETECT and is kept
   **only** as the migration backstop, shrinking to a structural guarantee as the
   extraction completes. CLAUDE.md 铁律: a contract expressible as ②③④ must not
   stay at ① scan.
4. **Decompose the god-package (SRP).** issue-lifecycle and PR-lifecycle have
   independent reasons to change; splitting corrects the over-merge and bounds blast
   radius (a PR-review change cannot touch issue lifecycle — different package).
5. **Target decomposition ≠ this spec's scope.** The full SRP-correct decomposition
   is **four** bounded contexts (§4); this spec implements **only the first ratchet**
   — extracting `github-devloop-pr` — because that is the desync root-cause path.
   intake and integration are real but independent seams, each its own later ratchet.
   One migration changes one thing (behavior-preserving + 绝不大爆改).

## 4. Package topology

### Target (the SRP-correct end-state): four bounded contexts + `std`

The sshx clustering triplet (§16) converged on four packages, each earning its
boundary by an independent reason-to-change, executed as **three independent
ratchets** so each migration changes one thing:

| Package | departments | independent 变更原因 (the seam) | ratchet |
|---|---|---|---|
| **`github-devloop`** (issue) | `consensus_result, loop, observe_issue, decompose, implement` | the managed-issue parent saga | residual parent |
| **`github-devloop-pr`** | `open_pr, observe_pr, review_*, fix, merge` | PR comment stream = sole PR-phase authority (the desync class) | **R1 (this spec)** |
| **`github-devloop-integration`** | `sync_scan, sync_conflict, rollup_scan, rollup_merge, substrate_ref_scan, pr_freshness_scan` | branch-promotion / repo topology — *not* issue→PR→merge lifecycle | **R2 (separate)** |
| **`github-devloop-intake`** | `intake_scan, intake_probe, intake_judge` | enable/decline policy + untrusted→managed *trust/prompt-injection* gate | **R3 (gated, see §13)** |

`reconcile`, `comment_handoff`, and `liveness_scan` are current cross-saga mutators
and split **by owning authority** (terminal/state-writing authority lives in the
owning package; only authority-neutral builders move to `std`). Until R2/R3,
integration and intake stay folded inside `github-devloop`.

### This spec's scope = Ratchet 1 only (extract PR)

The diagram below is the **R1 end-state**: PR splits out; intake + integration stay
in the parent for now; the boundary is a durable child pointer plus level-triggered
polling of the child PR state marker.

```
                     std/ (shared, symlinked)
   Tier S: std.saga · std.saga_conformance · version-CAS · source_ref
   Tier R: std.github · std.git · std.ports · std.testing · std.devloop_*  ← shared devloop kernel
        ▲                                          ▲
        │ require("std.*")                         │ require("std.*")
        │                                          │
┌───────┴───────────────────┐   pr-delegation ┌────┴─────────────────────┐
│  github-devloop  (parent) │ ───────────────▶ │ github-devloop-pr(child) │
│  issue saga               │                  │  PR saga                 │
│  intake/consensus/implement│  poll child     │  open_pr observe_pr      │
│  observe_issue decompose   │ ◀ state:v1      │  review_* fix merge      │
│  loop reconcile dep-gate   │                  │  pr_freshness_scan       │
│  + composes github-proxy,  │                  │  + composes github-proxy,│
│    consensus, devloop-pr   │                  │    consensus             │
│  issue transition table +  │                  │  PR transition table +   │
│  single-root conformance   │                  │  single-root conformance │
└────────────────────────────┘                  └─────────────────────────┘
```

- **`github-devloop`** (parent / issue saga) — composed package
  (`fkst.toml` `[event_deps]` packages: `github-proxy`, `consensus`, `github-devloop-pr`). Owns the issue
  transition table. While `awaiting-pr`, `observe_issue` reads `pr-delegation`,
  fetches the child PR, and CAS-resumes the issue only from a matching child
  terminal `state:v1`.
- **`github-devloop-pr`** (child / PR saga) — composed package
  (`fkst.toml` `[event_deps]` packages: `github-proxy`, `consensus`). Owns the PR transition table and all
  PR-phase departments + PR-specific core. It advances the PR from `pr-open` to its
  terminal state on the PR comment stream; it does not push a terminal event back to
  the parent.
- **Shared kernel → `std` — authority-neutral primitives ONLY, never a god-kernel**
  (the only blessed shared root; peer cross-package require is forbidden by G9). The
  restart/saga/conformance *framework* is already Tier S `std.saga`. Lift to a Tier R
  `std.devloop_*` **only** what is genuinely entity-/saga-neutral: the marker
  grammar parser/builder *parameterized by saga namespace*, `source_ref`/entity
  helpers, version-CAS ordering, logging/error-facts, bounded validators, generic
  queue-dispatch and restart/liveness/conformance support. **Do NOT lift a monolithic
  `std.devloop_core`** — all three workers flagged that a god-kernel would re-smuggle
  PR-phase symbols back into the issue package (defeating the partition). Transition
  rows, policy, and payload/request builders stay **package-local**, split by owner —
  not moved as `core.lua` wholesale.
- **Boundary = `pr-delegation` + poll.** The parent stores only the child pointer;
  the child stores its own state. Composed conformance covers the transition row and
  liveness metadata; PR B makes the forward `implementing → awaiting-pr` edge live.

Beyond R1, the target (§4 table) assigns the remaining clusters by their own
reason-to-change, each as a **separate** ratchet (§13) — not bundled into the
desync fix:

- **integration-topology** (`sync_scan, sync_conflict, rollup_scan, rollup_merge,
  substrate_ref_scan` + `pr_freshness_scan`) **earns its own package** (R2). It is a
  branch-promotion / repo-topology control plane, not issue or PR lifecycle; leaving
  it in the parent keeps a "lying god-package". `pr_freshness_scan` belongs here, not
  in PR — it consumes `devloop_branch_tick` (branch-topology), not PR-phase events.
- **intake** (`intake_scan/probe/judge`) is a **gated candidate** (R3). It is a real
  bounded context (enable-policy + untrusted→managed trust gate), but today
  `intake_judge` writes issue thinking state *directly* — a wide seam. R3 is allowed
  only **after** the boundary is first narrowed to a reliable `issue_enable` event
  (intake stops writing issue lifecycle state), then re-evaluated against over-split.
- cross-cutting observability (`doctor`, `observability`, `dead_letter`,
  `ensure_repo`, test harness) stays shared/parent unless provably saga-owned.

## 5. State split

### Issue package — authority = issue comment stream

```
unmanaged → thinking → dependency_wait → ready → implementing → awaiting-pr → merged ✓
                                                       │              │
                                                       ▼              ▼
                                                  impl-failed       blocked
```

Issue-saga states: `thinking, dependency_wait, ready, implementing, awaiting-pr,
impl-failed, merged, blocked`. `merged` stays the existing issue success-terminal
name (the `done` rename is a separate behavior-change, out of scope). The PR phases
do not exist in this package's namespace.

### PR package — authority = PR comment stream

```
pr-open ──→ reviewing ──→ review-meta ──→ merge-ready ──→ merging ──→ merged ✓
              │   ▲            │                              │
              ▼   │ (new head) ▼                             ▼
            fixing ┘    (fix|block decision)        closed-unmerged ✗ / blocked ✗

terminals: merged ✓ · closed-unmerged ✗ · blocked ✗
```

PR-saga states: `pr-open, reviewing, fixing, review-meta, merge-ready, merging,
merged, closed-unmerged, blocked` (`closed-unmerged` is new — the explicit
"closed without merging" terminal, today implicit). These are the existing
pr-open…merging rows relocated to the PR package with their budgets unchanged.

## 6. The delegation state `awaiting-pr`

`awaiting-pr` is the **single** issue state between implementation completion and
parent terminal/resume. It stores only the child PR pointer and the parent lineage;
it never projects or owns the child's PR phase.

Issue stream — immutable delegation pointer plus the issue state:

```
state:v1   state="awaiting-pr"  version=<issue implementation lineage>
pr-delegation:v1 {
  proposal=<parent issue proposal>, pr_proposal=<child PR proposal>,
  pr=<child PR number>, version=<parent implementation lineage>,
  delegation=<generation>
}
```

PR stream — the child's origin and its own state:

```
pr-origin:v1 { parent_issue, parent_proposal_id, impl_version, generation }
state:v1   state="pr-open|reviewing|...|merged|closed-unmerged|blocked" ...
```

The **child PR `state:v1` marker is the authority** for PR progress. The parent
issue may display derived text such as "awaiting PR #N (child is merge-ready)", but
that is a projection only; automation consumes only the issue `awaiting-pr` marker,
`pr-delegation:v1`, and the child PR's own `state:v1` marker. There is no
issue-side PR-phase projection as authority, no `pr-terminal` marker, and no
`child-completed` marker. Version-CAS on the parent issue is the only parent resume
mechanism.

Marker authority is derived from the entity. The existing `state:v1` wire format is
kept (behavior-preserving — no marker flag-day), but the issue package treats
`awaiting-pr` as the only issue-side post-implementation wait state. Labels remain
hints only: an issue label may be `fkst-dev:awaiting-pr`; labels such as
`fkst-dev:pr-open/reviewing/merge-ready` are not issue authority.

## 7. Boundary A — delegation (issue → PR), idempotent ensure

Delegation is an **idempotent ensure**, recoverable from any partial write. The
correlation key is the deterministic implementation branch derived from
`(issue, impl_version, generation)`; once a PR exists, `owner/repo#pr/N` plus
`pr_proposal` is the durable child identity.

`ensure_pr_child(issue, impl_version, generation)` (parent package):

1. Compute the deterministic branch/head from the implementation.
2. Find the existing PR for that branch, or create it.
3. Ensure the PR stream carries `pr-origin:v1` + initial `state pr-open`.
4. Ensure the issue stream carries `pr-delegation:v1` pointing at that PR.
5. Any step already done ⇒ success (idempotent).

There is **no transactional outbox** and no `devloop_pr_open` correctness contract in
PR A. The durable effects are the GitHub facts themselves: the PR `pr-origin` /
`state pr-open` marker and the issue `pr-delegation` marker. On retry, the ensure
function re-finds/adopts by branch or existing delegation; it does not rely on an
in-process API response and it does not open a second PR after a crash.

In PR A this path is still **dark**: live `implement` continues to raise the existing
`devloop_open_pr` event and does not enter `awaiting-pr`. The forward flip that makes
`implementing → awaiting-pr` reachable is PR B.

## 8. Boundary B — return is pull reconciliation, not push delivery

`awaiting-pr` is `dependency_wait`'s twin: a level-triggered reconcile, like a
Kubernetes controller. On every relevant issue poll, `observe_issue` re-reads the
parent issue state, reads `pr-delegation:v1`, fetches the delegated child PR, reads
the child PR's trusted `state:v1`, and applies an idempotent parent CAS only when the
child state is terminal.

`observe_issue` while current issue state is `awaiting-pr`:

1. Re-read the issue under the normal issue lock and confirm current state is still
   `awaiting-pr`.
2. Read `pr-delegation:v1` from the issue comments. Missing, wrong version, wrong
   proposal, or malformed child pointer is fail-closed/no-op; no stale CAS.
3. Fetch the child PR named by the delegation and read its trusted `state:v1` marker
   from the PR comment stream. The child marker is the durable fact.
4. If the child state is nonterminal (`pr-open`, `reviewing`, `fixing`,
   `review-meta`, `merge-ready`, `merging`), defer/no-op. The existing liveness
   sweep redrives via `github_entity_changed`; no terminal queue is needed.
5. If the child state is terminal and its lineage matches the delegation:
   - child `merged` → parent issue `merged`, plus the existing issue-close side
     effect in real-write mode.
   - child `closed-unmerged` → parent issue `ready` with the next implementation
     generation, or `blocked` if the reimplementation budget is exhausted.
   - child `blocked` → parent issue `blocked` with a child-state WHY.

Idempotency is parent version-CAS: re-polling after the parent marker is visible sees
`merged`/`ready`/`blocked` and does not append a second transition. There is no
persist-before-ACK step, no `on_pr_terminal` department, no durable `pr-terminal`
fact, no `child-completed` marker, no per-operation fence, and no queue-return
contract. The child PR's state marker is already the durable source of truth; the
sweep is the backstop.

Resume only on a child terminal — never on `merge-ready`. `merge-ready` remains a
transient PR-local capability bound to a head SHA; copying it to the issue would
re-create duplicated authority.

## 9. Head-bound merge-ready invariant (the head-nudge incident, encoded)

`merge-ready` is valid **only** for the exact PR head SHA it was computed against.
Before `merging`, the gate re-verifies `current_pr_head_sha ==
merge_ready.head_sha`; if the head moved, `merge-ready` is invalidated and the PR
returns to `fixing`/`reviewing`. This mirrors GitHub's required-checks model and
prevents a push (human or bot) from silently invalidating readiness — the failure
mode the operator head-nudge hit.

## 10. Conformance (structural first, scan as migration backstop)

- **Structural target:** `ISSUE_STATES ∩ PR_PHASE_STATES = ∅` because they become
  different packages with different state namespaces. Each package passes its own
  single-root conformance (every non-terminal state has budget + watchdog +
  guaranteed termination + WHY).
- **PR A boundary:** the current package declares the boundary as
  `awaiting-pr + pr-delegation + poll`, not as a push return queue. Conformance
  checks the `awaiting-pr` row as a `child_workflow_wait` poll row driven by
  `github-proxy.github_entity_changed`, with required facts `state`,
  `pr-delegation`, and child PR `state`.
- **No push return queue:** `devloop_pr_terminal`, `on_pr_terminal`, `pr-terminal`,
  and `child-completed` are not part of the contract. The parent reducer reads the
  child PR marker directly.
- **Scan backstop (migration only):** until PR B contracts the issue-side PR-phase
  allowlist to zero, the scanner remains in its current PR A allowlist posture. PR B
  performs the forward flip and shrinks the issue-side PR-phase allowlist to 0.
- Labels are hints only: an issue label may be `awaiting-pr`; it must not mirror a
  PR phase.

## 11. Liveness

`awaiting-pr` has liveness class **`child_workflow_wait`** — distinct from
`pr-open` / `reviewing` / `merge-ready`. Its watchdog does not count PR review or
merge time against the parent. The parent only asks: "does my delegated child have a
terminal state yet?"

The liveness shape is pull-based:

- child nonterminal & healthy under the PR row's contract → **defer/no-op**;
- child state marker stale or missing → the sweep redrives observation with
  `github-proxy.github_entity_changed`;
- child terminal visible and delegation matches → parent CAS resumes from child
  state;
- child missing/broken beyond bounded `child_workflow_wait` budget → issue `blocked`
  with WHY.

The single backstop is the existing liveness sweep. It redrives entity observation;
it does not synthesize `devloop_pr_terminal`. The resolver family remains
`child-state`, but it names no push queue: it reads the child PR state marker.

## 12. Naive failure modes (and the repair)

Idempotent ensure-functions + reconciliation-from-durable-facts (not "trust events
more") close each:

| Failure mode | Result | Repair |
|---|---|---|
| Parent writes `awaiting-pr` before child exists | parent waits forever | CAS only after PR start fact visible (§7); key = deterministic branch |
| Child PR created, parent pointer write fails | orphan PR | `ensure_pr_child` idempotent; reconciler writes missing `pr-delegation` if parent valid |
| Parent resumes from "some PR for this issue" | wrong PR completes wrong attempt | resume requires delegation child id/version == terminal child id (§8.2) |
| Parent poll misses a transient delivery wakeup | parent resumes on the next poll | `awaiting-pr` re-reads the child PR `state:v1`; the sweep redrives `github_entity_changed` |
| `merge-ready` copied back to the issue | original desync returns | issue package has no PR-phase symbol (§3/§6); resume only on terminal (§8) |
| `merge-ready` not bound to head SHA | a push silently invalidates readiness | head-bound invariant (§9) re-verifies head before merge |
| Issue projection consumed by automation | cache becomes authority | projection is display-only (§6); automation reads the PR stream |
| Boundary mis-wired | parent cannot locate child or child state | `pr-delegation` + child-state required facts fail closed; PR B contracts the forward edge separately |

## 13. Migration — three independent harness-first ratchets

The four-package target (§4) is reached by independent inventory-ratchets, never a
big-bang on the live state machine. One migration changes one thing.

### Ratchet 1 — saga split substrate

**PR A — dark trust+sweep substrate (this implementation).** Behavior-preserving:
`implement` still raises the existing `devloop_open_pr`, so `awaiting-pr` remains
unreachable from live production. PR A installs the pull-shaped `awaiting-pr` row,
the `replay_awaiting_pr` reducer, observe_issue wiring beside `dependency_wait`, the
`pr-delegation` + child-state contract, and deletes the unused push return/outbox
apparatus (`devloop_pr_terminal`, `on_pr_terminal`, `pr-terminal`,
`child-completed`, PR-open handoff). The beautiful form is one source of truth: the
child PR state marker.

**PR B — forward flip + contract.** Make `implementing → awaiting-pr` reachable via
`ensure_pr_child`, stop issue-side PR-phase promotion/writes, and shrink the
issue-side PR-phase scanner allowlist to 0. This is the behavior change and gets its
own review/canary.

**Extraction follow-up.** Move PR-phase departments + PR-specific core to
`github-devloop-pr`; lift only authority-neutral helpers to `std.devloop_*`. With the
allowlist at 0 and independent conformance passing, remove the migration scan.

### Ratchet 2 — extract `github-devloop-integration` (separate, after R1)

Move `sync_scan, sync_conflict, rollup_scan, rollup_merge, substrate_ref_scan` and
`pr_freshness_scan` into the integration package. This remains separate from the
PR-saga desync fix.

### Ratchet 3 — extract `github-devloop-intake` (gated)

Allowed only after intake stops writing issue lifecycle state directly and emits a
narrow `issue_enable` event. Until then intake stays folded in `github-devloop`.

## 14. Non-goals / YAGNI

- **No generic hierarchical-saga engine.** The packages use the existing
  `std.saga.department` shape; the child is the smallest `await_child` primitive.
- **No `std.devloop_core` god-kernel** (§4) — only authority-neutral primitives lift
  to `std`; transition rows/policy/payloads stay package-local.
- **No bundling R2/R3 into R1.** integration and intake are extracted as their own
  later ratchets (§13), not in the desync fix.
- **No generic `github-branch-topology` extraction** in R2 (kept in the
  `github-devloop` product namespace, tied to `FKST_DEVLOOP`).
- **No `merged → done` rename** (separate behavior-change PR if ever wanted).
- **No `state:v2` marker flag-day** (entity-derived `saga_kind` keeps `state:v1`).
- **No fix to the merge-ready-not-merging gate bug here** (separate; this design
  only makes it legible + supplies the head-bound invariant).

## 15. Open decisions (to settle in writing-plans)

1. Topology detail: keep `github-devloop` as parent+composer (recommended,
   minimal — 1 new package) vs. a thin top Facade over two siblings (3 packages).
2. Exact module inventory: which `core/*` are shared-kernel (→ `std.devloop_*`),
   issue-specific, PR-specific, or integration/cross-cutting. This is Step-0 work.
3. `closed-unmerged` → issue `ready`(new generation) vs `blocked`: the
   replacement-budget threshold and where it is counted (issue generation lineage).
4. Whether PR `blocked` maps to issue `blocked` directly or routes through the
   existing decomposition (fix-drop → smaller issues) flow.
5. Home of `child_workflow_wait` in `liveness_contract.lua` and its
   `actionable_epoch` source (delegation time).
6. Whether the issue-side projection is rendered in v1 or deferred (display-only,
   non-load-bearing).

## 16. Adversarial record

Design produced by `sshx` inline consensus: 3 peer-invisible codex thinking
workers (minimal / structural / delete, read-only) + 1 cross-model ChatGPT Pro
oracle, meta-judged; then **operator review (user-as-oracle)**.

- **minimal** (`/tmp/saga-minimal.log`): "Do not build a new generic sub-saga
  layer; make the PR entity the sole authority + collapse the issue PR phase into
  one delegation state."
- **structural** (`/tmp/saga-structural.log`): "Adopt the full hierarchical
  sub-saga; the issue must never carry PR sub-phase again." → conformance partition,
  scoped parsing, child id/lineage.
- **delete** (`/tmp/saga-delete.log`): "Delete issue-level PR-phase tracking
  entirely; parent issue saga with one `awaiting-pr` + PR child saga." →
  pointer-only delegation, terminal-only return.
- **oracle / ChatGPT Pro** (cross-model): "The bug was duplicated authority, not
  missing hierarchy → single authority + smallest `await_child` primitive." Added:
  deterministic correlation IDs + idempotent `ensure` recoverable from partial
  writes; **resume only on terminal, never merge-ready**; the head-bound invariant;
  the naive failure modes; issue-side PR status as non-authoritative projection.
- **operator review (user-as-oracle)**: upgraded the partition from a conformance
  *scan* within one package to a **package boundary** (two packages). Rationale: the
  partition is expressible at level ④ capability-restriction (disjoint namespaces),
  and CLAUDE.md 铁律 forbids leaving an expressible structural contract at level ①
  scan; the move also decomposes a 30-department over-merged god-package along its
  true SRP seam. Same `美 = 真理探测器` pattern the operator has caught before — the
  AI optimized a proxy (the scan) when a structural solution (the boundary) was
  available.

**Meta-judge: `implement`.** End-state unanimous across the four sshx perspectives;
the operator review strengthens it from DETECT to PREVENT. The lone tension —
structural's "full new sub-saga layer" vs. minimal/delete/oracle's "delete the
duplication + smallest join" — resolves to: the two-package split **is** the
structural invariant achieved with least machinery (the PR entity already owns a
state machine via #7; reuse `std.saga.department`), satisfying BEAUTY GATE
(删无可删 of the scan, illegal states unrepresentable) and structural integrity at
once.

### Round 2 — clustering triplet (how many packages, where the seams)

A second `sshx` round (3 peer-invisible codex workers, codex-cli 0.141.0) tested the
operator's proposed `intake | issue | pr` 3-way against the actual department +
core-module clustering (verified at source):

- **minimal** (`/tmp/sshx-split/result-minimal.json`, verdict `propose`): smallest
  cut = **2-way** {issue+intake} | {pr}; fold integration for now; warned that
  bundling integration is a big-bang and that a monolithic `std.devloop_core` would
  re-smuggle PR phases into the issue package.
- **delete** (`/tmp/sshx-split/result-delete.json`, verdict `propose`): **3-way**
  {issue+intake} | {pr} | {branch-promotion}; *fold intake* (3 depts don't earn a
  package; `intake_judge` writes issue state directly) and instead *delete
  integration out of the lifecycle*.
- **structural** (`/tmp/sshx-split/result-structural.json`, verdict `revise`):
  **4-way** {intake} | {issue} | {pr} | {integration}; intake earns a package *iff*
  its boundary is first narrowed to a reliable `issue_enable` event; `pr_freshness`
  belongs to integration.

**Meta-judge (round 2): `meta-layer convergence`.** Not unanimous on N (2 vs 3 vs 4),
but compatible. Unanimous: PR earns its own package; reconcile/comment_handoff/
liveness split by authority; **no `std.devloop_core` god-kernel**; the operator's
intake-as-package is wrong *as proposed* (intake leaks authority today). Resolved
conflicts: intake = real bounded context but R3-gated on a narrowed boundary
(structural's end-state ∧ minimal/delete's caution); integration = own package but a
separate R2 (delete/structural commit ∧ minimal's "no big-bang"); `pr_freshness` →
integration. Converges to a **4-package target executed as 3 ratchets, R1 = this
spec (extract PR only)**. The operator's split instinct was right; the *first* seam
is PR, and the under-named third seam is branch-promotion, not intake.

⟦AI:FKST⟧
