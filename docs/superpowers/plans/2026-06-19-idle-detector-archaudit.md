# Idle-Detector + Archaudit Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a tiny reliable idle signal package and a bounded composed architecture-audit consumer that files small, concrete GitHub issue-create intents only when the engine is currently idle.

**Architecture:** `cron idle_tick -> idle-detector.idle_gate -> system_idle -> archaudit.audit -> github-proxy.github_issue_create_request`. `system_idle` is a reliable hint, not permission; `archaudit.audit` re-checks current idle before one bounded read-only codex judgment. Large content never enters reliable payloads; payloads carry only control fields plus `source_ref`.

**Tech Stack:** Lua packages for fkst-substrate, `std.saga.department(spec, handlers)`, `std.ports.install` for GitHub fake injection, `std.codex.judgment_codex_opts`, `std.strings`, `std.env`, `std.error_facts`, `fkst.test.run_department`, `fkst.test.mock_command`, `std.testing.run_fake`, `std.github_fake`, `scripts/run.sh test`, `scripts/run.sh test-composed`, `scripts/run.sh check`.

---

## File Structure

`packages/idle-detector/core.lua`  
Single responsibility: local observe wrapper, observe-fact idle predicate, and tiny `system_idle` payload builder. No package namespace references, no durable state, no marker parsing.

`packages/idle-detector/raisers/idle_poll.lua`  
Single responsibility: cron raiser for reliable `idle_tick`, default interval `30m`.

`packages/idle-detector/departments/idle_gate/main.lua`  
Single responsibility: saga department that consumes `idle_tick`, drops stale cron slots, re-derives current idleness, and raises `system_idle` only on clean idle facts.

`packages/idle-detector/tests/core_test.lua`  
Single responsibility: unit coverage for observe parsing, busy/DLQ/anomaly predicates, and payload shape.

`packages/idle-detector/tests/integration_idle_gate_test.lua`  
Single responsibility: engine-PASS integration coverage for now-independent stale routing using a 1970 cron slot. Freshness boundary precision is covered in pure helpers, not department wall-clock tests.

`packages/idle-detector/std`  
Symlink to `../../std`, matching existing package layout.

`packages/archaudit/composed.deps`  
Single responsibility: composed conformance dependencies, exactly `idle-detector` and `github-proxy`.

`packages/archaudit/core.lua`  
Single responsibility: local observe wrapper, codex prompt, strict JSON parser, finding validator, stable `dedup_key` helper, and `github-proxy.issue-create.v1` request builder. The observe wrapper is intentionally duplicated from `idle-detector` at tiny scale because G9 forbids peer cross-package `require` and `std.observe` is explicitly out of scope.

`packages/archaudit/departments/audit/main.lua`  
Single responsibility: one judgment pipeline from fresh `idle-detector.system_idle` hint to bounded issue-create requests.

`packages/archaudit/tests/core_test.lua`  
Single responsibility: unit coverage for parser, validator, dedup key, and issue-create payload shape.

`packages/archaudit/tests/integration_audit_test.lua`  
Single responsibility: engine-PASS integration coverage for now-independent stale/malformed idle-hint routing plus fake-department all-or-nothing batch behavior using fixed constants. Freshness/expiry boundary precision is covered in pure helpers, not department wall-clock tests.

`packages/archaudit/std`  
Symlink to `../../std`, matching existing package layout.

## Defaults

| Setting | Default |
| --- | --- |
| Idle cron interval | `30m` |
| Idle tick stale budget | `10m` |
| Archaudit freshness budget | `10m` |
| Codex timeout | `9m` |
| Max issues per idle event | `ARCHAUDIT_MAX_ISSUES_PER_IDLE`, default `3` |
| GitHub write posture | dry-run unless `FKST_GITHUB_WRITE=1`, delegated to `github-proxy` |
| Target repo | `FKST_GITHUB_REPO` |
| Label | `archaudit` if present, else `{}` |
| Department retry | `retry = false` |

## Test Harness Notes

`fkst.test.run_department(path, event, opts)` is used only with supported opts confirmed in the package test corpus and runner contract: `env`, `cwd`, and `path_prepend`. The supported result surface is `exit_code` plus captured `raises`; tests must not depend on unsupported `opts.now` injection or `result.stderr`. `std.error_facts` exposes structured fact fields through `error_fact_fields(error_class, queue, dept, message, context)`: `error_class`, `fingerprint`, optional `source_ref`, optional `attempt`, and optional `terminal`, with callers adding `WHY=` text. The engine exposes `now()` to departments but the department harness has no now-injection primitive, so freshness and expiry boundary precision is proven in pure helpers that take explicit `now_seconds`, `reference_ts`/`detected_seconds`, `expires_seconds`, and `budget` with fixed ISO/epoch constants. Department `run_department` tests cover only now-independent routing, such as a 1970 timestamp that is always stale; they do not assert department-level fresh or expired boundaries against the host clock. Fake-port tests may use a fixed in-process `now()` only to reach `act` for non-time behavior such as label advice and all-or-nothing batch emission; those tests are not freshness/expiry boundary proof.

## Explicit Non-Goals

Do not build any of these: mechanical Python scanner, `check_repo_architecture.py`, detector registry, `check_repo_audit.py`, allowlist/ratchet/slicer/umbrella/finite manifest, rule-of-three or recurrence counting or guard graduation, `cycle_id` plus rank, evidence validator beyond JSON shape plus file/line/required fields, consensus/oracle review before filing, v1/v2 phasing, held-out challenge suite, severity/confidence/category/owner/symbol/excerpt-hash/class-fingerprint taxonomies, second archaudit department, persistent idle state or durable idle replay, github-devloop lifecycle mirroring, label/mode gates beyond github-proxy's `FKST_GITHUB_WRITE` dry-run posture, `std.observe` abstraction, engine Rust, new host facts, idle markers, local dedup DB.

## Task 1: Create `idle-detector` Skeleton and Cron Raiser

**Files:**
- Create: `packages/idle-detector/std`
- Create: `packages/idle-detector/raisers/idle_poll.lua`
- Create: `packages/idle-detector/tests/raiser_test.lua`
- Test: `scripts/run.sh test idle-detector`

- [ ] **Step 1: Create the package directory and std symlink.**

```bash
mkdir -p packages/idle-detector/raisers packages/idle-detector/tests
ln -s ../../std packages/idle-detector/std
```

- [ ] **Step 2: Write the failing cron shape test.**

```lua
-- packages/idle-detector/tests/raiser_test.lua
local t = fkst.test

return {
  test_idle_poll_cron_shape = function()
    local raiser = dofile("raisers/idle_poll.lua")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "30m")
    t.eq(raiser.produces, "idle_tick")
  end,
}
```

- [ ] **Step 3: Run and confirm the test fails before the raiser exists.**

```bash
scripts/run.sh test idle-detector
```

Expected: FAIL, with `raisers/idle_poll.lua` missing or not loadable.

- [ ] **Step 4: Implement the raiser.**

```lua
-- packages/idle-detector/raisers/idle_poll.lua
return {
  type = "cron",
  interval = "30m",
  produces = "idle_tick",
}
```

- [ ] **Step 5: Run and confirm the package test passes.**

```bash
scripts/run.sh test idle-detector
```

Expected: PASS, with at least one engine report-json PASS for `packages/idle-detector/tests/raiser_test.lua`.

- [ ] **Step 6: Commit.**

```bash
git add packages/idle-detector
git commit -m "feat(idle-detector): add slow idle poll cron raiser"
```

## Task 2: Implement `idle-detector/core.lua`

**Files:**
- Create: `packages/idle-detector/core.lua`
- Create/Modify: `packages/idle-detector/tests/core_test.lua`
- Test: `scripts/run.sh test idle-detector`

- [ ] **Step 1: Write failing core tests.**

