# Capability Layering P0 Implementation Plan

> **Status:** P0 plan accepted via sshx inline-consensus (2026-06-15). Thinking triplet → `implement`; plan review: architecture approve×2; quality/tests verified content/paths/buckets/hooks correct, with one residual advisory on self-check strength (below). Accepted by explicit user decision.
>
> **Residual verification-hardening TODO (fold in during execution — a self-check robustness item, not a plan-content defect):**
> - Task 2 / Task 5 `core.lua` check: compare the *interleaved* `-- @capability:` header + `require(...).install(M)` sequence against the expected per-bucket run map (not only the bare require order + comment-only guard), so a missing or mis-placed header fails closed.
> - Task 5 scope check: use `git status --porcelain` (includes untracked) instead of `git diff --name-only`, so newly added files cannot escape the scope assertion.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the zero-structural-risk clarity phase for capability layering: one operational boundary map, shallow machine-greppable annotations, and grouped command docs/help with no behavior change.

**Architecture:** The accepted design stays the rationale; `docs/dev/capability-boundary-map.md` becomes the operational map. Lua changes are comments only: one shallow `@capability` tag per github-devloop department/raiser entrypoint plus grouped comments in `core.lua`; no module moves, queue changes, marker changes, or contract changes. Shell and README edits only clarify Development / Operations / Diagnosis command surfaces and the host-preflight vs saga-doctor distinction.

**Tech Stack:** Markdown, Lua comments, Bash help comments, README docs; verification via `rg`, `bash -n`, `git diff`, `scripts/run.sh check`, and `scripts/run.sh test`.

Spec: `docs/superpowers/specs/2026-06-15-capability-layering-design.md`

---

## Context / Why

The design converged on two planes plus one cross-cutting diagnosis control loop:

- Plane 1: generic autonomous-development mechanism for any GitHub repo.
- Plane 2: fkst run/ops policy for this dogfood deployment.
- Diagnosis: a control loop that observes facts, detects errors or stalls, and feeds Plane 1 with repair issues or bounded redrive.

The P0 harness is clarity-first and reversible. Classify by effect/action, not by filename and not by the data touched. 中文补充：P0 只让边界可见、可 grep，不移动代码、不改行为。

## NOT in P0 / Deferred to P1/P2

- No module moves or file splits.
- No `require(...)`, `M.spec`, queue, event schema, marker, label, symbol, or contract changes.
- No `doctor host`, `doctor saga`, or other CLI alias split.
- No `github-proxy` devloop leak cleanup.
- No `github-devloop/core/*.lua` decomposition.
- No `fkst-substrate` work.
- No `check_repo.py` guard yet.
- No annotation of department-local helper modules or `core/*` submodules in P0.
- No behavior compatibility branch, deprecated shim, or legacy path.

---

### Task 1: Create the Operational Boundary Map

**Files:**
- Create: `docs/dev/capability-boundary-map.md`
- Read-only input: `docs/superpowers/specs/2026-06-15-capability-layering-design.md`

- [ ] **Step 1: Write `docs/dev/capability-boundary-map.md`**

Create the document in English-primary form with brief Chinese auxiliary notes. Keep it under 1000 lines and end it with `⟦AI:FKST⟧`.

Required structure:

```markdown
# Capability Boundary Map

> Status: P0 operational map. Rationale lives in `docs/superpowers/specs/2026-06-15-capability-layering-design.md`.
> Scope: `github-devloop` P0 annotations plus repo command-surface wording.
> 中文摘要：本文是执行边界图，不替代设计说明；P0 只做注释和文档清晰化。

## Context / Why
## Model: Two Planes + Diagnosis Control Loop
## Decision Rule
## Annotation Grammar
## File-Level Bucket Manifest
## Fuzzy-Boundary Notes
## P0 / P1 / P2 Boundary
## Branch & PR
## Risk & Rollback
```

The model section must include the two-planes-plus-diagnosis-control-loop diagram from the accepted design in compact form.

The decision rule must include the exact phrase:

