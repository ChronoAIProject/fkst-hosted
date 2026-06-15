# gh/git Ports & Adapters Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every package off direct `gh`/`git` programming onto a neutral Tier R adapter (`std.github` / `std.git`), so business departments speak domain operations and tests assert behavior at the port — not literal command strings (resolves #633).

**Architecture:** Three surfaces — S1 neutral adapters (`std.github`/`std.git`, build+exec+parse → neutral tables), S2 package-owned durable write-intents (`github-proxy.v1`), S3 package-owned marker/CAS guards. Departments receive port handles via a `make_department(ports)` constructor; production binds real adapters, tests bind in-memory fakes. Migration is a vertical-slice strangler: Wave 0 builds the foundation, then one department per slice (reads-first) moves its builders private into the adapter, exposes domain ops, rewrites its tests to port fakes, deletes the old builders, and closes a conformance ratchet on its file.

**Tech Stack:** Lua on the fkst-substrate engine; `std/` shared lib (symlinked per package); `scripts/run.sh test` (conformance + `*_test.lua`); `fkst.test.mock_command` / `command_calls` / `capture_raises`; `scripts/check_repo.py` G-gates. No engine (Rust) change.

**Spec:** `docs/superpowers/specs/2026-06-15-ports-adapters-design.md` (v5). Read §4 (three surfaces), §5.6 (injection seam), §6 (oracle), §9 (migration) before executing.

---

## Decomposition note (read first)

This is one subsystem (the gh/git adapter) but a large migration across ~20 command-using departments. This plan is fully concrete for the parts executed first and shared by all:

- **Part A — Wave 0** (Tasks 0.1–0.6): the foundation everything depends on. Full bite-sized TDD. Execute first, out-of-band-style (additive, behavior-neutral; nothing migrates yet).
- **Part B — Slice template**: the exact repeatable procedure every per-dept slice follows. Defined once (DRY).
- **Part C — Exemplar slice (`loop`)**: Part B applied end-to-end to the smallest read-only department, with complete code. The first real migration and the worked reference for all later slices.
- **Part D — Per-dept backlog**: the ordered list of remaining department slices, each grounded to its real ops/builders/files. Each backlog entry becomes one autonomous-pipeline issue (or one subagent task); its concrete per-step code is generated at execution time against that department's then-current source — that is the vertical-slice design, not a placeholder. Part C is the worked example each one mirrors.

Do **not** expand Part D into 20 copies of Part C up front — that violates the strangler design (each slice must be cut against live code) and YAGNI. File Part D entries as issues; each is implemented via Part B + Part C as reference.

---

## Part A — Wave 0: Foundations

Behavior-neutral and additive: no department changes, no builder moves. CI stays green throughout. The two existing `gh_exec` wrappers (`github-proxy/core/gh_rate.lua:38`, `github-devloop/core/base.lua:918`) stay live and are deleted only by the final slice, after their last caller migrates.

### Task 0.1: Prove nested `std/` require resolves (risk R1 spike)

**Files:**
- Create: `std/github/probe.lua`
- Create: `std/git/probe.lua`
- Test: `packages/github-proxy/tests/std_nested_require_test.lua`

- [ ] **Step 1: Write the failing test**

```lua
-- packages/github-proxy/tests/std_nested_require_test.lua
local gh_probe = require("std.github.probe")
local git_probe = require("std.git.probe")

return {
  test_nested_std_require_resolves = function()
    assert(gh_probe.ok == true, "std.github.probe must resolve")
    assert(git_probe.ok == true, "std.git.probe must resolve")
  end,
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `scripts/run.sh test github-proxy`
Expected: FAIL — `module 'std.github.probe' not found`.

- [ ] **Step 3: Create the probe modules**

```lua
-- std/github/probe.lua
return { ok = true }
```
```lua
-- std/git/probe.lua
return { ok = true }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `scripts/run.sh test github-proxy`
Expected: PASS (`std.github.probe` resolves via the `?.lua` package.path substitution one directory deeper).

**If it FAILS** (engine loader does not walk nested dirs): switch the whole plan to flat naming — `std/github_issue.lua` requireable as `std.github_issue`, etc. Record the decision at the top of this plan and rename all `std/github/<m>` → `std/github_<m>` and `std.github.<m>` → `std.github_<m>` in later tasks. Re-run Step 4 with `std/github_probe.lua`.

- [ ] **Step 5: Commit**

```bash
git add std/github/probe.lua std/git/probe.lua packages/github-proxy/tests/std_nested_require_test.lua
git commit -m "test(std): prove nested std.github.* / std.git.* require resolves (R1 spike)"
```

### Task 0.2: Canonical adapter exec wrapper + `new(exec)` handle skeleton

The new adapter owns ONE exec wrapper (rate-limit detection + `error_class` facts), lifted from `github-proxy/core/gh_rate.lua` and `github-devloop/core/base.lua`. The old wrappers stay until their callers migrate.