```lua
-- packages/idle-detector/tests/core_test.lua
local core = require("core")
local t = fkst.test

local function observe_idle()
  return {
    schema = "fkst.observe.v1",
    queues = {
      { queue = "idle_tick", ready = 0, leased = 0, retry = 0, dlq = 0 },
      { queue = "github_poll_tick", pending = 0, inflight = 0, delayed = 0, dead_letters = 0 },
    },
    anomalies = {},
    dlq = {},
  }
end

return {
  test_idle_predicate_accepts_zero_queue_and_empty_anomalies = function()
    local idle, why = core.is_idle_observe(observe_idle())
    t.eq(idle, true)
    t.is_nil(why)
  end,

  test_idle_predicate_fails_closed_on_missing_required_fact_groups = function()
    local facts = observe_idle()
    facts.queues = nil
    t.raises(function() core.is_idle_observe(facts) end)

    facts = observe_idle()
    facts.anomalies = nil
    t.raises(function() core.is_idle_observe(facts) end)

    facts = observe_idle()
    facts.dlq = nil
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_fails_closed_on_unknown_schema = function()
    local facts = observe_idle()
    facts.schema = "fkst.observe.v2"
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_fails_closed_on_malformed_top_level = function()
    t.raises(function() core.is_idle_observe("not facts") end)
    local facts = observe_idle()
    facts.queues = "not a table"
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_rejects_ready_work = function()
    local facts = observe_idle()
    facts.queues[1].ready = 1
    local idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("ready", 1, true) ~= nil)
  end,

  test_idle_predicate_rejects_leased_retry_and_dlq = function()
    for field, value in pairs({ leased = 1, retry = 1, dlq = 1 }) do
      local facts = observe_idle()
      facts.queues[1][field] = value
      local idle, why = core.is_idle_observe(facts)
      t.eq(idle, false)
      t.is_true(why:find(field, 1, true) ~= nil)
    end
  end,

  test_idle_predicate_fails_closed_on_missing_each_busy_dimension_group = function()
    for _, field in ipairs({ "ready", "leased", "retry", "dlq" }) do
      local facts = observe_idle()
      facts.queues[1][field] = nil
      t.raises(function() core.is_idle_observe(facts) end)
    end
  end,

  test_idle_predicate_fails_closed_on_ambiguous_and_unknown_metric_groups = function()
    local facts = observe_idle()
    facts.queues[1].pending = 0
    t.raises(function() core.is_idle_observe(facts) end)

    facts = observe_idle()
    facts.queues[1] = { queue = "proposal", unexpected = 0 }
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_rejects_anomalies = function()
    local facts = observe_idle()
    facts.anomalies = { { type = "terminal-failure", queue = "demo" } }
    local idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("anomaly", 1, true) ~= nil)
  end,

  test_observe_wrapper_parses_json = function()
    local observed = core.observe(function(cmd)
      t.eq(cmd.cmd, "fkst-framework observe --json")
      t.eq(cmd.timeout, 30)
      return {
        stdout = '{"schema":"fkst.observe.v1","queues":[],"anomalies":[],"dlq":[]}',
        stderr = "",
        exit_code = 0,
      }
    end)
    t.eq(observed.schema, "fkst.observe.v1")
  end,

  test_observe_wrapper_fails_closed_on_unknown_schema = function()
    t.raises(function()
      core.observe(function(_cmd)
        return {
          stdout = '{"schema":"fkst.observe.v2","queues":[],"anomalies":[],"dlq":[]}',
          stderr = "",
          exit_code = 0,
        }
      end)
    end)
  end,

  test_observe_wrapper_fails_closed_on_command_failure = function()
    t.raises(function()
      core.observe(function(_cmd)
        return { stdout = "", stderr = "boom", exit_code = 1 }
      end)
    end)
  end,

  test_system_idle_payload_is_small_and_source_ref_backed = function()
    local payload = core.build_system_idle_payload("2026-06-19T01:00:00Z", "idle_tick/2026-06-19T01:00:00Z", "2026-06-19T01:10:00Z")
    t.eq(payload.schema, "idle-detector.system-idle.v1")
    t.eq(payload.detected_at, "2026-06-19T01:00:00Z")
    t.eq(payload.source_ref.kind, "host-observe")
    t.eq(payload.source_ref.ref, "idle_tick/2026-06-19T01:00:00Z")
    t.eq(payload.expires_at, "2026-06-19T01:10:00Z")
    t.is_nil(payload.queues)
    t.is_nil(payload.metrics)
  end,

  test_freshness_verdict_is_pure_and_deterministic = function()
    local reference = core.iso_timestamp_epoch_seconds("2026-06-19T01:00:00Z")
    t.eq(core.freshness_verdict(reference, reference + 60, 600), "fresh")
    t.eq(core.freshness_verdict(reference, reference + 600, 600), "fresh")
    t.eq(core.freshness_verdict(reference, reference + 601, 600), "stale")
    t.eq(core.freshness_verdict(reference, reference - 60, 600), "fresh")
    t.raises(function() core.freshness_verdict(nil, reference, 600) end)
  end,

  test_skip_fact_fields_are_pure_and_structured = function()
    for _, case in ipairs({
      { why = "busy queue=proposal ready=1" },
      { why = "busy dlq>0" },
      { why = "unreadable observe facts: observe failed" },
      { why = "malformed observe facts: missing metric group" },
      { why = "stale idle_tick slot" },
    }) do
      local fact = core.skip_fact("idle_gate", {
        queue = "idle_tick",
        payload = {
          source_ref = { kind = "cron", ref = "idle-detector/idle_poll/2099-01-01T00:00:00Z" },
        },
      }, case.why, true)
      t.is_true(fact:find("tag=SKIP", 1, true) ~= nil)
      t.is_true(fact:find("error_class=terminal-skip", 1, true) ~= nil)
      t.is_true(fact:find("source_ref=cron:idle-detector/idle_poll/2099-01-01T00:00:00Z", 1, true) ~= nil)
      t.is_true(fact:find("terminal=true", 1, true) ~= nil)
      t.is_true(fact:find("WHY=" .. case.why, 1, true) ~= nil)
    end
  end,
}
```

- [ ] **Step 2: Run and confirm failure.**

```bash
scripts/run.sh test idle-detector
```

Expected: FAIL, with `module 'core' not found` or missing core functions.

- [ ] **Step 3: Implement `core.lua`.**

```lua
-- packages/idle-detector/core.lua
local M = {}
local error_facts = require("std.error_facts")

local observe_schema = "fkst.observe.v1"

local function int_value(value)
  if type(value) == "number" then
    if value < 0 or math.floor(value) ~= value then
      error("idle-detector: malformed observe metric")
    end
    return value
  end
  if type(value) == "string" and value:match("^%d+$") then
    return tonumber(value)
  end
  error("idle-detector: malformed observe metric")
end

local function required_metric(row, names, group)
  local found = nil
  for _, name in ipairs(names) do
    if row[name] ~= nil then
      if found ~= nil then
        error("idle-detector: ambiguous observe metric")
      end
      found = { value = int_value(row[name]), name = name }
    end
  end
  if found == nil then
    error("idle-detector: missing observe metric group: " .. tostring(group))
  end
  return found.value, found.name
end

local function required_list(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("idle-detector: malformed observe " .. name)
  end
  return value
end

local function validate_observe_facts(facts)
  if type(facts) ~= "table" then
    error("idle-detector: malformed observe facts")
  end
  if facts.schema ~= observe_schema then
    error("idle-detector: unknown observe schema")
  end
  required_list(facts, "queues")
  required_list(facts, "anomalies")
  required_list(facts, "dlq")
  return facts
end

function M.observe(exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("idle-detector: observe requires exec_sync")
  end
  local result = run({ cmd = "fkst-framework observe --json", timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("idle-detector: observe failed: " .. tostring(result and result.stderr or "no result"))
  end
  local ok, decoded = pcall(json.decode, result.stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("idle-detector: observe returned malformed JSON")
  end
  return validate_observe_facts(decoded)
end

function M.is_idle_observe(facts)
  validate_observe_facts(facts)
  local queues = facts.queues
  for _, row in ipairs(queues) do
    if type(row) ~= "table" then
      error("idle-detector: malformed queue observe row")
    end
    if type(row.queue) ~= "string" or row.queue == "" then
      error("idle-detector: malformed queue observe name")
    end
    local queue = row.queue
    local ready, ready_name = required_metric(row, { "ready", "pending", "due", "available", "depth" }, "ready")
    if ready > 0 then
      return false, "busy queue=" .. queue .. " " .. ready_name .. "=" .. tostring(ready)
    end
    local leased, leased_name = required_metric(row, { "leased", "inflight", "in_flight", "running", "active" }, "leased")
    if leased > 0 then
      return false, "busy queue=" .. queue .. " " .. leased_name .. "=" .. tostring(leased)
    end
    local retry, retry_name = required_metric(row, { "retry", "retries", "retry_pending", "delayed", "backoff" }, "retry")
    if retry > 0 then
      return false, "busy queue=" .. queue .. " " .. retry_name .. "=" .. tostring(retry)
    end
    local dlq, dlq_name = required_metric(row, { "dlq", "dead", "dead_letters", "dead_letter" }, "dlq")
    if dlq > 0 then
      return false, "busy queue=" .. queue .. " " .. dlq_name .. "=" .. tostring(dlq)
    end
  end
  if #facts.dlq > 0 then
    return false, "busy dlq>0"
  end
  if #facts.anomalies > 0 then
    return false, "busy anomaly>0"
  end
  return true, nil
end

function M.build_system_idle_payload(detected_at, observe_ref, expires_at)
  local payload = {
    schema = "idle-detector.system-idle.v1",
    detected_at = tostring(detected_at),
    source_ref = {
      kind = "host-observe",
      ref = tostring(observe_ref),
    },
  }
  if expires_at ~= nil then
    payload.expires_at = tostring(expires_at)
  end
  return payload
end

function M.iso_timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d)[%-:](%d%d)[%-:](%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year, month, day = tonumber(year), tonumber(month), tonumber(day)
  hour, minute, second = tonumber(hour), tonumber(minute), tonumber(second)
  if month < 1 or month > 12 or day < 1 or day > 31 or hour > 23 or minute > 59 or second > 59 then
    return nil
  end
  if month <= 2 then
    year = year - 1
    month = month + 12
  end
  local era = math.floor(year / 400)
  local yoe = year - era * 400
  local doy = math.floor((153 * (month - 3) + 2) / 5) + day - 1
  local doe = yoe * 365 + math.floor(yoe / 4) - math.floor(yoe / 100) + doy
  return (era * 146097 + doe - 719468) * 86400 + hour * 3600 + minute * 60 + second
end

function M.freshness_verdict(reference_ts_seconds, now_seconds, budget_seconds)
  if type(reference_ts_seconds) ~= "number" or type(now_seconds) ~= "number" or type(budget_seconds) ~= "number" then
    error("idle-detector: malformed freshness timestamp")
  end
  if now_seconds - reference_ts_seconds > budget_seconds then
    return "stale"
  end
  return "fresh"
end

function M.skip_fact(dept, event, why, terminal)
  local fields = error_facts.error_fact_fields("terminal-skip", type(event) == "table" and event.queue or nil, dept, why, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  return "idle-detector dept=" .. tostring(dept) .. " tag=SKIP " .. table.concat(fields, " ")
end

return M
```

