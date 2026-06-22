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
- **Keep the operator checkout pinned on `dev`; make EVERY change in a dedicated worktree via a PR — never edit the pinned checkout.** The checkout this skill + `dogfood.sh` are loaded from stays on `dev` (pull to keep current; never commit to it, leave it on a feature branch, or modify its working tree — a stale or feature-branch checkout silently loads a stale skill/tooling and dirties the reference of what `dev` actually is). Make every operator change — an out-of-band hotfix, a skill / `dogfood.sh` / doc edit, anything — in a SEPARATE git worktree cut from `dev` (`git worktree add <path> -b <type>/<topic> origin/dev`), commit on that branch, and open a PR to `dev` (CI-gated, squash-merged); the pinned checkout then `git pull`s `dev` to pick it up. The dogfood supervises run from their own `dogfood.sh`-managed checkouts (separate again), kept current by `dogfood.sh restart`. This is the operator side of "autonomous changes flow integration → rollup → dev": even out-of-band operator work lands through a reviewed PR, not a dirty checkout.
- Trust only marker-as-fact (bot-authored state markers) with version-CAS; GitHub is eventually-consistent (read-after-write lag), not strong-consistency - expect transient "marker not yet visible; retrying".
- When unsure or facing a design-layer problem, do NOT improvise a workaround that bypasses a deliberate gate, and do NOT pop up a question — run `sshx` to think it through and decide, and file an issue for the record.

## Operating loop

The operator tooling is **`dogfood.sh`** in this skill directory (`.claude/skills/dogfood-github-devloop/dogfood.sh`) — one DRY multi-tool that encodes the launch env, the restart-to-deploy invariant, the BIN-freshness rebuild, and the board sweep, so each wake is a few clean calls instead of ad-hoc bash. Commands: `status | doctor | config | board | bin | start | stop | restart | logs` (each takes `[packages|substrate|website|all]`).

**Per-machine config (we run 3 repos across 2 machines).** The script is identical on every host; device-specific values — `DOGFOOD_ROOT` (worktree/scratch base), `BOT` (gh login), `INTEGRATION_BRANCH` (e.g. `integration` vs `integration-<device>`), `DOGFOOD_REPOS` (which repos this host drives), and the STABLE `DUR_*` durable roots — are sourced from `dogfood.config.sh` (gitignored, per-machine; copy `dogfood.config.example.sh`). Everything has a generic default (paths derive from `$DOGFOOD_ROOT`), precedence is env > config > default. Run `dogfood.sh config` to print what THIS host resolved (verify the `DUR_*` match the running supervises before a restart, or it strands in-flight events). A second machine just needs its own `dogfood.config.sh` (its `BOT`, `integration-<device>`, paths, durable roots) — no script edits.

**Checkout discipline: main checkout = `dev`, read-only; all work in worktrees.** Keep the main checkouts (`~/fkst-packages`, `~/fkst-substrate`) on `dev`, current, and treat them as a clean, immutable `dev` mirror — NEVER run the framework (`scripts/run.sh test/run/supervise`) or create work branches in a main checkout. The engine writes `.fkst/runtime*` scratch relative to its project-root, so running in a main checkout pollutes it (scattered `.fkst/runtime*`, plus per-package `packages/<pkg>/.fkst/` when a package is the project-root) and drifts the branch. All work runs in independent worktrees: dogfood supervises in their `$DOGFOOD_ROOT/*-dogfood` worktrees (`dogfood.sh` handles this), and out-of-band fixes/PRs in a fresh `git worktree add … origin/dev` removed after merge. (Editing a skill/config text file in the main checkout is not "running" — it does not pollute — but persist it via a worktree PR like any change.)

**Directory layout (one base dir per machine; replicate on every host).** Every dogfood checkout, durable store, log, and runtime scratch lives under `$DOGFOOD_ROOT` (per-machine, e.g. `~/.fkst-dogfood`), so a second machine reproduces the whole setup by pointing its `dogfood.config.sh` at its own `$DOGFOOD_ROOT`. Each target dir holds the target's own clone (`HOST` = the supervise's `--project-root`) and, for the non-`packages` targets, a sibling fkst-packages clone (`PKGSRC`) that supplies the shared devloop trio. `dogfood.sh` derives these paths and syncs them; create them once (one clone per slot):