**Files:**
- Create: `std/github/shell.lua` (private toolkit: `shell_single_quote`, `url_encode`, `is_git_ref_safe`)
- Create: `std/github/exec.lua` (the gh exec wrapper + error classification)
- Create: `std/git/exec.lua` (the git exec wrapper)
- Create: `std/github.lua` (entry: `new(exec)` → handle)
- Create: `std/git.lua` (entry: `new(exec)` → handle)
- Test: `packages/github-proxy/tests/std_github_exec_test.lua`

- [ ] **Step 1: Write the failing test** (pins rate-limit + error_class behavior, copied from the current `is_gh_rate_limited` / `gh_error_class` semantics in `github-proxy/core.lua:147-176`)

```lua
-- packages/github-proxy/tests/std_github_exec_test.lua
local t = require("fkst.test")
local gh = require("std.github")

return {
  test_exec_classifies_rate_limit = function()
    local handle = gh.new(function(_cmd)
      return { stdout = "", stderr = "API rate limit exceeded for user", exit_code = 1 }
    end)
    local ok, err = pcall(function() return handle._exec("gh api x", 10, "ctx") end)
    assert(ok == false)
    assert(err.class == "gh-rate-limited", "rate-limit stderr must classify as gh-rate-limited")
    assert(err.retryable == true)
  end,
  test_exec_classifies_generic_failure = function()
    local handle = gh.new(function(_cmd)
      return { stdout = "", stderr = "fatal: not found", exit_code = 1 }
    end)
    local ok, err = pcall(function() return handle._exec("gh api y", 10, "ctx") end)
    assert(ok == false)
    assert(err.class == "gh-command-failed")
  end,
  test_exec_returns_result_on_success = function()
    local handle = gh.new(function(_cmd) return { stdout = "ok", stderr = "", exit_code = 0 } end)
    local out = handle._exec("gh api z", 10, "ctx")
    assert(out.stdout == "ok")
  end,
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `scripts/run.sh test github-proxy`
Expected: FAIL — `module 'std.github' not found`.

- [ ] **Step 3: Implement shell.lua, exec.lua, and the handle**

```lua
-- std/github/shell.lua
local M = {}
function M.shell_single_quote(v) return "'" .. tostring(v):gsub("'", "'\\''") .. "'" end
function M.url_encode(v) return (tostring(v or ""):gsub("([^%w%-%._~])", function(c) return string.format("%%%02X", string.byte(c)) end)) end
-- is_git_ref_safe: copy verbatim from github-proxy/core.lua:53-78 (do not weaken the validation)
function M.is_git_ref_safe(v) --[[ paste the exact body from github-proxy/core.lua:53-78 ]] end
return M
```
```lua
-- std/github/exec.lua  (rate-limit + error_class, lifted from github-proxy/core.lua:147-198)
local M = {}
local function stderr_of(r) return type(r) == "table" and tostring(r.stderr or "") or "" end
function M.is_rate_limited(result)
  local s = stderr_of(result):lower()
  for _, needle in ipairs({ "api rate limit exceeded", "secondary rate limit", "was submitted too quickly",
                            "http 429", "status 429", "429 too many requests", "too many requests" }) do
    if s:find(needle, 1, true) then return true end
  end
  if s:find("abuse", 1, true) and s:find("rate", 1, true) then return true end
  return false
end
function M.error_class(result) return M.is_rate_limited(result) and "gh-rate-limited" or "gh-command-failed" end
function M.run(exec, cmd, timeout, context)
  local result = exec({ cmd = cmd, timeout = timeout })
  if type(result) ~= "table" or tonumber(result.exit_code) ~= 0 then
    local class = M.error_class(result)
    error({ class = class, retryable = class == "gh-rate-limited", result = result,
            message = "std.github: " .. tostring(context) .. " failed: " .. class .. ": " .. stderr_of(result) })
  end
  return result
end
return M
```
```lua
-- std/github.lua  (entry; grows one op per slice)
local exec_wrap = require("std.github.exec")
local M = {}
function M.new(exec)
  assert(type(exec) == "function", "std.github.new requires an exec function")
  local handle = {}
  function handle._exec(cmd, timeout, context) return exec_wrap.run(exec, cmd, timeout, context) end
  -- domain ops (read_issue, read_pr, create_issue, ...) are added per slice.
  return handle
end
return M
```
```lua
-- std/git.lua  (entry; mirror of std.github.new)
local M = {}
function M.new(exec)
  assert(type(exec) == "function", "std.git.new requires an exec function")
  local handle = {}
  function handle._exec(cmd, timeout, context)
    local r = exec({ cmd = cmd, timeout = timeout })
    if type(r) ~= "table" or tonumber(r.exit_code) ~= 0 then
      error({ class = "git-command-failed", result = r, message = "std.git: " .. tostring(context) .. " failed" })
    end
    return r
  end
  return handle
