---
name: dogfood-github-devloop
description: Use when dogfooding the github-devloop autonomous self-development system — run its supervise loop, keep the pipeline flowing, and when it stalls decide whether to manually drive a bootstrap-blocking issue to completion or sshx-hotfix the defect.
---

# Dogfood github-devloop

github-devloop is an autonomous issue→develop→PR→review→merge loop on GitHub driven by a long-running `fkst-framework supervise`; this skill is the operating doctrine for dogfooding it.

The system IS the dogfood - our job is to keep the pipeline FLOWING, file good issues, and fix the SYSTEM-level real defects it exposes.

## Read first

Load context before acting; do not re-derive the doctrine from scratch.

Read (`CLAUDE.md` is at the repo root; the rest are memory entries, recalled by exact name):

- `CLAUDE.md`
- `github-devloop-self-hosting`
- `github-devloop-integration-branch-config`
- `operating-mode-issue-driven-self-drive`
- `no-unilateral-arch-or-destructive-ops`
- `github-devloop-review-stall-selfheal-boundary`
- `reliable-retry-primitive`

`CLAUDE.md` carries repo doctrine, branch/package rules, the `github-devloop` state machine, the integration topology, and the merge gate.

The memory files carry detail and incident history. This skill is the decision tree.

## Non-negotiable guardrails (read before any intervention)

- A hotfix fixes ONLY the one defect. Do NOT unilaterally change architecture, switch branch topology, delete/modify remote branches, or bypass the integration buffer + rollup gate.
- Destructive/irreversible remote ops - close PR, delete branch, force-push, change default branch - require explicit USER confirmation FIRST, even under `/goal` "continue" pressure. Deleting a base branch auto-closes its open PRs; always check in-flight PR dependencies before touching a branch.
- Engine (Rust) changes belong in the sibling `fkst-substrate` repo, never here. A package fix must work package-side; an engine need is a separate substrate PR.
- Autonomous changes flow through the integration buffer → rollup → dev. They do NOT go direct to dev - the ONE exception is an out-of-band infra hotfix when the automation path itself is broken/stopped (routing through it would be circular): that goes direct to dev, CI-gated.
- Trust only marker-as-fact (bot-authored state markers) with version-CAS; GitHub is eventually-consistent (read-after-write lag), not strong-consistency - expect transient "marker not yet visible; retrying".
- When unsure or facing a design-layer problem, STOP and ask + file an issue; do not improvise a workaround that bypasses a deliberate gate.

## Operating loop

1. Run `github-devloop` supervise with real write posture, from a worktree checked out to the merged `dev`. It is a long-running foreground process — run it detached (background it and redirect output to a log) so the loop survives across turns.

   Fresh runtime+durable roots mean a clean restart. `<gh-login>` is the `gh auth` user, which is the trusted bot marker author.

   ```sh
   FKST_GITHUB_REPO=<owner/repo> FKST_GITHUB_WRITE=1 FKST_GITHUB_BOT_LOGIN=<gh-login> \
   FKST_DEVLOOP_UPSTREAM_BRANCH=dev FKST_DEVLOOP_INTEGRATION_BRANCH=integration FKST_DEVLOOP_ROLLUP_MERGE=auto \
   FKST_RUNTIME_ROOT=<fresh> FKST_DURABLE_ROOT=<fresh> \
   "$BIN" supervise --project-root <worktree> \
     --package-root <worktree>/packages/github-devloop --package-root <worktree>/packages/github-proxy --package-root <worktree>/packages/consensus \
     --framework-bin "$BIN"
   ```

2. Watch it. Use "What to observe". If state keeps advancing, do NOT intervene - observe, and file issues only for real system defects.
3. When it stalls, go to the Stall decision tree.

## What to observe

Per monitoring pass, check:

- Supervise alive + 0 panic.
- State transitions advancing: consensus, review, implement, and merge activity; not only `github_poll`.
- Churn regression absent: `integration == dev`, no `+0/-0` rollup PR.
- No recurring `dead_letter publish failed` in steady-state. If it recurs, it is a real robustness gap, such as marker-lag retry exhaustion.
- Reviews not stuck in `reviewing` across runs with no transition. That is a mid-loop stall.
- GraphQL quota healthy.

A stall means consensus/review activity stops while only polling continues.

## Stall decision tree

1. Pipeline flowing: transitions continue, supervise is alive, and there is no repeated panic/DLQ/stall. Keep observing; do not intervene.
2. Stalled: ask, "Is there an open issue whose fix IS the automation defect currently blocking the loop?" This is a bootstrap blocker: the issue that fixes the very thing blocking it. The system cannot self-fix it because the dependency is circular.
3. If YES — and the issue is bootstrap-STUCK (consensus-approved but blocked by the circular defect), NOT consensus-REJECTED (see Anti-patterns): manually drive THAT issue to completion out-of-band. Use "Out-of-band bootstrap fix". Then nudge the already-stuck work.
4. If NO, meaning this is a fresh systemic defect: diagnose it, file an issue for the record, then apply the SAME out-of-band hotfix path.
5. After a fix lands: close the resolved issue (housekeeping, so intake does not re-process it) and any redundant/superseded PR the running system produced in parallel (PR-close needs the destructive-op confirmation). Otherwise the system redundantly re-implements it.

## Out-of-band bootstrap fix (the sshx procedure)

Use this path for both branch 3 and branch 4 in the Stall decision tree.

- Run the `sshx` skill on the fix: thinking triplet (3 peer-invisible codex workers, read-only) → meta-judge → implementation worker (isolated git worktree, workspace-write) → review triplet → fix-or-done.
- Codex worker template:

  ```sh
  timeout N codex exec --sandbox read-only|workspace-write --skip-git-repo-check --cd "$PWD" "<brief>" </dev/null
  ```

  Run workers in the background. Each worker emits `===CONCLUSION===`.

- Land it: PR direct to `dev`. This is the out-of-band exception: the broken pipeline cannot carry its own fix. Watch CI; squash-merge when green.
- Restart supervise: stop the old one, update the worktree to the merged `dev`, relaunch with fresh roots so the fixed code loads.

## Nudge after fix & close

- The fix prevents FUTURE failures but does not revive ALREADY-stuck work. Re-enter each stuck item into the now-fixed pipeline.
- For a stuck PR review, advance the PR head with an empty commit so a fresh `head_sha` yields a new proposal id that bypasses consensus dedup:

  ```sh
  git fetch origin <branch>
  NEW=$(git commit-tree FETCH_HEAD^{tree} -p FETCH_HEAD -m "nudge: re-trigger review")
  git push origin $NEW:refs/heads/<branch>
  ```

- Close the resolved issue so intake does not redundantly re-process it (housekeeping — a resolved issue closes freely).
- Close any redundant/superseded autonomous PR — PR-close follows the destructive-op guardrail: confirm first.

## Anti-patterns

- Band-aiding a stall by restarting repeatedly instead of fixing the defect.
- Re-deriving doctrine instead of reading `CLAUDE.md` and memory.
- Letting a redundant autonomous PR, where the system re-fixes what you already fixed out-of-band, auto-merge. Close it.
- Forcing a consensus-REJECTED issue through manually without explicit user authorization. That overrides a substantive judgment and is distinct from a bootstrap-STUCK-but-approved issue.
- Treating eventually-consistent GitHub lag, such as transient "not yet visible", as a defect. Only steady-state recurrence is.

## References, not restatements

- `CLAUDE.md`
- `github-devloop-self-hosting`
- `github-devloop-integration-branch-config`
- `operating-mode-issue-driven-self-drive`
- `no-unilateral-arch-or-destructive-ops`
- `github-devloop-review-stall-selfheal-boundary`
- `reliable-retry-primitive`

The detailed launch flags, substrate engine facts, and incident history live in those references; this skill stays the decision tree.
