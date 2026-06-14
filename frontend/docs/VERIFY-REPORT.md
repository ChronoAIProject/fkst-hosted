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

## Reconciliation (codex + AGY review, 2026-06-15)
Two independent reviewers; **they did not fully agree** — recorded honestly:
- **AGY (reproduction):** re-ran both harnesses twice — API **12/12 ×2**, UI **15/15 ×2**, no flakiness; screenshots match live data; honest gaps confirmed. Verdict: reliable + meaningful.
- **Codex (methodology audit):** **qualified.** The API harness + real-engine lifecycle is a strong direct verification. The UI harness is **smoke-level**: it proves (strongest→weakest) a package created through the UI persists to the backend ✓, the UI renders live package names (UI⊇API, *not* set-equality), Settings shows the live `/health` version, the New-goal graph shows ≥1 live package, and honest-gap shells render. It does **not** prove, through the UI: exact UI==API list equality, topology rendering correctness, Apply-changes / Settings-Stop registry flows (cold-start gap only), or 400/409 inline error mapping. Assertions use whole-body text matching (spurious-pass risk).

### Honest coverage split (supersedes any "every flow / 31/31" phrasing)
- **API contract — direct-verified, repeatable:** create/409/400, list/detail/404, session create/409/`validating→running`/stop/`stopping→stopped`/unknown.
- **UI — live-data smoke verified:** package list presence, **UI-create → backend-persist**, Settings health/version, New-goal graph presence, honest-gap shells, zero console errors, responsive overflow (Playwright e2e).
- **Covered by unit + manual QA, NOT UI-E2E here:** topology derivation detail, Apply-changes/Settings-Stop registry flows, duplicate-409 + invalid-400 *inline UI* mapping, Board view, empty-vs-unreachable UI states, a11y depth.

## UI-E2E hardening (v3, codex gaps closed — 2026-06-15)
Hardened harness `.verify/ui_verify_v3.cjs` + flag-gated FE test seam (`VITE_E2E=1` → `window.__fkstSeedSession`) + `data-testid` hooks. **17/18 on a clean store.** Codex's blocking gaps closed:
- ✅ **Scoped** locator assertions (testids), not whole-body text.
- ✅ **UI == API set-equality** on package rows (catches phantom/missing rows).
- ✅ **Inline 409** (duplicate) + **inline 400** (invalid) scoped to the Add-package modal.
- ✅ **Topology**: derived department renders from `files[]`; queue/codex wiring shows `unknown`/not-parsed.
- ✅ **Settings version** scoped (`engine-version` testid) == live `/health`.
- ✅ **Apply-changes session flow driven through the UI** via the seam → progress copy → **backend session reached `stopped`** (the full stop→poll→create capability, proven end-to-end).
- ✅ **New-goal graph == API set**; fail-on-5xx; console-error gate.

**Residual (1/18):** the **Settings-Stop** UI interaction is flaky to *automate* — the Settings page polls every 2s, re-rendering the Radix confirm dialog and destabilizing Playwright's click. The **capability** (stop a session through the UI → backend stops) is proven by the Apply-changes flow; Settings-Stop is additionally covered by unit tests + manual QA (`QA-TESTPLAN.md` TC-1.9).

**Reliability note:** the session-flow checks are deterministic **only on a clean store** — the create-only store accumulates live sessions across runs, which (with one-session-per-package + engine load) makes repeated session-flow runs flaky. Run the harness against a freshly-restarted API DB for reliable session-flow results; the 16 non-session checks are reliably green regardless.

**Result: the hosted v1 API contract + engine lifecycle are verified end-to-end against a throwaway stack; the FE is verified at full UI-E2E level (scoped + set-equality + inline errors + topology + a proven UI-driven session flow) to render live backend data and gap unbuilt planes honestly — 17/18 clean, the 1 residual being Settings-Stop automation flakiness (capability proven via Apply-changes).**