end
return M
```
(Create empty `std/git/exec.lua` returning `{}` for now, or fold its trivial wrapper into `std/git.lua` as above.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `scripts/run.sh test github-proxy`
Expected: PASS (3 exec tests green).

- [ ] **Step 5: Commit**

```bash
git add std/github.lua std/git.lua std/github/shell.lua std/github/exec.lua std/git/exec.lua packages/github-proxy/tests/std_github_exec_test.lua
git commit -m "feat(std): canonical gh/git adapter exec wrapper + new(exec) handle skeleton"
```

### Task 0.3: Tier S abstract effect/truth oracle interface

The saga ①②③ oracle observes effects through this interface so it stays GitHub-agnostic (Tier S). It records two effect kinds: S1 adapter writes (from a fake) and S2 raised intents (from `capture_raises`).

**Files:**
- Create: `std/oracle.lua`
- Test: `packages/github-proxy/tests/std_oracle_test.lua`

- [ ] **Step 1: Write the failing test**

```lua
-- packages/github-proxy/tests/std_oracle_test.lua
local oracle = require("std.oracle")
return {
  test_effect_set_unions_writes_and_raises = function()
    local rec = oracle.recorder()
    rec.record_write({ op = "post_comment", target = "owner/repo#issue/42" })
    rec.record_raise({ queue = "github-proxy.comment_request", dedup_key = "k1" })
    local effects = rec.effects()           -- normalized, order-independent multiset
    assert(#effects == 2)
    local key = oracle.effect_key(effects[1])
    assert(type(key) == "string" and #key > 0, "every effect must have a stable string key")
  end,
  test_delivery_equivalence_ignores_reads = function()
    local rec = oracle.recorder()
    rec.record_read({ op = "read_issue" })   -- reads are not effects
    assert(#rec.effects() == 0)
  end,
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `scripts/run.sh test github-proxy`
Expected: FAIL — `module 'std.oracle' not found`.

- [ ] **Step 3: Implement the interface**

```lua
-- std/oracle.lua  (Tier S, GitHub-agnostic)
local M = {}
function M.effect_key(e)
  if e.kind == "write" then return "W:" .. tostring(e.op) .. ":" .. tostring(e.target or "") .. ":" .. tostring(e.dedup_key or "") end
  return "R:" .. tostring(e.queue) .. ":" .. tostring(e.dedup_key or "")
end
function M.recorder()
  local writes, raises = {}, {}
  return {
    record_write = function(w) w.kind = "write"; table.insert(writes, w) end,
    record_raise = function(r) r.kind = "raise"; table.insert(raises, r) end,
    record_read  = function(_r) end,          -- reads are not effects; ignored
    effects = function()
      local all = {}
      for _, w in ipairs(writes) do table.insert(all, w) end
      for _, r in ipairs(raises) do table.insert(all, r) end
      return all
    end,
  }
end
-- delivery_1 vs delivery_2 effect multisets must be equal for idempotency (②).
function M.same_effects(a, b)
  local function bag(list) local m = {} for _, e in ipairs(list) do local k = M.effect_key(e); m[k] = (m[k] or 0) + 1 end return m end
  local ba, bb = bag(a), bag(b)
  for k, v in pairs(ba) do if bb[k] ~= v then return false end end
  for k, v in pairs(bb) do if ba[k] ~= v then return false end end
  return true
end
return M
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `scripts/run.sh test github-proxy`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add std/oracle.lua packages/github-proxy/tests/std_oracle_test.lua
git commit -m "feat(std): Tier S abstract effect/truth oracle interface (write+raise effect set)"
```

### Task 0.4: Injection seam — `std.testing.run_fake` + fake adapter skeletons

`run_fake(dept, event)` invokes a fake-bound department's `.pipeline(event)` directly while capturing `raise(...)` (via the `capture_raises` pattern at `claim_contract_test.lua:57`) and exposing the fake's recorded S1 writes — so business tests assert effects, never command strings.

**Files:**
- Create: `std/testing.lua`
- Create: `std/github_fake.lua` (Tier R in-memory GitHub model implementing the read ops used so far)
- Create: `std/git_fake.lua` (Tier R in-memory git model)
- Test: `packages/github-proxy/tests/std_run_fake_test.lua`

- [ ] **Step 1: Write the failing test** (a trivial fake-bound department: reads an issue, raises one intent)

```lua
-- packages/github-proxy/tests/std_run_fake_test.lua
local run_fake = require("std.testing").run_fake
local gh_fake = require("std.github_fake")

local function make_test_department(ports)
  local function pipeline(event)
    local issue = ports.github.read_issue(event.payload.source_ref)   -- S1 read
    if issue.state == "OPEN" then
      raise("demo.request", { dedup_key = "d:" .. issue.number })       -- S2 intent
    end
  end
  return { spec = { consumes = { "demo" } }, pipeline = pipeline }
end

return {
  test_run_fake_captures_raises_and_reads = function()
    local model = gh_fake.model({ issues = { ["owner/repo#issue/42"] = { number = 42, state = "OPEN" } } })
    local dept = make_test_department({ github = gh_fake.new(model), git = nil })
    local result, effects = run_fake(dept, { payload = { source_ref = { kind = "external", ref = "owner/repo#issue/42" } } })
    assert(#effects.raises == 1, "must capture the S2 raise")
    assert(effects.raises[1].queue == "demo.request")
    assert(effects.raises[1].payload.dedup_key == "d:42")
  end,
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `scripts/run.sh test github-proxy`
Expected: FAIL — `module 'std.testing' not found`.

- [ ] **Step 3: Implement run_fake + the GitHub fake (read_issue only for now)**

```lua
-- std/testing.lua
local M = {}
-- capture_raises pattern (prior art: github-devloop/tests/claim_contract_test.lua:57)
local function capture_raises(fn)
  local old = raise
  local raised = {}
  raise = function(queue, payload) table.insert(raised, { queue = queue, payload = payload }) end
  local ok, result = pcall(fn)
  raise = old
  if not ok then error(result) end
  return result, raised
end
function M.run_fake(dept, event)
  assert(type(dept.pipeline) == "function", "dept must expose .pipeline (make_department returns {spec, pipeline})")
  local result, raises = capture_raises(function() return dept.pipeline(event) end)
  return result, { raises = raises }   -- fake-recorded S1 writes are read off the model directly by the test
end
return M
```
```lua
-- std/github_fake.lua  (Tier R; grows one op per slice, mirroring std.github's domain ops)
local M = {}
function M.model(seed) return { issues = (seed and seed.issues) or {}, writes = {} } end
function M.new(model)
  local h = {}
  function h.read_issue(source_ref) return model.issues[source_ref.ref] or error("fake: unknown issue " .. tostring(source_ref.ref)) end
  -- write ops record into model.writes (added per slice), e.g.:
  -- function h.post_comment(ref, body) table.insert(model.writes, { op = "post_comment", target = ref.ref }) end
  return h
end
return M
```
```lua
-- std/git_fake.lua  (Tier R; grows per slice)
local M = {}
function M.model(seed) return { refs = (seed and seed.refs) or {}, writes = {} } end
function M.new(model)
  local h = {}
  function h.show_ref(branch) return model.refs[branch] end
  return h
end
return M
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `scripts/run.sh test github-proxy`
Expected: PASS — the fake-bound department runs, the S2 raise is captured.

- [ ] **Step 5: Commit**

```bash
git add std/testing.lua std/github_fake.lua std/git_fake.lua packages/github-proxy/tests/std_run_fake_test.lua
git commit -m "feat(std): run_fake injection-seam helper + gh/git fake skeletons (capture_raises + fake reads)"
```

### Task 0.5: Conformance ratchet seed (no new gh/git construction outside the adapter)

Add a `check_repo.py` G-gate that forbids new `gh`/`git` command **construction** and direct **exec** outside `std/github/` and `std/git/`, with an allowlist seeded with every file that currently does so (the allowlist may only shrink).

**Files:**
- Modify: `scripts/check_repo.py` (add the gate + allowlist loader)
- Create: `migration/gh-git-adapter.allowlist` (one path per currently-offending file)

- [ ] **Step 1: Write the failing check** (add to `scripts/check_repo.py`; run-as-test)

The gate flags a line that BOTH builds a command string whose head matches `^gh%s` / `^git%s` (a string literal beginning `"gh "` or `"git "`) AND is in a file not under `std/github/` or `std/git/` and not on the allowlist. Mentions inside test/doc/prompt text without exec are NOT flagged — match construction passed to `exec_sync`/`gh_exec`, not any textual occurrence.

- [ ] **Step 2: Run check to verify it fails (allowlist empty)**

Run: `python3 scripts/check_repo.py`
Expected: FAIL — dozens of files flagged (`commands.lua`, `branches.lua`, every dept that builds commands).

- [ ] **Step 3: Seed the allowlist with exactly those files**

Generate `migration/gh-git-adapter.allowlist` from the failing report (one relative path per line). Every current offender is grandfathered; new code is gated from day one.

- [ ] **Step 4: Run check to verify it passes**

Run: `python3 scripts/check_repo.py`
Expected: PASS (all offenders allowlisted; the gate is now armed for new code).

- [ ] **Step 5: Commit**

```bash
git add scripts/check_repo.py migration/gh-git-adapter.allowlist
git commit -m "feat(check_repo): G-gate ratchet — no new gh/git construction outside std/github|git (allowlist-seeded)"
```

### Task 0.6: Run the full suite

- [ ] **Step 1: Run everything**

Run: `scripts/run.sh test`
Expected: PASS (Wave 0 is additive; all existing tests unchanged + the new std tests green).

- [ ] **Step 2: Commit any test-list bookkeeping if required, else proceed.**

---

## Part B — Per-Dept Slice Template (every migration slice follows this)

For a department `D` with read ops `R*` and/or write ops `W*` and the builders/parsers `B*` it calls (from Part D), do:

1. **Add the domain op(s) to the adapter.** For each distinct op `D` needs, add a public method on the `std.github` / `std.git` handle that internally builds the command (moving `B*` in as **private** functions of the relevant `std/github/<area>.lua` submodule), execs via `handle._exec`, and parses into the neutral return shape (§5.2). Write an adapter-local contract test pinning the exact command string + parse (this is the ONE place command strings live).
2. **Grow the fake.** Add the matching method to `std/github_fake.lua` / `std/git_fake.lua` (reads serve from the model; writes record into `model.writes` and, for guarded writes, also `record_write` on an oracle recorder). Keep fake return shapes identical to the real adapter's.
3. **Convert the department to `make_department(ports)`.** Wrap the existing `done`/`act` (or `pipeline`) so it takes `ports` and calls `ports.github.<op>` / `ports.git.<op>` instead of `core.<B>` + `gh_exec`/`exec_sync` + `core.parse_<...>`. Production `main.lua` ends with `local M = make_department(production_ports()); M.make_department = make_department; return M` where `make_department` returns `{ spec = dept.spec, pipeline = _G.pipeline }` (see spec §5.6 — `std.department` sets `_G.pipeline` and returns only `{spec}`).
4. **Rewrite that department's tests to the port.** Replace `t.mock_command("gh …", …)` setups with a `std.github_fake` model + `run_fake(make_department(fakes), event)`; assert on returned reads, recorded writes, and captured raises. Delete the command-string mocks for `D`.
5. **Delete the now-unused builders/parsers** that no other department still calls (grep first). If a builder is still shared, leave it until its last caller migrates.
6. **Shrink the ratchet.** Remove `D`'s `main.lua` (and any builder file fully drained) from `migration/gh-git-adapter.allowlist`.
7. **Run `scripts/run.sh test` green, then commit** (`feat(<pkg>): migrate <D> to std.github/std.git port`).

Each slice is independently mergeable and behavior-preserving (the adapter command strings are byte-identical to the builders they replace; the contract test in step 1 pins that).

---

## Part C — Exemplar Slice: migrate `loop` (the first real slice, fully worked)

`loop` (github-devloop, `departments/loop/main.lua`, 139 lines, READ-ONLY) performs one read: `gh_issue_view_loop` via `core.gh_issue_view_loop_cmd` + `core.parse_issue_view_loop`. Migrate it to `ports.github.read_issue(source_ref)`.

### Task C.1: Add `read_issue` to the adapter

**Files:**
- Create: `std/github/issue.lua`
- Modify: `std/github.lua` (wire `read_issue` onto the handle)
- Modify: `std/github_fake.lua` (already has `read_issue`; ensure return shape matches)
- Test: `packages/github-devloop/tests/std_github_issue_test.lua`

- [ ] **Step 1: Write the failing adapter contract test** (pins the command + parse; the issue-view command/JSON shape is `core.gh_issue_view_loop_cmd` + `core.parse_issue_view_loop` in `github-devloop/core/commands.lua` / `core/parsers.lua` — read those for the exact `--json` field list and shape before writing this)

```lua
-- packages/github-devloop/tests/std_github_issue_test.lua
local gh = require("std.github")
return {
  test_read_issue_builds_command_and_parses = function()
    local seen
    local handle = gh.new(function(opts)
      seen = opts.cmd
      return { stdout = '{"number":42,"state":"OPEN","title":"t","labels":[{"name":"fkst-dev:enabled"}],"comments":[],"assignees":[]}', stderr = "", exit_code = 0 }
    end)
    local issue = handle.read_issue({ kind = "external", ref = "owner/repo#issue/42" })
    assert(seen:match("^gh "), "read_issue must build a gh command")
    assert(seen:find("42", 1, true), "command must target issue 42")
    assert(issue.number == 42 and issue.state == "OPEN")
    assert(issue.labels[1] == "fkst-dev:enabled")
  end,
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/run.sh test github-devloop`
Expected: FAIL — `attempt to call field 'read_issue' (a nil value)`.

- [ ] **Step 3: Implement `std/github/issue.lua` and wire it**

Move the `gh_issue_view*` command builder + the issue-view parser in as private functions (copy the exact body from `github-devloop/core/commands.lua` and `core/parsers.lua`; keep the command string byte-identical), then expose `read_issue`:

```lua
-- std/github/issue.lua
local shell = require("std.github.shell")
local M = {}
local function build_view_cmd(repo, number) --[[ paste exact body of core.gh_issue_view_loop_cmd (the generic issue view) ]] end
local function parse_issue(stdout)            --[[ paste exact body of core.parse_issue_state / parse_issue_view_loop → neutral Issue ]] end
local function repo_and_number(source_ref)
  local repo, kind, number = tostring(source_ref.ref):match("^([^#]+)#([a-z]+)/(%d+)$")
  assert(kind == "issue", "read_issue requires an issue source_ref")
  return repo, tonumber(number)
end
function M.install(handle)
  function handle.read_issue(source_ref)
    local repo, number = repo_and_number(source_ref)
    local out = handle._exec(build_view_cmd(repo, number), 30, "gh issue view")
    return parse_issue(out.stdout)
  end
end
return M
```
```lua
-- std/github.lua — add inside M.new(exec), after defining handle._exec:
require("std.github.issue").install(handle)
```

- [ ] **Step 4: Run to verify it passes**

Run: `scripts/run.sh test github-devloop`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add std/github/issue.lua std/github.lua packages/github-devloop/tests/std_github_issue_test.lua
git commit -m "feat(std.github): add read_issue domain op (builder+parser now private)"
```

### Task C.2: Convert `loop` to `make_department(ports)` + rewrite its tests

**Files:**
- Modify: `packages/github-devloop/departments/loop/main.lua`
- Modify: `packages/github-devloop/tests/<loop test file>` (find with `grep -rl "departments/loop" packages/github-devloop/tests`)
- Modify: `migration/gh-git-adapter.allowlist` (remove the `loop` line)

- [ ] **Step 1: Rewrite the loop test to the port** (replace its `t.mock_command(core.gh_issue_view_loop_cmd(...), …)` setup)

```lua
-- in the loop test: build a fake-bound dept and assert effects, not command strings
local run_fake = require("std.testing").run_fake
local gh_fake = require("std.github_fake")
local loop = require("departments.loop.main")

local function fake_dept(issue)
  local model = gh_fake.model({ issues = { ["owner/repo#issue/7"] = issue } })
  return loop.make_department({ github = gh_fake.new(model), git = nil }), model
end
-- test_converge_round_reraises_proposal: build issue with the converge marker, run_fake, assert raises[1].queue == "consensus.proposal" (mirror the existing assertion, now over captured raises)
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/run.sh test github-devloop`
Expected: FAIL — `loop.make_department` is nil.

- [ ] **Step 3: Wrap `loop` in `make_department(ports)`** — replace the direct `core.gh_issue_view_loop_cmd` + `gh_exec` + `core.parse_issue_view_loop` read with `ports.github.read_issue(source_ref)`; keep all marker/version-CAS/business logic unchanged (that is S3/business, stays).

```lua
-- packages/github-devloop/departments/loop/main.lua (shape)
local std = require("std.saga")  -- if loop adopts done/act in the same slice; else keep its current pipeline
local function make_department(ports)
  local function pipeline(event)
    -- ... was: local out = gh_exec(core.gh_issue_view_loop_cmd(repo, n)); local issue = core.parse_issue_view_loop(out.stdout)
    local issue = ports.github.read_issue(event.payload.source_ref)
    -- ... unchanged converge-round marker logic, raise("consensus.proposal", ...) ...
  end
  pipeline = core.wrap_pipeline_failure("loop", pipeline)
  _G.pipeline = pipeline
  return { spec = M.spec, pipeline = pipeline }
end
local M = make_department(function() return { github = require("std.github").new(exec_sync), git = require("std.git").new(exec_sync) } end)
M.make_department = make_department
return M
```

- [ ] **Step 4: Run to verify it passes**

Run: `scripts/run.sh test github-devloop`
Expected: PASS (loop tests green via the port; no command-string mocks remain for loop).

- [ ] **Step 5: Drain + ratchet + commit** — if `gh_issue_view_loop_cmd` / `parse_issue_view_loop` now have no other callers (`grep -rn gh_issue_view_loop_cmd packages/`), delete them from `core/commands.lua` / `core/parsers.lua`. Remove `departments/loop/main.lua` from the allowlist. Then:

```bash
scripts/run.sh test github-devloop
git add -A
git commit -m "feat(github-devloop): migrate loop to std.github.read_issue port; drop loop builders; shrink ratchet"
```

---

## Part D — Per-Dept Slice Backlog (ordered; each entry → one issue)

Execute in phase order (reads-first = lowest risk). Each row is one slice via Part B (mirroring Part C). File each as a github-devloop issue titled `migrate <dept> to std.github/std.git port`, body = the row + a pointer to this plan §B/§C. Group the tiny single-read depts where noted; split the worktree-heavy ones.

**Phase 1 — read-only `gh` reads (12 depts; lowest risk; do first, can batch the 1-read ones):**

| Dept (pkg) | Ops | Builders/parsers to move → port op | Notes |
|---|---|---|---|
| `loop` (gl) | read_issue | `gh_issue_view_loop_cmd`+`parse_issue_view_loop` → `read_issue` | **Done as Part C (exemplar).** |
| `consensus_result` (gl) | read_issue | `gh_issue_view_result_cmd` → `read_issue` | 1 read. |
| `intake_scan` (gl) | list_open_issues, read_issue | `gh_issue_list_intake_cmd`, `gh_issue_view_intake_scan_cmd` → `list_open_issues`, `read_issue` | 2 reads. |
| `intake_probe` (gl) | list_open_issues | `gh_issue_list_intake_probe_cmd` → `list_open_issues` | 1 read. |
| `liveness_scan` (gl) | list_open_issues, list_open_prs | `gh_issue_list_observe_cmd`, `gh_pr_list_observe_cmd` → `list_open_issues`, `list_open_prs` | adds `read_pr`/list to `std/github/pr.lua`. |
| `observe_pr` (gl) | read_issue (×2 views) | `gh_issue_view_result_cmd`, `gh_issue_view_reviewing_cmd` → `read_issue` | both are issue views. |
| `reconcile` (gl) | read_issue, read_pr | `gh_issue_view_loop_cmd`, `gh_pr_view_origin_cmd` → `read_issue`, `read_pr` | first `read_pr` consumer → build `std/github/pr.lua`. |
| `review_loop` (gl) | read_issue, read_pr | `gh_issue_view_review_loop_cmd`, `gh_pr_view_origin_cmd` | reuses `read_issue`/`read_pr`. |
| `review_pr` (gl) | read_issue, read_pr | `gh_issue_view_review_cmd`, `gh_pr_view_origin_cmd` | reuses. |
| `review_result` (gl) | read_pr | `gh_pr_view_origin_cmd` → `read_pr` | reuses `read_pr`. |
| `github_poll` (gp) | list_open_issues, list_open_prs | `gh_issue_list_cmd`, `gh_pr_list_cmd` (+ cache) → `list_open_issues/prs` | github-proxy entity poll; keep cache logic in dept. |
| `observe_issue` / `comment_handoff` (gl) | none (pure routing) | — | adopt `make_department(ports)` shape only (ports unused); enables uniform conformance. Optional. |

**Phase 2 — read+write `gh`/`git` (8 depts; medium risk; introduces S2 intents + S3 guards):**

| Dept (pkg) | Ops | Builders to move → port op | Notes |
|---|---|---|---|
| `github_comment` (gp) | post_comment (W) | `write_comment_request` → `post_comment` execute | S2 intent already exists; move execute into `std.github.post_comment`. |
| `github_pr_comment` (gp) | post_comment (W) | `write_comment_request` (pr) → `post_comment` | shares `post_comment`. |
| `github_issue_label` (gp) | read_pr (guard), set_labels (W) | `fetch_pr_view`, `apply_entity_labels`, `apply_issue_labels` → `read_pr`, `set_labels` | S3 guard (state) stays in dept; execute → adapter. |
| `rollup_merge` (gl) | read_pr | `gh_pr_view_merge_cmd` → `read_pr` | emits label/comment intents (already S2). |
| `open_pr` (gl) | git: is_ancestor, show_ref, rev_parse | `git_is_ancestor_cmd`, `git_show_ref_cmd`, `git_rev_parse_branch_cmd` → `std.git` ops | git-only; build `std/git/branch.lua`. |
| `rollup_scan` (gl) | read_pr/list (R) + git FF/merge/push (W) | `gh_pr_*` + `git_ahead_count/fetch/merge_no_ff/push_worktree` → `std.github`+`std.git` | split: gh reads first, then git writes. |
| `github_pr_open` (gp) | read_issue/pr (guards) + git_push + pr_create + comments + labels | `gh_issue_comment_cmd`, `gh_pr_create_cmd`, `gh_pr_view_head_oid_cmd`, `gh_pr_list_head_cmd`, `git_push_branch_cmd`, `git_is_ancestor_cmd`, `git_show_ref_branch_cmd` (+more) | **highest proxy complexity — split into 2–3 issues**: (a) gh reads, (b) git push/ancestry, (c) pr_create + comment/label writes. |
| `pr_freshness_scan` (gl) | gh reads + 14 git ops (merge/push/worktree) | `gh_*` + `git_worktree_*`/`merge_no_ff`/`push`/`unmerged_paths` | worktree-heavy → build `std/git/worktree.lua`, `std/git/merge.lua`; split reads vs git. |
| `sync_scan` (gl) | git-only (FF/merge/push/worktree) | 14 `git_*_cmd` → `std.git` ops | pure git; reuses `std/git/branch.lua`+`worktree.lua`+`merge.lua`. |

**Phase 3 — codex + git/gh (6 depts; highest risk; codex STAYS in dept, only builders move):**

| Dept (pkg) | gh/git ops to move (codex untouched) | Notes |
|---|---|---|
| `intake_judge` (gl) | read_issue | only `gh_issue_view_intake_judge_cmd` → `read_issue`; `spawn_codex_sync` stays. |
| `review_meta` (gl) | read_issue, read_pr | `gh_issue_view_fix_cmd`, `gh_pr_view_origin_cmd`; codex stays. |
| `decide` (consensus) | none (gh) | only `mkdir_p_cmd`/`read_runtime_root_cmd` (fs, out of scope); codex stays. Optional shape-only. |
| `decompose` (gl) | gh reads + pr_comment (W) | `gh_issue_list_decompose_children_cmd`, `gh_issue_view_decompose_cmd`, `gh_pr_view_origin_cmd`, `gh_pr_comment_cmd`; codex stays. |
| `sync_conflict` (gl) | 13 git ops (merge/commit/push) | all `git_*` → `std.git`; `spawn_codex_sync` (conflict judgment) stays. |
| `fix` (gl) | read_pr/issue + 23 git ops (worktree/merge/push) | `gh_*` + `git_worktree_*`/`merge`/`commit`/`push`/`fetch`; **split into 2–3 issues** (gh reads, git worktree, git push/merge); codex stays. 798 lines — also a candidate for sub-module split. |
| `implement` (gl) | read_issue + 21 git ops (worktree/merge/commit/push) | `gh_issue_view_implement_cmd` + `git_worktree_*`/`merge`/`commit`/`push`; **split into 2–3 issues**; codex stays. |

**Phase 4 — finalize (after every slice above lands):**

| Task | Detail |
|---|---|
| Delete the two old `gh_exec` wrappers | remove `github-proxy/core/gh_rate.lua` exec wrapper + `github-devloop/core/base.lua:918` once no department calls them (grep `gh_exec` → empty outside `std/`). |
| Close the ratchet fully | allowlist empty → flip the §0.5 gate to hard-fail if the allowlist file is non-empty or still exists. |
| Saga oracle amendment (coordination dependency) | in the `feat/std-and-saga-harness` workstream, amend the saga ①②③ oracle from `fkst.test.command_calls()` command-multiset to `std.oracle` write-intent/effect set (spec §6, §11). Cross-spec — coordinate, do not do it from this branch. |

**Builder/parser drain order** is driven by the table: `std/github/issue.lua` (Phase 1) → `std/github/pr.lua` (Phase 1, from `reconcile`) → `std/github/comment.lua`+`label.lua` (Phase 2) → `std/github/graphql.lua` (blocked_by, Phase 2/3) → `std/git/branch.lua` (Phase 2, `open_pr`) → `std/git/worktree.lua`+`merge.lua`+`diff.lua` (Phase 2/3, freshness/sync/fix/implement). Each submodule is created by the first slice that needs it and grown by later slices.

---

## Self-Review

**1. Spec coverage** (each spec section → task):
- §4 three surfaces → S1 adapter (Tasks 0.2, C.1, all D); S2 intents (Phase 2 `github_comment`/`label`/`pr_open`); S3 guards stay in dept (Part B step 3, Phase 2 notes). ✓
- §5.2 neutral return shapes → grown per op (C.1 `read_issue`; Part B step 1). ✓
- §5.3 adapter file layout → File Structure + drain order. ✓
- §5.4 body_source / topology preserved → Phase 2 write slices keep S2 intents; no content in payload (executor preserves current intent payloads). ✓ (caveat below)
- §5.6 injection seam → Task 0.4 (`run_fake`), Part B step 3, Part C.2 (`make_department` returning `{spec, pipeline=_G.pipeline}`). ✓
- §6 oracle → Task 0.3 (`std.oracle`) + Phase 4 saga amendment. ✓
- §8 ratchet → Task 0.5 + Part B step 6 + Phase 4. ✓
- §9 vertical-slice strangler → Parts B/C/D. ✓
- §10/§11 R1 nested require → Task 0.1; substrate independence → no engine task anywhere. ✓

**2. Placeholder scan:** Wave 0 + Part C steps have complete code or an explicit "paste exact body from <file:line>" instruction (a deliberate behavior-preserving copy, not a vague TODO — the contract test pins it). Part D rows are concrete task specs (dept, ops, builders, files), not placeholder steps — they are issue inputs whose per-step code is cut against live code at execution (the strangler design; stated in the Decomposition note). No `TBD`/`handle edge cases`/`similar to`.

**3. Type/name consistency:** `make_department(ports)` returns `{ spec, pipeline }` everywhere; `std.github.new(exec)`/`std.git.new(exec)` → handle with `read_issue`/`read_pr`/… and private `_exec`; `std.github_fake.new(model)`/`std.git_fake.new(model)` mirror the handle; `run_fake(dept, event) -> result, { raises }`; `std.oracle.recorder()` with `record_write`/`record_raise`/`effects`/`same_effects`. Consistent across 0.2 ↔ 0.4 ↔ C.1 ↔ C.2 ↔ B.

**Known caveat (carry into execution):** the test seam (`run_fake` + `make_department`) is the one mechanism the final review round flagged as needing empirical proof — Tasks 0.4 and C.2 are exactly that proof. Execute them TDD-first; if `_G.pipeline` capture or `raise` capture behaves unexpectedly under the real engine test mode, that is the signal to revisit §5.6 before scaling to Part D (do not file Part D issues until C is green).

⟦AI:FKST⟧
