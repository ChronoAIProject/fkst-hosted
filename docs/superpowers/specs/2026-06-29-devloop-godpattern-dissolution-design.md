# devloop "god-lib" dissolution — SPEC (deep-adversarial: 3 codex constructive + 3 refuters + ChatGPT Pro)

Status: design. Produced by sshx deep-adversarial consensus over `libraries/devloop` (103 files / 21,621 LOC). Constructive (minimal/structural/delete) + 3 adversarial refuters (behavior / m-table / boundary) + ChatGPT Pro oracle + the prior audit `2026-06-23-libs-redecomposition-design.md`.

## 0. The adversarial finding that reframed the goal (实事求是)

The literal framing — "relocate devloop out to a ~20-file kernel" — is **REFUTED** by converging evidence:

1. **Prior audit already judged devloop a cohesive single-product kernel, NOT a god-lib** (`docs/superpowers/specs/2026-06-23-libs-redecomposition-design.md:15`): "devloop (20k): mostly REFUTED as a problem — a large cohesive single-product kernel (one consumer family). Deeper splits (merge, autonomy_ledger, commands, schema/policy) DEFERRED until a real trigger (no current non-devloop consumer)." The named triggers (三次法则) have not fired.
2. **Relocate-out violates G9**: `git_mechanics` is required (via lib_deps) by 5 packages, `github_proxy_entity_view` by 8. Moving either into one sibling forces the others to peer-require that sibling — G9 forbids it.
3. **Demote-to-forge/workflow games the metric + leaks policy**: `git_mechanics` (`github-devloop:` lock keys/validation), `github_proxy_entity_view` (devloop cache/marker-read authority policy), `context_bundle` (github-devloop issue/PR/diff/risk), `autonomy_ledger` (trusted devloop markers + version ordering), `prompts` (devloop actor/review semantics) are **product-policy modules**. Moving them to generic libs just relocates the god-lib boundary (workflow's no-policy-string ratchet forbids it anyway).
4. **The hard coupling is the mutable late-bound `M` + ordered installers, NOT file placement** (m-table refuter). File movement reduces line count while preserving full `M` access = big-bang rewrite disguised as relocation.

**Therefore the real, achievable, evidence-backed target is to dissolve the god _PATTERN_, not relocate files:** kill the shared mutable-`M` `install(M)` pattern (→ typed modules with explicit deps), kill the `devloop.*` wildcard export (→ explicit exports), fix the one confirmed generic-logic leak, and lock it with a symbol-level coupling ratchet. devloop stays a cohesive typed kernel; its files mostly stay; relocation/demotion stays DEFERRED behind the prior audit's triggers.

## 1. Measurable "no god-lib" = a COUPLING metric (NOT file/line count) — `scripts/check_repo_devloop_godlib.py` + shrink-only `migration/devloop-godlib.inventory`
- `libraries/devloop/**`: `function *.install(M)` / `S.install(M)` count → shrink-only → **0**.
- assignments to a shared install receiver (`M.<symbol> = …` inside install fns) → per-symbol shrink-only allowlist → **0** (modules export typed tables / `new(deps)` instead).
- `packages/*/core.lua`: `require("devloop.*").install(M)` calls → shrink-only → **0**.
- `libraries/devloop/fkst.toml` exports: `devloop.*` wildcard → **explicit ≤N module paths** (no wildcard).
- visibility: shrinks to lifecycle owners only as packages stop importing devloop.
- **replay-coverage invariant** (safety): every `restart_lifecycle_states` entry + every handoff kind MUST have a registered replayer — CI-enforced — so no typed-extraction slice can strand a live state (the boundary refuter's FATAL #1).
- Size is secondary/advisory; the BINDING target is the coupling counts above (file-count alone is gameable).

## 2. Layering (most stays in devloop as a typed kernel)
| Concern | Disposition |
|---|---|
| version/CAS state ordering, state-marker grammar, restart table, liveness/replay contract, convergence facts, comment_handoff, lifecycle conformance, merge/decompose/autonomy/prompts/context_bundle | **STAY in devloop**, converted to typed modules (no install(M)). Cohesive single-product kernel. |
| the ONE confirmed generic leak: generic liveness/restart validator (no `github-devloop` strings; markers/queues injected as data) | → **workflow** (per prior audit; fixes archaudit→devloop). Verify if already landed on dev. |
| merge subsystem, autonomy_ledger, commands→forge, schema/policy splits | **DEFERRED** behind the prior audit's named triggers (三次法则; no non-devloop consumer yet). |
| relocate to sibling packages | **REJECTED** for shared modules (G9). |

## 3. Mechanism: dissolve shared-M install → typed modules (incremental, behavior-preserving)
- A module stops calling `install(M)`; it `return`s a table or `new(deps)` with explicit named deps. Consumers switch to `local x = require("devloop.x")` (+ inject deps), and that module's `install(M)` is deleted in the SAME PR. No shim, no dual surface.
- Package `core.lua` keeps a package-LOCAL flat facade for departments/tests, but composes typed deps itself; devloop modules never mutate it.
- **Break the load-order cycle FIRST** (m-table refuter FATAL #2): `ready_split` ↔ `replayer` are coupled via late-bound `M.resolve_replay_payload_fields` / `M.replay_ready_state`. Extract a typed **replay-field / fact-resolution seam** before any replayer work.
- Ratchet bans new install(M)/M.* immediately; inventory shrinks only; per-module declared deps so hidden ambient-M deps fail mechanically.

## 4. Ordered slices (each: one PR, CI-gated, behavior-preserving — byte-equal markers/payloads/queues/transitions; any semantic change = SEPARATE PR)
1. **Ratchet baseline + replay-coverage invariant.** `check_repo_devloop_godlib.py` + inventory at current counts (install(M), M.* writes, wildcard export); + the replay-coverage invariant over restart_lifecycle_states. Growth forbidden. (no runtime diff — pure conformance.)
2. **KEYSTONE (corrected): typed replay-field / fact-resolution seam.** Extract the shared replay-field resolver (`resolve_replay_payload_fields` etc.) into a typed module to break the `ready_split`↔`replayer` late-bound-M cycle. Proof: restart/replay/hidden-state tests byte-equal effects.
3..N. **Convert one concern at a time to typed modules**, deleting its install(M) (start low-fan-in: e.g. strings/logging/parsers/config → then markers/payloads/state → then replayers). Each: golden marker/payload/transition equality + per-concern ratchet shrink.
N+1. **Wildcard export → explicit** `libraries/devloop/fkst.toml` exports.
N+2. **The one generic-leak fix** (liveness validator → workflow) IF not already on dev; drop archaudit→devloop.
(Merge/decompose/autonomy relocation: DEFERRED — out of scope until a trigger fires.)

## 5. Risks / non-goals
- Live state machine: replay-coverage invariant (slice 1) + per-slice byte-equal golden + hidden-state/restart conformance. The keystone is the cycle-break seam, NOT PR-replayer relocation (that drags half of M / can strand — refuted).
- Boundary: no relocation of shared modules (G9). No demotion of product-policy modules to generic libs (metric gaming).
- Semantic drift: any changed effect/transition/terminal/dedup-key/marker → separate behavior-change PR.
- Scope honesty: this dissolves the god-PATTERN (install/M/wildcard coupling). It does NOT relocate the cohesive kernel out; that remains deferred per the prior audit's triggers. "No god-lib" = coupling metric at 0, not file count at ~20.
- Engine: no Rust changes unless a primitive is proven needed → separate fkst-substrate PR.

⟦AI:FKST⟧