```markdown
Classify by effect/action, not by filename and not by the data it touches.
```

The annotation grammar section must define only these Lua tags:

```lua
-- @capability: plane1|plane2|diagnosis
-- @capability-hook: diagnosis
```

Rules to document:
- `@capability` appears once per `packages/github-devloop/departments/*/main.lua` and once per `packages/github-devloop/raisers/*.lua`.
- In departments, place the tag after module imports and before `local M = {}` or the first module body.
- In raisers, place the tag before the returned table.
- `@capability-hook: diagnosis` appears only on fuzzy hooks: `observe_issue`, `observe_pr`, `reconcile`, `rollup_scan`.
- In `packages/github-devloop/core.lua`, tags are shallow block comments around existing contiguous install runs. They are not a per-function manifest.
- Do not annotate department-local helper modules or `core/*` submodules in P0.

The file-level bucket manifest must list:

Plane 1 departments: `comment_handoff`, `consensus_result`, `decompose`, `fix`, `implement`, `intake_judge`, `intake_probe`, `intake_scan`, `loop`, `merge`, `open_pr`, `review_loop`, `review_meta`, `review_pr`, `review_result`, `observe_issue`, `observe_pr`, `reconcile`.

Plane 1 raisers: `intake_poll`, `intake_probe_poll`, `merge_queue_poll`.

Plane 2 departments: `ensure_repo`, `sync_scan`, `sync_conflict`, `pr_freshness_scan`, `rollup_merge`, `substrate_ref_scan`, `rollup_scan`.

Plane 2 raisers: `branch_poll`, `ensure_repo_poll`, `substrate_ref_poll`.

Diagnosis departments: `dead_letter`, `doctor`, `liveness_scan`, `observability`.

Diagnosis raisers: `liveness_poll`, `observability_poll`.

Explicitly excluded: `autochrono`, `github-autochrono`, package tests, department-local helpers, and `github-devloop/core/*` submodules for P0 annotation purposes.

The fuzzy-boundary notes must cover:
- `observe_issue`, `observe_pr`, `reconcile`: Plane 1 entrypoints with diagnosis hooks.
- `rollup_scan`: Plane 2 entrypoint with a diagnosis hook via rollup-health issue creation.
- Observability is split by consumer action: generic engine facts belong in substrate, board rendering is Plane 2, starvation/gap/health detection is Diagnosis, reaper behavior is Plane 1.
- The two doctors are distinct: `scripts/doctor.sh` is host-preflight; `github-devloop/departments/doctor` is saga-doctor.

The P0/P1/P2 section must list P1 split targets here, not in Lua comments:
- `packages/github-devloop/core/observability.lua` -> census/dashboard Plane 2, reaper Plane 1, starvation/conflict Diagnosis.
- Reconcile trigger logic -> split by trigger class: Plane 1 deterministic consensus/review/fix true-stall backstops vs Diagnosis timeout reconcile. Current repo has no standalone `core/reconcile.lua`; relevant current surfaces include `departments/reconcile/main.lua`, `core/reconcile_requests.lua`, and timeout-reconcile call sites.
- `packages/github-devloop/core/prompts.lua` and `packages/github-devloop/prompts/sync_conflict.lua` -> split Plane 2 sync-conflict prompt from Plane 1 implementation/intake/decompose/review prompt builders.
- `packages/github-devloop/core/payloads.lua` -> split Plane 2 board feed/digest rendering from Plane 1 proposal payload builders.
- `packages/github-devloop/core/config.lua` -> split generic posture knobs from dogfood topology knobs.
- `packages/github-devloop/core/commands.lua` and `packages/github-devloop/core/branches.lua` -> split caller-by-caller.
- Clean `github-proxy` devloop leaks: `fkst-dev:*` label policy, devloop marker guards, and intake label assumptions move behind generic caller-supplied guards.

- [ ] **Step 2: Verify the doc is complete and small**

Run:

```bash
bash <<'BASH'
set -euo pipefail

lines=$(wc -l < docs/dev/capability-boundary-map.md)
test "$lines" -lt 1000 || { echo "line-count-too-high:$lines"; exit 1; }

fail=0
while IFS= read -r anchor; do
  rg -n --fixed-strings "$anchor" docs/dev/capability-boundary-map.md >/dev/null || {
    echo "missing-anchor:$anchor"
    fail=1
  }
done <<'EOF'
Plane 1
Plane 2
Diagnosis
Classify by effect
@capability
NOT in P0
⟦AI:FKST⟧
EOF

test "$fail" = 0
BASH
```

Expected: line count below 1000; every required anchor is present.

---

### Task 2: Add Machine-Greppable Lua Comment Annotations

**Files:**
- Modify: `packages/github-devloop/core.lua`
- Modify: `packages/github-devloop/departments/*/main.lua`
- Modify: `packages/github-devloop/raisers/*.lua`

- [ ] **Step 1: Add department tags**

For every `packages/github-devloop/departments/*/main.lua`, insert exactly one capability header after the existing import line(s). Do not reorder imports, specs, queues, symbols, or functions.

Plane 1:
- `comment_handoff`, `consensus_result`, `decompose`, `fix`, `implement`, `intake_judge`, `intake_probe`, `intake_scan`, `loop`, `merge`, `open_pr`, `review_loop`, `review_meta`, `review_pr`, `review_result`:

```lua
-- @capability: plane1
```

Plane 1 with diagnosis hook:
- `observe_issue`, `observe_pr`, `reconcile`:

```lua
-- @capability: plane1
-- @capability-hook: diagnosis
```

Plane 2:
- `ensure_repo`, `sync_scan`, `sync_conflict`, `pr_freshness_scan`, `rollup_merge`, `substrate_ref_scan`:

```lua
-- @capability: plane2
```

Plane 2 with diagnosis hook:
- `rollup_scan`:

```lua
-- @capability: plane2
-- @capability-hook: diagnosis
```

Diagnosis:
- `dead_letter`, `doctor`, `liveness_scan`, `observability`:

```lua
-- @capability: diagnosis
```

- [ ] **Step 2: Add raiser tags**

For every `packages/github-devloop/raisers/*.lua`, insert exactly one capability header before the returned table. Do not change cron intervals, produced queues, or returned table shape.

Plane 1:
- `intake_poll`, `intake_probe_poll`, `merge_queue_poll`

Plane 2:
- `branch_poll`, `ensure_repo_poll`, `substrate_ref_poll`

Diagnosis:
- `liveness_poll`, `observability_poll`

No raiser gets `@capability-hook` in P0.

- [ ] **Step 3: Annotate `packages/github-devloop/core.lua` install runs**

Add shallow block comments around contiguous `require(...).install(M)` runs in the current order. Do not reorder any `require(...)` line. Use tags as section headers; the next tag starts the next run.

Current-order run map:

- `plane1`: `core.base`, `core.config`, `core.strings`, `core.commands`, `core.github_proxy_entity_view`
- `plane2`: `core.branches`
- `plane1`: `core.forks`, `core.parsers`, `core.merge_gate`, `core.review_carry_over`, `core.queue`, `core.merge_batch`
- `diagnosis`: `core.logging`, `core.error_facts`, `core.conflict_telemetry`
- `plane1`: `core.state`
- `diagnosis`: `core.state_gap`
- `plane1`: `core.markers`, `core.pr_safety`, `core.intake_service_class`, `core.intake_scan`, `core.impl_failure`, `core.payloads`, `core.convergence`, `core.saga`, `core.decompose`, `core.restart`, `core.review_redrive`, `core.pr_review_replayer`, `core.replayer`
- `diagnosis`: `core.liveness`
- `plane1`: `core.prompts`, `core.intake_class`, `core.requests`, `core.reconcile_requests`, `core.pr_label_requests`, `core.review_meta_requests`, `core.entity`, `core.implement_attempt`, `core.dependencies`, `core.validators`
- `diagnosis`: `core.observability_bounds`, `core.rollup_health`, `core.queue_starvation`, `core.observability`
- `plane2`: `core.release_notes`, `core.ensure_repo`, `core.substrate_ref`
- `plane1`: `core.context_bundle`, `core.operator_commands`, `core.claims`
- `diagnosis`: `core.doctor`

