# `github-devloop` Autonomous Development State Machine: Design and Staged Implementation Plan

This design turns the `sshx` loop (consensus -> implementation -> review -> consensus -> merge) into
a long-running state machine on the `fkst` engine, using GitHub issues and PRs as state carriers. It
converged through an `sshx` thinking triplet (minimal / structural / delete).

⟦AI:FKST⟧

## 1. Architecture Overview

- Add the GitHub-aware **composed package `github-devloop`** (`fkst.toml` `[event_deps]` packages:
  `github-proxy`, `consensus`) to orchestrate the existing `consensus` engine into an autonomous
  issue -> implementation -> PR -> merge development loop.
- **GitHub and git are the only state sources (doctrine):**
  - GitHub is an **eventually consistent authenticated fact source**, not a strong-consistency KV.
  - Comment **`fkst:github-devloop:state:v1` HTML marker = current state fact**. Trust only markers
    written by this bot author (`FKST_GITHUB_BOT_LOGIN`); ignore ordinary user-forged markers.
  - The state marker also carries `version="<dedup>"`. A transition applies only when the latest
    trusted state is in `from_states` and the incoming event version is greater than or equal to the
    current marker version; late old events skip as stale.
  - Issue / PR **`fkst-dev:<state>` label = best-effort UI hint**. Each transition writes the target
    state set-exclusively and clears other state labels, but correctness does not depend on labels.
  - Other comment **HTML markers = attempt / consensus result / loop count / decomposition links**.
    When read as facts, they are also trusted only from this bot author and follow the existing
    `github-proxy` marker idempotency model.
  - **git branch / PR = implementation fact**.
  - Every poll re-derives state from GitHub / git. Business state is **not** stored in `<RT>` or
    cache; crash recovery is another poll.
  - GitHub has no atomic compare-and-append. All department transitions for the same issue use the
    same `with_lock` key to serialize in-process transitions. Marker writes are idempotent by dedup;
    every reliable delivery re-derives from source and self-heals labels / comments. There remains a
    small race window between read-CAS and asynchronous marker write, but old events cannot overwrite
    newer markers; the system converges under eventual-consistency semantics.
  - No-consensus no longer runs a meta-escalation codex. Convergence rounds are recorded in trusted
    bot converge-round / review-converge-round markers. True stall is handled by the deterministic
    `reconcile` department, which **does not run codex**. Inside `with_lock`, it re-derives state,
    idempotently skips visible same-round reconcile / review-reconcile markers, pins the current
    state and version segment, then writes `blocked`. Because reconcile is a deterministic `drop`
    decision, there is no remaining window where two nondeterministic codex workers can write
    contradictory results for the same version.
- **Safety**: opt in by `fkst-dev:enabled` label on issue / PR. `FKST_GITHUB_WRITE` is the only
  posture switch: unset means dry-run; `1` means direct autonomous real writes. Merge is still
  protected by deterministic gates: trusted markers, independent `review-result:v1 approve`,
  head-bound facts, CI / mergeability, `--match-head-commit`, and server-enforced branch protection.
  Each loop segment has a budget.

## 2. State Machine (Complete Target Transitions)

> Closure review added the target transitions for **failure paths** (implementing / fixing / merging
> failure), **pre-merge CI / conflict failure**, **manual escape** (label removed / changed), and
> **manual re-entry** (blocked reopened). The implemented issue segment currently performs only
> `nil -> thinking` through observe intake; the other escape / re-entry paths are target design.

State marker:
`<!-- fkst:github-devloop:state:v1 proposal="<id>" state="<S>" version="<dedup>" -->`.
Terminal states: `impl-failed`, `blocked`, `merged`. The `fkst-dev:<state>` label is only a
self-healing UI hint. `needs-human` means an unimplemented phase stops in that state for human
handling; later phases automate it. Loop counts use GitHub markers, not `<RT>`; after a crash,
polling re-derives them.

### Issue Segment

