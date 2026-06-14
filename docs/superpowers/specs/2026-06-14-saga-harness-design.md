# Design: SAGA harness — force "来了就做，做过就不做" as the only writable shape

Status: proposal · Date: 2026-06-14 · Repo: fkst-packages
Depends on: `2026-06-14-std-shared-library-design.md` (this harness lives in `std/saga.lua`).

---

## 1. Problem (实证)

The system constrains queues and departments precisely to make complex things
simple. The core contract is **at-least-once delivery + idempotent processing**:
"消息来了就处理，处理过了就不处理". The engine already guarantees "delivered ≥1
time"; the package must guarantee **idempotency**.

Empirically, **almost every dogfood bug is the same bug: the "处理过了" predicate
is wrong.** Classifying ~130 recent dogfood fixes: `never 6 · stall 5 · liveness
5 · restart 4+1 · starvation 3 · resync 3 · unbounded 1 · park 1 · forever 1` —
overwhelmingly *liveness* ("the good thing never happened"), and 215/258 fix
touches landed in `github-devloop`. **None** were engine-delivery defects; the
engine delivered faithfully and the *package behavior on top* was wrong:

| bug | what the "done" predicate got wrong |
|---|---|
| #601 spam | **missing**: an echo had no done-predicate → redelivery re-emitted |
| #587 park round 0 | **too wide**: `dedup_key` treated a *new* round as "already done" → swallowed a legitimate re-raise |
| #472 lost hand-off | **not durable**: the done-fact did not survive restart |
| #582 label no resync | **wrong anchor**: "I did it" ≠ "external truth is the target state" → must level-reconcile |
| #550 / #588 scan never runs | **never arrived**: bare-vs-namespaced dispatch mismatch → "arrived but not processed" |

Two predicates underlie all of it: (1) *did this message reach the handler that
owns it* (routing), and (2) *was it really already processed* (idempotency:
durable + re-derived-from-source + neither too wide nor too narrow).

Why the test harness misses this: test mode calls `fkst.test.run_department`
directly, **bypassing the router**, and runs **one clean pass**. It cannot
reproduce the delivery dynamics where the bugs live (duplication, restart,
dedup-collision, multi-round, budget). The error-handling net is blind too,
because a liveness violation ("good thing never happened") produces **no error
fact** (CLAUDE.md 活性⟂安全).

## 2. The shape already exists in embryo (build, don't invent)

This package is *already* a `"saga"` persistence class
(`github-devloop/core.lua: M.persistence_class() == "saga"`), and
`github-devloop/core/saga.lua` already has the discipline **at the per-effect
level**:

```lua
-- core/saga.lua (existing)
function M.effect_once(opts)            -- opts = { effect_id, completion_check, perform }
  if opts.completion_check() then       -- completion_check == "done"  ("做过就不做")
    return { action = "skip" }
  end
  return { action = "perform", result = opts.perform() }  -- perform == "act" ("来了就做")
end
```

Its own doc comment already encodes the #587/#601 lesson: *"An idempotent effect
must be guarded by `completion_check`, not by a write-once 'started' marker."*

**This spec lifts `effect_once` from per-effect to per-department.** That is the
Rule-of-Three third strike (the pattern already recurs across departments), so
promotion to a class-level primitive is justified, not premature.

## 3. Goal / Non-goals

**Goal.** Make "来了就做，做过就不做" the **only writable department shape**, with
the failure modes above caught by a **mechanically-generated, mandatory gate** an
AI (Claude / codex / the autonomous devloop) must pass to merge.

**Non-goals (Lua phase).**
- Not testing the engine's delivery *mechanism* (redb/lease/ack/retry) — that is
  the engine's own responsibility (loom/property/crash tests, out of scope here).
- Not the full dynamic-delivery fidelity harness (real router under restart) —
  that needs the engine; see §8.
- Not changing the engine (Lua-first); engine work is Phase 3 only.

## 4. The three teeth

### Tooth 1 — Forced shape (必须这么写 / 禁止其他写法)

`std/saga.lua` exposes the **only** legal way to define a department. The author
fills two holes; the framework owns control flow:

