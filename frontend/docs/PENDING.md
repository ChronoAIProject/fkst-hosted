# FKST Frontend — Pending / Outstanding Items

As of 2026-06-15. The FE (Waves 0–3) is implemented, dual-reviewed, merged to local `develop`, and verified against a live hosted backend. This records what is **not done / deferred / to-correct** so a human picks up with full context. Branch: `feat/frontend-init`.

## 1. Pull request — BLOCKED on repo access
- `ctkm-aelf` has read-only (`push: false`) on `ChronoAIProject/fkst-hosted`; forking disabled. No PR can be opened yet.
- Ready: implementation on `feat/frontend-init`; draft PR body at `FKST/PR-DRAFT.md`; changesets present; all local gates green.
- When write access is granted, per `CLAUDE.md`: open a `feature_request` issue → push branch → `gh pr create` into `develop` with `Closes #N` → ensure changeset → auto-merge on green.

## 2. UI-E2E verification — ✅ RESOLVED (2026-06-15)
Hardened harness `.verify/ui_verify_v3.cjs` now passes **19/19, three consecutive runs (1 clean store + 2 populated)** against the live throwaway backend. Evidence: `.verify/ui_verify_v3_result.log`. Every previously-owed correction was closed *by strengthening the harness + re-verifying*, not by relabeling:

- **Apply-changes "stop→poll→create" — now fully proven.** Added the `__fkstGetSession` read-only seam (`src/lib/hooks/session-registry.tsx`, flag-gated under `VITE_E2E`); the harness now selects the package, drives Apply, and asserts the old session reached `stopped` **and** a **new** session id was created (`progress=true oldFinal=stopped newSession=new`).
- **Server-authoritative 400 — now proven.** A `../escape.lua` path-traversal file (passes client Zod, rejected by the backend per `packages/model.rs` rule 3f) is verified end-to-end: network 400 observed, inline error rendered, package **not** persisted (`srv400=true created=false`). The client-side Zod 400 path is also still asserted — two distinct 400 paths.
- **Settings-Stop — now proven + deterministic.** The earlier "flake" was a **selector bug**, not Radix/polling: every package row renders a "Stop session" button (disabled without a tab session), so `.first()` matched a disabled button when another package sorted first. Fixed by adding `data-testid={`stop-session-${packageName}`}` to the enabled trigger (`settings-screen.tsx`) and targeting it directly → `ack=true final=stopped`.
- **Stale totals relabeled.** `VERIFY-REPORT.md` "owed" banner removed; the historical "UI layer — 18/18" section is marked superseded; the authoritative result is the v3 19/19 section.
- **Reliability — resolved.** Passes on both a clean and a populated store (unique packages per run + scoped selectors). Engine prerequisite recorded: `FKST_HOSTED_ENGINE_TEMP_ROOT` must pre-exist, else the engine fails session start with "io error" and sessions terminate `failed` instantly (environment, not FE).

Source changes (on `feat/frontend-init`): the `__fkstGetSession` seam + the `stop-session-<pkg>` testid. Both are production-safe (`data-testid` is inert in prod; the seam stays strictly `VITE_E2E`-gated).

## 3. Upstream backend issues to file (FE renders honest gaps until they land)
- `GET /api/v1/sessions?package_name=` (session lookup) — removes the §1.1 cold-start gap so the UI can manage sessions it didn't create this tab.
- Posture/config **read** endpoint — so Settings shows real `FKST_GITHUB_WRITE` + deployment knobs instead of `unknown`.
- Tighten backend **CORS** from `allow_origin(Any)` to the real FE origin before production.

## 4. v1 scope gaps — by design (honest disabled now; light up later)
- NyxID auth + all GitHub-plane data & actions (Overview/Goals/Goal live data; create-issue/label/comment/close-PR).
- Host-agent / redb / runs diagnostics.
- All render disabled + honest notes; tracked for the M4–M6 milestones in `IMPLEMENTATION-PLAN.md`.

## 5. Local verify stack (throwaway) — teardown when done
- Currently running for verification: `mongod` :27017, `fkst-hosted-api` :8080 (real engine wired), `vite preview` :4180 (built with `VITE_E2E=1`).
- Throwaway artifacts live under `FKST/.verify/` (engine build, mongo-data, harness scripts, screenshots) — outside the repo.
- Teardown: kill the three processes; `rm -rf FKST/.verify`; optional `brew uninstall mongodb-community@7.0`.

## 6. Notes
- The frontend hasn't been pushed, so `frontend-ci.yml` has not run on a remote — it is correct by construction + green local gates (`npm ci → lint → typecheck → build → vitest → build-storybook`, + Playwright e2e).
- The `VITE_E2E` seam is strictly flag-gated. **Never build/deploy production with `VITE_E2E=1`.**
- Docker Desktop would not launch on the verification host (`-1712`); the stack was stood up natively (built engine + brew mongod). Not an FE concern.

## 7. Housekeeping / decisions awaiting the maintainer
- **Branch + worktree cleanup**: ~20 superseded per-task branches exist (`fe-w0-*` … `fe-w3-*`, `fe-w2-f34-topology`, `fe-w2-x-integration`, `fe-e2e-hardening`, `fe-e2e-hardening-pm`, `fe-qa-testplan`) plus their heca worktrees — all merged into local `develop` and folded into `feat/frontend-init`. Safe to prune the merged branches + archive the worktrees, leaving `main`, `develop`, `feat/frontend-init`. Not yet done.
- **Local `develop` vs the feature branch**: the FE landed by merging the per-task branches directly into local `develop` (local-only mode). `feat/frontend-init` is the conventional branch carrying the same state for the eventual PR. If strict `CLAUDE.md` flow matters, `develop` should be reset to `origin/develop` and the work flow only through `feat/frontend-init` → PR. Not done (no impact while local-only).
- **Verify stack**: leave running (FE preview :4180, API :8080, mongod :27017) for exploration, or tear down per §5 — awaiting your call.
- **PR trigger**: open the PR the moment write access is granted (§1).

## 8. Task tracker (this engagement)
- Done: Waves 0–3 (build+review+merge), hosted-flow verification, codex+AGY reconcile, FE-init branch + this doc, **UI-E2E hardening final pass (§2 — 19/19 ×3, all codex gaps closed)**.
- Open (decisions/access, not code): PR when write access lands (§1), file the 3 upstream backend issues (§3), branch/worktree cleanup (§7), verify-stack teardown (§5). No substantive FE work item remains.
