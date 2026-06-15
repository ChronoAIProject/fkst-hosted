# FKST Frontend — Pending / Outstanding Items

As of 2026-06-15. The FE (Waves 0–3) is implemented, dual-reviewed, merged to local `develop`, and verified against a live hosted backend. This records what is **not done / deferred / to-correct** so a human picks up with full context. Branch: `feat/frontend-init`.

## 1. Pull request — BLOCKED on repo access
- `ctkm-aelf` has read-only (`push: false`) on `ChronoAIProject/fkst-hosted`; forking disabled. No PR can be opened yet.
- Ready: implementation on `feat/frontend-init`; draft PR body at `FKST/PR-DRAFT.md`; changesets present; all local gates green.
- When write access is granted, per `CLAUDE.md`: open a `feature_request` issue → push branch → `gh pr create` into `develop` with `Closes #N` → ensure changeset → auto-merge on green.

## 2. UI-E2E verification — corrections still owed (codex re-audit)
Verified on a clean store (**17/18**, `.verify/ui_verify_v3.cjs`): scoped `data-testid` assertions, **UI==API set-equality** (packages, new-goal graph), inline **409** (duplicate), client-side validation error, topology smoke (derived dept + unknown wiring), scoped Settings version == live `/health`, **Apply-changes UI→backend old-session-stopped**, flag-gated seam safety.

**To correct / finish (deferred):**
- **`VERIFY-REPORT.md` wording still overclaims** (codex re-audit flagged):
  - "Apply-changes → stop→**poll→create**" — the harness only asserts the *old* session stopped. To claim full create, assert a **new** session is created (a `__fkstGetSession` seam addition + assertion is drafted in `.verify` but **unverified** — not committed).
  - "inline **400**" — the verified case trips **client-side** Zod, not the server. A **server-authoritative 400** (path-traversal that passes client validation, backend rejects, confirmed via the 400 response) is drafted in `.verify` but **unverified**.
  - Remove/relabel the stale **v1/v2** sections that conflict with the v3 result (e.g. "UI layer — 18/18").
  - **Settings-Stop**: state "shared stop API + one UI stop path (Apply-changes) proven; Settings-Stop UI is unit/manual covered" — *not* "proven via Apply-changes".
- **Settings-Stop UI automation is flaky**: the Settings page polls every 2s, re-rendering the Radix confirm dialog so Playwright's click never stabilizes (tried wait-for-visible, force-click, retry-to-open). Capability is otherwise covered by unit tests + manual QA (`QA-TESTPLAN.md` TC-1.9).
- **Reliability**: session-flow checks are deterministic **only on a clean store** (the create-only store accumulates live sessions; with one-session-per-package + engine load, repeated runs go flaky). The harness should reset/namespace a fresh API DB per run; today it relies on restarting the API on a fresh `MONGODB_DB`. The 16 non-session checks are reliably green regardless.

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