Mixed modules such as `core.config`, `core.commands`, `core.payloads`, `core.prompts`, `core.observability`, and `core.reconcile_requests` stay shallow in P0. Their real splits are listed in the boundary map P1 section.

- [ ] **Step 4: Verify entrypoint buckets and core install comments**

Run:

```bash
bash <<'BASH'
set -euo pipefail

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT
expected="$tmp_dir/expected-capabilities"
actual="$tmp_dir/actual-capabilities"
expected_hooks="$tmp_dir/expected-hooks"
actual_hooks="$tmp_dir/actual-hooks"
core_expected="$tmp_dir/expected-core-installs"
core_actual="$tmp_dir/actual-core-installs"
fail=0

cat >"$expected" <<'EOF'
department/comment_handoff plane1
department/consensus_result plane1
department/dead_letter diagnosis
department/decompose plane1
department/doctor diagnosis
department/ensure_repo plane2
department/fix plane1
department/implement plane1
department/intake_judge plane1
department/intake_probe plane1
department/intake_scan plane1
department/liveness_scan diagnosis
department/loop plane1
department/merge plane1
department/observability diagnosis
department/observe_issue plane1
department/observe_pr plane1
department/open_pr plane1
department/pr_freshness_scan plane2
department/reconcile plane1
department/review_loop plane1
department/review_meta plane1
department/review_pr plane1
department/review_result plane1
department/rollup_merge plane2
department/rollup_scan plane2
department/substrate_ref_scan plane2
department/sync_conflict plane2
department/sync_scan plane2
raiser/branch_poll plane2
raiser/ensure_repo_poll plane2
raiser/intake_poll plane1
raiser/intake_probe_poll plane1
raiser/liveness_poll diagnosis
raiser/merge_queue_poll plane1
raiser/observability_poll diagnosis
raiser/substrate_ref_poll plane2
EOF

: >"$actual"
check_file() {
  kind=$1
  name=$2
  file=$3
  count=$(rg -c '^-- @capability:' "$file" || true)
  if [ "$count" != 1 ]; then
    echo "bad-capability-count:$count:$file"
    fail=1
    return
  fi
  bucket=$(sed -n 's/^-- @capability: //p' "$file")
  case "$bucket" in
    plane1|plane2|diagnosis) ;;
    *) echo "bad-capability-value:$bucket:$file"; fail=1 ;;
  esac
  printf '%s/%s %s\n' "$kind" "$name" "$bucket" >>"$actual"
}

while IFS= read -r file; do
  name=${file#packages/github-devloop/departments/}
  name=${name%/main.lua}
  check_file department "$name" "$file"
done < <(find packages/github-devloop/departments -mindepth 2 -maxdepth 2 -name main.lua -print | sort)

while IFS= read -r file; do
  name=${file#packages/github-devloop/raisers/}
  name=${name%.lua}
  check_file raiser "$name" "$file"
done < <(find packages/github-devloop/raisers -maxdepth 1 -type f -name '*.lua' -print | sort)

LC_ALL=C sort "$expected" >"$expected.sorted"
LC_ALL=C sort "$actual" >"$actual.sorted"
diff -u "$expected.sorted" "$actual.sorted" || fail=1

cat >"$expected_hooks" <<'EOF'
packages/github-devloop/departments/observe_issue/main.lua
packages/github-devloop/departments/observe_pr/main.lua
packages/github-devloop/departments/reconcile/main.lua
packages/github-devloop/departments/rollup_scan/main.lua
EOF
(rg -n '^-- @capability-hook: diagnosis$' packages/github-devloop/departments packages/github-devloop/raisers | sed 's/:.*//' | LC_ALL=C sort >"$actual_hooks") || true
bad_hooks=$(rg -n '^-- @capability-hook:' packages/github-devloop/departments packages/github-devloop/raisers | rg -v '^[^:]+:[0-9]+:-- @capability-hook: diagnosis$' || true)
if [ -n "$bad_hooks" ]; then
  printf 'bad-capability-hooks:\n%s\n' "$bad_hooks"
  fail=1
fi
diff -u "$expected_hooks" "$actual_hooks" || fail=1

cat >"$core_expected" <<'EOF'
require("core.base").install(M)
require("core.config").install(M)
require("core.strings").install(M)
require("core.commands").install(M)
require("core.github_proxy_entity_view").install(M)
require("core.branches").install(M)
require("core.forks").install(M)
require("core.parsers").install(M)
require("core.merge_gate").install(M)
require("core.review_carry_over").install(M)
require("core.queue").install(M)
require("core.merge_batch").install(M)
require("core.logging").install(M)
require("core.error_facts").install(M)
require("core.conflict_telemetry").install(M)
require("core.state").install(M)
require("core.state_gap").install(M)
require("core.markers").install(M)
require("core.pr_safety").install(M)
require("core.intake_service_class").install(M)
require("core.intake_scan").install(M)
require("core.impl_failure").install(M)
require("core.payloads").install(M)
require("core.convergence").install(M)
require("core.saga").install(M)
require("core.decompose").install(M)
require("core.restart").install(M)
require("core.review_redrive").install(M)
require("core.pr_review_replayer").install(M)
require("core.replayer").install(M)
require("core.liveness").install(M)
require("core.prompts").install(M)
require("core.intake_class").install(M)
require("core.requests").install(M)
require("core.reconcile_requests").install(M)
require("core.pr_label_requests").install(M)
require("core.review_meta_requests").install(M)
require("core.entity").install(M)
require("core.implement_attempt").install(M)
require("core.dependencies").install(M)
require("core.validators").install(M)
require("core.observability_bounds").install(M)
require("core.rollup_health").install(M)
require("core.queue_starvation").install(M)
require("core.observability").install(M)
require("core.release_notes").install(M)
require("core.ensure_repo").install(M)
require("core.substrate_ref").install(M)
require("core.context_bundle").install(M)
require("core.operator_commands").install(M)
require("core.claims").install(M)
require("core.doctor").install(M)
EOF
(rg '^require[(]"core[.][^"]+"[)][.]install[(]M[)]$' packages/github-devloop/core.lua >"$core_actual") || true
diff -u "$core_expected" "$core_actual" || fail=1
core_bad_added=$(git diff HEAD -- packages/github-devloop/core.lua | rg '^\+[^+]' | rg -v '^\+[[:space:]]*-- @capability: (plane1|plane2|diagnosis)$' || true)
if [ -n "$core_bad_added" ]; then
  printf 'bad-core-added-lines:\n%s\n' "$core_bad_added"
  fail=1
fi

test "$fail" = 0
BASH
```

