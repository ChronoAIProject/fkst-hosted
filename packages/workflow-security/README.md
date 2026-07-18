# workflow-security

`workflow-security` is a **genuinely multi-step security-review adapter** built on the
shared `workflow.engine.*` kernel. It runs a four-step review pipeline over a repository and
files findings as GitHub issues. It copies **no** engine logic — the branching / idempotency
/ frontier / marker / CAS-key machinery lives once in the kernel and is reached through
`bindings.lua`.

## What it does

Given a review request, it drives the built-in `security-review` `fkst.workflow.v1` template:

1. **profile-stack** (generated) — enumerate declared dependencies from manifests/lockfiles.
2. **match-dependencies** (generated) — match them against GitHub Security Advisories.
3. **audit-code-tests** (generated) — audit code, tests and security best practices.
4. **file-findings** (generated, final) — consolidate into a strict findings array, which the
   executor files as dedup-idempotent `github-proxy` issues labelled `fkst-security`.

Each step is a codex **analysis** run (the codex runner is injected). Per-step completion
advances the frontier; on terminal-done the findings are filed. Re-reconcile never
double-files: each step is idempotent on `materialization.child_dedup_key`, and every filed
issue carries a per-finding dedup key that `github-proxy` de-duplicates again.

## It claims its OWN work (never the dev intake seam)

`workflow-security` is **not** part of the development topology. It never consumes
`github-devloop-intake.devloop_intake_candidate` (the single-consumer INTAKE_POLICY_SET
invariant stays intact). It claims work through its own path only:

- the `fkst-security` **label** on a review issue (discovered during polling), and
- its own cron tick `raisers/security_poll.lua` (`workflow_security_tick`) + its own
  `security_review_request` queue.

Its marker namespace is `fkst:workflow-security`, so its issue markers never collide with a
co-resident adapter's.

## Network-egress decision (step 2): option (c) — zero new egress

There is no generic outbound-HTTP capability in the repo, and step 2 needs an online vuln
index. This adapter takes the **recommended zero-new-egress fallback**: the
`match-dependencies` codex step queries the **GitHub Security Advisories** REST surface
through the ambient repository CLI that `github-proxy` already authorizes. No new network
capability is introduced, and the raw CLI invocation text lives only in
`prompts/match-dependencies.md` (data), never in the package's Lua.

## Files

| File | Role |
|------|------|
| `fkst.toml` | `lib_deps=[contract, workflow, testkit, forge, devloop]`, `event_deps=[github-proxy]`; no dev intake. |
| `bindings.lua` | Composes the four kernel seams + own intake; the only GitHub/runner touch-point. |
| `security_logic.lua` | Finding model, dedup key, findings→issue-create builder, built-in constants. |
| `records.lua` | The built-in `security-review` template + `records()` provider. |
| `completion.lua` | Pure child-status reader (maps a step's durable result to the 5-status enum). |
| `discovery.lua` | `platform.discovery`/`lease`: labelled-issue reads + marker parsing (own namespace). |
| `executor.lua` | `executor.raise_step`/`emit_terminal`: spawns codex, posts markers, files findings. |
| `catalog.lua` | Blueprint provider (built-in + `FKST_WORKFLOW_CATALOG_ROOT`, one validator). |
| `intake.lua` | Own intake handlers (label + own queue), stamps the blueprint, drives the tick. |
| `blueprints/security-review.json` | On-disk equivalent of the built-in template (catalog-lint validated). |
| `prompts/*.md` | Human mirrors of each step's codex instruction. |
| `departments/*/main.lua` | Thin saga-shaped wrappers over the kernel handlers. |

## Template id-collision warning

Host files under `FKST_WORKFLOW_CATALOG_ROOT` load through the same `blueprint.validate`. A
host-authored template whose `id` collides with the built-in `security-review` id **silently
disqualifies BOTH** peers (the kernel's duplicate-id rule). Choose a distinct id for host
templates.

## Tests

```sh
scripts/run.sh test workflow-security
```

- `core_test` — findings decode/validate, dedup key, issue-create builder, completion
  mapping, built-in blueprint validity.
- `namespaced_dispatch_test` — every department accepts its consumed queues under their
  production namespaced names and rejects foreign queues.
- `fire_raiser_security_poll_test` — producer-liveness: the `security_poll` cron tick routes
  to `security_select` (trace asserts consumer_result/source_payload/raised/routed_to).