- [ ] **Step 4: Run and confirm pass.**

```bash
scripts/run.sh test idle-detector
```

Expected: PASS for `core_test.lua` and `raiser_test.lua`.

- [ ] **Step 5: Commit.**

```bash
git add packages/idle-detector/core.lua packages/idle-detector/tests/core_test.lua
git commit -m "feat(idle-detector): observe wrapper and idle predicate"
```

## Task 3: Implement `idle_gate` Saga Department

**Files:**
- Create: `packages/idle-detector/departments/idle_gate/main.lua`
- Create/Modify: `packages/idle-detector/tests/integration_idle_gate_test.lua`
- Test: `scripts/run.sh test idle-detector`

- [ ] **Step 1: Write failing integration tests.**

```lua
-- packages/idle-detector/tests/integration_idle_gate_test.lua
local t = fkst.test

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/idle-detector/" .. tostring(name),
    },
  }
end

local function event(ts)
  local slot = ts or "1970-01-01T00:00:00Z"
  return {
    queue = "idle_tick",
    ts = slot,
    payload = {
      schema = "idle-detector.idle-tick.v1",
      slot = slot,
      source_ref = { kind = "cron", ref = "idle-detector/idle_poll/" .. slot },
    },
  }
end

local function mock_observe(stdout, exit_code)
  t.mock_command("fkst-framework observe --json", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "observe failed",
    exit_code = exit_code or 0,
  })
end

return {
  test_idle_gate_drops_stale_cron_slot = function()
    local result = t.run_department("departments/idle_gate/main.lua", event("1970-01-01T00:00:00Z"), opts("stale"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#t.command_calls(), 0)
  end,
}
```

- [ ] **Step 2: Run and confirm failure.**

```bash
scripts/run.sh test idle-detector
```

Expected: FAIL, with `departments/idle_gate/main.lua` missing.

- [ ] **Step 3: Implement `idle_gate/main.lua` using the saga head-spec idiom.**

```lua
-- packages/idle-detector/departments/idle_gate/main.lua
local core = require("core")
local error_facts = require("std.error_facts")
local saga = require("std.saga")

local spec = {
  consumes = { "idle_tick" },
  produces = { "system_idle" },
  stall_window = "30s",
  retry = false,
}

local stale_budget_seconds = 10 * 60

local function iso_from_seconds(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(seconds))
end

local function tick_slot(event)
  local payload = type(event) == "table" and event.payload or {}
  return payload.slot or payload.cron_slot or payload.detected_at or (type(event) == "table" and event.ts)
end

local function log_skip(reason, event)
  log.warn(core.skip_fact("idle_gate", event, reason, true))
end

local function wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, err = pcall(fn, event)
    if ok then
      return err
    end
    local fields = error_facts.error_fact_fields("caught-failure", type(event) == "table" and event.queue or nil, dept, err, {
      source_ref = error_facts.event_source_ref(event),
    })
    table.insert(fields, "error=" .. error_facts.one_line(err))
    log.error("idle-detector dept=" .. dept .. " tag=FAILURE " .. table.concat(fields, " "))
    error(err, 0)
  end
end

local function slot_is_stale(slot, now_seconds)
  local slot_seconds = core.iso_timestamp_epoch_seconds(slot)
  if slot_seconds == nil then
    return true, "malformed or missing idle_tick slot"
  end
  if core.freshness_verdict(slot_seconds, now_seconds, stale_budget_seconds) == "stale" then
    return true, "stale idle_tick slot"
  end
  return false, nil
end

local function idle_done(event)
  if type(event) ~= "table" or event.queue ~= "idle_tick" then
    error("idle-detector: idle_gate consumed unknown queue")
  end
  local now_seconds = now()
  local stale, why = slot_is_stale(tick_slot(event), now_seconds)
  if stale then
    log_skip(why, event)
    return true
  end
  return false
end

local function act_idle(event)
  local slot = tick_slot(event)
  local ok, facts_or_err = pcall(core.observe)
  if not ok then
    log_skip("unreadable observe facts: " .. tostring(facts_or_err), event)
    return
  end
  local ok_idle, idle, why = pcall(function()
    local is_idle, idle_why = core.is_idle_observe(facts_or_err)
    return is_idle, idle_why
  end)
  if not ok_idle then
    log_skip("malformed observe facts: " .. tostring(idle), event)
    return
  end
  if not idle then
    log_skip(why or "system busy", event)
    return
  end
  raise("system_idle", core.build_system_idle_payload(
    slot,
    "idle_tick/" .. tostring(slot),
    iso_from_seconds(core.iso_timestamp_epoch_seconds(slot) + stale_budget_seconds)
  ))
end

local M = saga.department(spec, {
  done = idle_done,
  act = act_idle,
  wrap = wrap_pipeline_failure,
  name = "idle_gate",
})
M.pipeline = _G.pipeline
return M
```

- [ ] **Step 4: Run and confirm pass.**

```bash
scripts/run.sh test idle-detector
```

Expected: PASS. This also verifies G5 for `integration_idle_gate_test.lua`.

- [ ] **Step 5: Commit.**

```bash
git add packages/idle-detector/departments/idle_gate/main.lua packages/idle-detector/tests/integration_idle_gate_test.lua
git commit -m "feat(idle-detector): gate system_idle on current observe facts"
```

## Task 4: Create `archaudit` Skeleton and Core

**Files:**
- Create: `packages/archaudit/std`
- Create: `packages/archaudit/composed.deps`
- Create: `packages/archaudit/core.lua`
- Create: `packages/archaudit/tests/core_test.lua`
- Test: `scripts/run.sh test archaudit`

Contract bound from `packages/github-proxy/core/issue_create.lua`: `validate_issue_create_payload` accepts `repo <= 200`, `title <= 240`, `body <= 12000`, `dedup_key <= 512` with marker-safe characters, `source_ref.kind <= 80`, and `source_ref.ref <= 200`. The archaudit builder must satisfy these by construction or reject with a structured failure; do not truncate content to force a payload through validation.

- [ ] **Step 1: Create package directories, std symlink, and composed deps.**

```bash
mkdir -p packages/archaudit/departments/audit packages/archaudit/tests
ln -s ../../std packages/archaudit/std
printf 'idle-detector\ngithub-proxy\n' > packages/archaudit/composed.deps
```

- [ ] **Step 2: Write failing core tests.**

