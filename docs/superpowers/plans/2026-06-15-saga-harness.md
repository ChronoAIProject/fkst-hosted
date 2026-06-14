# SAGA Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make "来了就做，做过就不做" the only writable department shape by lifting the existing `core/saga.lua` `effect_once` from per-effect to per-department, with an auto-applied idempotency+progress oracle and a shrink-only ratchet that forces migration.

**Architecture:** `std/saga.lua` exposes `department{consumes, done, act, ...}` — the framework owns the `if done(e) then skip else act(e)` control flow; the author fills only `done`/`act`. A shared test helper drives the ①②③ oracle (deliver once → progress; deliver twice + restart → no-op; near-key new → not-done) against a stateful external-truth fake. A `check_repo.py` ratchet + `migration/saga-handler.allowlist` forbids new free-form departments and drains the old ones one PR at a time.

**Tech Stack:** Lua, `fkst.test.run_department` / `command_calls` / `mock_command`, `scripts/check_repo.py`.

Spec: `docs/superpowers/specs/2026-06-14-saga-harness-design.md` · Depends on: `2026-06-15-std-shared-library.md` (Tasks 1–2 must be done first).

---

### Task 1: `std/saga.lua` — the `department{done, act}` constructor

**Files:**
- Create: `std/saga.lua`
- Test: `std/tests/saga_department_test.lua` (or a package-hosted probe per the std test-discovery decision)

- [ ] **Step 1: Write the failing test** with a synthetic department

```lua
local saga = require("std.saga")
local t = fkst.test

local function make(done_value, calls)
  return saga.department{
    name = "probe",
    consumes = { "probe_q" },
    done = function(_event) return done_value end,
    act = function(_event) calls.n = (calls.n or 0) + 1; return "acted" end,
  }
end

return {
  test_spec_exposes_consumes = function()
    local m = make(false, {})
    t.eq(m.spec.consumes[1], "probe_q")
  end,
  test_act_runs_when_not_done = function()
    local calls = {}
    make(false, calls)
    local r = pipeline({ queue = "probe_q", payload = {} })
    t.eq(calls.n, 1); t.eq(r, "acted")
  end,
  test_skips_when_done = function()
    local calls = {}
    make(true, calls)
    pipeline({ queue = "probe_q", payload = {} })
    t.is_nil(calls.n)
  end,
}
```

- [ ] **Step 2: Run, verify it fails** — Run: `scripts/run.sh test <host pkg>` → FAIL (`module 'std.saga' not found`).

- [ ] **Step 3: Implement `std/saga.lua`**

```lua
local S = {}

local function validate(opts)
  if type(opts) ~= "table" then error("std.saga.department requires opts") end
  if type(opts.consumes) ~= "table" or #opts.consumes == 0 then
    error("std.saga.department requires a non-empty consumes list")
  end
  if type(opts.done) ~= "function" then error("std.saga.department requires done") end
  if type(opts.act) ~= "function" then error("std.saga.department requires act") end
end

-- Lift effect_once from per-effect to per-department: done == completion_check,
-- act == perform. "来了就做，做过就不做".
function S.department(opts)
  validate(opts)
  local spec = {
    consumes = opts.consumes,
    produces = opts.produces,
    stall_window = opts.stall_window,
    retry = opts.retry,
    fanout = opts.fanout,
    ephemeral = opts.ephemeral,
  }
  local raw = function(event)
    if opts.done(event) then
      if opts.on_skip then opts.on_skip(event) end
      return nil
    end
    return opts.act(event)
  end
  -- Tier R may inject a failure-fact wrapper (e.g. github-devloop's
  -- core.wrap_pipeline_failure); default is bare propagation.
  local fn = raw
  if type(opts.wrap) == "function" then fn = opts.wrap(opts.name or "department", raw) end
  _G.pipeline = fn
  return { spec = spec }
end

return S
```

- [ ] **Step 4: Run, verify it passes** — Run: `scripts/run.sh test <host pkg>` → PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(std/saga): department{done,act} constructor lifting effect_once to per-event"`

---

### Task 2: The ①②③ idempotency oracle + stateful external-truth fake

**Files:**
- Create: `std/saga_conformance.lua` (the oracle helper)
- Create: `std/truth_fake.lua` (in-memory gh/marker state: records writes → serves reads)
- Test: `std/tests/saga_conformance_test.lua`

- [ ] **Step 1: Write the failing oracle test** — a synthetic department that writes
  a marker on first delivery and is idempotent on the second.

```lua
local conf = require("std.saga_conformance")
local t = fkst.test
-- assert_idempotent runs: ① deliver once (act runs, ≥1 write-class effect),
-- ② deliver again against the truth the first wrote (zero NEW write-class effects).
return {
  test_idempotent_department_passes = function()
    conf.assert_idempotent(t, {
      dept = "departments/probe/main.lua",
      event = { queue = "probe_q", payload = { id = "x" } },
    })
  end,
  test_non_idempotent_department_fails = function()
    t.raises(function()
      conf.assert_idempotent(t, { dept = "departments/leaky/main.lua",
        event = { queue = "probe_q", payload = { id = "x" } } })
    end)
  end,
}
```

- [ ] **Step 2: Run, verify it fails.**

