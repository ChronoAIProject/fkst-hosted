# Monotone lifecycle-gate harness - make the "transient-cursor" saga bug class CI-red, not runtime-discovered

Status: DESIGN SPEC - package-side harness now (Unit A, shipped); stronger form deferred (Unit B); an engine typed-lifecycle primitive was considered and rejected on engine-layering grounds (Section 5).
Date: 2026-06-21
Author: sshx implementation worker, from converged minimal / structural / delete design.

This spec is intentionally narrow. It defines one canonical package-side way to ask
"has this lifecycle ever reached milestone P?", a conformance ratchet that forbids
known bypasses, and the responsibility-signature extension that lifts the check from
grep to schema. The stronger form, if the residual ever becomes a real recurring case,
stays package-side (a closed-world gate-API); it does not belong in the generic engine
(see Section 5 for the layering rationale and the deferral decision).

## 1. Problem and Bug Class

The bug class is precise:

> A monotone gate asks "has X happened?" or "has this lifecycle reached phase P or
> later?", but the implementation reads the transient current-phase cursor and
> compares it with `== "phase"` instead of reading the durable append-only milestone
> fact.

The cursor is a latest-state view. In the current code, `std.devloop_state.current_state`
scans trusted `state:v1` markers and keeps the greatest candidate by version and
stage ordering (`std/devloop_state.lua:426`, `std/devloop_state.lua:431`,
`std/devloop_state.lua:445`). That is correct for routing decisions that need "where
is the saga now?" It is the wrong surface for monotone gates that need "did this ever
start/reach P?".