```text
 (unmanaged) --+fkst-dev:enabled--> intake --raise proposal--> thinking

 thinking --approve----------------> ready
 thinking --reject-----------------> (blocked)
 thinking --converge & not stall---> thinking          # self-loop: write converge-round marker, resend round+1 with narrowed_question
 thinking --converge & true-stall--> thinking          # router sees round>=3 and unchanged question+verdict digests across 3 rounds -> raise devloop_reconcile (state remains thinking)
 thinking --codex failed-----------> thinking          # reliable delivery retries automatically, no state advance

 # reconcile (deterministic, no codex; no split, no direct human escalation, no forced advance on no consensus)
 thinking --reconcile drop---------> (blocked)          # abandon this framing: no-actionable-framing-after-N-rounds

 ready --[P1] stop-----------------> needs-human
 ready --[P3] implement------------> implementing       # no push / no PR is currently prompt-level only

 implementing --ok----------------> pr-open
 implementing --fail--------------> impl-failed [needs-human terminal]
```

### PR Segment

```text
 pr-open --poll-------------------> reviewing
 pr-open --PR closed--------------> (blocked)

 reviewing --approve--------------> merge-ready
 reviewing --reject---------------> fixing
 reviewing --unresolved-----------> reviewing          # review_loop narrowed self-loop
 reviewing --true-stall-----------> (blocked)          # devloop_review_reconcile

 fixing --ok----------------------> reviewing
 fixing --no new head-------------> review-meta

 review-meta --fix----------------> fixing
 review-meta --block--------------> (blocked)

 merge-ready --CI+mergeable OK + review approve-> merging
 merge-ready --CI red/conflict----> fixing              # go back to fix; do not force merge
 merge-ready --missing write switch/CI pending--> merge-ready  # dry-run, no advance

 merging --ok---------------------> (merged) close issue
 merging --fail-------------------> retry               # merge races / command failures use reliable delivery retry
```

### Cross-Cutting Escape (Any State, Fail-Closed)

```text
 any state --fkst-dev:enabled removed-----------> (unmanaged) stop processing
 any state --state label changed to illegal/multiple--> next valid state-marker transition set-exclusive self-heals
 (blocked) --human removes blocked + re-adds enabled--> intake          # manual re-entry
```

## 3. Package Layout (Reuse > Extend > Create)

- **`consensus`** (reuse, unchanged): source-agnostic consensus engine. Both consensus segments use
  it.
- **`github-proxy`** (extend, keep thin I/O): issue / PR fact snapshots (labels + parsed markers),
  label write requests, marker comments. Label requests do not enforce state preconditions; they
  apply only best-effort UI hints. Later additions include issue-create / PR-create / PR-merge
  requests.
- **`autochrono` / `github-autochrono`** (unchanged): keep the simple reply flow; do not insert
  devloop logic.
- **`github-devloop`** (new composed package): the state-machine body: state <-> label mapping,
  converge-round counting, true-stall reconciliation, worktree implementation, and PR lifecycle.

## 4. Phases (Each Independently Shippable and Testable)

**Phase 0 (foundation quality, already in progress)**: consensus parser special-symbol labels
`⟦FKST:VERDICT⟧` / `⟦FKST:REPLY⟧` plus neutralization; `autochrono` lossless `proposal_id`.
Consensus is the state-machine core, so stabilize it first.

**Phase 1 (minimal recoverable loop)**: issue -> design consensus -> GitHub state / result writeback
+ no-consensus loop / stuck.

- Add a bounded no-consensus event to `consensus`. It was initially `consensus_unresolved` and was
  later redesigned as `consensus_converge` with `narrowed_question`; otherwise no-consensus is
  silent and the loop cannot drive.
- Add `github-proxy`: `github_entity_snapshot` (issue + labels + markers),
  `github_label_request`, and marker comments.
- Add `github-devloop` departments: `observe_issue` (opt-in snapshot -> `consensus.proposal`) and
  `consensus_result` (`consensus.consensus_reached` -> `ready|blocked` state marker + result comment
  marker + label hint).
- Loop: no-consensus marker counted retry; over budget -> `stuck` state marker (stop, Phase 2 takes
  over).
- Tests: opt-in filtering, approve -> ready, reject -> blocked, retry, budget -> stuck, dry-run
  external writes.

**Phase 2 (replaced by the converge -> reconcile redesign)**: originally stuck -> meta-escalation
(`ACTION: implement|split|block`). The current no-consensus model is convergence:
`loop` consumes `consensus_converge`, writes a converge-round marker, resends with
`narrowed_question` at round+1, and the router detects true stall (round >= 3 with unchanged
question+verdict digests across three consecutive rounds) -> `devloop_reconcile` -> deterministic
`reconcile` department `drop` to `blocked` without codex, child-proposal splitting, or direct human
escalation. Authoritative details are in `docs/dev/consensus-converge-redesign.md` and `README.md`.

