local t = fkst.test

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/idle-detector/" .. tostring(name),
      FKST_DURABLE_ROOT = "/tmp/fkst-packages-test/idle-detector/durable-" .. tostring(name),
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

local function mock_observe(stdout, exit_code)
  t.mock_command('fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', {
    stdout = stdout or "",
    stderr = exit_code == 0 and "" or "observe failed",
    exit_code = exit_code or 0,
  })
end

local function observe_json(generated_at_ms, queue_json, deliveries_json, dead_letters_json, truncated_json)
  return table.concat({
    '{"schema_version":1',
    ',"generated_at_ms":' .. tostring(generated_at_ms or 1781830860000),
    ',"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"}',
    ',"limits":{"max_deliveries":500,"max_dead_letters":500}',
    ',"truncated":' .. (truncated_json or '{"deliveries":false,"dead_letters":false}'),
    ',"queues":' .. (queue_json or '[{"queue":"proposal","depth":0,"pending":0,"in_flight":0,"retrying":0,"oldest_pending_age_ms":null}]'),
    ',"deliveries":' .. (deliveries_json or "[]"),
    ',"dead_letters":' .. (dead_letters_json or "[]"),
    "}",
  }, "")
end

local function assert_skip_with_observe(case_name, observe_stdout, exit_code)
  mock_observe(observe_stdout, exit_code or 0)
  local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts(case_name))
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
end

return {
  test_idle_gate_uses_observe_time_to_raise_fresh_idle = function()
    mock_observe(observe_json(1781830860000), 0)
    local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts("fresh"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "system_idle")
    t.eq(result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
  end,

  test_idle_gate_accepts_cron_slot_and_event_ts_fallbacks = function()
    mock_observe(observe_json(1781830860000), 0)
    local cron_event = event("2026-06-19T01:00:00Z")
    cron_event.payload.slot = nil
    cron_event.payload.cron_slot = "2026-06-19T01:00:00Z"
    local cron_result = t.run_department("departments/idle_gate/main.lua", cron_event, opts("cron-slot"))
    t.eq(cron_result.exit_code, 0)
    t.eq(cron_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")

    mock_observe(observe_json(1781830860000), 0)
    local ts_event = event("2026-06-19T01:00:00Z")
    ts_event.payload.slot = nil
    ts_event.payload.cron_slot = nil
    ts_event.payload.detected_at = nil
    local ts_result = t.run_department("departments/idle_gate/main.lua", ts_event, opts("event-ts-slot"))
    t.eq(ts_result.exit_code, 0)
    t.eq(ts_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
  end,

  test_idle_gate_uses_observe_time_to_drop_stale_slot = function()
    mock_observe(observe_json(1781831461000), 0)
    local result = t.run_department("departments/idle_gate/main.lua", event("2026-06-19T01:00:00Z"), opts("stale"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_idle_gate_skips_observe_derived_busy_states = function()
    for _, case in ipairs({
      {
        name = "pending",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":1,"pending":1,"in_flight":0,"retrying":0,"oldest_pending_age_ms":1000}]'),
      },
      {
        name = "in-flight",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":1,"pending":0,"in_flight":1,"retrying":0,"oldest_pending_age_ms":null}]'),
      },
      {
        name = "retrying",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":1,"pending":0,"in_flight":0,"retrying":1,"oldest_pending_age_ms":null}]'),
      },
      {
        name = "depth",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":1,"pending":0,"in_flight":0,"retrying":0,"oldest_pending_age_ms":null}]'),
      },
    }) do
      assert_skip_with_observe("busy-" .. case.name, case.observe, 0)
    end
  end,

  test_idle_gate_skips_deliveries_or_dead_letters = function()
    assert_skip_with_observe("deliveries", observe_json(1781830860000, nil, '[{"delivery_id":"d1","queue":"proposal","dept":"decide","status":"pending","attempt":1}]', nil), 0)
    assert_skip_with_observe("dead-letters", observe_json(1781830860000, nil, nil, '[{"delivery_id":"dead","queue":"proposal","dept":"decide","attempts":1,"replayable":true,"permanent":false}]'), 0)
  end,

  test_idle_gate_skips_truncated_observe_lists_without_raising_idle = function()
    assert_skip_with_observe("truncated-deliveries", observe_json(1781830860000, nil, nil, nil, '{"deliveries":true,"dead_letters":false}'), 0)
    assert_skip_with_observe("truncated-dead-letters", observe_json(1781830860000, nil, nil, nil, '{"deliveries":false,"dead_letters":true}'), 0)
  end,

  test_idle_gate_skips_observe_read_failure = function()
    assert_skip_with_observe("observe-failure", "", 1)
  end,

  test_idle_gate_logs_terminal_skip_on_observe_read_failure = function()
    mock_observe("", 1)
    local previous_warn = log.warn
    local logs = {}
    log.warn = function(message)
      table.insert(logs, tostring(message))
    end
    package.loaded["departments.idle_gate.main"] = nil
    local ok, err = pcall(function()
      local dept = require("departments.idle_gate.main")
      dept.pipeline(event("2026-06-19T01:00:00Z"))
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

  test_idle_gate_skips_malformed_json_and_malformed_slot_without_raising_idle = function()
    assert_skip_with_observe("malformed-json", "{not json", 0)

    mock_observe(observe_json(1781830860000), 0)
    local malformed_slot = event("not-a-time")
    local result = t.run_department("departments/idle_gate/main.lua", malformed_slot, opts("malformed-slot"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_idle_gate_skips_malformed_observe_shapes = function()
    for _, case in ipairs({
      {
        name = "missing-generated-at",
        observe = '{"schema_version":1,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "wrong-generated-at-type",
        observe = '{"schema_version":1,"generated_at_ms":"1781830860000","source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "missing-source",
        observe = '{"schema_version":1,"generated_at_ms":1781830860000,"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "missing-limits",
        observe = '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "missing-truncated",
        observe = '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "non-boolean-truncated",
        observe = observe_json(1781830860000, nil, nil, nil, '{"deliveries":"false","dead_letters":false}'),
      },
      {
        name = "non-integer-limits",
        observe = '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":1.5,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}',
      },
      {
        name = "non-table-queues",
        observe = observe_json(1781830860000, '"bad"'),
      },
      {
        name = "keyed-queues",
        observe = observe_json(1781830860000, '{"proposal":{"depth":0,"pending":0,"in_flight":0,"retrying":0}}'),
      },
      {
        name = "keyed-deliveries",
        observe = observe_json(1781830860000, "[]", '{"one":{}}'),
      },
      {
        name = "keyed-dead-letters",
        observe = observe_json(1781830860000, "[]", "[]", '{"one":{}}'),
      },
    }) do
      assert_skip_with_observe("malformed-" .. case.name, case.observe, 0)
    end
  end,

  test_idle_gate_skips_missing_real_queue_metrics = function()
    for _, case in ipairs({
      {
        name = "missing-depth",
        observe = observe_json(1781830860000, '[{"queue":"proposal","pending":0,"in_flight":0,"retrying":0}]'),
      },
      {
        name = "missing-pending",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":0,"in_flight":0,"retrying":0}]'),
      },
      {
        name = "missing-in-flight",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":0,"pending":0,"retrying":0}]'),
      },
      {
        name = "missing-retrying",
        observe = observe_json(1781830860000, '[{"queue":"proposal","depth":0,"pending":0,"in_flight":0}]'),
      },
    }) do
      assert_skip_with_observe("metric-" .. case.name, case.observe, 0)
    end
  end,
}
