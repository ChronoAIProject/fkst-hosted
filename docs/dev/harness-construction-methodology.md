# Harness Construction Methodology — building a closed, fail-closed harness for any requirement

A *harness* is the pairing of (1) a CLOSED governing theory for a class of problem with (2) MECHANICAL, fail-closed enforcement that forces every violation out of the system instead of trusting discipline. The system's recurring success pattern is: take a requirement, anchor it on a theory that can be CLOSED (finitely enumerated), then make the engine/tests/runtime/deploy MECHANICALLY reject anything outside that closure. This document fixes that pattern into a repeatable methodology the system applies to itself -- at research/design time, at test time, and at deploy time. Reference frames it composes: shift-left, defense-in-depth, design-by-contract, fail-closed boundary mediation, and Gödel/halting humility (a system cannot statically prove all properties about itself).

## 1. The eight-stage loop

1. **FRAME** -- name the governing theory / prior art / industry best practice that dominates this class of problem, and state any DELIBERATE deviation + why.

   No non-trivial requirement proceeds without a named reference frame. Mechanized today by the harness-first doctrine in `fkst-packages/CLAUDE.md:54`: before execution, identify the mature theory / industry practice / prior art, explain the deliberate deviations, and make the judgment pipeline challenge claims of novelty before existing practice is ruled out (`fkst-packages/CLAUDE.md:56`).

2. **CLOSE** -- convert the requirement into a FINITE, enumerated, deny-by-default contract.

   The closed contract enumerates legal states, terminal states, legal queues, legal source kinds, legal SDK surface, required facts, output obligations, and forbidden defaults. Closure means: anything not enumerated is illegal. Mechanized today by:

   - Static graph shape through `M.spec` (`consumes`, `produces`, `fanout`, `stall_window`) as package doctrine in `fkst-packages/CLAUDE.md:16`.
   - `serde(deny_unknown_fields)` for graph-scan declarations in `fkst-substrate/crates/fkst-framework/src/supervise/graph_scan.rs:27` and `fkst-substrate/crates/fkst-framework/src/supervise/graph_scan.rs:44`.
   - The fixed Lua SDK surface in `fkst-substrate/SPEC.md:71` and `fkst-substrate/SPEC.md:73`.
   - SAGA/liveness closure requiring every NON-terminal state to declare `output_obligation`, a positive `budget`, and `on_timeout` in `packages/github-devloop/core/liveness.lua:27`, with the specific non-terminal checks at `packages/github-devloop/core/liveness.lua:41`, `packages/github-devloop/core/liveness.lua:44`, and `packages/github-devloop/core/liveness.lua:47`.

3. **BOUND / TIER** -- decide WHERE each invariant can actually live and be decided.

   The valid tiers are constitution/identity tier, engine/package-behavior tier, test-only contract, runtime-only guard, and deploy/operator policy. DELETE anything outside its boundary. This is the binding correction from the "delete" perspective: NOT every invariant deserves static conformance; forcing a dynamic/environmental property into a static gate is enforcement theater. Mechanized today by:

   - Tier I/II/III identity rules in `fkst-substrate/SPEC.md:9`, `fkst-substrate/SPEC.md:11`, and `fkst-substrate/SPEC.md:12`.
   - The commitment that `conformance` is a non-overridable gate in `fkst-substrate/SPEC.md:50`.
   - The rule that conformance reads files and Lua graph but does not become a workflow engine in `fkst-substrate/SPEC.md:56`, `fkst-substrate/SPEC.md:58`, and `fkst-substrate/SPEC.md:59`.
   - The substrate discipline that Tier II must stay small and evidence-backed in `fkst-substrate/SPEC.md:88` through `fkst-substrate/SPEC.md:92`.