```lua
-- departments/<dept>/main.lua — the only legal shape
return std.department {
  consumes = { "devloop_ready" },

  -- "做过就不做": durable, re-derived from the fact source (git/gh/marker). Pure, read-only.
  done = function(event) return <work for this event is an established fact?> end,

  -- "来了就做": runs only when not done. Effects only via primitives (raise/spawn/gh-write/marker-write).
  act  = function(event) ... end,

  -- optional: spec fields the department still owns
  produces = { ... }, stall_window = "2m", retry = { ... },
}
```

`std.department{}` returns the existing engine contract (`M.spec` table +
generated `pipeline`) so the runtime sees no difference. The generated pipeline
is exactly `effect_once` lifted to the event, wrapped by the existing failure
fact logger:

```lua
pipeline = wrap_pipeline_failure(name, function(event)
  if done(event) then log_skip(name, event); return end   -- idempotent no-op
  return act(event)                                        -- forward progress
end)
```

The author **cannot** write dispatch or idempotency wiring — the framework gives
only `done`/`act`, not the control flow. That is the physical meaning of
"forbid other ways" (enforced at the gate now; at the loader after Phase 3).

Static checks (extend `core`/conformance, fail-closed): department not built via
`std.department` → reject; wall-clock/random in `done`/`act` control path →
reject; module-level mutable state used as memory → reject; `done` must read only
(no external writes) → reject; every declared `consumes` queue routes (no silent
skip-foreign fallthrough — generalize the existing `unsupported_payload_test`
invariant to be framework-derived from `consumes`).

> Scan/tick departments fit naturally: a level-triggered reconcile is
> `done = "this entity already in target state?"`, `act = reconcile`. #582's class
> is absorbed by the shape itself.

### Tooth 2 — Forced property (必须过的测试，引擎/框架自动生成)

Once the shape is fixed, the only thing left to get wrong is the `done`
predicate. So `std/saga.lua` **auto-generates** this test per department from
`consumes` — the author writes nothing:

> For each consumed queue, take one event of that queue:
> ① deliver once → `act` must run and produce an effect or a follow-on raise → **routing + progress** (no silent drop, no spin)
> ② deliver twice with a restart between → second delivery must take the `done` branch with **zero new external mutating effects** → **idempotency + durability**
> ③ a "near-key but content-different" new event → must **not** hit `done` → **predicate not too wide** (#587)

`done` missing → ② red; `done` not durable → ② red; `done` too wide → ③ red;
routing wrong → ① red.

**Idempotency 判等 (the precise oracle).** "External mutating effect" = the set of
*write-class* external commands + marker writes recorded by
`fkst.test.command_calls`. Read-class commands (`gh issue view`, `gh pr diff`)
may repeat freely; write-class (`gh issue comment`, `gh pr merge`, label writes,
marker writes) must produce **zero new** entries on the second delivery. The
oracle compares the write-class command multiset between delivery 1 and delivery 2.

**Restart + stateful-truth fixture (the one non-trivial component).** Test mode
has no cross-call memory, so calling `run_department` twice *is* a clean restart
replay — **except** the second call must see the *external truth the first call
wrote*. `std` therefore ships a **stateful external-truth fake**: a small
in-memory model of gh/marker state that records delivery 1's writes and serves
them as delivery 2's reads. This is the "外部真相模型，写一次共享" — it lives in
`std` (shared test infra), not per test. ③'s "near-key new event" is generated
per queue from a queue-declared "what counts as new" key projection (e.g. loop
round / head sha), so the probe is mechanical, not hand-written per department.

### Tooth 3 — Forced gate (强制必须这样)

- Teeth 1 + 2 become **required** conformance results (the engine's
  `fkst.test.report.v1` already drives the G5 file-coverage gate; these become
  required entries).
- `scripts/check_repo.py` gains the **ratchet** (see §5): every department must be
  `std.department` shape unless grandfathered on a shrink-only allowlist.
- The autonomous **devloop merge gate** already requires CI green + branch
  protection; the harness conformance becomes one more required status check.
  No AI-produced PR merges unless it is this shape and passes ①②③.

## 5. Migration — strangler fig + ratchet (gradual, non-breaking, self-terminating)

Reconciles "慢慢迁、不破坏现有代码" with the no-compat doctrine: the dual-shape
window is **finite and self-deleting**, not a permanent compat layer.

- **Phase 0 — define in `std` (additive, zero breakage).** Add
  `std.department{done,act}` + the ①②③ oracle + the stateful-truth fixture. A
  new-shape department is indistinguishable to the runtime from a free-form one.
  Existing ~20 departments unchanged; CI stays green.
- **Phase 1 — ratchet on (new code forced now, old grandfathered).**
  `scripts/check_repo.py` gains a G-gate + `migration/saga-handler.allowlist`
  listing every current free-form department. On the list → old path allowed,
  ①②③ not required. Not on the list → must be new shape + must pass ①②③. **The
  allowlist can only shrink; a new department file that is not new-shape is red.**
  So "forbid other ways" is in force for all *new* code from day one.
- **Phase 2 — drain one department per PR (no deadline).** Each PR rewrites
  `departments/<dept>/main.lua` to `done/act`, deletes its allowlist line, and
  gets ①②③ for free. ~20 small, independently-mergeable PRs — ideal devloop
  dogfood. Order: leaf scan/tick reconcilers first (validate the shape), then
  `implement` / `merge` / `fix`. Allowlist monotonically shrinks: `20 → … → 0`.
- **Phase 3 — self-terminate (back to one form).** When the allowlist is empty,
  conformance flips: if the free-form path still exists in the engine or the
  allowlist file still exists, it goes **red**, forcing the final PR that deletes
  them. End state: only `std.department` exists. (Phase 3's engine-side loader
  rejection + the dynamic-delivery property tests are the substrate's job — see §8.)

## 6. Layering (per 分层归属 doctrine)

| Thing | Home | Phase |
|---|---|---|
| `department{done,act}` + ①②③ oracle + stateful-truth fixture | `std/saga.lua` (Tier S) | 0 |
| allowlist + ratchet G-gate | `scripts/check_repo.py` | 1 |
| per-department `done`/`act` | each department | 2 |
| delete free-form path; engine loader rejection; dynamic-delivery property tests | substrate (engine) | 3 |

## 7. `done` / `act` contract (precise)

- `done(event) -> boolean`. Pure, **read-only**, must re-derive from the fact
  source (git / gh / marker via primitives), must be durable (a fact that
  survives restart), must be neither too wide (#587) nor too narrow (#601). May
  call read-class primitives; **must not** perform any write-class effect.
- `act(event) -> any`. Runs only when `not done`. All side effects via primitives
  (`raise`, `spawn_codex*`, gh writes, marker writes). Must, on success, leave a
  durable fact such that a subsequent `done(event)` returns true (the closure
  property that makes ② pass). Failures propagate (caught by
  `wrap_pipeline_failure` → error fact → L1 DLQ → L2 triage).

## 8. Engine-side (out of scope for the Lua phase; recorded for completeness)

- **Routing fidelity under real delivery.** The Lua oracle uses
  `run_department`, which bypasses the router, so it cannot test "the real router
  delivered the namespaced event here" (#550 dynamic form). Mitigation now:
  routing becomes a *static* property (every raised queue has a consumer; every
  consumed queue is raised somewhere) checkable by `check_repo.py` across the
  graph without running the router. The dynamic at-least-once/restart delivery
  invariants are the **engine's own** test responsibility (loom + proptest +
  redb crash recovery), to be added in fkst-substrate, not here.
- **Physical "only one shape".** Until Phase 3 the prohibition is enforced by the
  CI ratchet; after promotion the engine loader rejects free-form `pipeline`
  physically (unrepresentable, not just gated).

## 9. Risks / open questions

- **R1 — departments that resist `done`/`act`.** Pure pollers that must act every
  tick: model as `done = false`, `act = raise candidate events`, with idempotency
  enforced at the *downstream consumer*. Validate this holds for all ~20 before
  committing to "only legal shape".
- **R2 — ③ key projection per queue.** "What counts as new" must be declared per
  consumed queue. Risk of getting it wrong = false greens. Mitigation: default to
  the existing `dedup_key` / version fields; require an explicit projection where
  those are insufficient.
- **R3 — 1000-line cap interaction.** Several `github-devloop` test files already
  sit at the 999–1000 line cap. Auto-generated ①②③ should *reduce* hand-written
  test volume; ensure the generator does not push files over the cap.
- **R4 — oracle fidelity of the stateful-truth fake.** If the fake diverges from
  real gh/marker semantics, ② can false-green. Mitigation: build the fake from
  the same primitives the real path reads; keep it minimal (records writes →
  serves reads), and treat any unmodeled command as fail-closed.

⟦AI:FKST⟧
