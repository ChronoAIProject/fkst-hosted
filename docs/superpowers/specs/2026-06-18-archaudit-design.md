# Idle-Detector + Archaudit - Minimal Design Spec

## Intent
Replace the rejected heavy archaudit framework with two small packages: a generic idle signal and a bounded audit consumer.

The system does only this: when current engine facts say the system is idle, run one read-only codex architecture audit and file small evidence-backed issues through `github-proxy`.

## Prior Art
This is a tiny architecture fitness function run on an idle trigger.
Idle is a generic background-work signal: one producer may raise it, and each consumer must re-check whether it is still safe to act.

## Architecture
```text
cron idle_tick -> idle-detector.idle_gate -> system_idle
  -> archaudit.audit -> github-proxy.github_issue_create_request
```

All queues in this chain are reliable. Stale idle hints are handled by a consume-time freshness gate and current-idle re-check, not by ephemeral delivery.

Keeping `system_idle` reliable is a substrate-contract correction, forced by reliable `source_ref` propagation and schema-validation rules, not a product-scope expansion.

Both departments use `std.saga.department(spec, handlers)` with the head-spec shape: `local spec = { ... }` after `require` statements and before helper functions, then `saga.department(spec, handlers)`.

## Package 1: `idle-detector`
`idle-detector` is flat: bare own queues (`idle_tick`, `system_idle`), zero external package namespace references, and reads only engine observe/health facts plus `std`.

Layout:
```text
packages/idle-detector/
  core.lua
  raisers/idle_poll.lua
  departments/idle_gate/main.lua
  tests/*_test.lua
```

`raisers/idle_poll.lua` is a cron raiser producing `idle_tick`. Default interval is `30m`, configurable, and intentionally slow so it does not compete for resources.

`departments/idle_gate/main.lua` consumes reliable `idle_tick` and produces reliable `system_idle`. It first drops stale cron slots: `system_idle.detected_at` derives from the `idle_tick` cron slot or `event.ts`, not wall-clock processing time, and slots older than a small budget raise nothing. It then re-derives idleness from the existing framework `observe` / `health` surface only: queue depths, in-flight or leased deliveries, retry-pending counts, and DLQ/anomaly counts.

There is no `std.observe` adapter today. `idle_gate` uses a tiny local wrapper in `core.lua` that runs the framework observe surface, for example `fkst-framework observe --json` as used by `scripts/run.sh health`, through the engine exec primitive and parses the result. Tests inject facts at this seam and do not invoke real observe. Do not add engine Rust, new host facts, idle markers, or durable idle state.

Fail closed: any read failure, unknown or malformed facts, or nonzero busy signal skips terminal-with-structured-`WHY` and raises nothing.

On idle, raise only this tiny payload:
```lua
{
  schema = "idle-detector.system-idle.v1",
  detected_at = "<time>",
  source_ref = { kind = "host-observe", ref = "<stable observe slot>" },
  expires_at = "<optional time>"
}
```

No queue contents, metrics, audit intent, repo info, persisted idle state, idle marker family, or durable idle lifecycle. `core.lua` may contain only the idle predicate and payload builder.

## Package 2: `archaudit`
`archaudit` is composed. Its `fkst.toml` declares:
```toml
kind = "package.composed"

[event_deps]
packages = ["idle-detector", "github-proxy"]
```

Its only cross-package links are namespaced queues: consume `idle-detector.system_idle`, produce `github-proxy.github_issue_create_request`.

It is independent of `github-devloop`: it feeds ordinary issues into the existing intake path like a human filing an issue. It does not mirror intake, consensus, review, PR, merge, labels-as-state, or devloop lifecycle.

Layout:
```text
packages/archaudit/
  fkst.toml
  core.lua
  departments/audit/main.lua
  tests/*_test.lua
```

`departments/audit/main.lua` is the only department. This is one legitimate judgment pipeline, like `consensus.decide` or `autochrono.propose`: idle trigger -> freshness gate -> bounded codex judgment -> issue intents.

No second outbox department. `audit` raises issue-create requests directly. `core.lua` may contain only the codex prompt, strict parser, issue-request builder, and `dedup_key` helper.

## Audit Flow
1. Accept only queue `idle-detector.system_idle` and schema `idle-detector.system-idle.v1`.
2. Treat `system_idle` as a hint, not permission.
3. Drop if event age exceeds the freshness budget or `expires_at` is past.
4. Re-derive current idle from the same local observe wrapper; if unreadable or now busy, skip terminal-with-`WHY`.
5. Read `FKST_GITHUB_REPO`; missing or malformed repo is a structured failure and creates no issue.
6. Spawn one bounded read-only codex judgment, timeout about `8m` to `10m`.
7. Strictly parse JSON; validate only JSON shape, required fields, file existence, and line existence.
8. Cap accepted findings to `ARCHAUDIT_MAX_ISSUES_PER_IDLE`, default `3`.
9. For each non-duplicate finding, raise one `github-proxy.github_issue_create_request`.

Expected conditions, including stale or expired events, system now busy, or unreadable observe facts, are benign terminal skip-with-`WHY`: no issue and no retry storm. Failure conditions, including malformed or unknown schema or queue, missing or malformed repo, codex timeout or failure, malformed JSON, or validation failure, expose a narrow structured failure (`error_class`, `fingerprint`, `source_ref`, `WHY`) under expose-don't-swallow. Do not benign-return them as success; do not file issues.

## Codex Contract
Codex must read repository files and `CLAUDE.md` itself via `source_ref`; find only concrete architecture-doctrine violations; cite exact `file:line`; propose small local refactors; return at most `N` strict-JSON findings; and not edit files, run `gh`/`git`, invent rules, report vague smells, create umbrellas, group unrelated problems, or special-case big items.