Expected: every department and raiser has exactly one `@capability` tag matching the manifest; only `observe_issue`, `observe_pr`, `reconcile`, and `rollup_scan` have exactly one diagnosis hook; `core.lua` has the original install sequence exactly once and in order, with only `-- @capability:` comment headers added.

- [ ] **Step 5: Verify Lua diff is comments only**

Run:

```bash
bash <<'BASH'
set -euo pipefail

non_comment=$(git diff HEAD -- packages/github-devloop | rg '^[+-][^+-]' | rg -v '^[+-][[:space:]]*(--|$)' || true)
if [ -n "$non_comment" ]; then
  printf 'non-comment Lua diff:\n%s\n' "$non_comment"
  exit 1
fi

scripts/run.sh test github-devloop
BASH
```

Expected: no non-comment Lua diff; github-devloop tests pass.

---

### Task 3: Group `scripts/run.sh` Help and Doctor Wording

**Files:**
- Modify: `scripts/run.sh`
- Modify: `scripts/doctor.sh`

- [ ] **Step 1: Rewrite only the top help/comment block in `scripts/run.sh`**

Group commands in the help comments as:

```text
Development:
  scripts/run.sh check
  scripts/run.sh test [-v|--verbose] [package]
  scripts/run.sh test-composed
  scripts/run.sh run <package> <department> [event-json]
  scripts/run.sh run <package> <department> --event-file <path>

Operations:
  scripts/run.sh supervise <package>
  scripts/run.sh board [--refresh] [--ttl seconds] [--stall seconds]
  scripts/run.sh build

Diagnosis:
  scripts/run.sh doctor
      Host-preflight: read-only git/cargo/rustc, fkst-framework BIN, codex, gh auth, and FKST_* host facts.
  scripts/run.sh doctor github-devloop-ops
      Saga-doctor: read-only package-side saga/liveness doctor for the configured running GitHub repository.
```