```lua
-- packages/archaudit/tests/core_test.lua
local core = require("core")
local t = fkst.test

local finding_json = '[{"file":"packages/idle-detector/core.lua","line":1,"rule":"SRP","why":"Mixed responsibilities.","suggested_fix":"Extract the extra responsibility."}]'

return {
  test_parse_findings_accepts_strict_array = function()
    local parsed = core.parse_findings_json(finding_json)
    t.eq(#parsed, 1)
    t.eq(parsed[1].file, "packages/idle-detector/core.lua")
    t.eq(parsed[1].line, 1)
    t.eq(parsed[1].rule, "SRP")
  end,

  test_parse_findings_rejects_non_json_and_extra_shape = function()
    t.raises(function() core.parse_findings_json("not json") end)
    t.raises(function() core.parse_findings_json('{"file":"x"}') end)
    t.raises(function() core.parse_findings_json('"scalar"') end)
    t.raises(function() core.parse_findings_json('42') end)
    t.raises(function() core.parse_findings_json('[{"file":"x","line":"bad","rule":"SRP","why":"w","suggested_fix":"f"}]') end)
  end,

  test_parse_findings_accepts_legitimate_empty_array = function()
    local parsed = core.parse_findings_json("[]")
    t.eq(#parsed, 0)
  end,

  test_validate_finding_checks_file_and_line = function()
    local finding = core.parse_findings_json(finding_json)[1]
    t.eq(core.validate_finding(finding), true)
    finding.line = 999999
    t.eq(core.validate_finding(finding), false)
  end,

  test_dedup_key_is_stable_and_bounded = function()
    local key = core.dedup_key("owner/repo", {
      file = "packages/idle-detector/core.lua",
      line = 1,
      rule = "SRP",
    })
    t.eq(key, core.dedup_key("owner/repo", {
      file = "packages/idle-detector/core.lua",
      line = 1,
      rule = "SRP",
    }))
    t.is_true(key:find("archaudit/owner/repo/packages/idle-detector/core.lua/1/SRP/", 1, true) == 1)
  end,

  test_issue_request_shape_matches_github_proxy_contract = function()
    local finding = core.parse_findings_json(finding_json)[1]
    local payload = core.build_issue_create_request("owner/repo", finding, true)
    t.eq(payload.schema, "github-proxy.issue-create.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.title, "Archaudit: packages/idle-detector/core.lua:1 SRP")
    t.eq(payload.labels[1], "archaudit")
    t.eq(payload.source_ref.kind, "repo-site")
    t.eq(payload.source_ref.ref, "owner/repo#packages/idle-detector/core.lua:1#archaudit-create-intent")
    t.is_true(payload.body:find("archaudit-dedup: " .. payload.dedup_key, 1, true) ~= nil)
  end,

  test_issue_request_rejects_overlong_source_ref_from_long_file_path = function()
    local long_file = "packages/" .. string.rep("longsegment/", 15) .. "core.lua"
    t.raises(function()
      core.build_issue_create_request("owner/repo", {
        file = long_file,
        line = 1,
        rule = "SRP",
        why = "Concrete issue.",
        suggested_fix = "Small fix.",
      }, true)
    end)
  end,

  test_issue_request_rejects_long_or_malformed_repo = function()
    local finding = core.parse_findings_json(finding_json)[1]
    t.raises(function() core.build_issue_create_request("owner/" .. string.rep("r", 201), finding, true) end)
    t.raises(function() core.build_issue_create_request("owner repo", finding, true) end)
  end,

  test_issue_request_omits_missing_label = function()
    local finding = core.parse_findings_json(finding_json)[1]
    local payload = core.build_issue_create_request("owner/repo", finding, false)
    t.eq(#payload.labels, 0)
  end,

  test_freshness_and_expiry_verdicts_are_pure_and_deterministic = function()
    local detected = core.iso_timestamp_epoch_seconds("2026-06-19T01:00:00Z")
    local expires = core.iso_timestamp_epoch_seconds("2026-06-19T01:10:00Z")
    local expires_past_while_detected_fresh = core.iso_timestamp_epoch_seconds("2026-06-19T01:02:00Z")
    t.eq(core.idle_hint_freshness(detected, nil, detected + 60, 600), "fresh")
    t.eq(core.idle_hint_freshness(detected, expires, detected + 60, 600), "fresh")
    t.eq(core.idle_hint_freshness(detected, nil, detected + 600, 600), "fresh")
    t.eq(core.idle_hint_freshness(detected, expires, detected + 601, 600), "stale")
    t.eq(core.idle_hint_freshness(detected, expires, expires, 600), "expired")
    t.eq(core.idle_hint_freshness(detected, expires_past_while_detected_fresh, detected + 180, 600), "expired")
    t.eq(core.idle_hint_freshness(detected, detected - 1, detected, 600), "expired")
    t.raises(function() core.idle_hint_freshness(nil, expires, detected, 600) end)
    t.raises(function() core.idle_hint_freshness(detected, nil, nil, 600) end)
  end,

  test_observe_predicate_fails_closed_on_missing_each_busy_dimension_group = function()
    for _, row in ipairs({
      { queue = "proposal", leased = 0, retry = 0, dlq = 0 },
      { queue = "proposal", ready = 0, retry = 0, dlq = 0 },
      { queue = "proposal", ready = 0, leased = 0, dlq = 0 },
      { queue = "proposal", ready = 0, leased = 0, retry = 0 },
    }) do
      t.raises(function()
        core.is_idle_observe({ schema = "fkst.observe.v1", queues = { row }, anomalies = {}, dlq = {} })
      end)
    end
  end,

  test_observe_predicate_fails_closed_on_ambiguous_and_unknown_metric_groups = function()
    t.raises(function()
      core.is_idle_observe({
        schema = "fkst.observe.v1",
        queues = { { queue = "proposal", ready = 0, pending = 0, leased = 0, retry = 0, dlq = 0 } },
        anomalies = {},
        dlq = {},
      })
    end)
    t.raises(function()
      core.is_idle_observe({
        schema = "fkst.observe.v1",
        queues = { { queue = "proposal", unexpected = 0 } },
        anomalies = {},
        dlq = {},
      })
    end)
  end,

  test_skip_fact_fields_are_pure_and_structured = function()
    for _, why in ipairs({
      "stale system_idle hint",
      "expired system_idle hint",
      "observe-unreadable: observe failed",
      "current observe busy ready=1",
      "current observe dlq>0",
    }) do
      local fact = core.skip_fact("audit", {
        queue = "idle-detector.system_idle",
        payload = {
          source_ref = { kind = "host-observe", ref = "idle_tick/2026-06-19T01:00:00Z" },
        },
      }, why, true)
      t.is_true(fact:find("tag=SKIP", 1, true) ~= nil)
      t.is_true(fact:find("error_class=terminal-skip", 1, true) ~= nil)
      t.is_true(fact:find("source_ref=host-observe:idle_tick/2026-06-19T01:00:00Z", 1, true) ~= nil)
      t.is_true(fact:find("terminal=true", 1, true) ~= nil)
      t.is_true(fact:find("WHY=" .. why, 1, true) ~= nil)
    end
  end,

  test_failure_fact_fields_are_pure_distinct_and_structured = function()
    local fingerprints = {}
    for _, case in ipairs({
      { class = "missing-repo", why = "missing FKST_GITHUB_REPO" },
      { class = "malformed-repo", why = "malformed FKST_GITHUB_REPO" },
      { class = "codex-timeout", why = "codex timeout" },
      { class = "codex-nonzero", why = "codex nonzero exit" },
      { class = "malformed-json", why = "codex output is malformed JSON" },
      { class = "non-array-json", why = "codex output is not a JSON array" },
      { class = "validation-failure", why = "invalid file or line" },
      { class = "observe-malformed", why = "observe malformed or unknown shape" },
    }) do
      local fact = core.failure_fact("audit", "FAILURE", case.class, {
        queue = "idle-detector.system_idle",
        payload = {
          source_ref = { kind = "host-observe", ref = "idle_tick/2026-06-19T01:00:00Z" },
        },
      }, case.why, true)
      t.is_true(fact:find("tag=FAILURE", 1, true) ~= nil)
      t.is_true(fact:find("error_class=" .. case.class, 1, true) ~= nil)
      t.is_true(fact:find("source_ref=host-observe:idle_tick/2026-06-19T01:00:00Z", 1, true) ~= nil)
      t.is_true(fact:find("terminal=true", 1, true) ~= nil)
      t.is_true(fact:find("WHY=" .. case.why, 1, true) ~= nil)
      local fingerprint = fact:match("fingerprint=([^%s]+)")
      t.is_true(fingerprint ~= nil and fingerprints[fingerprint] == nil)
      fingerprints[fingerprint] = true
    end
  end,
}
```

- [ ] **Step 3: Run and confirm failure.**

```bash
scripts/run.sh test archaudit
```

Expected: FAIL, with `module 'core' not found`.

- [ ] **Step 4: Implement `core.lua`.**