Target violations include god-class, god-state, coupling, SRP, Demeter, DIP, and similar local drift.

Minimal finding schema:
```json
{"file":"packages/example/core.lua","line":42,"rule":"SRP","why":"Mixed validation and rendering responsibilities.","suggested_fix":"Extract rendering without changing queues or schema."}
```

## Issue Creation
Each accepted finding raises:
```lua
{
  schema = "github-proxy.issue-create.v1",
  repo = "<FKST_GITHUB_REPO>",
  title = "Archaudit: <file>:<line> <rule>",
  body = "<bounded file:line + rule + why + small fix + marker>",
  labels = { "archaudit" },
  dedup_key = "<stable hash of repo + file + line + rule>",
  source_ref = { kind = "repo-site", ref = "<repo>#<file>:<line>#archaudit-create-intent" }
}
```

If the `archaudit` label does not exist, use `labels = {}`. Label availability is not a gate.
The body includes hidden marker `archaudit-dedup: <dedup_key>` as human-visible evidence. Archaudit does not query this marker itself; duplicate suppression authority stays in `github-proxy` through payload `dedup_key` plus its own issue-create marker/search. Line-number dedup can duplicate after code moves; smarter recurrence is out of scope.

## Delivery Semantics
`system_idle` is reliable. Both consumptions are reliable: `idle_gate` consumes `idle_tick`, and `audit` consumes `idle-detector.system_idle`.

Do not mark any queue in this chain `ephemeral`.

Substrate contract rule: reliable events require `source_ref`, and schema validation rejects a department that consumes only ephemeral queues but produces to reliable downstream. Since `audit` produces reliable `github_issue_create_request`, it cannot consume `system_idle` ephemerally. Since `idle_gate` produces reliable `system_idle`, it consumes `idle_tick` reliably.

The engine reliable-delivery `source_ref` requirement is satisfied by the inherited upstream `source_ref` propagated through reliable cron -> `idle_tick` -> `system_idle` -> issue-create delivery. The explicit `source_ref` in the `github-proxy.github_issue_create_request` payload is package-level data for `github-proxy`, identifying the audited repo-site/create-intent, and is not relied on as an engine delivery-identity override.

A reliably delivered stale `idle_tick` is dropped by `idle_gate`; a stale `system_idle` is consumed, re-checked, skipped-with-`WHY`, acked, and not retried into a storm. Minor backlog risk after long archaudit downtime is acceptable because cron is slow, output is capped, dedup is external, and engine admission is bounded.

Default `M.spec.retry = false` for both departments. A tiny bounded retry is acceptable only for transient observe/codex infrastructure failures.

Reliable payloads carry only small control fields and `source_ref`; source code and other large content are fetched by codex from the repo.

## Anti-Flood
Exactly two controls:

1. Per-run cap: `ARCHAUDIT_MAX_ISSUES_PER_IDLE`, default `3`.
2. Duplicate suppression through existing `github-proxy` issue-create idempotency: payload `dedup_key` plus `github-proxy`'s own marker/search.

These prevent duplicate storms, not three new genuinely distinct plausible-but-wrong findings per run; that residual is accepted under the lightweight constraint, with no recurrence or validator machinery.

No local dedup database, recurrence counter, seen-finding ledger, ratchet, history table, or umbrella tracker.

## Out of Scope
Mechanical Python scanner; `check_repo_architecture.py`; detector registry; `check_repo_audit.py`; allowlist, ratchet, slicer, umbrella, finite manifest; rule-of-three, recurrence counting, guard graduation; `cycle_id` plus rank; evidence validator beyond JSON shape and file/line/required fields; consensus/oracle review before filing; v1/v2 phasing; held-out challenge suite; severity/confidence/category/owner/symbol/excerpt-hash/class-fingerprint taxonomies; second archaudit department; persistent idle state or durable idle replay; github-devloop lifecycle mirroring; label/mode gates beyond `github-proxy`'s `FKST_GITHUB_WRITE` dry-run posture.

## Tests
`idle-detector`: positive fake observe idle -> raises `system_idle`; negative busy/DLQ/observe failure -> terminal skip-with-`WHY`, no raise.

`archaudit`: positive fresh idle + mocked codex finding + no duplicate -> one issue-create request with correct `dedup_key`, `source_ref`, body, and marker. Negative stale idle, malformed/timeout codex, duplicate marker, and dry-run no real write.

Use fake observe facts at the local observe-wrapper seam, `fkst.test.mock_command` for codex, and `std.github_fake` / `std.testing.run_fake` for GitHub behavior. Pin `event.ts`/now and env fixtures, including freshness budget, `expires_at`, `FKST_GITHUB_REPO`, `FKST_GITHUB_WRITE`, and label availability, so tests do not depend on wall clock or host state. Each `*_test.lua` must satisfy G5 with at least one engine PASS.

Run `scripts/run.sh test idle-detector`, `scripts/run.sh test archaudit`, composed conformance over `idle-detector` + `archaudit` + `github-proxy`, and standard ratchets.

## Implementation Slices
1. `idle-detector`: tiny core, `idle_poll`, reliable `idle_gate`, positive/negative tests.
2. `archaudit`: deps, tiny core, reliable `audit`, freshness gate, bounded mockable codex, cap, issue-create request, tests.
3. Run package tests, composed conformance, and G5.
4. Replace the heavy archaudit design doc with this minimal two-package spec.

## Defaults
Cron `30m`; freshness budget `10m`; codex timeout `8m` to `10m`; max issues per idle event `3`; dry-run unless `FKST_GITHUB_WRITE=1`; repo from `FKST_GITHUB_REPO`; label `archaudit` if present else none; retry `false` unless tiny bounded retry is chosen.

⟦AI:FKST⟧
