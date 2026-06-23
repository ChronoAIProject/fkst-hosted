# Cross-package integration-test harness design

Status: design (2026-06-24). Driven by a 4-perspective adversarial analysis (sshx codex minimal/structural/delete + ChatGPT Pro), unanimously converged. ⟦AI:FKST⟧

## Problem (verified gap)

The system has three test layers, all package-local or single-hop:

1. **Department-level** (77 files): `testkit.testing.run_fake(dept, event)` / `t.run_department(path, event)` run ONE department in-process with fakes, asserting its raises. (`libraries/testkit/testing.lua`, substrate `test_runner.rs`.)
2. **`fire_raiser`** (substrate `test_fire_raiser.rs`): 1-HOP — runs a raiser, routes its events through the real `delivery_router`, runs each direct consumer ONCE, captures the consumers' raises as a **trace** assertion. It does NOT re-route those raises, so it does not exercise multi-hop flow.
3. **Composed-graph conformance**: validates the STATIC graph (`produces`/`consumes` wiring, `published_seam` cross-package produce constraints — `graph_scan.rs`), not runtime behavior.

**No deterministic cross-package, multi-hop, full-flow test exists.** The transitive, stateful composition of the ~6-package lifecycle (issue→consensus→ready/implement→PR→review→merge) under the substrate's **real delivery semantics** (ordering, durable redelivery, version-CAS guards, fail-closed decisions) is validated ONLY by **live dogfood** against real GitHub — which is not a deterministic test.

This is exactly where locally-correct handlers compose incorrectly. The recent ②③④ refactor regressions (lost write-time guard, wrong version ordering, fail-open vs fail-closed) **passed all unit/department tests + CI green** and were caught only by adversarial code review + live dogfood. That is the class this harness must catch deterministically.

## Design (converged 忠于本质 shape)

An **engine-owned, deterministic "run the event graph to quiescence" test runtime** that reuses the REAL production machinery — a *controlled mode of substrate*, NOT a second router. Prior art: Temporal's test framework (real workers + test service + time-skipping) and FoundationDB's deterministic simulation (whole system in one process under synchronized simulated time). Borrow the principles at this system's much smaller scale.

```
 repository-lifecycle scenario (fixtures + injected source event)
        │
        ▼
 substrate-testkit :: TestRuntime          ← engine-owned, deterministic
        ├── real graph registration/scan
        ├── real DeliveryRouter::publish   (delivery_router.rs:86)
        ├── real event envelopes + source_ref enforcement
        ├── real redb durable queue (temp root)
        ├── real lease / dispatch / ack / retry / DLQ (consumer.rs)
        ├── real raised-event re-publish (consumer.rs:445)
        └── real Lua handlers from every package in the graph
                │  drives deliveries single-threaded, sorted, to quiescence
                ▼
        fake gh / git / codex  (existing forge.*_fake + mock_command)
```

### Three-way ownership

- **substrate-core** exposes injectable sources of nondeterminism: clock (virtual / time-skip), identifier source, scheduler/dispatch ordering, and delivery fault hooks. (The router/event/source paths currently read wall-clock — `delivery_router.rs:619`, `event.rs`, `source_runner.rs:341` — so clock control is necessarily a substrate concern.)
- **substrate-testkit** (engine-owned, beside the engine because only it understands delivery readiness/retries/acks/timers) owns `TestRuntime`: queue driving, **quiescence detection**, bounded `max_steps`/`max_deliveries`, restart support, tracing, fault injection. Exposed to Lua as a test SDK surface (e.g. `fkst.test.run_graph(...)`).
- **packages `testkit`** owns ergonomic scenario builders, the gh/git/codex fakes, fixtures, and domain-level milestone/trace assertions. It **must not route events itself** (re-implementing the router is the failure mode below).

### Determinism contract

Temp redb durable root; injected virtual clock with explicit time-skip; single-threaded sorted delivery dispatch; bounded `max_steps`/`max_deliveries` (no scheduler waits, no sleeps, no wall-clock polling, no real network); external `gh`/`git`/`codex` only via existing fakes/`mock_command`, unmocked effects fail-closed; reliable delivery already enforces `source_ref` + payload bounds. A scenario run must be byte-reproducible.

## Scope

### NOW (this effort)
- **substrate**: the `TestRuntime` primitive + injectable clock/ordering + the `fkst.test.run_graph` (name TBD) Lua surface. (fkst-substrate PR — engine change.)
- **packages**: thin `testkit` scenario helpers + **2–4 keystone smoke tests** (fkst-packages PR, depends on the substrate primitive):
  1. **autochrono tracer-bullet**: source issue → `consensus.proposal` → fake consensus approval → `reply` (smallest real multi-hop).
  2. **github-devloop thin path**: issue observe → consensus result → ready/implement → open PR → review approve → merge-ready/merge **dry-run**, with fake gh/git/codex.
  3. **negative bounded-flow**: no-consensus/converge reaches deterministic `reconcile`/`blocked` (not livelock).
  4. (optional) one fail-closed / DLQ / missing-`source_ref` case.
- Each of the recent ②③④ regressions also gets its **narrowest department-level** regression test — the system test is an *additional* defense, not a substitute for local invariant tests.

### DEFER (with trigger)
Exhaustive full issue→merge lifecycle matrix, concurrency/races, real-supervise/real-GitHub reproduction, long CI timing. **Trigger to expand**: the next dogfood-only multi-hop regression escapes, or any PR touches queue wiring / reliable delivery / `source_ref`/dedup propagation / consensus-loop routing / PR review / merge gating.

## The single ugliest risk + how it is made unrepresentable

**The harness becomes a second router/supervisor.** If the test runtime re-implements routing/durable/raise semantics (e.g. in Lua testkit by chaining `fire_raiser` hops), it tests a cleaner fiction and yields false greens — duplicating the source of truth. **Mitigation (mechanical):** `TestRuntime` must call the production `DeliveryRouter::publish`, `DeliveryStore`, raise-authority, derived raised-event publish, and ack/retry paths directly — it adds only a deterministic *driver* (clock, ordering, quiescence loop) around them. A conformance check should assert the test runtime constructs no parallel routing implementation.

## Substrate vs packages split (cross-repo)

- **fkst-substrate PR**: `substrate-testkit::TestRuntime` + injectable nondeterminism in substrate-core + the `fkst.test.run_graph` Lua surface + substrate's own Rust tests of the runtime (deterministic, quiescence, fault injection). Engine change → substrate repo, per repo doctrine.
- **fkst-packages PR**: `testkit` scenario builders + the 2–4 keystone smoke tests + the dept-level regression tests. Depends on the substrate primitive being available (substrate-ref bump).

## Acceptance

- A keystone smoke test that, run against the pre-fix ②③④ code, would have **failed deterministically** (e.g. the merge-ready→reviewing stale-label case, or the no-consensus livelock) — proving the harness catches the dogfood-only class.
- Two runs of the same scenario produce identical traces (determinism).
- The runtime reuses production router/store/publish (no parallel implementation); conformance/asserts enforce it.
- Existing test layers unchanged; the new layer is additive.