Keep all actual command syntax. Keep the grouped help within the existing `usage()` extraction window if possible; if the extraction range must change, change only that range for help visibility and do not touch dispatch behavior.

Do not add aliases. Do not change `case` dispatch. Do not change `cmd_doctor` semantics. Invalid `doctor` usage must still exit non-zero.

- [ ] **Step 2: Mirror wording in `scripts/doctor.sh`**

Update only comments/help text so `scripts/doctor.sh` describes itself as host-preflight. Preserve output format: `DOCTOR <item> ok|missing ...`. Preserve command syntax as `scripts/run.sh doctor`.

- [ ] **Step 3: Verify shell syntax and help**

Run:

```bash
bash <<'BASH'
set -euo pipefail

bash -n scripts/run.sh scripts/doctor.sh
help=$(scripts/run.sh --help)
fail=0
while IFS= read -r anchor; do
  printf '%s\n' "$help" | rg -n --fixed-strings "$anchor" >/dev/null || {
    echo "missing-help-anchor:$anchor"
    fail=1
  }
done <<'EOF'
Development
Operations
Diagnosis
Host-preflight
Saga-doctor
EOF
test "$fail" = 0

if scripts/run.sh doctor bogus >/tmp/fkst-doctor-invalid.out 2>/tmp/fkst-doctor-invalid.err; then
  echo 'doctor-bogus-unexpectedly-succeeded'
  exit 1
fi
BASH
```

Expected: `bash -n` passes; help shows all three groups and both doctor meanings; invalid doctor usage exits non-zero.

- [ ] **Step 4: Review the diff**

Run:

```bash
git diff -- scripts/run.sh scripts/doctor.sh
```

Expected: help/comment wording only, plus at most the help extraction range if required for `--help` visibility. No dispatch or doctor behavior changes.

---

### Task 4: Group README Command Listing

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the boundary-map pointer**

Near the package/command overview, add one short pointer:

```markdown
Capability boundaries for github-devloop are tracked in [`docs/dev/capability-boundary-map.md`](docs/dev/capability-boundary-map.md); the accepted rationale remains in [`docs/superpowers/specs/2026-06-15-capability-layering-design.md`](docs/superpowers/specs/2026-06-15-capability-layering-design.md).
```

Do not duplicate the manifest in README.

- [ ] **Step 2: Group the command listing**

Replace the flat `scripts/run.sh` command list with three groups:

Development:
- `scripts/run.sh check`
- `scripts/run.sh test`
- `scripts/run.sh test github-proxy`
- `scripts/run.sh test-composed`
- `scripts/run.sh run ...`

Operations:
- `scripts/run.sh supervise <package>`
- `scripts/run.sh board ...`
- `scripts/run.sh build`

Diagnosis:
- `scripts/run.sh doctor` as host-preflight.
- `scripts/run.sh doctor github-devloop-ops` as saga-doctor.

Preserve the actual command syntax and existing operational caveats: `run` stays read-only by default, `supervise` remains the real foreground event loop, and `build` remains local-only.

- [ ] **Step 3: Verify anchors**

Run:

```bash
bash <<'BASH'
set -euo pipefail

fail=0
while IFS= read -r anchor; do
  rg -n --fixed-strings "$anchor" README.md >/dev/null || {
    echo "missing-readme-anchor:$anchor"
    fail=1
  }
done <<'EOF'
Development
Operations
Diagnosis
host-preflight
saga-doctor
capability-boundary-map
EOF

test "$fail" = 0
BASH
```

