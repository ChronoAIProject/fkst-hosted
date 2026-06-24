local core = require("core")
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
    queue = "idle-detector.idle_tick",
    ts = slot,
    payload = {
      schema = "idle-detector.idle-tick.v1",
      slot = slot,
      source_ref = { kind = "cron", ref = "idle-detector/idle_poll/" .. slot },
    },
  }
end

local function observe_facts(opts)
  opts = opts or {}
  local facts = {
    schema_version = opts.schema_version or 1,
    source = opts.source or {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = opts.limits or { max_deliveries = 500, max_dead_letters = 500 },
    truncated = opts.truncated or { deliveries = false, dead_letters = false },
    queues = opts.queues or {
      { queue = "proposal", depth = 0, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil },
    },
    deliveries = opts.deliveries or json.decode("[]"),
    dead_letters = opts.dead_letters or json.decode("[]"),
  }
  if not opts.omit_generated_at then
    facts.generated_at_ms = opts.generated_at_ms or 1781830860000
  end
  if opts.omit_source then facts.source = nil end
  if opts.omit_limits then facts.limits = nil end
  if opts.omit_truncated then facts.truncated = nil end
  if opts.omit_queues then facts.queues = nil end
  return facts
end

local function mock_observe(snapshot)
  t.mock_observe(snapshot or observe_facts())
end

local function assert_skip_with_observe(case_name, snapshot)
  mock_observe(snapshot)
  local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts(case_name))
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
end

local function with_core_observe_error(message, fn)
  local original = core.observe
  core.observe = function()
    error(message)
  end
  local ok, result = pcall(fn)
  core.observe = original
  if not ok then
    error(result, 0)
  end
  return result
end