4. **PLACE THE STRONGEST REAL GATE (shift-left)** -- put each invariant at the earliest layer that can ACTUALLY decide it.

   Static/conformance is correct when the property is finite and statically inspectable. Test gates are correct when the property is behavioral. Runtime fail-closed guards are correct when the property is dynamic, environmental, or undecidable. Deploy/branch-protection is correct when the property is an operational policy. Earlier is better, but never place a gate at a layer that cannot decide the property.

   Mechanized today, static layer:

   - Host conformance runs `runtime-layout`, `project-layout`, `locale-catalogs`, `graph-scan`, `department-non-empty`, and `schema-validation` in `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:32` through `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:83`; the same check set is declared in `fkst-substrate/SPEC.md:56` and `fkst-substrate/SPEC.md:57`.
   - `schema-validation` delegates to `fkst_common::validation::validate` in `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:180`.
   - `validate` rejects undeclared consumed/produced queues in `fkst-substrate/crates/fkst-common/src/validation.rs:111` through `fkst-substrate/crates/fkst-common/src/validation.rs:128`.
   - `validate` rejects missing department Lua files in `fkst-substrate/crates/fkst-common/src/validation.rs:137` through `fkst-substrate/crates/fkst-common/src/validation.rs:149`.
   - `validate` rejects invalid `stall_window` in `fkst-substrate/crates/fkst-common/src/validation.rs:150` through `fkst-substrate/crates/fkst-common/src/validation.rs:159`, and rejects bad retry declarations (zero `max_attempts`, non-positive `base`/`cap`, `cap < base`) in `validate_retry_decl` at `fkst-substrate/crates/fkst-common/src/validation.rs:258` through `fkst-substrate/crates/fkst-common/src/validation.rs:281`.
   - `validate` rejects isolated queues in `fkst-substrate/crates/fkst-common/src/validation.rs:197` through `fkst-substrate/crates/fkst-common/src/validation.rs:206`.
   - `validate_queue_contract` rejects multi-consumer or feedback queues unless `fanout = true` in `fkst-substrate/crates/fkst-common/src/validation.rs:227` through `fkst-substrate/crates/fkst-common/src/validation.rs:255`.

   Mechanized today, runtime layer:

   - Reliable delivery requires `FKST_DURABLE_ROOT`, source-derived `SourceRef`, lease/fencing, retry/backoff, and DLQ in `fkst-substrate/SPEC.md:79`.
   - `spawn_codex_sync` and `spawn_codex` carry a wall-clock timeout cap in `fkst-substrate/SPEC.md:80`.
   - Boundary resources are enumerated in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:37` through `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:83`; `exec_sync` and codex timeout are part of the `wall-clock` adapter entry at `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:75`.
   - Package-side concurrency and CAS are expressed through `with_lock`, trusted markers, and version ordering in `fkst-packages/CLAUDE.md:18`.

5. **FORCE-OUT** -- the gate must hard-fail, refuse progression, throw, timeout, dead-letter, or block.

   A documentation warning is NOT enforcement. The gate is non-overridable. Mechanized today by:

   - Conformance returning exit status `1` on any failed check in `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:75` through `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:83`.
   - `HostCheck::fail` emitting `FAIL` status rows in `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:206` and `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:214` through `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:220`.
   - Graph validation rejecting bad graphs before they can run, through `fkst-substrate/crates/fkst-common/src/validation.rs:77` and its rejection sites.
   - The substrate doctrine rejecting warning-only interfaces: "do not use documentation warnings instead of narrowing the interface" in `fkst-substrate/CLAUDE.md:149`.

6. **REGISTER** -- record the invariant as a first-class registry row.

   A registry row names: `name`, `reference_frame`, `tier`, `allowed/forbidden`, `enforcement_layer + gate id`, `negative_case`, and `verification_command`. This makes "the Nth occurrence" visible for Rule of Three (`fkst-packages/CLAUDE.md:61`) and makes the meta-gate machine-checkable.

   Mechanized today only partially:

   - The indexed-registry loader pattern already rejects unsorted, duplicate, and mismatched entries in `packages/github-devloop/core/registry.lua:13` through `packages/github-devloop/core/registry.lua:35`, `packages/github-devloop/core/registry.lua:61` through `packages/github-devloop/core/registry.lua:87`, and `packages/github-devloop/core/registry.lua:106` through `packages/github-devloop/core/registry.lua:121`.
   - Restart/liveness transition rows already use this pattern in `packages/github-devloop/core/restart.lua:18`, `packages/github-devloop/core/restart.lua:20`, and `packages/github-devloop/core/restart.lua:61`.
   - Transition row files exist under `packages/github-devloop/core/restart/transitions/*.lua` and are indexed by `packages/github-devloop/core/restart/transitions/index.lua`.

   This stage is a PROPOSED first-class artifact today. The loader and row discipline exist; a dedicated invariant registry does not yet.

7. **META-GATE (self-apply)** -- a change is INCOMPLETE unless it names its invariant and lands or references its enforcing gate.

   A proposal/review answer must name its `invariant`, `reference_frame`, and `tier`, then land or reference the enforcing gate. If the property is runtime-only, the answer must declare that escape hatch and justify why static/test/deploy cannot decide it. The methodology PRESCRIBES that the judgment pipeline -- intake / consensus / review -- enforce this predicate and reject `enforcing_gate=none` unless the change is explicitly trivial, and it applies to ITSELF. **Status:** today this holds only at the doctrine level (the "Mechanized today by" anchors below); the exact seven-field predicate (Section 4) is the prescribed mechanization and is NOT yet a built field gate.

   Approximated today (doctrine level) by:

   - The substrate completeness predicate: a change is complete only when it has evidence for any trusted-base expansion, keeps tier boundaries, records facts in git/filesystem/fcntl, has classifiable failures, and has applicable tests or conformance in `fkst-substrate/CLAUDE.md:153`.
   - The package judgment pipeline doctrine: gates are codex judgment pipelines plus event flow, not per-event human labels, in `fkst-packages/CLAUDE.md:68`.
   - The consensus/review flow for `github-devloop`, where issue design, implementation, PR review, fix, and merge each move through marker-backed state and fail-closed gates in `fkst-packages/CLAUDE.md:11` and `docs/dev/devloop-design.md:110` through `docs/dev/devloop-design.md:121`.

8. **HONEST GAP (Gödel humility)** -- the system can FORCE declared invariants but cannot PROVE it has discovered all relevant ones.

   The self-applying layer therefore stays biased toward: "missing gate = incomplete change." Consensus/review must always ask: what is the NEXT invariant class we have not yet named? Static conformance is for finite inspectable contracts; runtime fail-closed is for undecidable/environmental contracts; human/consensus judgment is for choosing the theory, the scope, and the acceptable loss of evolvability.

## 2. Same loop, three phases

| Phase | Where the eight-stage loop lands |
| --- | --- |
| R&D / design | `FRAME`, `CLOSE`, and `BOUND` happen in the proposal. Today the gate is harness-first consensus/review meta-judgment, which challenges prose-only plans and unframed novelty at the doctrine level. The exact `META-GATE` predicate (reject `enforcing_gate=none`, require named deviations) is the PRESCRIBED hardening of that gate (Section 4), not yet a built field gate. Anchors: `fkst-packages/CLAUDE.md:54`, `fkst-packages/CLAUDE.md:56`, `fkst-packages/CLAUDE.md:68`. |
| Test | The gate is executable contract tests (`*_test.lua`) plus conformance. Behavioral invariants land here. `G5` -- every `*_test.lua` yields at least one engine-enumerated PASS -- is the meta-check that the test gate itself is real. Anchors: `fkst-packages/CLAUDE.md:73`, `scripts/run.sh:142` through `scripts/run.sh:194`, and `fkst-substrate/SPEC.md:82`. |
| Deploy | The gate is CI green plus branch protection / required status checks server-side. Irreversible actions such as merge require the strongest real gates: trusted marker, independent review approval, head-bound proof, CI/mergeability, and server-side branch protection. Anchors: `.github/workflows/ci.yml:62` through `.github/workflows/ci.yml:71`, `fkst-packages/CLAUDE.md:32`, and `docs/dev/devloop-design.md:121`. |

It is the SAME eight-stage loop each time; what shifts per phase is chiefly the gate layer selected at stage 4 -- and with it the authority surface: proposal/review meta-judgment at R&D, executable tests + conformance at test time, CI + server-side branch protection at deploy.

## 3. Gate-layer map for this system

1. **Static / conformance gate**

   The concrete mechanisms are `fkst-framework conformance`, graph scan, and schema validation. Conformance check ids are enumerated in `fkst-substrate/SPEC.md:56` through `fkst-substrate/SPEC.md:58` and implemented in `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:32` through `fkst-substrate/crates/fkst-framework/src/host_conformance.rs:83`. Graph scan owns the static `M.spec` surface and deny-unknown fields at `fkst-substrate/crates/fkst-framework/src/supervise/graph_scan.rs:6`, `fkst-substrate/crates/fkst-framework/src/supervise/graph_scan.rs:27`, and `fkst-substrate/crates/fkst-framework/src/supervise/graph_scan.rs:44`. Schema validation rejects orphan queues, undeclared queues, missing department Lua, bad `stall_window`, bad retry, and multi-consumer-without-fanout in `fkst-substrate/crates/fkst-common/src/validation.rs:77` through `fkst-substrate/crates/fkst-common/src/validation.rs:255`.

2. **Test gate**

   The concrete mechanisms are `*_test.lua`, `fkst-framework test`, `fkst.test.mock_command`, `fkst.test.command_calls`, `fkst.test.run_department`, and `scripts/run.sh test`. Package doctrine names `scripts/run.sh test` as the single local and CI entrypoint in `fkst-packages/CLAUDE.md:73`. Engine test mode mocks external command boundaries and fails closed on unmocked calls in `fkst-substrate/SPEC.md:82`. `scripts/run.sh` uses `--report-json` as the authoritative tally and enforces `G5` coverage in `scripts/run.sh:11` through `scripts/run.sh:13` and `scripts/run.sh:142` through `scripts/run.sh:194`.

3. **Runtime gate**

   The concrete mechanisms are version-CAS, trusted markers, `with_lock`, lease+fencing, wall-clock timeout, retry+backoff, DLQ, and source-ref re-derivation. Package doctrine anchors marker trust, version order, and `with_lock` in `fkst-packages/CLAUDE.md:18`. Reliable delivery anchors lease/fencing, retry/backoff, DLQ, `source_ref`, and `FKST_DURABLE_ROOT` in `fkst-packages/CLAUDE.md:20` through `fkst-packages/CLAUDE.md:24` and `fkst-substrate/SPEC.md:79`. Codex wall-clock timeout is fixed in `fkst-substrate/SPEC.md:80`. Boundary resources are enumerated in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:37` through `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:83`.

4. **Deploy gate**

   The concrete mechanisms are CI workflow, branch protection, required status checks, and merge-time head binding. CI builds the engine and runs `scripts/run.sh test` in `.github/workflows/ci.yml:62` through `.github/workflows/ci.yml:71`. Package doctrine requires CI green for PR merge in `fkst-packages/CLAUDE.md:84`. `github-devloop` merge requires trusted `review-result:v1 approve`, trusted head-bound `merge-ready:v1`, same head, CI/mergeability, `--match-head-commit`, and server-side branch protection in `fkst-packages/CLAUDE.md:11`, `fkst-packages/CLAUDE.md:32`, and `docs/dev/devloop-design.md:121`.

## 4. The meta-gate, made executable

Under the prescribed meta-gate (not yet built; see Status below), the judgment pipeline accepts a proposal/review answer only if it carries these exact fields:

| Field | Required meaning |
| --- | --- |
| `reference_frame` | The named governing theory / prior art / industry practice. |
| `invariant` | The closed property being protected. |
| `tier` | One of: constitution/identity, engine/package-behavior, test-only, runtime-only, deploy/operator policy. |
| `enforcement_layer` | Static/conformance, test, runtime, deploy, or explicit runtime-only escape. |
| `enforcing_gate` | Concrete gate id, file, check, test, runtime guard, CI rule, or branch-protection rule. |
| `negative_case` | The violation that must fail. |
| `verification_command` | The command or operational check proving the gate fails the negative case. |

Empty or absent `enforcing_gate` means revise, not approve, unless the change is explicitly trivial. A runtime-only escape is acceptable only when it names the reason static/test/deploy cannot decide the property and names the runtime guard that blocks, times out, retries, DLQs, or refuses progression.

**Status:** this is the mechanization the methodology prescribes for itself; it is NOT enforced today. The seven-field predicate is a candidate for a future conformance/prompt change filed through self-drive. This document is doc-only and does not build that gate; today's enforcement is the doctrine-level harness-first review and the substrate completeness predicate referenced in stage 7.

## 5. Worked counter-example: fkst-packages#521

`fkst-packages#521` is the canonical missing-harness regression. The implicit invariant "EVERY external call must be time-bounded" existed only as convention: adapters often defaulted GitHub calls to a timeout, for example `packages/github-devloop/core/base.lua:895` through `packages/github-devloop/core/base.lua:914` and `packages/github-proxy/core/gh_rate.lua:15` through `packages/github-proxy/core/gh_rate.lua:31`. But there was NO universal gate. A codex call held a per-entity `with_lock` across a slow `gh` read; combined with the engine per-department dispatch cap of `1`, this head-of-lined observe/intake and froze the pipeline.

Stage-by-stage, what should have happened:

1. **FRAME** -- the governing theory should have been the boundary-resource axiom: every boundary access is enumerated, mediated, metered, budgeted, and timed out. The current engine registry already names this class in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:1` through `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:5`, and enumerates `codex.process`, `shell.process`, `git.process`, `runtime.filesystem`, and `wall-clock` in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:37` through `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:83`.

2. **CLOSE** -- "external call" is a finite, statically enumerable surface: `exec_sync`, `gh_exec`, `spawn_codex_sync`, `spawn_codex`, git SDK calls, and package adapters. The fixed Lua SDK surface is named in `fkst-substrate/SPEC.md:73`; codex timeout is named in `fkst-substrate/SPEC.md:80`; boundary-resource adapters are named in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:41`, `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:50`, `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:59`, and `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:77`.

3. **BOUND / TIER** -- this is a runtime-only + static-scan invariant. The static scan can find raw external calls and require a positive bounded timeout. The actual bound is enforced at runtime by the adapter timeout, because call duration is environmental and cannot be decided by conformance alone.

4. **PLACE THE STRONGEST REAL GATE** -- the correct placement is both:

   - A static scan rejecting any `exec_sync`, `gh_exec`, `spawn_codex_sync`, `spawn_codex`, or adapter-wrapped external call lacking a positive bounded timeout.
   - The runtime wall-clock timeout on the external process itself, already expressed for codex in `fkst-substrate/SPEC.md:80` and for boundary wall-clock budget in `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:75` through `fkst-substrate/crates/fkst-framework/src/boundary_resource.rs:80`.

5. **FORCE-OUT** -- conformance/CI should fail the build when the scan finds an unbounded external call. A warning or prompt reminder would not satisfy the harness. This follows the non-overridable conformance rule in `fkst-substrate/SPEC.md:50` and the rejection of warning-only interfaces in `fkst-substrate/CLAUDE.md:149`.

6. **REGISTER** -- add one registry row:

   - `name`: `external-call-timeout`
   - `reference_frame`: `boundary-resource axiom`
   - `tier`: `runtime-only + static-scan`
   - `allowed/forbidden`: allow external calls only through enumerated adapters with a positive bounded timeout; forbid raw/unbounded external calls.
   - `enforcement_layer + gate id`: static scan + runtime timeout.
   - `negative_case`: raw `exec_sync("gh ...")` or `spawn_codex_sync({ prompt = ... })` without a positive timeout.
   - `verification_command`: a proposed `external-call-timeout` static check wired into `scripts/run.sh check <pkg>` (and ultimately `fkst-framework conformance`), shipped with a negative fixture -- an `exec_sync`/`gh_exec`/`spawn_codex_sync` call lacking a positive timeout -- that the check must report and fail on. (No such check exists yet; this names the command the registry row would carry once built.)

7. **META-GATE** -- the #521 fix is incomplete until it lands or references that scan. `enforcing_gate=none` should force revision, because the prior failure was not a missing convention; it was a missing harness.

8. **HONEST GAP** -- the deeper next-class invariant is: no slow boundary call inside a held cross-entity lock. The external-call timeout gate bounds the first failure class; it does not prove the system has discovered every lock/resource interaction. The gap-bias should surface the next registry row rather than claiming the class is closed forever.

This example is WHY the methodology exists: a convention existed, local adapters mostly followed it, but no mechanical gate forced the class closed.

## 6. The invariant registry (PROPOSED artifact)

The proposed first-class invariant registry row shape is:

| Field | Meaning |
| --- | --- |
| `name` | Stable invariant id, sorted in an index. |
| `reference_frame` | Governing theory / prior art / industry practice. |
| `tier` | Constitution/identity, engine/package-behavior, test-only, runtime-only, deploy/operator policy, or a justified combination. |
| `allowed/forbidden` | Finite contract: what is legal and what is illegal. |
| `enforcement_layer + gate id` | Concrete static/test/runtime/deploy gate. |
| `negative_case` | Minimal violation that must fail. |
| `verification_command` | Command or operational check proving the negative case fails. |

The registry should reuse the existing sorted/dedup/mismatch-rejecting loader pattern in `packages/github-devloop/core/registry.lua:13` through `packages/github-devloop/core/registry.lua:35` and `packages/github-devloop/core/registry.lua:61` through `packages/github-devloop/core/registry.lua:87`. The precedent is the restart/liveness transition registry loaded in `packages/github-devloop/core/restart.lua:61`, with rows under `packages/github-devloop/core/restart/transitions/*.lua`. This artifact is PROPOSED / self-drive candidate, not yet built.

The methodology's own success criterion is that a reviewer can mechanically answer, for any change: "what invariant, what theory, which tier, which gate, what fails on violation?" An unanswerable change is incomplete by definition.

⟦AI:FKST⟧