```lua
-- packages/archaudit/core.lua
local M = {}
local strings = require("std.strings")
local error_facts = require("std.error_facts")

local file_limit = 240
local rule_limit = 80
local why_limit = 1000
local fix_limit = 1000
local github_proxy_limits = {
  repo = 200,
  title = 240,
  body = 12000,
  dedup_key = 512,
  source_ref_kind = 80,
  source_ref_ref = 200,
}

local function bounded(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function marker_safe(value)
  return tostring(value):find('[<>"\r\n]') == nil
end

local function assert_request_field(ok, field)
  if not ok then
    error("archaudit: invalid issue-create field: " .. tostring(field), 0)
  end
end

function M.validate_repo(repo)
  if not strings.is_bounded_string(repo, github_proxy_limits.repo) then
    return false
  end
  if strings.split_repo(repo) == nil then
    return false
  end
  return tostring(repo):find("^[%w._-]+/[%w._-]+$") ~= nil
end

local function one_line(value)
  return tostring(value or ""):gsub("%s+", " ")
end

local function body_text(finding, dedup_key)
  return table.concat({
    "Architecture doctrine violation:",
    "",
    "File: " .. tostring(finding.file) .. ":" .. tostring(finding.line),
    "Rule: " .. tostring(finding.rule),
    "",
    "Why:",
    tostring(finding.why),
    "",
    "Suggested fix:",
    tostring(finding.suggested_fix),
    "",
    "<!-- archaudit-dedup: " .. tostring(dedup_key) .. " -->",
  }, "\n")
end

local observe_schema = "fkst.observe.v1"

local function int_value(value)
  if type(value) == "number" then
    if value < 0 or math.floor(value) ~= value then
      error("archaudit: observe-malformed-metric")
    end
    return value
  end
  if type(value) == "string" and value:match("^%d+$") then
    return tonumber(value)
  end
  error("archaudit: observe-malformed-metric")
end

local function required_metric(row, names, group)
  local found = nil
  for _, name in ipairs(names) do
    if row[name] ~= nil then
      if found ~= nil then
        error("archaudit: observe-ambiguous-metric")
      end
      found = { value = int_value(row[name]), name = name }
    end
  end
  if found == nil then
    error("archaudit: observe-missing-metric-group: " .. tostring(group))
  end
  return found.value, found.name
end

local function required_list(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("archaudit: observe-malformed-" .. name)
  end
  return value
end

function M.validate_observe_facts(facts)
  if type(facts) ~= "table" then
    error("archaudit: observe-malformed-top-level")
  end
  if facts.schema ~= observe_schema then
    error("archaudit: observe-unknown-schema")
  end
  required_list(facts, "queues")
  required_list(facts, "anomalies")
  required_list(facts, "dlq")
  return facts
end

function M.observe(exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("archaudit: observe requires exec_sync")
  end
  local result = run({ cmd = "fkst-framework observe --json", timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("archaudit: observe-unreadable: " .. tostring(result and result.stderr or "no result"))
  end
  local ok, decoded = pcall(json.decode, result.stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("archaudit: observe-malformed-json")
  end
  return M.validate_observe_facts(decoded)
end

function M.is_idle_observe(facts)
  M.validate_observe_facts(facts)
  for _, row in ipairs(facts.queues) do
    if type(row) ~= "table" then
      error("archaudit: observe-malformed-queue-row")
    end
    if type(row.queue) ~= "string" or row.queue == "" then
      error("archaudit: observe-malformed-queue-name")
    end
    for _, names in ipairs({
      { "ready", "pending", "due", "available", "depth" },
      { "leased", "inflight", "in_flight", "running", "active" },
      { "retry", "retries", "retry_pending", "delayed", "backoff" },
      { "dlq", "dead", "dead_letters", "dead_letter" },
    }) do
      local value, name = required_metric(row, names, names[1])
      if value > 0 then
        return false, "current observe busy " .. tostring(name) .. "=" .. tostring(value)
      end
    end
  end
  if #facts.dlq > 0 then
    return false, "current observe dlq>0"
  end
  if #facts.anomalies > 0 then
    return false, "current observe anomaly>0"
  end
  return true, nil
end

function M.build_prompt(repo, max_findings)
  return table.concat({
    "You are an architecture audit judge for repo " .. tostring(repo) .. ".",
    "Read repository files and CLAUDE.md yourself from the local checkout.",
    "Do not edit files. Do not run gh. Do not run git.",
    "Find only concrete architecture-doctrine violations: god-class, god-state, coupling, SRP, Demeter, DIP, or similar local drift.",
    "Every finding must cite an exact file and line and propose a small local refactor.",
    "Do not report vague smells, umbrellas, grouped unrelated problems, invented rules, or special-case big items.",
    "Return strict JSON only: an array of at most " .. tostring(max_findings) .. " objects.",
    'Object schema: {"file":"packages/example/core.lua","line":42,"rule":"SRP","why":"...","suggested_fix":"..."}',
  }, "\n")
end

function M.parse_findings_json(stdout)
  local raw = strings.trim(stdout or "")
  if raw:sub(1, 1) ~= "[" or raw:sub(-1) ~= "]" then
    error("archaudit: malformed-json: codex output is not a JSON array")
  end
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok then
    error("archaudit: malformed-json: codex output is malformed JSON")
  end
  if type(decoded) ~= "table" then
    error("archaudit: non-array-json: codex output is not a JSON array")
  end
  local count = 0
  for key, _value in pairs(decoded) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      error("archaudit: non-array-json: codex output is not a JSON array")
    end
    if key > count then
      count = key
    end
  end
  if count ~= #decoded then
    error("archaudit: malformed-json: codex output is not a dense JSON array")
  end
  local findings = {}
  for index, item in ipairs(decoded) do
    if type(item) ~= "table"
      or not bounded(item.file, file_limit)
      or type(item.line) ~= "number"
      or item.line < 1
      or math.floor(item.line) ~= item.line
      or not bounded(item.rule, rule_limit)
      or not bounded(item.why, why_limit)
      or not bounded(item.suggested_fix, fix_limit) then
      error("archaudit: invalid-finding-shape: index=" .. tostring(index))
    end
    table.insert(findings, {
      file = item.file,
      line = item.line,
      rule = item.rule,
      why = item.why,
      suggested_fix = item.suggested_fix,
    })
  end
  return findings
end

function M.validate_finding(finding)
  if type(finding) ~= "table" or not bounded(finding.file, file_limit) or type(finding.line) ~= "number" then
    return false
  end
  local text = file.read(finding.file)
  if type(text) ~= "string" or text == "" then
    return false
  end
  local count = 0
  for _line in (text .. "\n"):gmatch("([^\n]*)\n") do
    count = count + 1
    if count == finding.line then
      return true
    end
  end
  return false
end

function M.dedup_key(repo, finding)
  local seed = table.concat({
    tostring(repo),
    tostring(finding.file),
    tostring(finding.line),
    tostring(finding.rule),
  }, "|")
  local readable = table.concat({
    "archaudit",
    strings.sanitize_key(repo, 120),
    strings.sanitize_key(finding.file, 160),
    tostring(finding.line),
    strings.sanitize_key(finding.rule, 80),
    strings.decimal_checksum(seed),
  }, "/")
  return readable:sub(1, 512)
end

function M.build_issue_create_request(repo, finding, label_available)
  assert_request_field(M.validate_repo(repo), "repo")
  local dedup_key = M.dedup_key(repo, finding)
  local title = "Archaudit: " .. tostring(finding.file) .. ":" .. tostring(finding.line) .. " " .. one_line(finding.rule)
  local body = body_text(finding, dedup_key)
  local source_ref_ref = tostring(repo) .. "#" .. tostring(finding.file) .. ":" .. tostring(finding.line) .. "#archaudit-create-intent"
  assert_request_field(strings.is_bounded_string(title, github_proxy_limits.title), "title")
  assert_request_field(strings.is_bounded_string(body, github_proxy_limits.body), "body")
  assert_request_field(strings.is_bounded_string(dedup_key, github_proxy_limits.dedup_key) and marker_safe(dedup_key), "dedup_key")
  assert_request_field(strings.is_bounded_string("repo-site", github_proxy_limits.source_ref_kind), "source_ref.kind")
  assert_request_field(strings.is_bounded_string(source_ref_ref, github_proxy_limits.source_ref_ref), "source_ref.ref")
  local labels = {}
  if label_available then
    labels = { "archaudit" }
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = title,
    body = body,
    labels = labels,
    dedup_key = dedup_key,
    source_ref = {
      kind = "repo-site",
      ref = source_ref_ref,
    },
  }
end

function M.iso_timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d)[%-:](%d%d)[%-:](%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year, month, day = tonumber(year), tonumber(month), tonumber(day)
  hour, minute, second = tonumber(hour), tonumber(minute), tonumber(second)
  if month < 1 or month > 12 or day < 1 or day > 31 or hour > 23 or minute > 59 or second > 59 then
    return nil
  end
  if month <= 2 then
    year = year - 1
    month = month + 12
  end
  local era = math.floor(year / 400)
  local yoe = year - era * 400
  local doy = math.floor((153 * (month - 3) + 2) / 5) + day - 1
  local doe = yoe * 365 + math.floor(yoe / 4) - math.floor(yoe / 100) + doy
  return (era * 146097 + doe - 719468) * 86400 + hour * 3600 + minute * 60 + second
end

function M.idle_hint_freshness(detected_seconds, expires_seconds, now_seconds, budget_seconds)
  if type(detected_seconds) ~= "number" or type(now_seconds) ~= "number" or type(budget_seconds) ~= "number" then
    error("archaudit: malformed idle hint timestamp")
  end
  if now_seconds - detected_seconds > budget_seconds then
    return "stale"
  end
  if expires_seconds ~= nil then
    if type(expires_seconds) ~= "number" then
      error("archaudit: malformed idle hint timestamp")
    end
    if expires_seconds <= now_seconds then
      return "expired"
    end
  end
  return "fresh"
end

function M.failure_fact(dept, tag, error_class, event, message, terminal)
  local fields = error_facts.error_fact_fields(error_class, type(event) == "table" and event.queue or nil, dept, message, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(message))
  return "archaudit dept=" .. tostring(dept) .. " tag=" .. tostring(tag) .. " " .. table.concat(fields, " ")
end

function M.skip_fact(dept, event, why, terminal)
  local fields = error_facts.error_fact_fields("terminal-skip", type(event) == "table" and event.queue or nil, dept, why, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  return "archaudit dept=" .. tostring(dept) .. " tag=SKIP " .. table.concat(fields, " ")
end

return M
```

- [ ] **Step 5: Run and confirm pass.**

```bash
scripts/run.sh test archaudit
```

Expected: PASS for `core_test.lua`. Composed single-package conformance is skipped because `composed.deps` exists.

- [ ] **Step 6: Commit.**

```bash
git add packages/archaudit
git commit -m "feat(archaudit): add composed package core contract"
```

## Task 5: Implement `archaudit.audit` Department

**Files:**
- Create: `packages/archaudit/departments/audit/main.lua`
- Create/Modify: `packages/archaudit/tests/integration_audit_test.lua`
- Test: `scripts/run.sh test archaudit`

- [ ] **Step 1: Write failing integration tests.**