**Phase 3**: `ready` CAS gates the attempt (`setup_worktree` + `spawn_codex` implementation). Failure
or no changes -> `impl-failed` state marker; changes -> `implementing` state marker +
branch/worktree marker. **Do not open a PR yet.**

**Phase 4**: with `FKST_GITHUB_WRITE=1`, run `gh pr create` + linkage marker. Dry-run records only
would-open; PR poll advances to reviewing.

**Phase 5a**: decision-only PR diff review consensus. When `observe_pr` enters `reviewing`, it
produces `devloop_reviewing`. `review_pr` re-fetches and confirms canonical issue state, then builds
a `github-devloop/pr-review/.../<head_sha>` `consensus.proposal` bound to the reviewed `head_sha`.
The payload carries only a short brief, `source_ref`, and `content_fetch`; consensus codex fetches
the complete PR diff and backing issue body from source. `review_result` re-reads the trusted PR
backpointer and current head, requires the current head to equal the reviewed `head_sha`, and CASes
the issue state marker to `merge-ready` on `approve` or `fixing` on `reject`. It also writes the
issue-versioned state marker, `review-result:v1` marker, `merge-ready:v1` fact marker, and
set-exclusive label. `approve` produces `devloop_merge_ready`; `reject` produces
`devloop_fixing`; it does not push or merge.

**Phase 5b (implemented; review side redesigned as converge -> reconcile)**: fix loop + review
convergence. `review_result` `reject` produces `devloop_fixing`. `fix` re-fetches and confirms the
canonical `fixing` marker, reject review marker, open same-repo PR, trusted PR origin, and
deterministic branch/head. If they all match, it runs codex in the deterministic branch worktree to
fix and commit. Updating the PR branch moves from dry-run to real writes only under
`FKST_GITHUB_WRITE=1`; before writing, it re-fetches issue / PR / head, performs non-force
`git push origin <branch>`, then verifies the PR head equals the new head. Success writes a new
`reviewing` marker whose version is the new-head fix-round canonical version from
`core.next_fix_version`, then produces `devloop_reviewing` again. Without the write switch, it does
not advance; with no changes, it enters `review-meta`. PR-review `consensus_converge` is consumed by
`review_loop`, which writes a `review-converge-round:v1` marker with `narrowed_question` and
re-reviews the same head. True stall produces `devloop_review_reconcile`, handled by `reconcile`
`drop` to `blocked`. `review_meta` maps `⟦FKST:ACTION⟧ fix|block` to `fixing|blocked`; it has no
`accept` path, and parse failures or ambiguity fail closed to `block`. It is no longer triggered by
review-loop budget; it is entered only when `fix` produces no new head, and it does not produce
`merge-ready`. The only `merge-ready` authority is PR-diff review consensus
`review-result:v1 approve`.

**Phase 6 (implemented)**: `merge` consumes `devloop_merge_ready`. Before writing, it re-fetches and
validates that canonical issue state is still the same-version `merge-ready` or retrying `merging`;
that a trusted head-bound `merge-ready:v1` comment-stream review-approval fact exactly matches the
event fields; that `review_proposal_id` still parses to the same repo / PR / version derivation
chain / reviewed `head_sha`; that `FKST_GITHUB_WRITE=1`; that a trusted `review-result:v1
decision="approve"` marker is bound to the same `review_proposal_id`, `review_dedup_key`, issue
proposal, reviewed `head_sha`, and version; and that the current PR is open, same-repo, with head
branch and `head_sha` unchanged, green `gh pr view --json statusCheckRollup`, and mergeable
`mergeable` / `mergeStateStatus`. `review_meta` has no `accept` path and cannot produce
`merge-ready`, so it cannot trigger merge. The only merge authority is the trusted head-bound
`review-result:v1 approve` backstop from PR-diff review consensus. `github-devloop` merge does not
use GitHub `reviewDecision`, `latestReviews`, or `addPullRequestReview`, and does not generate a
merge-time codex. Only after all gates pass does this bot write a trusted `merging:v1` marker, then
run ordinary `gh pr merge --merge --match-head-commit`, without admin override or branch-protection
bypass. It then writes the `merged` state marker, `merged:v1` marker, set-exclusive
`fkst-dev:merged`, and runs `gh issue close`. GitHub branch protection required status checks are a
real repo-ops prerequisite; the bot account must not have bypass / admin override. Lua
`statusCheckRollup` is only an early diagnostic backstop; the non-bypassable gate is GitHub's
server-side branch protection during `gh pr merge`. On retry, if the PR is still open with the same
head and not merged, all gates are re-derived and merge is attempted again. If the retry sees the PR
already MERGED, finalization is allowed only when this bot's matching current PR/head `merging:v1`
marker or canonical `merging` state is visible. External merge does not cause devloop to
automatically close the issue or write terminal markers. Missing trusted `review-result:v1 approve`,
missing trusted `merge-ready:v1` approval fact, missing write switch, pending CI, or unknown
mergeability stays dry-run or retry without advance. Red CI, clear unmergeability, or PR head
advance during pre-write re-fetch writes a `merge-gate:v1` marker and returns to `fixing`;
merge / close command failure retries as an error. Independence comes from codex context, proposal,
head-bound diff, and deterministic checks, not from GitHub account identity.