Expected: all anchors are present.

---

### Task 5: Final Zero-Structural-Risk Verification

**Files:** none. This task must not edit files.

- [ ] **Step 1: Confirm diff scope**

Run:

```bash
bash <<'BASH'
set -euo pipefail

unexpected=$(git diff --name-only HEAD -- | rg -v '^(docs/dev/capability-boundary-map.md|packages/github-devloop/core.lua|packages/github-devloop/departments/[^/]+/main.lua|packages/github-devloop/raisers/[^/]+[.]lua|scripts/run.sh|scripts/doctor.sh|README.md)$' || true)
if [ -n "$unexpected" ]; then
  printf 'unexpected diff paths:\n%s\n' "$unexpected"
  exit 1
fi
BASH
```

Expected: no unexpected paths. If the new boundary-map file is still untracked, stage intent-to-add or inspect `git status --short` before this check so it is visible in the branch diff.

- [ ] **Step 2: Confirm Lua edits are comments only**

Run:

```bash
bash <<'BASH'
set -euo pipefail

non_comment=$(git diff HEAD -- packages/github-devloop | rg '^[+-][^+-]' | rg -v '^[+-][[:space:]]*(--|$)' || true)
if [ -n "$non_comment" ]; then
  printf 'non-comment Lua diff:\n%s\n' "$non_comment"
  exit 1
fi
BASH
```

Expected: no non-comment Lua diff.

- [ ] **Step 3: Confirm no contract rename slipped in**

Run:

```bash
bash <<'BASH'
set -euo pipefail

non_annotation=$(git diff HEAD -- packages/github-devloop/departments packages/github-devloop/raisers packages/github-devloop/core.lua | rg '^[+-][^+-]' | rg -v '^[+-][[:space:]]*(-- @capability:|-- @capability-hook:|$)' || true)
if [ -n "$non_annotation" ]; then
  printf 'non-annotation Lua diff:\n%s\n' "$non_annotation"
  exit 1
fi
BASH
```

Expected: no `require(...)`, `M.spec`, `consumes`, `produces`, `fanout`, event queue, schema, marker, label, or symbol rename. Only `-- @capability...` comments and core install-run comment headers appear.

- [ ] **Step 4: Run repository checks and tests**

Run:

```bash
bash <<'BASH'
set -euo pipefail

scripts/run.sh check
scripts/run.sh test
BASH
```

Expected: both pass.

---

## Branch & PR

Start from `dev`; do not commit directly to `dev`.

```bash
git switch dev
git pull --ff-only
git switch -c docs/capability-layering-p0
```

After all verification passes, commit as one logical P0 clarity change:

```bash
git add docs/dev/capability-boundary-map.md packages/github-devloop/core.lua packages/github-devloop/departments packages/github-devloop/raisers scripts/run.sh scripts/doctor.sh README.md
git commit -m 'docs: clarify capability layering boundaries for P0'
```

PR body must be English-primary with brief Chinese auxiliary notes, include the commands and results from Task 5, and end with `⟦AI:FKST⟧`.

## Risk & Rollback

Main risk: annotation drift and merge churn. Mitigation: keep `docs/dev/capability-boundary-map.md` authoritative, keep Lua comments shallow, and defer real splits to P1.

Runtime risk is intentionally near zero: P0 changes comments, docs, and help wording only. Rollback is a straight revert of the boundary map, Lua comments, help comments, and README grouping.

## Self-Review

- Spec coverage: Task 1 captures the two-plane model, effect rule, manifest, fuzzy boundaries, and P1/P2 deferrals; Task 2 adds tags; Tasks 3-4 clarify command docs; Task 5 verifies zero structural risk.
- Placeholder scan: no TBDs, no behavior-changing TODOs, no unspecified tests.
- Contract safety: no queue/event/marker/CLI contract changes are planned; `doctor` remains the same command surface in P0.

⟦AI:FKST⟧