```lua
-- packages/archaudit/tests/integration_audit_test.lua
local testing = require("std.testing")
local github_fake = require("std.github_fake")
local core = require("core")
local t = fkst.test

local function opts(name, env)
  local base = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/archaudit/" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    ARCHAUDIT_MAX_ISSUES_PER_IDLE = "3",
    FKST_GITHUB_WRITE = "",
  }
  for key, value in pairs(env or {}) do
    base[key] = value
  end
  return {
    env = base,
  }
end

local function idle_event(extra)
  local detected_at = "1970-01-01T00:00:00Z"
  local payload = {
    schema = "idle-detector.system-idle.v1",
    detected_at = detected_at,
    expires_at = "1970-01-01T00:10:00Z",
    source_ref = { kind = "host-observe", ref = "idle_tick/" .. detected_at },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "idle-detector.system_idle",
    ts = payload.detected_at,
    payload = payload,
  }
end

local function fresh_idle_event()
  return idle_event({
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:10:00Z",
  })
end

local function mock_env(repo, max_issues)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = max_issues or "3", stderr = "", exit_code = 0 })
end

local function mock_idle_observe()
  t.mock_command("fkst-framework observe --json", {
    stdout = '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":0,"leased":0,"retry":0,"dlq":0}],"anomalies":[],"dlq":[]}',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_busy_observe()
  t.mock_command("fkst-framework observe --json", {
    stdout = '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":1,"leased":0,"retry":0,"dlq":0}],"anomalies":[],"dlq":[]}',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_observe(stdout, exit_code)
  t.mock_command("fkst-framework observe --json", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "observe failed",
    exit_code = exit_code or 0,
  })
end

local function mock_codex_findings(stdout, exit_code)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "codex timeout",
    exit_code = exit_code or 0,
  })
end

local function fake_audit_department(label_stdout)
  package.loaded["departments.audit.main"] = nil
  local model = github_fake.model()
  local label_calls = {}
  local github = github_fake.new(model)
  function github.label_list(repo, timeout)
    table.insert(label_calls, { repo = repo, timeout = timeout })
    return { stdout = label_stdout or "[]", stderr = "", exit_code = 0 }
  end
  local installed = require("departments.audit.main")
  t.eq(type(installed.make_department), "function")
  local dept = installed.make_department({ github = github, git = nil })
  dept.model = model
  return dept, model, label_calls
end

local function run_fake_at(dept, event, fixed_now_seconds)
  local previous_now = now
  now = function()
    return fixed_now_seconds
  end
  local ok, result = pcall(testing.run_fake, dept, event)
  now = previous_now
  if not ok then
    error(result, 0)
  end
  return result
end

local function run_fake_failure_at(dept, event, fixed_now_seconds)
  local previous_now = now
  now = function()
    return fixed_now_seconds
  end
  local ok, result = pcall(testing.run_fake_expecting_failure, dept, event)
  now = previous_now
  if not ok then
    error(result, 0)
  end
  return result
end

return {
  test_fake_fresh_idle_codex_finding_raises_issue_create_request = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Core has one concrete issue.","suggested_fix":"Move the local helper."}]', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 1)
    local raised = result.raises[1]
    t.eq(raised.queue, "github-proxy.github_issue_create_request")
    t.eq(raised.payload.schema, "github-proxy.issue-create.v1")
    t.eq(raised.payload.repo, "owner/repo")
    t.eq(#raised.payload.labels, 0)
    t.eq(raised.payload.source_ref.kind, "repo-site")
    t.is_true(raised.payload.body:find("archaudit-dedup: " .. raised.payload.dedup_key, 1, true) ~= nil)
  end,

  test_fake_caps_distinct_valid_findings_to_first_three = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings(table.concat({
      "[",
      '{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"First issue.","suggested_fix":"Fix first."}',
      ',{"file":"packages/archaudit/core.lua","line":1,"rule":"DIP","why":"Second issue.","suggested_fix":"Fix second."}',
      ',{"file":"packages/archaudit/core.lua","line":1,"rule":"Demeter","why":"Third issue.","suggested_fix":"Fix third."}',
      ',{"file":"packages/archaudit/core.lua","line":1,"rule":"God-state","why":"Fourth issue.","suggested_fix":"Fix fourth."}',
      "]",
    }, ""), 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].payload.title, "Archaudit: packages/archaudit/core.lua:1 SRP")
    t.eq(result.raises[2].payload.title, "Archaudit: packages/archaudit/core.lua:1 DIP")
    t.eq(result.raises[3].payload.title, "Archaudit: packages/archaudit/core.lua:1 Demeter")
  end,

  test_fake_mixed_valid_plus_invalid_batch_is_all_or_nothing_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings(table.concat({
      "[",
      '{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Valid issue.","suggested_fix":"Fix valid."}',
      ',{"file":"packages/archaudit/core.lua","line":999999,"rule":"DIP","why":"Invalid line.","suggested_fix":"Fix invalid."}',
      "]",
    }, ""), 0)
    local dept = fake_audit_department("[]")
    local event = fresh_idle_event()
    local result = run_fake_failure_at(dept, event, core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.is_true(tostring(result.failure.error):find("invalid file or line", 1, true) ~= nil)
    t.eq(#result.raises, 0)
    local fact = core.failure_fact("audit", "FAILURE", "validation-failure", event, "invalid file or line", true)
    t.is_true(fact:find("error_class=validation-failure", 1, true) ~= nil)
    t.is_true(fact:find("fingerprint=", 1, true) ~= nil)
    t.is_true(fact:find("source_ref=host-observe:idle_tick/2026-06-19T01:00:00Z", 1, true) ~= nil)
    t.is_true(fact:find("terminal=true", 1, true) ~= nil)
    t.is_true(fact:find("WHY=invalid file or line", 1, true) ~= nil)
  end,

  test_stale_idle_hint_skips_without_codex = function()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      detected_at = "1970-01-01T00:00:00Z",
      expires_at = "1970-01-01T00:10:00Z",
    }), opts("stale"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#t.command_calls(), 0)
  end,

  test_fake_current_busy_skips_without_codex = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_missing_queues_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe('{"schema":"fkst.observe.v1","anomalies":[],"dlq":[]}', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_unknown_schema_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe('{"schema":"fkst.observe.v2","queues":[],"anomalies":[],"dlq":[]}', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_malformed_top_level_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe('{"schema":"fkst.observe.v1","queues":"bad","anomalies":[],"dlq":[]}', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_missing_each_busy_dimension_group_is_structured_failure_no_issue = function()
    for _, observe_json in ipairs({
      '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","leased":0,"retry":0,"dlq":0}],"anomalies":[],"dlq":[]}',
      '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":0,"retry":0,"dlq":0}],"anomalies":[],"dlq":[]}',
      '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":0,"leased":0,"dlq":0}],"anomalies":[],"dlq":[]}',
      '{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":0,"leased":0,"retry":0}],"anomalies":[],"dlq":[]}',
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_json, 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_ambiguous_and_unknown_metric_groups_are_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe('{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","ready":0,"pending":0,"leased":0,"retry":0,"dlq":0}],"anomalies":[],"dlq":[]}', 0)
    local ambiguous_dept = fake_audit_department("[]")
    local ambiguous = run_fake_failure_at(ambiguous_dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#ambiguous.raises, 0)

    mock_env("owner/repo", "3")
    mock_observe('{"schema":"fkst.observe.v1","queues":[{"queue":"proposal","unexpected":0}],"anomalies":[],"dlq":[]}', 0)
    local unknown_dept = fake_audit_department("[]")
    local unknown = run_fake_failure_at(unknown_dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#unknown.raises, 0)
  end,

  test_fake_missing_repo_is_structured_failure_no_issue = function()
    mock_env("", "3")
    mock_idle_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_long_repo_is_structured_failure_no_issue = function()
    mock_env("owner/" .. string.rep("r", 201), "3")
    mock_idle_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_malformed_repo_is_structured_failure_no_issue = function()
    mock_env("owner repo", "3")
    mock_idle_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_malformed_codex_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings("not json", 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_timeout_codex_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings("", 124)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_codex_nonzero_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings("", 2)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_codex_non_array_json_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('{"file":"packages/archaudit/core.lua"}', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_codex_validation_failure_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":999999,"rule":"SRP","why":"Bad line.","suggested_fix":"Fix."}]', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_malformed_detected_at_is_structured_failure_no_issue = function()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      detected_at = "not-a-time",
    }), opts("malformed-detected-at"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_malformed_expires_at_is_structured_failure_no_issue = function()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      expires_at = "not-a-time",
    }), opts("malformed-expires-at"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

}
```

- [ ] **Step 2: Run and confirm failure.**

```bash
scripts/run.sh test archaudit
```

Expected: FAIL, with `departments/audit/main.lua` missing.

- [ ] **Step 3: Implement `audit/main.lua`.**