The canonical production example is bug #2, the parent/child PR delegation desync
that left the parent issue stuck `implementing` after the child PR had advanced and
merged (#1299). Before commit `3455700`, `child_start_visible` was derived from the
PR child current cursor:

```lua
local pr_origin = M.pr_origin_fact(issue.pr_comments or {})
local pr_state = M.current_entity_state(issue.pr_comments or {}, issue_proposal_id)
local child_start_visible = pr_origin ~= nil
  and tostring(pr_origin.proposal_id or "") == issue_proposal_id
  and tostring(pr_origin.issue_number or "") == tostring(issue_number)
  and pr_state ~= nil
  and pr_state.state == "pr-open"
  and tostring(pr_state.version or "") == tostring(impl_version)
```

Historical source: `3455700^:packages/github-devloop/core/pr_delegation.lua:169`.
When the PR advanced `pr-open -> reviewing -> merged`, the current cursor was no
longer `"pr-open"`, so the forward gate flipped false. The bug was not that the PR
had failed to start. The bug was that the start gate asked the wrong question.

The current fix proves the durable-fact shape. `ensure_pr_child` now checks the
append-only `pr_origin` start fact, proposal, issue number, implementation version,
branch, and base branch, without requiring the current child state to still be
`pr-open` (`packages/github-devloop/core/pr_delegation.lua:169`,
`packages/github-devloop/core/pr_delegation.lua:170`,
`packages/github-devloop/core/pr_delegation.lua:173`,
`packages/github-devloop/core/pr_delegation.lua:175`). The PR-open comment writer
already emits both the durable origin marker and the initial `state:v1` marker
(`packages/github-devloop/core/pr_delegation.lua:84`,
`packages/github-devloop/core/pr_delegation.lua:89`,
`packages/github-devloop/core/pr_delegation.lua:90`).

This recurrence family is broader than PR delegation:

- #887: `ready` mixed actionable readiness with dependency-held waiting, so a
  transient clock/cursor shaped as "current ready" was used for two liveness classes.
  CLAUDE.md records the one-state-one-liveness invariant and #887 as the root
  (`CLAUDE.md:216`, `CLAUDE.md:222`).
- Version-CAS stage ranking: `state:v1` markers carry `stage_rank`
  (`std/devloop_state.lua:43`, `std/devloop_state.lua:46`), and the current cursor
  orders by version and rank (`std/devloop_state.lua:216`, `std/devloop_state.lua:226`,
  `std/devloop_state.lua:260`). That makes `current_state` a latest cursor, not an
  "ever reached" fact.

Review and tests missed the class because they were correctness-blind to the
transient flip: the code passed composed tests and adversarial review while the
runtime incident exposed the missing monotone invariant. Per the repository harness
gradient, this must move from runtime discovery to a class-level CI invariant, and
eventually toward an untypeable typed surface: runtime -> per-case test -> CI scan
or schema -> typed gate-API (package-side; see Section 5 for why this stays out of the
generic engine). The marker stream is already append-only and
event-sourced; "ever reached P" is a durable monotone fact in that stream, not a
property of the latest cursor.

### Reference frame (prior art, cross-model sharpened)

Sharpened across models (ChatGPT Pro cross-model review), the precise framing is
*prefix-monotone facts versus volatile projections*:

- **Event sourcing** (Fowler): the append-only marker stream is the authority; the
  "current phase" is one projection, to be rebuilt from the log, never treated as the
  source of truth. A forward gate that reads the projection has inverted the authority.
- **TLA** (Lamport): the broken predicate asks a *temporal existence question* — "has
  this behavior prefix ever contained milestone P?" — but answers it with a state
  predicate over the *latest* state. It confused a property over a behavior prefix with
  a property of one current value.
- **CALM / CRDT monotonicity**: `reached(P)` is a grow-only fact (G-Set membership):
  monotone, coordination-free, and safe under the eventually-consistent marker stream.
  `current_state == P` is non-monotone — it can retract as the cursor advances — which
  is exactly why it is fragile under an append-only, read-after-write-lagging source.

The harness therefore is not merely "type the values"; it is "a forward gate may only
ask a monotone (grow-only) question of the lifecycle, and the one canonical monotone
query is `reached()` (Section 2); the non-monotone cursor projection must never back a
forward gate."

## 2. Canonical Monotone Surface: `std.devloop_state.reached`

Add one package-side canonical API:

```lua
std.devloop_state.reached(comments, proposal_id, milestone, opts)
```

It returns true once the trusted append-only lifecycle marker stream for
`proposal_id` has ever reached `milestone`, within the lineage/domain described by
`opts`. The helper compares ordered lifecycle phases with `>=`; callers never spell
`current.state == "literal"` for forward gates.

Companion helpers:

- `std.devloop_state.is_at_or_after(state_or_marker, milestone, opts)` for a single
  marker or state fact.
- `std.devloop_state.compare_phase(left, right, opts)` for the ordered phase
  comparison.
- A narrow milestone-domain option, for example `opts.domain = "github-devloop-pr"`
  or `opts.lineage_base = impl_version`, so terminal or recovery states from another
  branch of the lifecycle cannot accidentally imply an unrelated milestone.

The ordering source is the existing github-devloop lifecycle order and rank:
`std/devloop_base.lua` declares `state_graph`, `_state_order`, and
`_state_stage_rank` (`std/devloop_base.lua:110`, `std/devloop_base.lua:129`,
`std/devloop_base.lua:131`), and installs them on the package core
(`std/devloop_base.lua:929`, `std/devloop_base.lua:931`,
`std/devloop_base.lua:932`). `std.devloop_state.stage_rank` is the current public
rank accessor (`std/devloop_state.lua:56`). Because `std.devloop_state` already lives
in repo `std/` and is installed by `packages/github-devloop/core.lua`
(`packages/github-devloop/core.lua:36`), the helper is package-side and buildable now.
It is Tier R: repo-domain lifecycle semantics, not an engine primitive.

Before:

```lua
local current = M.current_entity_state(comments, proposal_id)
local child_start_visible =
  current ~= nil
  and current.state == "pr-open"
  and tostring(current.version or "") == tostring(impl_version)
```

After:

```lua
local child_start_visible = M.reached(
  comments,
  proposal_id,
  "pr-open",
  { lineage_base = impl_version, domain = "github-devloop-pr" }
)
```

The already-landed bug #2 fix used a more specific durable origin fact rather than
the current cursor (`packages/github-devloop/core/pr_delegation.lua:169`). This spec
generalizes that lesson: if the question is "did this lifecycle start/reach P?",
the only allowed shape is a durable milestone accessor, normally `reached()`.

Legitimate current-cursor reads remain available for current-state routing. The
boundary is semantic: forward gates use milestones; decision departments that route
based on the current state may still use the cursor.

## 3. Conformance Ratchet: G-MONOTONE-GATE

Add `G-MONOTONE-GATE` as a sibling repository ratchet to the existing scanners:

- `G-ADAPTER` is wired through `scripts/check_repo.py:917` and
  `scripts/check_repo.py:921`, with its allowlist at
  `migration/gh-git-adapter.allowlist` (`scripts/check_repo_gh_git_adapter.py:11`).
- `G-DEDUP` is wired through `scripts/check_repo.py:923` and
  `scripts/check_repo.py:925`, with `migration/code-dedup.allowlist`
  (`scripts/check_repo_dedup.py:13`).
- `G-CONTENT-TRUNCATION` is wired through `scripts/check_repo.py:982`, with
  `migration/content-truncation.allowlist`
  (`scripts/check_repo_content_truncation.py:12`).
- `G-SAGA-SPLIT` is a close precedent for "migration ratchet, not the structural
  boundary" (`scripts/check_repo_saga_split.py:2`, `scripts/check_repo_saga_split.py:4`,
  `scripts/check_repo_saga_split.py:21`, `scripts/check_repo_saga_split.py:22`).

