# Observability Command Audit Excerpts Plan

## Root Cause

This is an audit-boundary and data-classification bug. It is not a child-department stderr re-emission bug, and it is not a recurrence of `#1999`.

Verified local evidence:

- `/Users/auric/.fkst-dogfood/dogfood-rt-packages.1783580980/logs/framework-child/github-devloop-ops.observability-1783583689-071897000-561.log:307` contains an `external_command` audit line for `gh issue view 1999 --repo ChronoAIProject/fkst-packages --json 'title,body,comments,state,stateReason,assignees,author'`. That line has `EXIT=0`, `STDERR_BYTES=0`, and a `STDOUT_EXCERPT` containing the issue body text, including `fix-feedback-marker-missing`.
- The local `rg -n "fix-feedback-marker-missing" /Users/auric/.fkst-dogfood/dogfood-rt-packages.1783580980/logs/supervisor-1783580986-1793.log` check produced no match, while the framework-child log did match. The string entered observability logs through command-audit stdout excerpting, not through a live `github-devloop-pr.fix` child stderr emission in this slot.
- `packages/github-devloop-ops/departments/observability/common.lua:82` through `packages/github-devloop-ops/departments/observability/common.lua:91` fetches observability issue candidates through `core.observability_run_cmd`, calling `core.gh_issue_view_observe` and then parsing `view.stdout`.
- `libraries/devloop/commands/issue_reads.lua:7` through `libraries/devloop/commands/issue_reads.lua:24` defines the `observe` issue-view field set as `title,body,comments,state,stateReason,assignees,author`, and `libraries/devloop/commands/issue_reads.lua:139` through `libraries/devloop/commands/issue_reads.lua:141` routes `gh_issue_view_observe` through that field set.
- `libraries/devloop/parsers/issue.lua:236` through `libraries/devloop/parsers/issue.lua:247` preserves the fetched `body` and `comments` in `parse_issue_view_observe`.
- `packages/github-devloop-ops/departments/observability/census.lua:21` through `packages/github-devloop-ops/departments/observability/census.lua:37` stores the fetched issue as `entity.parent_issue`, so downstream observability code still needs full Lua-visible command output.
- `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/external_command.rs:690` through `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/external_command.rs:698` emits audit lines to stderr, and `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/external_command.rs:720` through `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/external_command.rs:736` formats every audited command with `STDOUT_BYTES`, `STDERR_BYTES`, `STDOUT_EXCERPT`, and `STDERR_EXCERPT`.
- `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/sdk_basic.rs:320` through `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/sdk_basic.rs:329` converts `output.stdout` and `output.stderr` into the Lua-visible `ExecResult`, so audit excerpt policy can change without removing full stdout/stderr from Lua callers.

## Goal

Health-monitor substring matches over framework logs must correspond to the departments or events that actually emitted those strings. Untrusted GitHub issue and PR bodies/comments must not be copied into framework command-audit excerpts, while command metadata remains observable and Lua callers still receive full stdout/stderr for parsers.

The implementation must preserve these diagnostic facts in command audit logs:

- rendered command
- exit code
- timeout flag
- stdout byte count
- stderr byte count

The implementation must not preserve content-bearing `STDOUT_EXCERPT` or `STDERR_EXCERPT` for GitHub reads that request untrusted body/comment fields.

## Governing Practice

Use structured logging and data minimization. Command audit logs are an observability boundary, not a content transport. The secure default is to log operational metadata and bounded classifications, not attacker-controlled payload bytes.

The FKST framing is the same as the existing harness doctrine: define one canonical way to perform content-bearing GitHub reads and make the unsafe bypass mechanically visible. The canonical path is:

1. A GitHub read helper declares whether its stdout/stderr may be excerpted in command audit logs.
2. Content-bearing helpers that return GitHub `body` or `comments` use a summary-only audit policy.
3. The SDK command boundary emits byte counts and command metadata, but omits output excerpts for summary-only commands.
4. Lua-visible stdout/stderr remains unchanged for the caller.

This deliberately does not use marker-specific redaction. A blacklist for `fix-feedback-marker-missing` would preserve the unsafe class and only hide one observed string. The illegal state is untrusted GitHub authored content flowing into framework audit excerpts at all.

## Approach

### 1. Add an explicit command audit output policy in `fkst-substrate`

Add a small enum on the substrate command execution path, for example:

- `AuditOutputPolicy::Excerpt`
- `AuditOutputPolicy::SummaryOnly`

Thread it through `CommandSpec` and the SDK option parsers used by `exec_sync` and `exec_argv`. The default remains excerpting so existing non-content command audits keep their current behavior. `SummaryOnly` changes only the audit line rendering:

- keep `CMD`, `EXIT`, `TIMED_OUT`, `STDOUT_BYTES`, and `STDERR_BYTES`
- omit `STDOUT_EXCERPT`
- omit `STDERR_EXCERPT`
- return unchanged `stdout` and `stderr` in `ExecResult`

Do not add a global environment switch, compatibility mode, or marker-specific filter. This is a per-command data classification choice.