```lua
-- packages/archaudit/departments/audit/main.lua
local core = require("core")
local codex = require("std.codex")
local env = require("std.env")
local saga = require("std.saga")
local ports_lib = require("std.ports")
local strings = require("std.strings")

local spec = {
  consumes = { "idle-detector.system_idle" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "10m",
  retry = false,
}

local freshness_budget_seconds = 10 * 60
local codex_timeout_seconds = 9 * 60
local allowed_env = {
  FKST_GITHUB_REPO = true,
  ARCHAUDIT_MAX_ISSUES_PER_IDLE = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("archaudit: env name is not allowed")
  end
  return 'printf %s "$' .. name .. '"'
end

local read_env = env.read_env(read_env_command)

local function log_fact(level, dept, tag, error_class, event, message, terminal)
  if tag == "SKIP" then
    log[level or "warn"](core.skip_fact(dept, event, message, terminal))
  else
    log[level or "warn"](core.failure_fact(dept, tag, error_class, event, message, terminal))
  end
end

local function fail(event, error_class, message)
  log_fact("error", "audit", "FAILURE", error_class, event, message, true)
  error("archaudit: " .. tostring(message), 0)
end

local function fresh_hint(payload, now_seconds)
  local detected = core.iso_timestamp_epoch_seconds(payload.detected_at)
  if detected == nil then
    error("malformed detected_at")
  end
  local expires = nil
  if payload.expires_at ~= nil then
    expires = core.iso_timestamp_epoch_seconds(payload.expires_at)
    if expires == nil then
      error("malformed expires_at")
    end
  end
  local verdict = core.idle_hint_freshness(detected, expires, now_seconds, freshness_budget_seconds)
  if verdict == "stale" then
    return false, "stale system_idle hint"
  end
  if verdict == "expired" then
    return false, "expired system_idle hint"
  end
  return true, nil
end

local function max_issues()
  local raw = strings.trim(read_env("ARCHAUDIT_MAX_ISSUES_PER_IDLE") or "")
  local value = tonumber(raw)
  if value == nil or value < 1 or value > 20 then
    return 3
  end
  return math.floor(value)
end

local function repo_from_env()
  local repo = strings.trim(read_env("FKST_GITHUB_REPO") or "")
  if repo == "" then
    return nil, "missing-repo", "missing FKST_GITHUB_REPO"
  end
  if not core.validate_repo(repo) then
    return nil, "malformed-repo", "malformed FKST_GITHUB_REPO"
  end
  return repo, nil, nil
end

local function has_archaudit_label(github, repo)
  local ok, result = pcall(function()
    return github.label_list(repo, 30)
  end)
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return false
  end
  local ok_json, labels = pcall(json.decode, result.stdout or "[]")
  if not ok_json or type(labels) ~= "table" then
    return false
  end
  for _, label in ipairs(labels) do
    if type(label) == "table" and label.name == "archaudit" then
      return true
    end
  end
  return false
end

local function parser_error_class(err)
  local text = tostring(err)
  if text:find("malformed-json", 1, true) ~= nil then
    return "malformed-json"
  end
  if text:find("non-array-json", 1, true) ~= nil then
    return "non-array-json"
  end
  if text:find("invalid-finding-shape", 1, true) ~= nil then
    return "validation-failure"
  end
  return "validation-failure"
end

local function run_codex(repo, max_count)
  local opts = codex.judgment_codex_opts(core.build_prompt(repo, max_count), ".")
  opts.timeout = codex_timeout_seconds
  local result = spawn_codex_sync(opts)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    local code = type(result) == "table" and tonumber(result.exit_code) or nil
    if code == 124 then
      error("codex-timeout: codex timeout")
    end
    error("codex-nonzero: codex nonzero exit")
  end
  return core.parse_findings_json(result.stdout)
end

local function audit_done(event)
  if type(event) ~= "table" or event.queue ~= "idle-detector.system_idle" then
    fail(event, "unknown-queue", "unknown queue")
  end
  local payload = event.payload or {}
  if payload.schema ~= "idle-detector.system-idle.v1" then
    fail(event, "unknown-schema", "unknown system_idle schema")
  end
  local now_seconds = now()
  local ok_fresh, fresh, why = pcall(fresh_hint, payload, now_seconds)
  if not ok_fresh then
    fail(event, "malformed-idle-hint", fresh)
  end
  if not fresh then
    log_fact("warn", "audit", "SKIP", "terminal-skip", event, why, true)
    return true
  end
  return false
end

local function make_department(ports)
  local function act_audit(event)
    local ok_observe, facts_or_err = pcall(core.observe)
    if not ok_observe then
      local message = tostring(facts_or_err)
      if message:find("observe%-unreadable", 1, false) ~= nil then
        log_fact("warn", "audit", "SKIP", "terminal-skip", event, message, true)
        return
      end
      fail(event, "observe-malformed", message)
    end
    local ok_idle, idle, why = pcall(core.is_idle_observe, facts_or_err)
    if not ok_idle then
      fail(event, "observe-malformed", tostring(idle))
    end
    if not idle then
      log_fact("warn", "audit", "SKIP", "terminal-skip", event, why or "current system busy", true)
      return
    end

    local repo, repo_error_class, repo_error = repo_from_env()
    if repo == nil then
      fail(event, repo_error_class, repo_error)
    end

    local count = max_issues()
    local ok_codex, findings_or_err = pcall(run_codex, repo, count)
    if not ok_codex then
      local message = tostring(findings_or_err)
      if message:find("codex%-timeout", 1, false) ~= nil then
        fail(event, "codex-timeout", "codex timeout")
      end
      if message:find("codex%-nonzero", 1, false) ~= nil then
        fail(event, "codex-nonzero", "codex nonzero exit")
      end
      fail(event, parser_error_class(message), message)
    end

    -- Build the whole capped batch before emitting any raise. Any invalid
    -- finding or github-proxy field bound fails the whole batch with zero raises.
    local label_available = has_archaudit_label(ports.github, repo)
    local requests = {}
    for _, finding in ipairs(findings_or_err) do
      if #requests >= count then
        break
      end
      if not core.validate_finding(finding) then
        fail(event, "validation-failure", "invalid file or line")
      end
      local ok_request, request_or_err = pcall(core.build_issue_create_request, repo, finding, label_available)
      if not ok_request then
        fail(event, "validation-failure", request_or_err)
      end
      table.insert(requests, request_or_err)
    end
    for _, request in ipairs(requests) do
      raise("github-proxy.github_issue_create_request", request)
    end
  end

  local previous_pipeline = _G.pipeline
  local department = saga.department(spec, {
    done = audit_done,
    act = act_audit,
    name = "audit",
  })
  department.pipeline = _G.pipeline
  _G.pipeline = previous_pipeline
  return department
end

local M = ports_lib.install(make_department)
_G.pipeline = M.pipeline
return M
```

Note: `archaudit.core` owns its own tiny local `fkst-framework observe --json` wrapper and validator instead of requiring `idle-detector.core`, because G9 forbids peer cross-package `require`. This is a sanctioned small duplication; it does not add `std.observe` and keeps all cross-package links in `archaudit.audit` to namespaced queues only.

- [ ] **Step 4: Run and confirm pass.**

```bash
scripts/run.sh test archaudit
```

Expected: PASS, including G5 for `integration_audit_test.lua`.

- [ ] **Step 5: Commit.**

```bash
git add packages/archaudit/departments/audit/main.lua packages/archaudit/tests/integration_audit_test.lua
git commit -m "feat(archaudit): audit idle hints into issue-create requests"
```

## Task 6: Add `std.testing.run_fake` GitHub Label Coverage

**Files:**
- Modify: `packages/archaudit/tests/integration_audit_test.lua`
- Test: `scripts/run.sh test archaudit`

- [ ] **Step 1: Add fake-port tests that prove label availability is advisory.**

The Task 5 integration file already imports `std.testing`/`std.github_fake` and defines `fake_audit_department`, `fresh_idle_event`, and `run_fake_at` with a fixed in-process `now()` for non-time fake-port coverage. Add these tests inside the returned test table:

```lua
test_run_fake_label_present_raises_labeled_issue = function()
  mock_env("owner/repo", "3")
  mock_idle_observe()
  mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete issue.","suggested_fix":"Small local fix."}]', 0)
  local dept, model, label_calls = fake_audit_department('[{"name":"archaudit"}]')
  local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
  t.eq(#result.raises, 1)
  t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
  t.eq(result.raises[1].payload.labels[1], "archaudit")
  t.eq(#label_calls, 1)
  t.eq(label_calls[1].repo, "owner/repo")
  t.eq(label_calls[1].timeout, 30)
  t.eq(#model.writes, 0)
  t.eq(#result.writes, 0)
end,

test_run_fake_label_missing_still_raises_unlabeled_issue = function()
  mock_env("owner/repo", "3")
  mock_idle_observe()
  mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete issue.","suggested_fix":"Small local fix."}]', 0)
  local dept, model, label_calls = fake_audit_department('[{"name":"bug"}]')
  local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
  t.eq(#result.raises, 1)
  t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
  t.eq(#result.raises[1].payload.labels, 0)
  t.eq(#label_calls, 1)
  t.eq(label_calls[1].repo, "owner/repo")
  t.eq(label_calls[1].timeout, 30)
  t.eq(#model.writes, 0)
  t.eq(#result.writes, 0)
end,
```

This uses the existing `std.ports.install` behavior from `std/ports.lua`, which attaches `.make_department` to the installed department for fake-port tests. Do not patch `std.ports.production_handles` and do not invoke or mock a real `gh` command in archaudit business tests. Command spelling belongs only in adapter-contract tests.

- [ ] **Step 2: Run and confirm pass.**

```bash
scripts/run.sh test archaudit
```

Expected: PASS. The fake-port test must not call a real `gh` binary.

- [ ] **Step 3: Commit.**

```bash
git add packages/archaudit/tests/integration_audit_test.lua
git commit -m "test(archaudit): prove label availability is not a gate"
```

## Task 7: Prove `github-proxy` Dedup and Dry-Run Boundaries

**Files:**
- Modify: `packages/github-proxy/tests/integration_issue_create_test.lua`
- Test: `scripts/run.sh test github-proxy`