`G-MONOTONE-GATE` discovers all transient lifecycle cursor reads in
`github-devloop*` production gate, transition, and handler code, then requires each
occurrence to be classified:

- `.state == "<lifecycle phase literal>"` in a predicate classified as a monotone
  lifecycle gate.
- `current_entity_state(...)` / `current_state(...)` cursor reads used to answer
  "has reached?", "start visible?", "ready for parent?", "gate open?", or similar
  forward visibility questions.

The ratchet uses `migration/monotone-gate.allowlist`, shrink-only to zero. New
violations outside the allowlist are CI-red; stale allowlist entries are CI-red;
allowlist growth relative to the base branch is CI-red, following the same pattern
as `saga-handler.allowlist` growth checks (`scripts/check_repo.py:933`,
`scripts/check_repo.py:945`, `scripts/check_repo.py:959`).

This is the harness essence: one canonical way (`reached()`), every bypass CI-red.
The scan is not the end-state. It is the migration backstop while Section 4 lifts
the distinction into the restart responsibility schema and Section 5 moves the
strongest form to substrate.

The scanner must not ban all `state ==` usage. Decision and routing code can
legitimately ask "what is the current state?" A department switching over the current
state for a current routing decision is not a monotone milestone gate. The scan
therefore uses broad discovery plus classification: migrated monotone gates must use
`reached()` or another approved milestone accessor, while legitimate current-routing
reads and not-yet-migrated debt must be listed in the shrink-only allowlist. A new
undeclared cursor read is CI-red until it is migrated or classified; "do not declare
gate_kind" is not an escape hatch.

## 4. `responsibility_signature` Extension

Current restart rows already carry `responsibility_signature` data. The helper is
loaded as part of the restart table (`std/devloop_restart.lua:93`,
`std/devloop_restart.lua:97`, `std/devloop_restart.lua:106`), and the current contract
requires fields such as `receiver_kind`, `driving_queue`, `state_kind`,
`liveness_class`, `input_fact_family`, `output_postcondition_family`, `phase_rank`,
`lineage_keys`, and `successors` (`std/devloop_restart_responsibility_contract.lua:165`,
`std/devloop_restart_responsibility_contract.lua:172`,
`std/devloop_restart_responsibility_contract.lua:175`,
`std/devloop_restart_responsibility_contract.lua:181`,
`std/devloop_restart_responsibility_contract.lua:186`,
`std/devloop_restart_responsibility_contract.lua:191`,
`std/devloop_restart_responsibility_contract.lua:194`). It also already distinguishes
`decision` and `gate` state kinds (`std/devloop_restart_responsibility_contract.lua:5`,
`std/devloop_restart_responsibility_contract.lua:8`,
`std/devloop_restart_responsibility_contract.lua:9`) and validates decision fanout
through `decision_type` (`std/devloop_restart_responsibility_contract.lua:285`,
`std/devloop_restart_responsibility_contract.lua:289`,
`std/devloop_restart_responsibility_contract.lua:292`).

Extend that schema with:

```lua
gate_kind = "monotone_milestone" -- or "decision", "current_route", ...
milestone_accessor = "std.devloop_state.reached"
milestone = "pr-open"
milestone_domain = "github-devloop-pr"
```

For a `gate_kind = "monotone_milestone"` row, conformance proves:

1. The gate references only `reached()` or approved milestone accessors.
2. The gate does not read `current_entity_state` / `current_state` directly.
3. The gate does not compare `.state == "<phase literal>"`.
4. The declared milestone belongs to the same lifecycle domain and lineage keys as
   the row.

