# Mature Built-In Software Workflows Design

Date: 2026-07-05

## Purpose

`github-devloop-workflow` needs a small built-in catalog for software-development workflows that are mature enough to run autonomously without inventing new lifecycle gates. The catalog must decompose only the few work shapes that are genuinely multi-step while preserving the existing `issue -> consensus -> PR-diff review -> CI -> merge` atom for every child issue.

## Governing Principle

XP and Continuous Delivery govern the design: every workflow step is a green, small, independently reviewable and mergeable vertical increment. The next step reads the prior merged result through `source_ref`.

TDD lives inside each child PR. Red-green-refactor is not modeled as separate workflow steps; each child writes its own tests and leaves them green. The built-in generators must not duplicate consensus, PR-diff review, CI, or merge gates because the existing devloop atom already provides them.

`no-changes` is fatal after deployment. If a child step produces no diff, the origin workflow becomes terminal blocked-with-WHY.

## Selection Model

`workflow_select` is a conservative router, not an eligibility oracle. It reads only the origin issue title/body and defaults to `none` unless that text unambiguously matches exactly one flow's `applies_when`.

There is no feasibility probe and no `EligibilityManifest`. A probe would create a second source of truth that predicts the same ground truth the first step already establishes: mergeable-diff evidence. Duplicating that authority would create drift.

## Eligibility Enforcement

The first step is the feasibility gate. If a flow is mis-selected because the issue text looked right but the repository has no architectural seam, is already fully tested, or has no such contract, the first child produces no changes. The deployed no-changes-fatal rule then blocks the origin with WHY after one honest child cycle.

The single ground truth is mergeable-diff evidence.

## Built-In Flows

### `software-feature-flow`

Method: Cockburn walking skeleton plus XP/CD vertical slice.

Applies when the origin title/body unambiguously asks to implement a new software capability or feature deliverable as one bounded slice with a real end-to-end path. It is not for epics, trackers, broad multi-slice features, trivial one-PR changes, bugfixes, docs-only work, or ambiguous requests.

Steps:

1. `walking-skeleton` — the thinnest executable end-to-end path wiring the major components for one tiny behavior. It compiles/runs and is green with a smoke or acceptance test. It proves the architecture and is not an empty scaffold.
2. `production-slice` — complete accepted behavior, edge cases, negative cases, and tests on top of the merged skeleton, all green.

### `software-refactor-flow`

Method: Feathers characterization tests plus Fowler behavior-preserving refactoring.

Applies when the origin title/body unambiguously asks to restructure or clean up existing code without changing observable behavior.

Steps:

1. `characterization-tests` — tests pinning the current externally observable behavior of the target code. These tests pass against current code and are tests-only.
2. `behavior-preserving-restructure` — internal restructuring on top of the merged characterization tests, keeping them and existing tests green with no observable behavior change.

### `software-contract-migration-flow`

Method: Fowler Parallel Change / expand-contract plus branch-by-abstraction.

Applies when the origin title/body unambiguously names an existing API, schema, event payload, CLI surface, adapter seam, or persisted format to change, replace, deprecate, or remove, where old and new must coexist temporarily.

Steps:

1. `expand` — add the new contract path plus backward-compatible adapter, dual-read-or-write path, or bridge, with executable contract tests. The old form still works.
2. `migrate` — move in-repo producers and consumers to the new form on top of the merged expand step, keeping tests green.
3. `contract` — remove the old form and temporary bridge, leaving the new form and green tests.

## Rejected Shapes

- Bugfix flow: a real bugfix is one green PR with the regression test and fix together.
- Design-contract or ADR step: mandatory ADR ceremony is not methodology-faithful; each child already gets consensus framing.
- Runtime variable-length or dynamic continuation: big features decompose at the origin level as separate issues, not inside one workflow.
- `feasibility_probe` / `EligibilityManifest`: these duplicate the first step's mergeable-diff evidence as a second source of truth.
- Spike, greenfield API, and docs flows: these are one PR or plain enable.

## Generator Contract

Every built-in step uses `content.kind = "generated"` in the same JSON blueprint format used by host catalogs. Each generator is source-agnostic: it reads the origin through `source_ref`, and non-first steps read the predecessor merged result through `source_ref`.

Each generator emits exactly one child issue title/body spec. It instructs the child to keep TDD/tests inside the child PR, stay green, and rely on devloop for consensus, PR-diff review, CI, and merge. It also names the no-changes fatal behavior so a mis-selected workflow fails fast through the deployed child result rule.

## Round Provenance

Rounds 1-2 converged on the two core flows, TDD inside children, and rejection of bugfix/ADR as separate workflow shapes.

Round 3 added eligibility gating and the contract migration flow.

Round 4 identified that eligibility is a repository fact, not a text fact, making a text-only eligibility gate uncheckable.

Round 5 resolved to Option F: conservative text router plus first-step-as-feasibility-gate, with no probe and one ground truth: mergeable-diff evidence.

⟦AI:FKST⟧