- [ ] **Step 1: Add deterministic downstream idempotency tests through `github-proxy.github_issue_create`.**

Extend the existing `github-proxy` issue-create integration test with an `archaudit` payload. This uses the package's own marker/search fixture and proves durable duplicate suppression belongs to `github-proxy`, not to a local archaudit duplicate ledger.

Add this helper after the existing `event(extra)` helper in `packages/github-proxy/tests/integration_issue_create_test.lua`:

```lua
local function archaudit_payload()
  local dedup_key = "archaudit/owner/repo/packages-archaudit-core-lua/1/SRP/12345"
  return {
    schema = "github-proxy.issue-create.v1",
    repo = "owner/repo",
    title = "Archaudit: packages/archaudit/core.lua:1 SRP",
    body = table.concat({
      "Architecture doctrine violation:",
      "",
      "File: packages/archaudit/core.lua:1",
      "Rule: SRP",
      "",
      "Why:",
      "Concrete issue.",
      "",
      "Suggested fix:",
      "Small local fix.",
      "",
      "<!-- archaudit-dedup: " .. dedup_key .. " -->",
    }, "\n"),
    labels = {},
    dedup_key = dedup_key,
    source_ref = {
      kind = "repo-site",
      ref = "owner/repo#packages/archaudit/core.lua:1#archaudit-create-intent",
    },
  }
end
```

Add these tests inside the existing returned test table:

```lua
  test_archaudit_duplicate_marker_search_skips_github_proxy_create = function()
    local payload = archaudit_payload()
    mock_write_env("1")
    mock_bot_env()
    mock_issue_create_search(string.format(
      '[{"number":99,"title":"Existing","state":"OPEN","body":"already created\\n%s","author":{"login":"fkst-test-bot"}}]\n',
      h.json_string(core.issue_create_marker(payload.dedup_key))
    ))
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("duplicate-marker", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 0)
  end,

  test_archaudit_issue_create_dry_run_does_not_search_or_create = function()
    mock_write_env("")

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = archaudit_payload(),
    }, opts("dry-run-archaudit"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
  end,
```

These tests deliberately exercise `github-proxy`'s existing marker/search contract: a trusted issue body containing `core.issue_create_marker(dedup_key)` makes duplicate delivery idempotent, and empty `FKST_GITHUB_WRITE` short-circuits before any real search/create write. They are not a substitute for archaudit's per-run cap; they prove the second anti-flood control.

- [ ] **Step 2: Run and confirm pass.**

```bash
scripts/run.sh test github-proxy
```

Expected: PASS. The duplicate-marker test reports one `gh issue list` call and zero `gh issue create` calls; the dry-run test reports zero search/create calls.

- [ ] **Step 3: Commit.**

```bash
git add packages/github-proxy/tests/integration_issue_create_test.lua
git commit -m "test(archaudit): prove github-proxy issue-create boundaries"
```

## Task 8: Verify Delivery Semantics, Conformance, and Ratchets

**Files:**
- Modify only if a verification failure requires a minimal fix in files created by Tasks 1-7.
- Test: `scripts/run.sh test idle-detector`
- Test: `scripts/run.sh test archaudit`
- Test: `scripts/run.sh test-composed`
- Test: `scripts/run.sh check`

- [ ] **Step 1: Run `idle-detector` package tests.**

```bash
scripts/run.sh test idle-detector
```

Expected: PASS. This covers flat conformance for bare queues `idle_tick` and `system_idle`, no `ephemeral`, reliable `source_ref` payload shape, and G5 engine PASS for every `*_test.lua`.

- [ ] **Step 2: Run `archaudit` package tests.**

```bash
scripts/run.sh test archaudit
```

Expected: PASS. Single-package conformance is skipped because `archaudit` is composed, but tests must pass and every `*_test.lua` must produce at least one report-json PASS.

- [ ] **Step 3: Run composed conformance.**

```bash
scripts/run.sh test-composed
```

Expected: PASS under the composed graph containing `idle-detector`, `archaudit`, and `github-proxy`. Confirm `archaudit/composed.deps` has exactly:

```text
idle-detector
github-proxy
```

- [ ] **Step 4: Run repository checks.**

```bash
scripts/run.sh check
```

Expected: PASS. Confirm the relevant ratchets stay green:

- `G5`: every `packages/*/tests/*_test.lua` has at least one engine PASS.
- `G-SAGA-HEAD`: `idle_gate/main.lua` and `audit/main.lua` declare `local spec = { ... }` after requires and before helper functions, then pass it to `saga.department(spec, handlers)`.
- `G-ADAPTER`: no raw `gh` or `git` command construction in package business logic; GitHub label checks use `std.github.label_list` behind `std.ports`.
- `G9`: no peer cross-package `require`; `archaudit` references sibling packages only by namespaced queues.
- `G-CONTENT-TRUNCATION`: no new content truncation into reliable payloads or codex prompts; issue bodies remain bounded request fields, while source code is read by codex from the repo.

- [ ] **Step 5: Commit any verification-only fixes.**

Only commit if a minimal correction was required:

```bash
git add packages/idle-detector packages/archaudit
git commit -m "fix(idle-detector,archaudit): satisfy package conformance"
```

If all verification passed without changes, skip this commit.

## Task 9: Verify the Minimal Archaudit Spec Is Already Current

**Files:**
- Read-only verification: `docs/superpowers/specs/2026-06-18-archaudit-design.md`
- Do not modify docs unless verification proves the file is not the minimal two-package spec.

- [ ] **Step 1: Confirm the heavy archaudit doc replacement is already done.**

```bash
sed -n '1,220p' docs/superpowers/specs/2026-06-18-archaudit-design.md
```

Expected: The file starts with `# Idle-Detector + Archaudit - Minimal Design Spec` and describes exactly the two-package plan: `idle-detector` flat plus `archaudit` composed.

- [ ] **Step 2: Do not rewrite the spec.**

No code or doc change is required for this slice. This is verification only because the current spec is already the minimal spec.

- [ ] **Step 3: Record verification in the implementation summary.**

Use this exact summary line:

```text
Verified docs/superpowers/specs/2026-06-18-archaudit-design.md is already the minimal idle-detector + archaudit spec; no doc replacement was needed.
```

## Self-Review

### Spec-Coverage Check

- Intent and architecture chain -> Tasks 1, 3, 5, 7.
- `idle-detector` flat package layout -> Tasks 1-3.
- Observe wrapper, idle predicate, payload builder -> Task 2.
- Reliable `idle_tick` consumption and `system_idle` production, no `ephemeral` -> Tasks 1, 3, 7.
- Stale cron-slot drop using event slot/time, not wall-clock processing time for `detected_at` -> Task 3.
- Terminal skip-with-WHY for stale/busy/DLQ/observe failure -> Task 3.
- `archaudit` composed deps and namespaced queues -> Tasks 4, 5, 7.
- `archaudit/core.lua` limited to prompt/parser/validator/dedup/request builder -> Task 4.
- Freshness/expiry gate and current idle re-check -> Task 5.
- `FKST_GITHUB_REPO` validation -> Task 5.
- One bounded read-only codex judgment with strict JSON -> Task 5.
- File/line validation only, no broader evidence validator -> Task 4.
- Per-run cap default 3 -> Task 5.
- Issue-create payload schema, dedup key, source_ref, body marker, label advisory behavior -> Tasks 4-6.
- No in-batch duplicate collapse in `archaudit`; the only local anti-flood control is the per-run cap -> Task 5.
- Durable duplicate suppression through github-proxy marker/search idempotency -> Task 7.
- Anti-flood exactly two controls: per-run cap 3 and github-proxy dedup -> Tasks 5 and 7.
- Defaults table -> Defaults section.
- Explicit non-goals -> Explicit Non-Goals section.
- Test suite and composed conformance -> Task 8.
- Existing minimal spec verification -> Task 9.

### Placeholder Scan

Before implementation is declared complete, run:

```bash
rg -n "[T]BD|[T]ODO|place[h]older|similar [t]o|co[p]y .*later|st[u]b" packages/idle-detector packages/archaudit docs/superpowers/plans/2026-06-19-idle-detector-archaudit.md
```

Expected: no hits that indicate incomplete implementation. Existing explanatory words in this plan are acceptable only if they are not markers for missing code.

### Type/Name Consistency Check

Verify these exact names before final commit:

- Flat package: `packages/idle-detector`
- Composed package: `packages/archaudit`
- Idle queues: `idle_tick`, `system_idle`
- Composed consumed queue: `idle-detector.system_idle`
- Composed produced queue: `github-proxy.github_issue_create_request`
- Idle schema: `idle-detector.system-idle.v1`
- Issue-create schema: `github-proxy.issue-create.v1`
- GitHub issue-create queue confirmed from `packages/github-proxy/departments/github_issue_create/main.lua`: `github_issue_create_request`
- Saga signature confirmed from `std/saga.lua`: `saga.department(spec, { done = ..., act = ... })`
- Cron raiser shape confirmed from existing raisers: `{ type = "cron", interval = "...", produces = "..." }`
- Test harness commands confirmed from `scripts/run.sh`: `scripts/run.sh test <pkg>`, `scripts/run.sh test-composed`, `scripts/run.sh check`

⟦AI:FKST⟧