return {
  test_idle_gate_uses_observe_time_to_raise_fresh_idle = function()
    mock_observe(observe_facts({ generated_at_ms = 1781830860000 }))
    local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts("fresh"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "system_idle")
    t.eq(result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
  end,

  test_idle_gate_accepts_cron_slot_and_event_ts_fallbacks = function()
    mock_observe(observe_facts({ generated_at_ms = 1781830860000 }))
    local cron_event = event("2026-06-19T01:00:00Z")
    cron_event.payload.slot = nil
    cron_event.payload.cron_slot = "2026-06-19T01:00:00Z"
    local cron_result = t.run_department("departments/idle_gate/main.lua", cron_event, opts("cron-slot"))
    t.eq(cron_result.exit_code, 0)
    t.eq(cron_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")

    mock_observe(observe_facts({ generated_at_ms = 1781830860000 }))
    local ts_event = event("2026-06-19T01:00:00Z")
    ts_event.payload.slot = nil
    ts_event.payload.cron_slot = nil
    ts_event.payload.detected_at = nil
    local ts_result = t.run_department("departments/idle_gate/main.lua", ts_event, opts("event-ts-slot"))
    t.eq(ts_result.exit_code, 0)
    t.eq(ts_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
  end,

  test_idle_gate_uses_observe_time_to_drop_stale_slot = function()
    mock_observe(observe_facts({ generated_at_ms = 1781831461000 }))
    local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts("stale"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_idle_gate_skips_observe_derived_busy_states = function()
    for _, case in ipairs({
      {
        name = "pending",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 1, pending = 1, in_flight = 0, retrying = 0, oldest_pending_age_ms = 1000 } } }),
      },
      {
        name = "in-flight",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 1, pending = 0, in_flight = 1, retrying = 0, oldest_pending_age_ms = nil } } }),
      },
      {
        name = "retrying",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 1, pending = 0, in_flight = 0, retrying = 1, oldest_pending_age_ms = nil } } }),
      },
      {
        name = "depth",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 1, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil } } }),
      },
    }) do
      assert_skip_with_observe("busy-" .. case.name, case.observe)
    end
  end,

  test_idle_gate_skips_deliveries_or_dead_letters = function()
    assert_skip_with_observe("deliveries", observe_facts({ deliveries = { { delivery_id = "d1", queue = "proposal", dept = "decide", status = "pending", attempt = 1 } } }))
    assert_skip_with_observe("dead-letters", observe_facts({ dead_letters = { { delivery_id = "dead", queue = "proposal", dept = "decide", attempts = 1, replayable = true, permanent = false } } }))
  end,

  test_idle_gate_skips_truncated_observe_lists_without_raising_idle = function()
    assert_skip_with_observe("truncated-deliveries", observe_facts({ truncated = { deliveries = true, dead_letters = false } }))
    assert_skip_with_observe("truncated-dead-letters", observe_facts({ truncated = { deliveries = false, dead_letters = true } }))
  end,

  test_idle_gate_skips_observe_read_failure = function()
    with_core_observe_error("idle-detector: observe-unreadable: synthetic observe failure", function()
      local dept = require("departments.idle_gate.main")
      dept.pipeline(event("2026-06-19T01:00:00Z"))
    end)
  end,

  test_idle_gate_fails_loud_when_observe_durable_root_unresolved = function()
    local previous_warn = log.warn
    local previous_error = log.error
    local warns = {}
    local errors = {}
    log.warn = function(message)
      table.insert(warns, tostring(message))
    end
    log.error = function(message)
      table.insert(errors, tostring(message))
    end
    local ok, err = pcall(function()
      with_core_observe_error("idle-detector: observe-durable-root-unresolved: FKST_DURABLE_ROOT must be set for fkst.observe", function()
        local dept = require("departments.idle_gate.main")
        dept.pipeline(event("2026-06-19T01:00:00Z"))
      end)
    end)
    log.warn = previous_warn
    log.error = previous_error
    t.eq(ok, false)
    t.eq(#warns, 0)
    t.is_true(tostring(err):find("observe-durable-root-unresolved", 1, true) ~= nil)
    t.is_true(tostring(errors[1] or ""):find("caught-failure", 1, true) ~= nil)
  end,

  test_idle_gate_logs_terminal_skip_on_observe_read_failure = function()
    local previous_warn = log.warn
    local logs = {}
    log.warn = function(message)
      table.insert(logs, tostring(message))
    end
    local ok, err = pcall(function()
      with_core_observe_error("idle-detector: observe-unreadable: synthetic observe failure", function()
        local dept = require("departments.idle_gate.main")
        dept.pipeline(event("2026-06-19T01:00:00Z"))
      end)
    end)
    log.warn = previous_warn
    if not ok then
      error(err, 0)
    end
    t.eq(#logs, 1)
    t.is_true(logs[1]:find("tag=SKIP", 1, true) ~= nil)
    t.is_true(logs[1]:find("error_class=terminal-skip", 1, true) ~= nil)
    t.is_true(logs[1]:find("terminal=true", 1, true) ~= nil)
    t.is_true(logs[1]:find("unreadable observe facts", 1, true) ~= nil)
  end,

  test_idle_gate_skips_malformed_snapshot_and_malformed_slot_without_raising_idle = function()
    assert_skip_with_observe("malformed-snapshot", "not facts")

    mock_observe(observe_facts({ generated_at_ms = 1781830860000 }))
    local malformed_slot = event("not-a-time")
    local result = t.run_department("departments/idle_gate/main.lua", malformed_slot, opts("malformed-slot"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_idle_gate_skips_malformed_observe_shapes = function()
    for _, case in ipairs({
      {
        name = "missing-generated-at",
        observe = observe_facts({ omit_generated_at = true, queues = {} }),
      },
      {
        name = "wrong-generated-at-type",
        observe = observe_facts({ generated_at_ms = "1781830860000", queues = {} }),
      },
      {
        name = "missing-source",
        observe = observe_facts({ omit_source = true, queues = {} }),
      },
      {
        name = "missing-limits",
        observe = observe_facts({ omit_limits = true, queues = {} }),
      },
      {
        name = "missing-truncated",
        observe = observe_facts({ omit_truncated = true, queues = {} }),
      },
      {
        name = "non-boolean-truncated",
        observe = observe_facts({ truncated = { deliveries = "false", dead_letters = false } }),
      },
      {
        name = "non-object-limits",
        observe = observe_facts({ limits = "bad", queues = {} }),
      },
      {
        name = "non-table-queues",
        observe = observe_facts({ queues = "bad" }),
      },
      {
        name = "keyed-queues",
        observe = observe_facts({ queues = { proposal = { depth = 0, pending = 0, in_flight = 0, retrying = 0 } } }),
      },
      {
        name = "keyed-deliveries",
        observe = observe_facts({ queues = {}, deliveries = { one = {} } }),
      },
      {
        name = "keyed-dead-letters",
        observe = observe_facts({ queues = {}, dead_letters = { one = {} } }),
      },
    }) do
      assert_skip_with_observe("malformed-" .. case.name, case.observe)
    end
  end,

  test_idle_gate_skips_missing_real_queue_metrics = function()
    for _, case in ipairs({
      {
        name = "missing-depth",
        observe = observe_facts({ queues = { { queue = "proposal", pending = 0, in_flight = 0, retrying = 0 } } }),
      },
      {
        name = "missing-pending",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 0, in_flight = 0, retrying = 0 } } }),
      },
      {
        name = "missing-in-flight",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 0, pending = 0, retrying = 0 } } }),
      },
      {
        name = "missing-retrying",
        observe = observe_facts({ queues = { { queue = "proposal", depth = 0, pending = 0, in_flight = 0 } } }),
      },
    }) do
      assert_skip_with_observe("metric-" .. case.name, case.observe)
    end
  end,
}