### 2. Expose the policy through the Lua SDK command boundary

Extend the Lua command options table accepted by `exec_sync` and `exec_argv` with one explicit field, such as `audit_output = "summary-only"`.

Validation must fail closed:

- absent field means the existing default excerpt policy
- `"summary-only"` selects `SummaryOnly`
- unknown strings error with a narrow message

This is intentionally an SDK boundary option rather than a package-only log wrapper, because the framework owns the audit emission.

### 3. Route content-bearing GitHub reads through summary-only helpers

In the package repository, add or extend helpers so every GitHub read helper that requests `body` or `comments` declares summary-only audit output.

The observed path is `core.gh_issue_view_observe`, called from `packages/github-devloop-ops/departments/observability/common.lua:82` through `packages/github-devloop-ops/departments/observability/common.lua:91`. That path must pass the new SDK option.

The guard must cover the broader helper class, not only observability. `libraries/devloop/commands/issue_reads.lua:7` through `libraries/devloop/commands/issue_reads.lua:24` already centralizes issue-view field sets. Use that table or the underlying forge adapter request shape to enforce this invariant:

- any issue-view helper whose fields include `body` or `comments` must use `audit_output = "summary-only"`
- adding a new content-bearing GitHub read without the policy fails a package-level guard test

If PR read helpers expose `body` or `comments`, apply the same rule there. This plan does not require filtering `libraries/devloop/content_provenance.lua`; that code protects codex context bundles, while this issue concerns framework command audit logs.

### 4. Keep observability parsing intact

Do not remove `body` or `comments` from `gh_issue_view_observe`. `libraries/devloop/parsers/issue.lua:236` through `libraries/devloop/parsers/issue.lua:247` and `entity.parent_issue` in `packages/github-devloop-ops/departments/observability/census.lua:36` depend on those values for observability behavior.

The fix is at the audit boundary. Parsers keep receiving full stdout.

## Acceptance Criteria

### Substrate tests

Add a unit test near `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/external_command.rs:720` proving that a summary-only audit line:

- contains `CMD=`
- contains `EXIT=`
- contains `TIMED_OUT=`
- contains `STDOUT_BYTES=`
- contains `STDERR_BYTES=`
- does not contain `STDOUT_EXCERPT`
- does not contain `STDERR_EXCERPT`
- does not contain a sentinel payload string such as `fix-feedback-marker-missing`

Add an SDK test near `/Users/auric/.fkst-dogfood/substrate-dogfood/sub/crates/fkst-framework/src/sdk_basic.rs:320` proving that a command run with `audit_output = "summary-only"` still returns the complete Lua-visible `stdout` and `stderr`.

### Package regression tests

Add a package-level regression for the observability issue-read path:

- mock `gh issue view` for an issue whose `body` or `comments` contains `fix-feedback-marker-missing`
- run the observability fetch path through the same helper used by `packages/github-devloop-ops/departments/observability/common.lua:82` through `packages/github-devloop-ops/departments/observability/common.lua:91`
- assert the parsed issue still contains the body/comment data needed by observability
- assert the emitted command audit line has byte counts but no `STDOUT_EXCERPT` or `STDERR_EXCERPT`
- assert the audit line does not contain `fix-feedback-marker-missing`

Add a guard test for `libraries/devloop/commands/issue_reads.lua`:

- enumerate every issue-view field set
- when the field set includes `body` or `comments`, require the helper path to choose summary-only audit output
- fail if a new content-bearing helper can be added without the policy

If PR helpers have analogous content-bearing field sets, add the same guard for them.

### End-to-end local verification

Run the affected local verification command from the package repository root:

```sh
scripts/run.sh test-affected
```

If substrate changes are made in the engine repository, also run the relevant substrate unit tests for `external_command` and `sdk_basic` before the package verification. Do not claim success if the engine `BIN` is unreachable.

## Non-Goals

- No production-code change in this docs-only PR.
- No monitor rewrite.
- No external health-monitor substring blacklist.
- No marker-specific redaction for `fix-feedback-marker-missing`.
- No removal of observability access to issue/PR bodies or comments.
- No state-machine, fix-loop, review-loop, merge-gate, or devloop lifecycle behavior change.
- No duplicate devloop gate or parallel content-provenance system.
- No GitHub labels, comments, or other GitHub state changes.

## Implementation Notes For The Follow-Up Change

- This plan intentionally assigns the audit policy mechanism to `fkst-substrate`, because `external_command.rs` emits the audit line. The package repository should only declare policy for its content-bearing GitHub reads.
- Reconcile this with open `#2049` before implementation so the repository does not grow two overlapping GitHub-content filtering loci. The expected split is: codex bundle content provenance remains in `libraries/devloop/content_provenance.lua`; command-audit excerpt minimization belongs at the SDK command boundary.
- The summary-only policy is data minimization, not loss of operational telemetry. Byte counts prove content volume; Lua-visible stdout/stderr preserve caller behavior; framework logs stop becoming a carrier for arbitrary GitHub authored text.