```
$DOGFOOD_ROOT/                          # per-machine base (config: DOGFOOD_ROOT; everything below derives from it)
  pkgs-dogfood/                         # fkst-packages clone — HOST==PKGSRC for the `packages` target
    packages/{github-devloop,github-proxy,consensus,…}   # committed package SOURCE (platform trio + the whole library)
  website-dogfood/
    site/                               # fkst-website clone — HOST (project-root for the website supervise)
      packages/site-board/              #   the website's OWN custom package, committed at its repo-root packages/
    pkgs/                               # fkst-packages clone — PKGSRC (supplies the trio to the website supervise)
  substrate-dogfood/
    sub/                                # fkst-substrate clone — HOST (project-root)
    pkgs/                               # fkst-packages clone — PKGSRC (supplies the trio to the substrate supervise)
  durable/<name>/                       # STABLE redb durable roots (DUR_*) — REUSED across restarts, never freshed on a normal restart
  <name>-sv-<ts>.log                    # supervise logs (LOGDIR)
  dogfood-rt-<name>.<ts>/               # fresh runtime scratch per launch (pruned by dogfood.sh)
```

**Packages load from repo-root `packages/`, never `.fkst/`.** `.fkst/` in ANY checkout is engine RUNTIME/build only (gitignored: `runtime/`, `durable/`, `substrate-src/`, `board-cache.json`) — it holds no committed code. Committed package source lives at repo-root `packages/<pkg>` in every repo. So `dogfood.sh` wires `--package-root <repo>/packages/<pkg>`: the shared trio (`github-devloop`/`github-proxy`/`consensus`) from `$PKGSRC/packages/`, plus each target's repo-local custom package (e.g. website's `site-board`) from `$HOST/packages/`. The trio is NOT vendored into target repos; it is supplied by that target's sibling `pkgs` clone. The three fkst-packages clones (`pkgs-dogfood`, `website-dogfood/pkgs`, `substrate-dogfood/pkgs`) are each synced independently — intentional isolation so concurrent restarts never race on one git tree, not duplicated logic.

**The dogfood RUNS ON the machine's integration branch, not `dev`.** `dogfood.sh restart`/`sync` put every dogfood run checkout (each target's `PKGSRC` behavior clone + its `HOST`) on `INTEGRATION_BRANCH` (e.g. `integration-<device>`), so the supervise loads this device's OWN pre-rollup code — it is the live integration test of its autonomous changes BEFORE they promote to dev (`feature → integration-<device> → rollup PR → dev`). The rollup target stays `dev` (`FKST_DEVLOOP_UPSTREAM_BRANCH`); the integration branch auto-syncs forward from dev (`sync_scan` ff's `dev → integration-<device>`, so it is always ≥ dev); the engine BIN still builds from `~/fkst-substrate` on `dev`; and the pinned operator/skill checkout (`~/fkst-packages`) stays on `dev`. So `doctor` compares the running behavior against `integration-<device>` (PKG-freshness) and the engine against `dev` (ENGINE-freshness).