## 5. Key Risks and Doctrine Constraints

- No-consensus must not be silent: on disagreement, consensus emits bounded `consensus_converge`
  (meta-judge narrowing) and drives `loop` / `review_loop` convergence. Without it, only
  poll-timeout remains, with races.
- State transitions **must use only latest state-marker CAS**. Labels cannot distinguish stale
  replay from legitimate removal and are only UI hints.
- Converge-round / true-stall counters **must use GitHub trusted-bot markers**, not `<RT>` / cache.
- Version ordering for one issue is `(updated_at ISO, loop round N, stage_rank)`. At the same
  timestamp, the larger `/loop/N` (or PR-side `/review-loop/N`) beats no loop or a smaller loop even
  if the latter has a later stage. Reconcile is deterministic `drop` with no codex nondeterminism;
  same-round replay is idempotent through reconcile / review-reconcile markers, so GitHub comment
  return order cannot affect current state.
- PR diffs / issue bodies may exceed the **64 KiB payload** boundary, so payloads carry only
  `source_ref`, a short brief, and control fields. Codex / departments that need content fetch the
  complete content from source.
- Restart-completeness follows crash-only / event-sourcing replay: every non-terminal state must
  have a marker-only kickoff derivation. `observe_issue` replays initial `thinking`, complete
  `thinking` convergence rounds, `ready`, `pr-open`, `fixing`, and `review-meta` from trusted
  markers; PR-side observe / merge departments cover `reviewing`, `review-converge`, `merge-ready`,
  and `merging`. A manual PR head nudge is only a `reviewing`-state lever; `fixing` and
  `review-meta` recovery is observe-driven. Recovery does not depend on a live delivery. When
  `fixing` has no parseable feedback marker, observe deterministically re-enters `reviewing` for the
  current head instead of waiting for a manual head nudge.
- Automated child issues / PRs / merge carry **runaway + permission** risk. Use only
  `FKST_GITHUB_WRITE` to switch between dry-run and real autonomy, and keep strict budgets plus the
  deterministic merge backstop.
- Phase 3 implement no-push / no-PR constraints are currently expressed in prompts; host-level
  sandboxing is later hardening.
- Labels can be changed by humans; the next transition self-heals with set-exclusive labels. State
  facts still come from the latest state marker.
- Merge **does not bypass** branch protection or CI. Merge requires trusted head-bound
  `merge-ready:v1`, independent trusted `review-result:v1 approve`, `FKST_GITHUB_WRITE=1`,
  CI / mergeability / head gates. `review_meta` has no `accept` path, only `fix|block`, and does not
  participate in merge authorization. The repository must configure branch protection required
  status checks; the bot must not have bypass / admin override. The package does not query or
  configure branch protection.
- A real supervisor should start from a pinned engine / package revision, not mutable dev HEAD. A
  bad automatic merge can affect future repo state, but cannot change code for a running instance.
- Residual risks: a compromised bot account can forge trusted markers; LLM independent review is a
  bot-derived judgment, not objective proof; branch protection is ops configuration and cannot be
  enforced in Lua; `sshx` does not authorize commit / push / merge.

## 6. Open Points

- Opt-in label name: `fkst-dev:enabled`, or reuse the existing GitHub label system.
- True no-consensus stall uses deterministic reconcile `drop` to `fkst-dev:blocked`; old
  `fkst-dev:stuck` / meta-escalation has been removed. Implementation failure uses the separate
  terminal state `fkst-dev:impl-failed`.
