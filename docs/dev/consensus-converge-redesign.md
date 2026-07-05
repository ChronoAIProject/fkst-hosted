# Consensus Convergence Redesign: From Blind Loops and Splitting to Meta-Judge Convergence and True-Stall Reconciliation

> **Status (2026-06): Phase 1 and Phase 2a of this redesign have been fully implemented and merged
> (PRs #44-#49).** The "previous state" section below describes the system before the redesign. The
> current authoritative state is in `README.md` (the consensus / `github-devloop` sections) and
> `CLAUDE.md`. Phase 2b, which makes consensus transparent by posting the three angles plus the
> meta-judge decision as comments and covers #11/#15, is described in the staging section.

## Motivation

Dogfood findings plus maintainer review showed that the prior no-consensus handling split too much
and was not correct. The long-running `consensus-rnd` design from `sshx` / `codex-refactor-loop`
has the right shape: a **meta-judge convergence** model where disagreement converges only to fixed
exits (reached / converging / true stall), and only true stall enters a meta-layer reconciliation.
It must **never split a proposal into child proposals and never directly escalate to a human**.

### Previous State

- `packages/consensus/decide`: three angles ran concurrently and used **unanimity aggregation**
  (all approve / all reject -> `consensus_reached`; otherwise `consensus_unresolved`). There was
  **no fourth meta-judge**, no preserved three-angle output for an arbiter, and no narrowed question.
- `github-devloop/review_loop` and `loop`: on `consensus_unresolved`, they **blindly reran** nearly
  the same proposal, changing only the `/loop/N` dedup segment and not narrowing the question. At
  `loop_budget=3`, they entered `devloop_stuck` / `devloop_review_meta`.
- Failure terminal path: the thinking-side `meta` single-codex path selected
  implement / **split** / block (`split` was the core problem, producing `blocked` plus
  "Suggested split"); the PR-side `review_meta` single-codex path selected fix / accept / block.

### Target: The `consensus-rnd` Model

```text
proposal(round R, [narrowed_question])
  -> 3 angles concurrently (peer-invisible, see narrowed_question but not one another)
  -> meta-judge (4th codex, reads the 3 angles) decides:
       reached:<framing>             -> consensus_reached (reached is final)
       converge:<narrowed_question>  -> consensus_converge (narrowed question + bounded angle digest)
  -> consumer router receives converge:
       resend proposal with round R+1 + narrowed_question, back to consensus
  -> repeat until reached, or until the router detects true stall
       (round >= 3 and angle positions show no narrowing or change across consecutive rounds)
  -> true stall -> reconcile:
       drop(no-actionable-framing) / re-design(concrete new directive) / re-cluster
       without split and without directly labeling a human
```

**Responsibility split** that keeps `consensus` a source-agnostic flat package:

- `packages/consensus`: for each proposal, run "angles -> meta-judge" and produce only
  `consensus_reached` or `consensus_converge`. It holds no state and no round counter.
- `github-devloop` as the composed router: owns convergence **rounds**, the true-stall
  **predicate**, and **reconciliation**. Rounds and digests are recorded in GitHub trusted-bot
  markers (marker-as-fact plus version CAS).

## Event Contract Changes (`packages/consensus`, Flat Package)

- `proposal` (consumed): add `round` (default `0`), optional `convergence_question` for this round's
  narrowed question, and optional `prior_round_digests` with bounded prior-angle summaries. The
  digests respect the 64 KiB payload boundary and carry only verdict + short reply + digest; they
  **do not expose full prior peer text**, preserving peer invisibility.
- `consensus_reached` (produced): unchanged, and remains the only "reached" exit.
- **Replace `consensus_unresolved` with `consensus_converge`** (produced): payload contains
  `proposal_id`, `round`, `narrowed_question`, bounded `angle_digests`, `source_ref`, and
  `dedup_key`. This is a complete contract replacement: remove `consensus_unresolved` from the
  current state.

## Department Changes

### `packages/consensus`

- `decide/main.lua`: after the three concurrent angles, do **not** directly aggregate unanimity.
  Add, or split into `judge/main.lua`, a fourth-codex **meta-judge** using `prompts/meta_judge.lua`.
  It reads the three angle outputs and emits either `reached:<framing>` or
  `converge:<narrowed_question>`, raising `consensus_reached` or `consensus_converge`. The
  meta-judge does **not** see the next round; it only generates the narrowed-question summary.

### `github-devloop`

- `review_loop` and `loop`: change from blind budget rerun to consuming `consensus_converge`,
  resending `proposal` with `round+1` and `narrowed_question`, and writing a converge-round marker
  (`proposal` / `round` / `dedup` / `question` / angle digest).
- **True-stall predicate** as router logic: read trusted markers bound to the same
  proposal / `source_ref` / version / head. If `round >= 3` and consecutive rounds show no
  meaningful narrowing or angle-text change, raise `devloop_review_reconcile` /
  `devloop_reconcile`. Rounds 1 and 2 cannot be stalled.
- **New reconciler department** consuming reconcile events: produce `drop` / `re-design` /
  `re-cluster`. `drop` writes terminal `blocked` with no actionable framing, but the semantics are
  "abandon this framing", not "split".

## Deletions

- Remove the `meta` `split` action. Abandon the #31/#35 direction of "make meta split execute";
  that was the wrong splitting model. The thinking-side stuck path changes from
  implement / split / block to the true-stall reconciler. The implement exit remains a reached
  consensus path.
- Remove the `consensus_unresolved` queue, the `review_loop` / `loop` blind budget-rerun path, and
  related constants, helpers, and tests.
- Do not use PR-side `review_meta accept` as a convergence substitute. **Keep** the merge gate's
  requirement for independent trusted `review-result approve` / `merge-ready` facts; `accept` is
  insufficient to merge.

## Staging

Each stage should go through the full `sshx` flow: thinking -> meta -> implementation -> review ->
PR -> CI -> merge.

- **Phase 1 — consensus engine**: add the meta-judge to `decide` (angles -> judge ->
  reached|converge); replace `consensus_unresolved` with `consensus_converge` carrying
  `narrowed_question` + bounded digest; add `round`, `convergence_question`, and `prior_digests` to
  proposal schema; update consensus unit tests and conformance. Downstream remains temporarily
  compatible by treating `consensus_converge` like the old unresolved event in `github-devloop`, so
  behavior is unchanged during staging.
- **Phase 2 — `github-devloop` convergence wiring**: `review_loop` / `loop` consume
  `consensus_converge`, resend with `narrowed_question`, write converge-round markers, route true
  stall to reconcile, add the reconciler department, and **remove `meta` split plus blind budget
  rerun**.
- **Phase 3 — cleanup**: remove dead code and old #31/#35 traces; update
  `docs/dev/devloop-design.md` and related memory.

## Risks and Constraints From the `sshx` Three-Angle Review

- **Peer invisibility must be preserved**: the meta-judge must not feed full prior peer text to the
  next round's angles; it may give only the narrowed question summary that it generated.
- **64 KiB payload boundary**: `prior_round_digests` carries only verdict + short reply + digest.
- **The PR-diff narrowed question must be precise**: it is not "review the whole diff again"; it is
  "judge only whether prior disagreement point X blocks merge or requires a fix". Otherwise the
  loop remains blind.
- **No hard round cap, but keep a cost guard**: the guard triggers only true-stall reconciliation,
  not direct block / split.
- Large-issue **decomposition** is a separate intentional mechanism, such as `consensus-rnd` #403
  epic -> child design issues, with gates. It is **not** automatic consensus splitting; this redesign
  does not introduce automatic decomposition.

⟦AI:FKST⟧
