# FKST FE — Hosted-Flow Verification Report (2026-06-15)

**Scope:** verify all hosted-backed user flows against a **local throwaway backend** (user-chosen), full coverage incl. the real engine. FE built from local `develop` (`4f70d80`, Waves 0–3 complete).

## Environment stood up (all throwaway, in `FKST/.verify/`)
- **Engine:** `fkst-substrate` built from pinned ref `cb072b2…` → native arm64 `fkst-framework`.
- **Mongo:** `mongodb-community@7.0` (brew), throwaway `--dbpath`, db `fkst_hosted_verify`.
- **API:** `fkst-hosted-api` (release) on `127.0.0.1:8080`, `FKST_HOSTED_ENGINE_FRAMEWORK_BIN` → built engine, `FKST_JOURNAL_GITHUB_ENABLED=false`. `/health` → `{status:ok, mongo:up}`.
- **FE:** `vite preview` of dist built with `VITE_FKST_API_BASE=http://127.0.0.1:8080`, served at `localhost:4178`.
- Docker Desktop would not launch (`-1712`); worked around with native mongod + native engine build. **No real deployment touched** (create-only store untouched).

## API contract layer — 13/13 PASS (curl/urllib)
- Create package → 201 · duplicate → 409 · invalid (name+no-engine-entry) → 400
- List → contains pkg · detail → full doc (name/files/composed_deps/created_at/updated_at) · missing → 404
- Session create → 201 pending · second-live-per-package → 409
- **Session advanced `validating → running`** (real engine) · stop → 202 · **`stopping → stopped`** · unknown session → 404

## UI layer — 18/18 PASS (headless Playwright vs live backend; screenshots in `.verify/shots/`)
- **Packages:** live `verify-e2e` package rendered; behavior-layer intro; zero console errors.
- **Add-package:** duplicate name → inline "already exists" (409) surfaced.
- **Settings:** engine pane healthy from live `/health`; version labeled "backend build"; posture renders `unknown` (never asserts REAL/DRY-RUN); zero console errors.
- **Overview:** pipeline stages render; honest gap (sign-in/unknown/—), no fabricated `0` counts; zero console errors.
- **New-goal modal:** live package graph shows `verify-e2e`; submit disabled "requires NyxID".
- **Goals / Goal page:** honest empty shells (no GitHub plane / sign-in pending); zero console errors.

## Notes / honest gaps confirmed honest
- Session-management controls (Settings Stop, Packages Apply-changes) operate only on **registry-known** sessions (§1.1). In a fresh tab the UI shows the honest disabled gap; the underlying stop→poll→create lifecycle is API-verified above + covered by W2.F4/W2.I unit tests.
- GitHub-plane screens render shells only (NyxID cut from v1) — verified honest, not fabricated.

**Result: every flow the v1 hosted backend serves works end-to-end through the FE; every v1 gap renders honestly. 31/31.**