- [ ] **Step 3: Implement `std/truth_fake.lua`** — a table that records write-class
  external commands (configurable verb set: `gh issue comment`, `gh ... --add-label`,
  `gh pr merge`, marker writes) and replays them as reads (`gh issue view` returns
  accumulated comments/markers). Unmodeled commands fail-closed.

- [ ] **Step 4: Implement `std/saga_conformance.lua`** using `fkst.test.run_department`
  twice with the truth fake threaded between deliveries, comparing the write-class
  `fkst.test.command_calls` multiset (delivery 2 must add zero). Provide
  `assert_idempotent(t, {dept, event})` and `assert_progress(t, {...})` (① act
  produced an effect or a raise) and `assert_predicate_not_wide(t, {dept, event,
  near_key_event})` (③).

> Classification of "write-class" lives in `std` (one list), so the rule is
> uniform across packages. Read-class commands may repeat freely.

- [ ] **Step 5: Run, verify it passes. Commit.**
`git commit -m "feat(std/saga): idempotency oracle (assert_idempotent/progress/predicate) + truth fake"`

---

### Task 3: Ratchet G-gate + allowlist

**Files:**
- Create: `migration/saga-handler.allowlist` (lists every current free-form department)
- Modify: `scripts/check_repo.py` (add `check_saga_handler_ratchet`)
- Test: `scripts/check_repo_test.py`

- [ ] **Step 1: Write the failing test** — a department NOT on the allowlist and NOT
  built via `saga.department` → `G10` violation; one ON the allowlist → no violation;
  adding a brand-new file not on the allowlist and not new-shape → violation.

- [ ] **Step 2: Run, verify it fails.**

- [ ] **Step 3: Generate the initial allowlist** — list all current
  `packages/*/departments/*/main.lua` (they are all free-form today).

```bash
ls -1 packages/*/departments/*/main.lua | sort > migration/saga-handler.allowlist
```

- [ ] **Step 4: Implement `check_saga_handler_ratchet`** — for each
  `packages/*/departments/*/main.lua`: if it `require`s `std.saga` and calls
  `.department{` → new shape (OK, must not be on allowlist). Else it must be on the
  allowlist (grandfathered). A path neither new-shape nor on the allowlist → `G10`.
  The allowlist may only shrink: fail if any listed path is missing OR if the list
  grew vs `git show HEAD:migration/saga-handler.allowlist` (best-effort; if git
  unavailable, just enforce membership). Register in `main()`.

- [ ] **Step 5: Run test + full check, verify green. Commit.**
`git commit -m "feat(saga): shrink-only ratchet (G10) + initial allowlist"`

---

### Task 4: Migrate the first leaf department (validate the shape end-to-end)

Pick a simple scan/tick reconciler in `github-devloop` (e.g. `doctor` or
`ensure_repo` — choose the one with the smallest, clearest done/act split after
reading it).

**Files:**
- Modify: `packages/github-devloop/departments/<leaf>/main.lua`
- Modify: `migration/saga-handler.allowlist` (remove the migrated line)
- Modify/Create: its `*_test.lua` to call `std.saga_conformance.assert_idempotent`

- [ ] **Step 1:** Read the department; identify the `done` predicate (its existing
  completion check / marker read) and the `act` body (its effects).
- [ ] **Step 2:** Rewrite it as `require("std.saga").department{ consumes=…, done=…,
  act=…, wrap = core.wrap_pipeline_failure, name="<leaf>" }`.
- [ ] **Step 3:** Remove its line from `migration/saga-handler.allowlist`.
- [ ] **Step 4:** Add `assert_idempotent` (and `assert_progress`) to its test.
- [ ] **Step 5:** Run `scripts/run.sh test github-devloop` → PASS (incl. the oracle and the ratchet now requiring this dept be new-shape).
- [ ] **Step 6:** Commit — `git commit -m "refactor(github-devloop): migrate <leaf> to std.saga department shape"`

---

### Task 5: Migrate a second department + record the gradual remainder

- [ ] **Step 1:** Repeat Task 4 for `decompose` (already uses `effect_once`, so the
  `done`/`act` split is the cleanest second case).
- [ ] **Step 2:** Confirm the allowlist shrank by two; full suite green.
- [ ] **Step 3:** In the PR body, record: Phase 2 drains the remaining departments
  one PR each (ideal devloop dogfood); Phase 3 (engine loader rejection of
  free-form + dynamic-delivery property tests) is fkst-substrate work, out of
  scope here per spec §8. Commit any docs note.

---

## Self-Review

- **Spec coverage:** Tooth 1 shape (Task 1), Tooth 2 oracle (Task 2), Tooth 3 gate
  (Task 3 ratchet; merge-gate required-check wiring noted as CI config), migration
  Phase 0/1 (Tasks 1–3) + Phase 2 first strikes (Tasks 4–5); Phase 3 engine items
  explicitly out of scope (Task 5 Step 3).
- **Placeholders:** the std test-discovery open item (shared with Plan 1 Task 1
  Step 7) is the only deferred decision, with a stated fallback.
- **Naming:** `saga.department`, `done`/`act`/`wrap`/`on_skip`, `assert_idempotent`,
  `G10`, `migration/saga-handler.allowlist` consistent across tasks.

⟦AI:FKST⟧