1. **Run / restart** with `dogfood.sh restart [name|all]`. It ensures the engine BIN is fresh, checks out the run checkouts to the machine's `INTEGRATION_BRANCH` (see Directory layout — rollup target stays `dev`), SIGKILLs the old supervise (releasing the redb lock), and relaunches detached with real write posture (`FKST_GITHUB_WRITE=1`), a fresh runtime scratch root, and the SAME stable durable root. Two invariants the script encodes (preserve them if you ever launch by hand):

   - **Restart-to-deploy** (per `CLAUDE.md`): `FKST_RUNTIME_ROOT` is scratch (fresh each launch); `FKST_DURABLE_ROOT` is the redb **persistent** store and is **REUSED across restarts** so in-flight persisted events resume. A *fresh* durable root throws the durable queue away and **strands mid-state issues** (stuck at `ready`/`implementing`, never re-triggered — #62/#78). Fresh it ONLY for a deliberate clean-slate wipe.
   - **BIN export + freshness**: `BIN` is exported in the env (not only `--framework-bin`) so the spawned implement/fix codex can run `scripts/run.sh test` (its worktree has no `.env`). AND — a direct `BIN supervise` launch does **NOT** auto-build like `scripts/run.sh` does, so the BIN silently goes **stale** when substrate `dev` moves (an engine `.rs` fix merges); `dogfood.sh` rebuilds it from substrate `origin/dev` before every start/restart (detect stale = substrate `origin/dev` ahead OR any crate `.rs` newer than the BIN; `dogfood.sh bin` does it standalone). A supervise on a stale BIN silently re-runs already-fixed engine defects.

2. **Keep the running code current — `dogfood.sh sync` does it in one call; only a RESTART reloads.** The supervise loads `packages/` and execs the engine BIN at STARTUP; syncing the worktree files or rebuilding the BIN does NOT reload a running process. So after ANY merge to `dev` (your out-of-band hotfix or an autonomous rollup) — **and after any substrate engine merge** — run **`dogfood.sh sync`**: it fast-forwards the pinned operator checkouts to `dev`, rebuilds the BIN, and **auto-restarts ONLY the supervises whose RUNNING code is a real package/engine change** (`pkg-stale`/`engine-stale`), leaving skill/docs-only `skew` and already-current processes running (a restart would only churn in-flight codex for no code change). Run it every wake (and after every merge you land); it is the standard keep-current action — `restart` is the unconditional force-restart for when you explicitly need one. Do NOT sync a worktree without restarting to "avoid churn": it leaves the process on stale code while the worktree looks current, hiding the staleness (observed: a supervise silently 22 engine-commits behind, re-running already-fixed defects, while the worktree read `current`). `dogfood.sh doctor` reports the **running process's** loaded `PKG_VERS`/`ENGINE_VER` (from its startup `code_provenance` log line) against the run branch — PKG vs `integration-<device>`, ENGINE vs `dev`: `PKG-STALE`/`ENGINE-STALE` means the live process is stale regardless of the worktree (and `sync` will restart it); `pkg-skew` means the run branch moved but only non-`packages/` files (no restart needed). It is the authoritative freshness signal — the worktree/BIN-file checks only say the NEXT restart will be current.
3. Watch it. Use "What to observe". If state keeps advancing, do NOT intervene - observe, and file issues only for real system defects.
4. When it stalls, go to the Stall decision tree.

## What to observe

**Every activation, FIRST sweep the full board with `dogfood.sh board` — do not skip to logs.** It classifies every open issue/PR across both repos (`fkst-dev:<state>` label + per-PR CI + recency) into `✓ flowing` / `tracking` / `parked` vs `⚠ STUCK` / `STRANDED` / `CI-RED` / `NO-CI`. The label is a fast UI hint; for any item flagged `⚠`, cross-check its authoritative `state:v1` marker / the linked PR before acting (a `STUCK implementing` can be a live codex whose issue `updated_at` is just old). The goal is to catch, per item, two failure shapes the log alone hides:

- **Stuck**: an item whose state has not advanced across passes (e.g. parked at `ready`/`reviewing`/`fixing`/`thinking` with no new marker), or an issue sitting with no state at all (intake never decided, or `decline`-dead-ended).
- **Misbehaving**: an item that transitioned WRONG vs the expected state machine — a sound issue `decline`d or `blocked`, a review reject that should have been a converge, a PR that re-opened/churned, a redundant autonomous PR duplicating an out-of-band fix, a `+0/-0` rollup.

Only after the board sweep, check the running process (`dogfood.sh doctor` rolls up supervise liveness + panic + code currency + BIN freshness + graphql in one line each):

- Supervise alive + 0 panic.
- State transitions advancing: consensus, review, implement, and merge activity; not only `github_poll`.
- Churn regression absent: `integration-<device> == dev`, no `+0/-0` rollup PR.
- No recurring `dead_letter publish failed` in steady-state. If it recurs, it is a real robustness gap, such as marker-lag retry exhaustion.
- Reviews not stuck in `reviewing` across runs with no transition. That is a mid-loop stall.
- GraphQL quota healthy.

A stall means consensus/review activity stops while only polling continues. A board item stuck or mis-transitioned with the pipeline otherwise flowing is ALSO a defect — diagnose it (often impoverished codex context: truncated input, no code access, or a terminal-reject gate that should converge), file a consensus-rnd-informed issue, and drive it.

**Platform composition is a config edit, never a skill edit.** Which agent packages the supervise loads + runs is `DEVLOOP_PKGS` in the per-machine `dogfood.config.sh` — that one list is where you look and where you change; `dogfood.sh` and this skill carry no package names, so adding a package as `packages/` grows never touches them. To decide what to load: `dogfood.config.example.sh` documents WHERE TO LOOK (each package's role = its `fkst.toml` kind + its depts' `consumes`/`produces`) and the CO-RUN RULE (an issue-PRODUCER agent — consumes a non-issue signal, produces github-proxy issue/comment requests, e.g. an audit agent — co-runs safely and files work into github-devloop's own intake; an issue-CONSUMER agent that claims/manages the issue lifecycle must run as its own separate supervise or it fights github-devloop). When such a producer agent is enabled, observe its filed output like any codex output — correct cadence (not a flood), gated to the right trigger (e.g. firing only when idle, not while the repo is busy), and SUBSTANTIVE (evidence over narrative); a flood of low-value items or output at the wrong time is a defect to diagnose + file.

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

- Land it: from the dedicated worktree (cut from `dev`, never the pinned operator checkout), push the branch and open a PR to `dev`. This is the out-of-band exception: the broken pipeline cannot carry its own fix. Watch CI; squash-merge when green.
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

### Re-entry by operator command (GitHub-surface, NOT state mutation)

Trusted operator comments (first line `fkst: <cmd>`, authored by the bot login) re-enter a stuck item without hand-mutating state. Each has a precondition — using the wrong one is **refused, not forced** (and you must not force it):

- `fkst: rereview` — re-runs consensus on a **stalled `thinking`** issue (requires the thinking to be stalled, not active); or re-triggers review on a `reviewing`/`blocked`/`review-meta` issue with an **OPEN** PR.
- `fkst: reready` — re-triggers implement on an issue at `ready`.
- `fkst: reimplement` — re-enters implement on an `impl-failed` issue.
- `fkst: reintake` — re-runs intake; **refused if the issue has any active devloop state** (only for a declined/stateless issue).

**Known re-entry GAPS — the old-instance-strand class. File/drive a SYSTEM issue; do NOT force a command past its precondition and do NOT hand-mutate the marker.** A fix prevents FUTURE failures but does not revive instances already frozen before it landed, and some frozen states have NO working operator re-entry at all:

- `pr-open`-frozen with the backing PR gone → no command fits (#271/#606 → #760).
- `thinking`/convergence-stalled with **version-desync** → `rereview` is applied but its replayed consensus result skip-stales (marker `intake/NNNN` vs proposal `<ts>/loop/N`), `reintake` refused (active state), `reready`/`reimplement` wrong-state (#542/#577).
- `reviewing`/`fixing` PR that can never obtain CI → merge gate never satisfiable (substrate #84/#83 → #103).

All the SAME class: a non-terminal marker no path advances, producing **zero error facts** (liveness-blind — the safety net sees nothing). The durable fix is framework-level, not another per-state patch: the liveness sweep must **force-terminate** ANY over-budget non-terminal state to `blocked`-with-WHY, mechanically conformance-enforced (carrier **#762**, wave under saga-mandatory **#375**). When you hit a fresh instance, drive that class issue rather than minting a new point-fix.

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
