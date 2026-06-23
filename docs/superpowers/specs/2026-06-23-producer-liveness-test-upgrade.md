# Test-system upgrade: producer-liveness via fire_raiser — every producer→consumer edge is trace-proven, not fixture-proven

Status: DESIGN — sshx adversarial (minimal/structural/delete codex triplet + ChatGPT Pro), converged `implement`.
Date: 2026-06-23
Scope: fkst-packages (the conformance + the migration); the engine `fkst.test.fire_raiser` is DONE (substrate).

## 1. Problem (the class, verified at source)

The only integration-test primitive was `run_department`, which **INJECTS a hand-built event** into one
department. So tests prove "given an IDEAL event the department logic runs" (CAN-RUN) but never "the REAL
declared raiser emits a payload its consumer ACCEPTS" (AUTO-TRIGGER / real wiring). This is the
«ideal-trigger/inject masks real-wiring-failure» class:

- **archaudit #1361**: the audit "passed its tests" (run_department injected an ideal payload) but in
  PRODUCTION every real cron tick was rejected — 0 issues. The first fix even RECURRED the class
  (it accepted the legacy-flat `audit_poll` but production namespaced graphs emit `archaudit.audit_poll`).
  Verification was the **painful** "wait 30min for the cron in the live supervise" or a **misleading**
  `scripts/run.sh run` manual path (which lacks the supervise shim PATH and falsely reports observe-unreadable).

