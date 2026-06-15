# FKST FE — Hosted-Flow Verification Report (2026-06-15)

> ✅ **Codex re-audit gaps closed (2026-06-15).** The earlier wording overclaims were resolved by *strengthening the harness and re-verifying*, not by relabeling: the `__fkstGetSession` seam now lets the harness assert a **new** session id after Apply-changes; a **server-authoritative 400** (path-traversal) case was added; the Settings-Stop "flake" was traced to a selector bug (`.first()` hitting a disabled row) and fixed with a scoped `data-testid`. **Hardened harness = 19/19, three consecutive runs (1 clean store + 2 populated).** The authoritative result is the **v3 hardening** section below; older per-section totals are historical.

**Scope:** verify all hosted-backed user flows against a **local throwaway backend** (user-chosen), full coverage incl. the real engine. FE built from local `develop` (Waves 0–3 complete); UI-E2E hardening on `feat/frontend-init`.

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

## UI layer — initial smoke pass (historical; superseded by the v3 19/19 hardened result below)
- **Packages:** live `verify-e2e` package rendered; behavior-layer intro; zero console errors.
- **Add-package:** duplicate name → inline "already exists" (409) surfaced.
- **Settings:** engine pane healthy from live `/health`; version labeled "backend build"; posture renders `unknown` (never asserts REAL/DRY-RUN); zero console errors.
- **Overview:** pipeline stages render; honest gap (sign-in/unknown/—), no fabricated `0` counts; zero console errors.
- **New-goal modal:** live package graph shows `verify-e2e`; submit disabled "requires NyxID".
- **Goals / Goal page:** honest empty shells (no GitHub plane / sign-in pending); zero console errors.

## Notes / honest gaps confirmed honest
- Session-management controls (Settings Stop, Packages Apply-changes) operate only on **registry-known** sessions (§1.1). In a fresh tab the UI shows the honest disabled gap. The stop→poll→create lifecycle behind those controls is now **UI-E2E proven** via the flag-gated seam (see v3 §Settings-Stop / §Apply-changes), in addition to API verification + W2.F4/W2.I unit tests.
- GitHub-plane screens render shells only (NyxID cut from v1) — verified honest, not fabricated.

## Reconciliation (codex + AGY review, 2026-06-15)
Two independent reviewers; **they did not fully agree** at the smoke stage — recorded honestly, with the codex gaps subsequently **closed by the v3 hardening** (19/19):
- **AGY (reproduction):** re-ran both harnesses twice — API **12/12 ×2**, UI smoke ×2, no flakiness; screenshots match live data; honest gaps confirmed. Verdict: reliable + meaningful.
- **Codex (methodology audit, smoke stage):** **qualified.** The API harness + real-engine lifecycle is a strong direct verification. The UI harness was then **smoke-level**: it proved a UI-created package persists ✓, UI renders live package names (UI⊇API, not set-equality), Settings shows the live `/health` version, the New-goal graph shows ≥1 live package, honest-gap shells render — but did **not** then prove, through the UI: exact UI==API equality, topology correctness, Apply-changes / Settings-Stop flows, or 400/409 inline mapping; assertions used whole-body text (spurious-pass risk).
- **Resolution (v3, this report):** every one of those was closed — scoped testids replace whole-body matching, UI==API set-equality on packages + new-goal graph, topology assertion, inline 409, **both** client-side and server-authoritative 400, and **both** session lifecycles (Settings-Stop, Apply-changes-with-new-session) driven through the UI. **19/19 ×3.**

### Coverage split (current, v3)
- **API contract — direct-verified, repeatable:** create/409/400, list/detail/404, session create/409/`validating→running`/stop/`stopping→stopped`/unknown.
- **UI — UI-E2E verified (scoped, 19/19 ×3):** UI==API package set-equality, UI-create→backend-persist, inline 409, client-side **and** server-authoritative 400, topology derivation, Settings health/version, **Settings-Stop** and **Apply-changes (stop→poll→create-new)** session lifecycles, New-goal graph==API set, honest-gap shells, zero console/5xx errors, responsive overflow (Playwright e2e).
- **Covered by unit + manual QA (not in this harness):** Board view, empty-vs-unreachable UI permutations, a11y depth.

## UI-E2E hardening (v3, codex gaps closed — 2026-06-15) — **19/19**
Hardened harness `.verify/ui_verify_v3.cjs` + flag-gated FE test seam (`VITE_E2E=1` → `window.__fkstSeedSession` / `__fkstClearSessions` / `__fkstGetSession`) + `data-testid` hooks. **19/19, three consecutive runs (1 clean store + 2 populated; evidence `.verify/ui_verify_v3_result.log`).** Every codex blocking gap closed *by verification*:
- ✅ **Scoped** locator assertions (testids), not whole-body text.
- ✅ **UI == API set-equality** on package rows (catches phantom/missing rows).
- ✅ **Inline 409** (duplicate) scoped to the Add-package modal.
- ✅ **Two distinct 400 paths**, both scoped to the modal: **client-side** (Zod rejects a bad name / missing engine entry before any POST) **and server-authoritative** (a `../escape.lua` path-traversal that *passes* client validation, the backend rejects with **400**, the network 400 is observed, the inline error renders, and the package is **not** persisted — `srv400=true created=false`).
- ✅ **Topology**: derived department renders from `files[]`; queue/codex wiring shows `unknown`/not-parsed.
- ✅ **Settings version** scoped (`engine-version` testid) == live `/health`.
- ✅ **Settings-Stop driven through the UI** (scoped `stop-session-<pkg>` testid → confirm dialog → ack copy) → **backend session reached `stopped`** (`ack=true final=stopped`).
- ✅ **Apply-changes driven through the UI** (select package → Apply) → progress copy → **old session reached `stopped`** *and* a **new session id created** (read back via `__fkstGetSession`) — the full stop→poll→create cycle, end-to-end (`progress=true oldFinal=stopped newSession=new`).
- ✅ **New-goal graph == API set**; fail-on-5xx; console-error gate on every screen.

**Settings-Stop note (was the prior "residual"):** the earlier flakiness was **not** Radix/polling — it was a selector bug. The Settings page renders a "Stop session" button for *every* package (disabled when this tab holds no session for it), so `getByRole('button',{name:/stop session/i}).first()` matched a **disabled** button whenever another package sorted first. Fixed by adding a scoped `data-testid={`stop-session-${packageName}`}` to the enabled trigger and targeting it directly. Now deterministic. (Still also covered by unit tests + manual QA, `QA-TESTPLAN.md` TC-1.9.)

**Reliability:** robust to store state. The harness creates unique packages per run, sessions reach terminal states, and all selectors are scoped — so it passes on both a clean store and a populated one (verified across the three runs). The prior "clean-store-only" caveat is resolved. Engine prerequisite: `FKST_HOSTED_ENGINE_TEMP_ROOT` must pre-exist (a missing temp root makes the engine fail session start with "io error" and every session terminates `failed` instantly — an environment, not an FE, issue).

**Result: the hosted v1 API contract + engine lifecycle are verified end-to-end against a throwaway stack; the FE is verified at full UI-E2E level — scoped assertions + UI==API set-equality + inline 409 + both client-side and server-authoritative 400 + topology + two UI-driven session lifecycles (Settings-Stop and Apply-changes stop→poll→create-new) — to render live backend data and gap unbuilt planes honestly. 19/19, three consecutive runs.**