For `gate_kind = "decision"` or `state_kind = "decision"`, current cursor reads remain
legal. Example: `awaiting-pr` is a gate row today (`packages/github-devloop/core/restart/transitions/awaiting_pr.lua:56`,
`packages/github-devloop/core/restart/transitions/awaiting_pr.lua:59`,
`packages/github-devloop/core/restart/transitions/awaiting_pr.lua:65`); a future
monotone gate on the same surface would declare `gate_kind = "monotone_milestone"`,
while a terminal-child-state decision remains a decision gate.

This lifts `G-MONOTONE-GATE` from a level-1 scan to a level-2 declarative invariant.
The package-side addition is deliberately minimal: one `gate_kind` axis plus the
accessor metadata needed for conformance. No external DSL, interpreter, YAML state
language, or broad type machinery is introduced.

## 5. Substrate Level-4 Primitive, Deferred

Unit A leaves one honest residual: the conformance scan is textual (the inherent
level-1 limit), so a gate that reads the marker stream and parses raw marker *text*
(not via the scanned `current_state` / `current_entity_state` / `.state == "<phase>"`
patterns) could still ask a non-monotone question and bypass the scan. Closing that
residual is the "untypeable endgame": a typed surface where a monotone gate accepts
only a milestone fact and the current cursor is not reachable from that API, so the
bug cannot be expressed.

**Decision (sshx adversarial: minimal / structural / delete thinking triplet
converged + ChatGPT Pro cross-model): DEFER Unit B, and when escalated, do it
PACKAGE-SIDE, not in the engine.** Two findings drove this:

1. **An engine typed-lifecycle would violate engine layering.** `fkst-substrate` must
   provide only generic, project-agnostic primitives and must never encode a specific
   project's lifecycle. The github-devloop phases (`pr-open` / `reviewing` / `merged`)
   are package-side business semantics, and the marker stream is package-side (GitHub
   comments), not engine-owned. A `MilestoneFact` vs `CurrentCursor` type that knows
   the github-devloop lifecycle belongs in the package, not the generic runtime. A
   genuinely generic engine "monotone fact vs current projection" capability is
   conceivable but speculative — the engine does not see the marker stream — so it is
   not justified now.

2. **Rule of Three / patterns serve the current problem.** Unit A already makes the
   known cursor-read monotone-gate class CI-red across all five packages. The
   raw-marker-text residual is narrow and not yet a recurring, verified case. Building
   the endgame before a second/third real instance appears is premature.

Escalation path, only if the residual becomes a real recurring case: a **package-side
closed-world gate-API typestate** — a lifecycle gate constructed through an API that
exposes only monotone milestone queries (`reached()`), so the raw cursor and raw
marker text are unreachable from a "gate" object, making an undeclared monotone gate
unrepresentable rather than merely CI-red (the level-3 `make-illegal-states-
unrepresentable` form that fits Lua). This is the next step beyond Sections 2 to 4 —
still package-side. The engine stays out of the github-devloop lifecycle.

Cross-model (ChatGPT Pro) verdict, recorded: **Unit A is probably sufficient as the
immediate shipped guardrail** (it CI-reds the known recurring class across all packages,
shrink-only allowlist, planted violation). The raw-marker-text residual is real but a
narrower, more deliberate failure mode than accidentally reading `current_state`. The
next strengthening is justified **only if cheap**, and its concrete cheap shape is
capability isolation by Lua module boundary, NOT a typed engine primitive: an opaque
positive gate DSL (`require_reached(P):and_reached(Q)` — no `not`, no cursor, no raw
marker text, no arbitrary callback) whose gates are declared as data and evaluated by a
runner; raw-marker parsing confined to ONE private module; `gates/` forbidden (by
conformance, like the G9 cross-package-require ban) from `require`-ing the raw-marker /
cursor modules; Unit A's scanner kept as the outer tripwire; one planted violation that
reparses raw marker text inside a gate. Because that shape requires restructuring (a
`gates/` boundary + a private parse module), it is not obviously cheap today, so it stays
deferred until the residual recurs or the restructuring becomes cheap. The generic
substrate primitive (`MonotoneFactSet` / `CurrentProjection`) is imaginable but must be
EXTRACTED from package policy when a second case appears, never guessed by the engine
(dependency inversion).

