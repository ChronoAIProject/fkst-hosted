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

- **UNATTENDED MODE — never pop up a question.** This runs unattended. Do NOT use AskUserQuestion; never block on a user prompt. When you are unsure or facing a judgment call (including a risky op below), run `sshx` to think it through and decide — independent worker perspectives are the gate, not a human prompt. Never act rashly, and never stall waiting for a human.
- **NEVER mutate program state by hand** (user constitution, 2026-06-11): no hand-written state/converge/review-result markers, no touching runtime/durable contents. State is produced ONLY by the program. Fix the PROGRAM first (self-drive preferred; sshx out-of-band only for bootstrap breakage of the program itself), THEN steer through GitHub-surface interfaces: issues, comments (incl. command comments once #278 lands), pushed commits (head-nudge), closing your own issues. The PR#223 hand-crafted re-entry marker is the named anti-example.
- A hotfix fixes ONLY the one defect. Do NOT unilaterally change architecture, switch branch topology, delete/modify remote branches, or bypass the integration buffer + rollup gate.
- Destructive/irreversible remote ops - close PR, delete branch, force-push, change default branch - are high-risk: do NOT act rashly, and (unattended) do NOT pop up. Vet via `sshx` (multi-angle), check in-flight PR dependencies + branch ancestry FIRST, and proceed only if sshx confirms it is both safe AND necessary. Deleting a base branch auto-closes its open PRs.
- Engine (Rust) changes belong in the sibling `fkst-substrate` repo, never here. A package fix must work package-side; an engine need is a separate substrate PR.
- Autonomous changes flow through the integration buffer → rollup → dev. They do NOT go direct to dev - the ONE exception is an out-of-band infra hotfix when the automation path itself is broken/stopped (routing through it would be circular): that goes direct to dev, CI-gated.
- Trust only marker-as-fact (bot-authored state markers) with version-CAS; GitHub is eventually-consistent (read-after-write lag), not strong-consistency - expect transient "marker not yet visible; retrying".
- When unsure or facing a design-layer problem, do NOT improvise a workaround that bypasses a deliberate gate, and do NOT pop up a question — run `sshx` to think it through and decide, and file an issue for the record.

## Operating loop

1. Run `github-devloop` supervise with real write posture, from a worktree checked out to the merged `dev`. It is a long-running foreground process — run it detached (background it and redirect output to a log) so the loop survives across turns.

   `FKST_RUNTIME_ROOT` is clearable scratch — a fresh one each run is fine. `FKST_DURABLE_ROOT` is the redb **persistent** delivery store and is **NOT scratch** (per `CLAUDE.md`): **reuse a STABLE durable root across restarts** so in-flight persisted events resume. A *fresh* durable root throws the whole durable queue away and **strands mid-state issues** whose advancing event was sitting in the abandoned store (e.g. an issue stuck at `ready`/`implementing` that never re-triggers implement — observed: #62/#78). Use a fresh durable root ONLY for a deliberate clean-slate wipe, never for a normal restart-to-deploy. `<gh-login>` is the `gh auth` user, which is the trusted bot marker author.

   **EXPORT `BIN` in the supervise environment** (not only `--framework-bin`). The spawned implement/fix codex runs `scripts/run.sh test` in its worktree to verify its own work passes CI before finishing; that resolution needs the engine BIN, which it inherits from the supervise process env (the codex worktree has no `.env` — it is gitignored). Launching bare (`"$BIN" supervise …` without `BIN=…` in the env) leaves the codex unable to run the suite, so the autonomous fix loop cannot close CI failures and every PR with a red check churns to `blocked`. `scripts/run.sh supervise` exports it for you; a direct launch must set `BIN=<engine-bin>` in the env list.

   ```sh
   BIN=<engine-bin> \
   FKST_GITHUB_REPO=<owner/repo> FKST_GITHUB_WRITE=1 FKST_GITHUB_BOT_LOGIN=<gh-login> \
   FKST_DEVLOOP_UPSTREAM_BRANCH=dev FKST_DEVLOOP_INTEGRATION_BRANCH=integration FKST_DEVLOOP_ROLLUP_MERGE=auto \
   FKST_RUNTIME_ROOT=<fresh-scratch> FKST_DURABLE_ROOT=<STABLE, reused across restarts> \
   "$BIN" supervise --project-root <worktree> \
     --package-root <worktree>/packages/github-devloop --package-root <worktree>/packages/github-proxy --package-root <worktree>/packages/consensus \
     --framework-bin "$BIN"
   ```

2. **Keep the running code current.** The supervise loads `packages/` from the worktree at STARTUP — branch auto-sync (`sync_scan` ff'ing `dev`→`integration`) propagates branch CONTENT but does NOT reload a running process's code. So after ANY code change merges to `dev` (your out-of-band hotfix, or an autonomous rollup), sync the worktree to the latest remote `dev` and restart the supervise PROMPTLY, so it always runs the latest code. A supervise left running on stale code re-introduces already-fixed defects.
3. Watch it. Use "What to observe". If state keeps advancing, do NOT intervene - observe, and file issues only for real system defects.
4. When it stalls, go to the Stall decision tree.

## What to observe

**Every activation, FIRST sweep the full board — do not skip to logs.** Enumerate every open issue and every open PR, and for each map its current state marker / label / last transition. The goal is to catch, per item, two failure shapes the log alone hides:

- **Stuck**: an item whose state has not advanced across passes (e.g. parked at `ready`/`reviewing`/`fixing`/`thinking` with no new marker), or an issue sitting with no state at all (intake never decided, or `decline`-dead-ended).
- **Misbehaving**: an item that transitioned WRONG vs the expected state machine — a sound issue `decline`d or `blocked`, a review reject that should have been a converge, a PR that re-opened/churned, a redundant autonomous PR duplicating an out-of-band fix, a `+0/-0` rollup.

Only after the board sweep, check the running process:

- Supervise alive + 0 panic.
- State transitions advancing: consensus, review, implement, and merge activity; not only `github_poll`.
- Churn regression absent: `integration == dev`, no `+0/-0` rollup PR.
- No recurring `dead_letter publish failed` in steady-state. If it recurs, it is a real robustness gap, such as marker-lag retry exhaustion.
- Reviews not stuck in `reviewing` across runs with no transition. That is a mid-loop stall.
- GraphQL quota healthy.

A stall means consensus/review activity stops while only polling continues. A board item stuck or mis-transitioned with the pipeline otherwise flowing is ALSO a defect — diagnose it (often impoverished codex context: truncated input, no code access, or a terminal-reject gate that should converge), file a consensus-rnd-informed issue, and drive it.

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
- **Post the review record ON the PR** (before or right after merge): a comment with each reviewer's verdict, blocking findings, fix-pass history, and suite evidence. The audit trail lives on GitHub, not in local `/tmp` logs — same marker-as-fact transparency the autonomous pipeline gives its own PRs (user feedback, PR#226).
- Restart supervise: stop the old one, update the worktree to the merged `dev`, relaunch with a fresh runtime scratch root but the **SAME stable durable root**, so the fixed code loads while the durable queue's in-flight events resume. Do NOT fresh the durable root on a normal restart — that strands mid-state issues (see the runtime/durable root note in the operating loop). Re-derive from GitHub markers also self-heals mid-state issues, but only for states that have a self-heal branch in `observe_issue`.

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