The deeper fact (the operator's insight, 2026-06-23): we already BUILT the capability to test real wiring
deterministically (`fire_raiser`, deployed), yet were still verifying with the painful cron-wait / manual
path. **The test system itself must be upgraded so the real wiring is tested deterministically by default.**

Verified: a producer-liveness conformance is NOT yet implemented (the existing `liveness_contract.lua` is
the #762 RESTART-state liveness, not producer-liveness). Producer fixtures (archaudit, idle-detector) today
hand-build / inject.

## 2. Harness (prior art)

- **Consumer-driven contract testing** (Pact): the consumer is verified against what the producer ACTUALLY
  emits, not a hand-written stand-in.
- **"Don't mock what you are testing"**: the thing under test IS the producer→consumer wiring; the test must
  NOT substitute a synthetic payload for the real source emission.
- **Test pyramid**: `run_department`/`mock_command` are the UNIT layer (department logic given an event);
  the producer-liveness `fire_raiser` test is the INTEGRATION layer (the real source→route→consume edge).
- **Conformance-as-prevention** (CLAUDE.md «Harness 的本质», «discover→harness-ify»): make the masking
  structurally impossible, not merely test-able.

## 3. Design principle (the converged invariant)

> The unit of producer-liveness conformance is the **event-graph EDGE** (raiser `R` emits to queue `Q`
> consumed by department `D`), NOT a test file. **Every declared producer emission edge MUST have a
> `fire_raiser`-based test that asserts on the real TRACE** (the consumer accepted the real emitted payload
> and the expected downstream behavior). Conformance is **TRACE-PROVEN, not fixture-proven**: a
> synthetic-injected (`run_department`) fixture does NOT satisfy producer-liveness — so the masking class is
> structurally impossible.

Three layers (the unanimous triplet boundary; GPT Pro: "add a producer-conformance layer ABOVE the unit
tests; make it impossible for `run_department` to masquerade as proof of real wiring"):

| Layer | Owns | Must NOT |
|---|---|---|
| **Engine (substrate, DONE)** | `fire_raiser` fires the real raiser, emits the real payload, routes to the consumer, returns the trace | — |
| **Conformance (libraries, structural)** | the rule "every declared raiser/producer-edge has a `fire_raiser` test that asserts the trace" — uncovered edges (not on the shrink-only allowlist) fail CI | understand business schemas |
| **Package (the test)** | assert domain behavior on `trace.source_payload` / `consumer_result` / `raised` (accepted + the expected produce/skip); mock the consumer's external deps (observe/gh/codex) deterministically | hand-build the producer payload — it is REAL via `fire_raiser` |

## 4. The conformance rule (the PREVENT)

```
for every declared raiser R (emit edge R -> queue Q -> consumer dept D) in a package:
    if no test in that package calls fire_raiser("R") AND asserts on its trace
       (consumer_result, source_payload, or raised):
        if R is on migration/producer-liveness.allowlist:  WARN (shrink-only debt)
        else:                                               CONFORMANCE FAIL (CI red)
```

- **Trace-proven, not fixture-proven** (minimal/structural): the test must INSPECT the trace
  (`source_payload`/`source_ref`/`routed_to`/`consumer_result`/`raised`), not merely CALL `fire_raiser`
  (a no-op call must not satisfy it). A `run_department`-injected fixture does NOT count.
- **Structural, not semantic** (delete + GPT Pro): the conformance checks "this edge has a trace-asserting
  `fire_raiser` test", it does NOT parse business schemas — those domain assertions live in the package test.
- **Shrink-only allowlist → 0** (the harness-first ratchet): existing uncovered producers are listed once;
  the allowlist only shrinks; CI fails on growth or a new uncovered edge.

## 5. Deterministic real-behavior tests (eliminating the cron-wait / manual-run)

The package test fires the REAL raiser and asserts the REAL produce/skip behavior — no cron-wait, no
`scripts/run.sh run`:

```
local trace = t.fire_raiser("audit_poll")           -- REAL raiser, REAL namespaced payload (un-mockable)
-- mock ONLY the consumer's external deps, deterministically:
--   mock_command('fkst-framework observe ... --json', <idle facts>)   -- the engine observe
--   forge.github_fake set so no recent audit issue -> audit_due       -- the gh read
t.assert(trace.consumer_result == "accepted")        -- real payload accepted (catches #1361)
t.assert(<trace.raised contains github_issue_create_request>)  -- the audit PRODUCED (the real behavior)
```

**Honesty — mocks cannot lie about the wiring** (GPT Pro's deepest risk): the producer→consumer **payload is
REAL** (emitted by `fire_raiser`, not caller-supplied) — it is the wiring under test and is un-mockable.
Only the consumer's EXTERNAL DEPENDENCIES (observe/gh/codex) are mocked. So a green test cannot be a lie
about the producer→consumer edge; at worst a mocked dep is unrealistic, which is the existing
contract-test/ports-fake concern, orthogonal to the wiring guarantee.

## 6. Scope boundary (do NOT over-apply — «模式服务当前问题»)

- `run_department` + `mock_command` **STAY** for UNIT-level tests (department logic given an event) and
  adapter-contract tests. They are the base of the pyramid.
- `fire_raiser` + the producer-liveness conformance apply to the **producer→consumer wiring** (every
  declared raiser). Do not force `fire_raiser` onto pure unit tests or non-producer logic.
- Telemetry/fanout/shared queues with no single producer edge are out of the producer-liveness rule.

## 7. Coordination

The conformance lives in `libraries/testkit` (the test/conformance tooling library) — another machine is
refactoring `libraries` (contract split / testkit). Note the seam: add the conformance as an additive
testkit module + one registration in `scripts/check_repo.py` (minimal edit, rebase on the library refactor
if it lands first), do not touch the moved contract/std bodies. The engine `fire_raiser` is DONE (substrate).

## 8. Migration — harness-first, pilot → ratchet to 0

1. **Pilot (archaudit)**: migrate the archaudit producer-liveness test to `fire_raiser("audit_poll")` +
   the deterministic full-produce test (§5) — this also DETERMINISTICALLY answers "does the audit produce?"
   (the live in-flight validation; replaces the painful cron-wait). [Validates the pattern end-to-end.]
2. **Conformance + allowlist**: add the §4 conformance; seed `migration/producer-liveness.allowlist` with
   every currently-uncovered declared producer; CI fails on growth.
3. **Migrate**: convert each producer's liveness fixture (idle-detector, github-* producers, …) to a
   trace-asserting `fire_raiser` test; allowlist shrinks toward 0.
4. **At 0**: every producer edge is trace-proven; the masking class is structurally impossible; remove the
   allowlist scaffolding.

## 9. Non-goals

- Not replacing `run_department` (it stays for the unit layer).
- The conformance does not understand business schemas (structural only).
- No big-bang migration (pilot → ratchet).
- No engine change (fire_raiser is done); this is fkst-packages (conformance + tests).

## 10. Adversarial record

`sshx`: minimal/structural/delete codex triplet + ChatGPT Pro, all `propose`/`implement`.

- **minimal**: uncovered declared raisers (not on the shrink-only allowlist) fail CI; the package assertion
  must inspect `trace.source_payload`/`source_ref`/`routed_to`/`consumer_result`/`raised`, not merely call
  fire_raiser.
- **structural**: assert domain behavior on `trace.source_payload`/`consumer_result`/`raised`; synthetic
  injected payloads no longer satisfy producer-liveness conformance, so ideal-trigger masking is prevented.
- **delete**: ratchet to zero; domain tests assert real produce/skip by firing the real raiser and mocking
  observe/gh/codex deterministically — eliminating cron waits and manual `scripts/run.sh` verification.
- **ChatGPT Pro**: producer conformance is TRACE-PROVEN not fixture-proven; the required unit is the
  event-graph EDGE, not a test file; add the layer ABOVE the unit tests so `run_department` cannot masquerade
  as real-wiring proof; keep tests honest by making the producer→consumer payload real (un-mockable) and only
  mocking the consumer's external deps.

```
[goal: producers' REAL wiring tested deterministically, masking class impossible]
   │ resolved-by
   ▼
[conformance unit = event-graph EDGE (R->Q->D), trace-proven] ◀─agree─ ChatGPT Pro + minimal/structural/delete
   │ depends-on                                                │ (not a test file; not fixture-proven)
   ▼                                                           │
[every declared producer edge MUST have a fire_raiser trace-asserting test] ──PREVENT──▶ [run_department-inject does NOT satisfy → masking impossible]
   │ enabled-by (DONE)                                         │ depends-on
   ▼                                                           ▼
[engine fire_raiser: real raiser→real payload→consumer→trace]   [honesty: payload REAL/un-mockable; only consumer deps mocked]
   │ boundary
   ▼
[layer ABOVE run_department (unit stays); shrink-only allowlist→0]
```

Meta-judge `implement`: unanimous on the edge-as-unit + trace-proven + layer-above-unit + shrink-ratchet
boundary; ChatGPT Pro's "trace-proven not fixture-proven" + "edge not file" is the keystone; the honesty
property (real un-mockable payload) closes the mock-lie risk. Pilot validates end-to-end (archaudit).

⟦AI:FKST⟧