Sandbox-loader status after the substrate #152 adoption: gate definitions under
`core/gates/` have exactly one legitimate access path, `std.devloop_gate.load_gate()`.
`G-MONOTONE-GATE-DSL` makes direct `require("core.gates.<name>")` and direct
`core/gates/<name>.lua` path loads CI-red outside the loader, including tests. The
loader runs each gate definition through substrate `restricted_lua_load({ source,
bindings, mode = "text", name })`, which evaluates the source in a fresh
capability-isolated Lua state with an empty `_ENV` and only the positive gate
constructors plus minimal scalar helpers explicitly granted. Ambient `require`,
`load`, `loadstring`, `_G`, `debug`, `package`, raw table primitives, metatable
access, `string.dump`, and the value-metatable path `("").dump` are unreachable.
The package still validates that the returned value is plain positive gate data.

The previous honest residual is closed: Lua's shared string value metatable no
longer exposes `string.dump` to gate definitions, because the sandbox boundary is
now host-owned and per-load instead of a package-level `_ENV` wrapper.

## 6. Migration Plan

Use an inventory ratchet, not a mega-PR.

1. Add `std.devloop_state.reached()` and companion helpers in `std/devloop_state.lua`.
   Cover marker scanning, trusted marker filtering, phase comparison, and
   lineage/domain options.
2. Add `scripts/check_repo_monotone_gate.py` and wire it into `scripts/check_repo.py`
   as `G-MONOTONE-GATE`, matching the existing repository-check pattern
   (`scripts/check_repo.py:967`, `scripts/check_repo.py:979`,
   `scripts/check_repo.py:982`, `scripts/check_repo.py:985`).
3. Seed `migration/monotone-gate.allowlist` with the current broad inventory of
   transient cursor reads and literal phase equality sites in `github-devloop*`
   production gate, transition, and handler code. Each entry carries a why and a
   tracking issue, following the shrink-only debt discipline in CLAUDE.md
   (`CLAUDE.md:259`, `CLAUDE.md:261`).
4. Migrate one slice at a time from cursor equality to `reached()`. The #1303
   `child_start_visible` fix is the proof-shape: durable start/milestone fact, not a
   current cursor (`packages/github-devloop/core/pr_delegation.lua:169`,
   `packages/github-devloop/core/pr_delegation.lua:176`,
   `packages/github-devloop/core/pr_delegation.lua:202`).
5. Shrink `migration/monotone-gate.allowlist` to zero. Do not combine this with PR
   package extraction, liveness rewrite, merge-gate changes, or adapter migration.

Acceptance:

- Every monotone lifecycle gate uses `std.devloop_state.reached()` or an explicitly
  approved milestone accessor.
- `migration/monotone-gate.allowlist` accounts for the current inventory and shrinks
  to zero over follow-up migrations; growth and stale entries are CI-red.
- `responsibility_signature` can classify monotone milestone gates separately from
  decision/routing states.
- Current-state routing code remains legal when declared as a decision/current-route
  surface.

## 7. Non-goals

1. No external DSL, interpreter, or YAML state language. `M.spec`, Lua tables, the
   restart transition table, and conformance already form the schema-checked DSL.
   Adding another language creates a second escape hatch and an unaudited bypass.
2. Do not literally ban `if` / `else` or all `state == "literal"` comparisons. Type
   the values and surfaces: milestone fact vs current cursor. A guard may keep an
   `if`; it just cannot reach the cursor when the gate is monotone.
3. Do not fold routing, egress, content-fetch, consensus, or arbitrary computation
   into the saga gate structure. The lifecycle table governs lifecycle state.
   Routing, ports/adapters, content rehydration, and consensus remain orthogonal
   disciplines.

## Open Questions and Risks

- Forward-gate vs decision classification has edge cases. The ratchet should start
  conservative and use `responsibility_signature.gate_kind` as the source of truth,
  not filename heuristics alone.
- `reached()` must handle GitHub eventual consistency without inventing a second
  truth source. Missing or lagging markers should return false or retry according to
  the existing gate/liveness contract, not guess.
- Raw `stage_rank >= milestone_rank` can be unsafe across unrelated lifecycle
  branches. The helper needs `opts` for domain/lineage so a high-rank terminal from a
  different path does not imply a milestone it never passed through.
- The substrate primitive scope should be kept small: distinct milestone and cursor
  types, not a full external lifecycle language.

⟦AI:FKST⟧
