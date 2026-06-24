local core = require("core")
local t = fkst.test

local function observe_idle()
  return {
    schema_version = 1,
    generated_at_ms = 1781830860000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 500, max_dead_letters = 500 },
    truncated = { deliveries = false, dead_letters = false },
    queues = {
      { queue = "idle_tick", depth = 0, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil },
      { queue = "github_poll_tick", depth = 0, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil },
    },
    deliveries = json.decode("[]"),
    dead_letters = json.decode("[]"),
  }
end

return {
  test_idle_predicate_accepts_real_zero_snapshot = function()
    local idle, why = core.is_idle_observe(observe_idle())
    t.eq(idle, true)
    t.is_nil(why)
  end,

  test_idle_predicate_fails_closed_on_missing_required_real_fields = function()
    for _, field in ipairs({
      "schema_version",
      "generated_at_ms",
      "source",
      "limits",
      "truncated",
      "queues",
      "deliveries",
      "dead_letters",
    }) do
      local facts = observe_idle()
      facts[field] = nil
      t.raises(function() core.is_idle_observe(facts) end)
    end
  end,

  test_idle_predicate_fails_closed_on_unknown_schema_version = function()
    local facts = observe_idle()
    facts.schema_version = 2
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_fails_closed_on_malformed_top_level = function()
    t.raises(function() core.is_idle_observe("not facts") end)
    local facts = observe_idle()
    facts.queues = "not a table"
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.generated_at_ms = "1781830860000"
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.source = "not a table"
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.limits.max_deliveries = 1.5
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.limits.max_dead_letters = "500"
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.truncated.deliveries = "false"
    t.raises(function() core.is_idle_observe(facts) end)
    facts = observe_idle()
    facts.truncated.dead_letters = 0
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_fails_closed_on_non_dense_observe_lists = function()
    for _, list_name in ipairs({ "queues", "deliveries", "dead_letters" }) do
      local keyed = observe_idle()
      keyed[list_name] = { keyed = {} }
      t.raises(function() core.is_idle_observe(keyed) end)

      local sparse = observe_idle()
      sparse[list_name] = {}
      sparse[list_name][1] = {}
      sparse[list_name][3] = {}
      t.raises(function() core.is_idle_observe(sparse) end)
    end
  end,

  test_idle_predicate_rejects_real_busy_queue_dimensions = function()
    for _, field in ipairs({ "depth", "pending", "in_flight", "retrying" }) do
      local facts = observe_idle()
      facts.queues[1][field] = 1
      local idle, why = core.is_idle_observe(facts)
      t.eq(idle, false)
      t.is_true(why:find(field, 1, true) ~= nil)
    end
  end,

  test_idle_predicate_fails_closed_on_missing_each_real_queue_dimension = function()
    for _, field in ipairs({ "depth", "pending", "in_flight", "retrying" }) do
      local facts = observe_idle()
      facts.queues[1][field] = nil
      t.raises(function() core.is_idle_observe(facts) end)
    end
  end,

  test_idle_predicate_fails_closed_on_malformed_queue_rows = function()
    local facts = observe_idle()
    facts.queues[1] = "bad"
    t.raises(function() core.is_idle_observe(facts) end)

    facts = observe_idle()
    facts.queues[1].queue = ""
    t.raises(function() core.is_idle_observe(facts) end)

    facts = observe_idle()
    facts.queues[1].pending = -1
    t.raises(function() core.is_idle_observe(facts) end)
  end,

  test_idle_predicate_rejects_deliveries_and_dead_letters = function()
    local facts = observe_idle()
    facts.deliveries = { { delivery_id = "d1", queue = "q", dept = "d", status = "pending", attempt = 1 } }
    local idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("deliveries=1", 1, true) ~= nil)

    facts = observe_idle()
    facts.dead_letters = { { delivery_id = "dead", queue = "q", dept = "d", attempts = 1, replayable = true, permanent = false } }
    idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("dead_letters=1", 1, true) ~= nil)
  end,

  test_idle_predicate_rejects_truncated_observe_lists_as_not_idle = function()
    local facts = observe_idle()
    facts.truncated.deliveries = true
    local idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("truncated deliveries", 1, true) ~= nil)

    facts = observe_idle()
    facts.truncated.dead_letters = true
    idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("truncated dead_letters", 1, true) ~= nil)
  end,

  test_observe_now_seconds_uses_generated_at_ms = function()
    t.eq(core.observe_now_seconds(observe_idle()), 1781830860)
  end,

  test_observe_wrapper_consumes_injected_snapshot = function()
    t.mock_observe(observe_idle())
    local observed = core.observe()
    t.eq(observed.schema_version, 1)
    t.eq(observed.generated_at_ms, 1781830860000)
  end,

  test_observe_wrapper_fails_closed_on_unknown_schema_version = function()
    local facts = observe_idle()
    facts.schema_version = 2
    t.mock_observe(facts)
    t.raises(function()
      core.observe()
    end)
  end,

  test_observe_wrapper_rejects_malformed_snapshot = function()
    t.mock_observe("not facts")
    t.raises(function()
      core.observe()
    end)
  end,

  test_observe_wrapper_reports_malformed_snapshot_error_class = function()
    t.mock_observe("not facts")
    local ok, err = pcall(function()
      core.observe()
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("idle-detector: malformed-observe-facts", 1, true) ~= nil)
  end,

  test_observe_wrapper_accepts_generic_options = function()
    t.mock_observe(observe_idle())
    local observed = core.observe({ limit = 10 })
    t.eq(observed.schema_version, 1)
    t.eq(#observed.queues, 2)
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

  test_iso_timestamp_parser_covers_invalid_and_january_dates = function()
    t.eq(core.iso_timestamp_epoch_seconds("not-a-time"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-13-01T00:00:00Z"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-01-01T00:00:00Z"), 1767225600)
  end,

  test_skip_fact_fields_are_pure_and_structured = function()
    for _, case in ipairs({
      { why = "busy queue=proposal pending=1" },
      { why = "busy dead_letters=1" },
      { why = "unreadable observe facts: observe failed" },
      { why = "malformed observe facts: malformed generated_at_ms" },
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
