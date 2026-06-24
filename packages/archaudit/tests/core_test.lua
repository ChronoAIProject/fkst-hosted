local core = require("core")
local t = fkst.test

local finding_json = '[{"file":"packages/idle-detector/core.lua","line":1,"rule":"SRP","why":"Mixed responsibilities.","suggested_fix":"Extract the extra responsibility."}]'

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
      { queue = "proposal", depth = 0, pending = 0, in_flight = 0, retrying = 0 },
    },
    deliveries = {},
    dead_letters = {},
  }
end

local function observe_idle_json()
  return '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}'
end

local function copy_contract(contract)
  local copied = {}
  for key, value in pairs(contract) do
    if type(value) == "table" then
      local nested = {}
      for nested_key, nested_value in pairs(value) do
        nested[nested_key] = nested_value
      end
      copied[key] = nested
    else
      copied[key] = value
    end
  end
  return copied
end

return {
  test_producer_liveness_contract_declares_overdue_progress = function()
    local contracts = core.producer_liveness_contracts()
    t.eq(#contracts, 1)
    local contract = contracts[1]
    t.eq(contract.producer_id, "archaudit.audit")
    t.eq(contract.trigger_source, "archaudit_tick")
    t.eq(contract.output_queues[1], "github-proxy.github_issue_create_request")
    t.eq(contract.escalation_queues, nil)
    t.eq(contract.eligibility_predicate, "overdue")
    t.eq(contract.max_staleness_seconds, core.audit_due_staleness_seconds())
    t.eq(contract.completion_budget_seconds, core.audit_due_completion_budget_seconds())
    t.eq(contract.force_at_seconds, core.audit_due_force_at_seconds())
    t.eq(contract.max_silence_seconds, core.audit_poll_interval_seconds())
    t.eq(contract.max_skip_budget, 0)
    t.eq(contract.progress_output, "github-proxy.github_issue_create_request")
    t.eq(contract.runtime_gate, "idle_when_not_overdue")
    t.eq(contract.adversarial_fixture, "busy_overdue")
  end,

  test_producer_liveness_contract_reuses_restart_liveness_watchdog = function()
    local errors = core.producer_liveness_contract_errors()
    t.eq(table.concat(errors, "\n"), "")
    local rows = core.producer_liveness_restart_rows()
    t.eq(#rows, 1)
    local row = rows[1]
    t.eq(row.from_state, "archaudit.audit")
    t.eq(row.driving_queue, "archaudit_tick")
    t.eq(row.watchdog.mode, "row-budget-bounds-receiver")
    t.eq(row.watchdog.budget_ms, core.audit_due_staleness_seconds() * 1000)
    t.eq(row.budget.minutes, core.audit_due_staleness_seconds() / 60)
    t.eq(row.liveness_contract.mode, "row-budget-bounds-receiver")
    t.eq(row.liveness_contract.receiver_bound_minutes, core.audit_poll_interval_seconds() / 60)
    t.eq(row.on_timeout.escalate_after_attempts, 1)
    t.eq(row.on_timeout.queue, "archaudit_tick")
    t.eq(row.producer_liveness.runtime_gate, "idle_when_not_overdue")
    t.eq(row.producer_liveness.adversarial_fixture, "busy_overdue")
  end,

  test_producer_liveness_contract_rejects_unbounded_silence = function()
    local contract = copy_contract(core.producer_liveness_contracts()[1])
    contract.max_silence_seconds = 0
    local errors = core.producer_liveness_contract_errors({ contract })
    t.is_true(table.concat(errors, "\n"):find("max_silence_seconds", 1, true) ~= nil)
  end,

  test_producer_liveness_contract_rejects_silence_without_watchdog_margin = function()
    local contract = copy_contract(core.producer_liveness_contracts()[1])
    contract.max_silence_seconds = contract.max_staleness_seconds
    local errors = core.producer_liveness_contract_errors({ contract })
    t.is_true(table.concat(errors, "\n"):find("budget.minutes must be at least", 1, true) ~= nil)
  end,

  test_audit_poll_raiser_interval_is_bounded_by_contract = function()
    local raiser = require("raisers.audit_poll")
    local contract = core.producer_liveness_contracts()[1]
    t.eq(raiser.interval, core.audit_poll_interval())
    t.eq(raiser.produces, contract.trigger_source)
    t.eq(raiser.interval, string.format("%dm", contract.max_silence_seconds / 60))
    t.is_true(contract.max_silence_seconds < contract.max_staleness_seconds)
  end,

  test_audit_due_force_at_leads_raw_deadline_by_completion_budget = function()
    local staleness = core.audit_due_staleness_seconds()
    local completion_budget = core.audit_due_completion_budget_seconds()
    t.eq(completion_budget, 2 * core.audit_poll_interval_seconds() + 15 * 60)
    t.eq(core.audit_due_force_at_seconds(staleness, completion_budget), staleness - completion_budget)
    t.is_true(core.audit_due_force_at_seconds() < staleness)
  end,

  test_parse_findings_accepts_strict_array = function()
    local parsed = core.parse_findings_json(finding_json)
    t.eq(#parsed, 1)
    t.eq(parsed[1].file, "packages/idle-detector/core.lua")
    t.eq(parsed[1].line, 1)
    t.eq(parsed[1].rule, "SRP")
  end,

  test_parse_findings_rejects_non_json_and_extra_shape = function()
    t.raises(function() core.parse_findings_json("not json") end)
    t.raises(function() core.parse_findings_json("[{]") end)
    t.raises(function() core.parse_findings_json('{"file":"x"}') end)
    t.raises(function() core.parse_findings_json('"scalar"') end)
    t.raises(function() core.parse_findings_json("42") end)
    t.raises(function() core.parse_findings_json('[{"file":"x","line":"bad","rule":"SRP","why":"w","suggested_fix":"f"}]') end)
  end,

  test_parse_findings_reports_malformed_json_decode_error = function()
    local ok, err = pcall(function()
      core.parse_findings_json("[{]")
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("archaudit: malformed-json: codex output is malformed JSON", 1, true) ~= nil)
  end,

  test_parse_findings_reports_non_array_json_for_keyed_table = function()
    local previous_json = json
    json = {
      decode = function(_stdout)
        return { keyed = { file = "packages/archaudit/core.lua", line = 1, rule = "SRP", why = "Why.", suggested_fix = "Fix." } }
      end,
    }
    local ok, err = pcall(function()
      core.parse_findings_json("[]")
    end)
    json = previous_json
    t.eq(ok, false)
    t.is_true(tostring(err):find("archaudit: non-array-json: codex output is not a JSON array", 1, true) ~= nil)
  end,

  test_parse_findings_rejects_sparse_or_keyed_arrays = function()
    t.raises(function() core.parse_findings_json('{"1":{"file":"x"}}') end)
    t.raises(function()
      local previous_json = json
      json = {
        decode = function(_stdout)
          local sparse = {}
          sparse[1] = { file = "packages/archaudit/core.lua", line = 1, rule = "SRP", why = "Why.", suggested_fix = "Fix." }
          sparse[3] = { file = "packages/archaudit/core.lua", line = 1, rule = "DIP", why = "Why.", suggested_fix = "Fix." }
          return sparse
        end,
      }
      local ok, err = pcall(core.parse_findings_json, "[]")
      json = previous_json
      if not ok then
        error(err, 0)
      end
    end)
  end,

  test_parse_findings_rejects_decoder_returning_scalar_or_sparse_table = function()
    local previous_json = json
    json = {
      decode = function(_stdout)
        return "not a table"
      end,
    }
    local ok_scalar, err_scalar = pcall(function()
      t.raises(function() core.parse_findings_json("[]") end)
    end)
    json = previous_json
    if not ok_scalar then
      error(err_scalar, 0)
    end

    previous_json = json
    json = {
      decode = function(_stdout)
        local sparse = {}
        sparse[1] = { file = "packages/archaudit/core.lua", line = 1, rule = "SRP", why = "Why.", suggested_fix = "Fix." }
        sparse[3] = { file = "packages/archaudit/core.lua", line = 1, rule = "DIP", why = "Why.", suggested_fix = "Fix." }
        return sparse
      end,
    }
    local ok_sparse, err_sparse = pcall(function()
      t.raises(function() core.parse_findings_json("[]") end)
    end)
    json = previous_json
    if not ok_sparse then
      error(err_sparse, 0)
    end
  end,

  test_parse_findings_accepts_legitimate_empty_array = function()
    local parsed = core.parse_findings_json("[]")
    t.eq(#parsed, 0)
  end,

  test_validate_finding_is_pure_shape_validation = function()
    local finding = core.parse_findings_json(finding_json)[1]
    t.eq(core.validate_finding_shape(finding), true)
    t.eq(core.validate_finding(finding), true)
    finding.line = 999999
    t.eq(core.validate_finding(finding), true)
    t.eq(core.validate_finding("not a finding"), false)
    local previous_read = file.read
    file.read = function(_path)
      error("ambient file.read should not be used by core validation")
    end
    local ok, err = pcall(function()
      t.eq(core.validate_finding(finding), true)
    end)
    file.read = previous_read
    if not ok then
      error(err, 0)
    end
  end,

  test_finding_line_exists_checks_provided_text = function()
    local finding = core.parse_findings_json(finding_json)[1]
    finding.line = 2
    t.eq(core.finding_line_exists(finding, "first\nsecond"), true)
    finding.line = 3
    t.eq(core.finding_line_exists(finding, "first\nsecond"), false)
    t.eq(core.finding_line_exists(finding, ""), false)
    t.eq(core.finding_line_exists({ file = finding.file, line = 1 }, "first"), false)
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
    local payload = core.build_issue_create_request("owner/repo", finding, true, "idle")
    t.eq(payload.schema, "github-proxy.issue-create.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.title, "Archaudit: packages/idle-detector/core.lua:1 SRP")
    t.eq(payload.labels[1], "archaudit")
    t.eq(payload.source_ref.kind, "repo-site")
    t.eq(payload.source_ref.ref, "owner/repo#packages/idle-detector/core.lua:1#archaudit-create-intent")
    t.is_true(payload.body:find("archaudit-dedup: " .. payload.dedup_key, 1, true) ~= nil)
    t.is_true(payload.body:find("Audit trigger: idle", 1, true) ~= nil)
    t.is_true(payload.body:find('fkst:archaudit:audit-run:v1 reason="idle"', 1, true) ~= nil)
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

  test_issue_request_sanitizes_marker_unsafe_dedup_seed = function()
    local payload = core.build_issue_create_request("owner/repo", {
      file = "packages/archaudit/core.lua",
      line = 1,
      rule = "SRP\nunsafe",
      why = "Concrete issue.",
      suggested_fix = "Small fix.",
    }, true)
    t.is_true(payload.dedup_key:find("\n", 1, true) == nil)
    t.is_true(payload.body:find("archaudit-dedup: " .. payload.dedup_key, 1, true) ~= nil)
  end,

  test_issue_request_omits_missing_label = function()
    local finding = core.parse_findings_json(finding_json)[1]
    local payload = core.build_issue_create_request("owner/repo", finding, false)
    t.eq(#payload.labels, 0)
  end,

  test_zero_finding_audit_run_request_carries_durable_marker = function()
    local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z")
    local payload = core.build_audit_run_issue_create_request("owner/repo", "stale", true, now_seconds, core.audit_due_staleness_seconds())
    t.eq(payload.schema, "github-proxy.issue-create.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.title, "Archaudit: audit completed with zero findings")
    t.eq(payload.labels[1], "archaudit")
    t.is_true(payload.dedup_key:find("archaudit-run/owner/repo/", 1, true) == 1)
    t.is_true(payload.body:find("Architecture audit completed with zero findings.", 1, true) ~= nil)
    t.is_true(payload.body:find("Audit trigger: stale", 1, true) ~= nil)
    t.is_true(payload.body:find('fkst:archaudit:audit-run:v1 reason="stale"', 1, true) ~= nil)
  end,

  test_audit_run_dedup_key_is_one_per_staleness_window = function()
    local staleness = core.audit_due_staleness_seconds()
    local first = core.iso_timestamp_epoch_seconds("2026-06-19T01:00:00Z")
    local same_window = core.iso_timestamp_epoch_seconds("2026-06-19T23:59:59Z")
    local next_window = core.iso_timestamp_epoch_seconds("2026-06-20T00:00:00Z")
    t.eq(core.audit_run_dedup_key("owner/repo", first, staleness), core.audit_run_dedup_key("owner/repo", same_window, staleness))
    t.is_true(core.audit_run_dedup_key("owner/repo", first, staleness) ~= core.audit_run_dedup_key("owner/repo", next_window, staleness))
    t.eq(core.audit_run_current_window_seen(first, same_window, staleness), true)
    t.eq(core.audit_run_current_window_seen(first, next_window, staleness), false)
  end,

  test_audit_tick_event_normalizes_real_namespaced_cron_payload = function()
    local trigger = core.normalize_audit_tick_event({
      queue = "archaudit.archaudit_tick",
      ts = 1782003600000,
      payload = { raiser = "archaudit.audit_poll" },
    })
    t.eq(trigger.reason, "stale")
    t.eq(trigger.slot, "1782003600000")
    t.eq(trigger.source_ref.kind, "cron")
    t.eq(trigger.source_ref.ref, "audit_poll/slot/1782003600000")

    local explicit_slot = core.normalize_audit_tick_event({
      queue = "archaudit.archaudit_tick",
      ts = 1782003600000,
      payload = {
        raiser = "archaudit.audit_poll",
        slot = "slot-value",
        cron_slot = "cron-slot-value",
        detected_at = "2026-06-20T01:00:00Z",
      },
    })
    t.eq(explicit_slot.slot, "slot-value")

    t.eq(core.normalize_audit_tick_event({
      queue = "archaudit.archaudit_tick",
      payload = { raiser = "archaudit.audit_poll", cron_slot = "cron-slot-value" },
    }).slot, "cron-slot-value")

    t.eq(core.normalize_audit_tick_event({
      queue = "archaudit.archaudit_tick",
      payload = { raiser = "archaudit.audit_poll", detected_at = "2026-06-20T01:00:00Z" },
    }).slot, "2026-06-20T01:00:00Z")

    t.eq(core.normalize_audit_tick_event({
      queue = "archaudit_tick",
      payload = { raiser = "audit_poll", cron_slot = "flat-slot" },
    }).slot, "flat-slot")

    t.eq(core.normalize_audit_tick_event({
      queue = "archaudit.archaudit_tick",
      payload = { raiser = "other", slot = "2026-06-20T01:00:00Z" },
    }), nil)
  end,

  test_audit_search_parses_and_trusts_bot_authored_marker_issues = function()
    local issues = core.parse_audit_issue_search('[{"number":7,"title":"Archaudit: x","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"idle\\" -->","createdAt":"2026-06-20T00:00:00Z","author":{"login":"fkst-test-bot"}},{"number":8,"body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","updatedAt":"2026-06-20T01:00:00Z","author":{"login":"human"}}]')
    t.eq(#issues, 2)
    t.eq(core.latest_audit_issue_seconds(issues, "fkst-test-bot"), core.iso_timestamp_epoch_seconds("2026-06-20T00:00:00Z"))
    t.eq(core.latest_audit_issue_seconds(issues, "human"), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(core.latest_audit_issue_seconds(issues, ""), nil)
  end,

  test_audit_due_verdict_uses_durable_marker_window = function()
    local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z")
    local staleness = core.audit_due_staleness_seconds()
    local completion_budget = core.audit_due_completion_budget_seconds()
    local force_at = core.audit_due_force_at_seconds(staleness, completion_budget)
    local issues = core.parse_audit_issue_search('[{"number":7,"body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"idle\\" -->","createdAt":"2026-06-20T00:30:00Z","author":{"login":"fkst-test-bot"}}]')
    local due, why = core.audit_due_verdict(issues, "fkst-test-bot", now_seconds, staleness, completion_budget)
    t.eq(due, false)
    t.eq(why, "recent audit issue marker")

    issues = core.parse_audit_issue_search('[{"number":7,"body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T02:15:00Z","author":{"login":"fkst-test-bot"}}]')
    due, why = core.audit_due_verdict(issues, "fkst-test-bot", now_seconds, staleness, completion_budget)
    t.eq(due, true)
    t.eq(why, "audit completion budget threshold elapsed")
    t.is_true(now_seconds - core.latest_audit_issue_seconds(issues, "fkst-test-bot") >= force_at)
    t.is_true(now_seconds - core.latest_audit_issue_seconds(issues, "fkst-test-bot") < staleness)

    issues = core.parse_audit_issue_search('[{"number":7,"body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T02:17:00Z","author":{"login":"fkst-test-bot"}}]')
    due, why = core.audit_due_verdict(issues, "fkst-test-bot", now_seconds, staleness, completion_budget)
    t.eq(due, false)
    t.eq(why, "recent audit issue marker")
    t.is_true(now_seconds - core.latest_audit_issue_seconds(issues, "fkst-test-bot") < force_at)

    issues = core.parse_audit_issue_search('[{"number":7,"body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-18T00:30:00Z","author":{"login":"fkst-test-bot"}}]')
    due, why = core.audit_due_verdict(issues, "fkst-test-bot", now_seconds, staleness, completion_budget)
    t.eq(due, true)
    t.eq(why, "audit max staleness elapsed")

    due, why = core.audit_due_verdict({}, "fkst-test-bot", now_seconds, staleness, completion_budget)
    t.eq(due, true)
    t.eq(why, "no durable audit issue marker")
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
    t.raises(function() core.idle_hint_freshness(detected, "not-number", detected, 600) end)
  end,

  test_iso_timestamp_parser_covers_invalid_and_january_dates = function()
    t.eq(core.iso_timestamp_epoch_seconds("not-a-time"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-13-01T00:00:00Z"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-01-01T00:00:00Z"), 1767225600)
  end,

  test_prompt_includes_strict_object_schema = function()
    local prompt = core.build_prompt("owner/repo", 2)
    t.is_true(prompt:find('Object schema: {"file":"packages/example/core.lua","line":42,"rule":"SRP"', 1, true) ~= nil)
  end,

  test_observe_validation_rejects_non_table_top_level = function()
    t.raises(function() core.validate_observe_facts("not facts") end)
  end,

  test_observe_predicate_accepts_real_idle_and_uses_generated_time = function()
    local idle, why = core.is_idle_observe(observe_idle())
    t.eq(idle, true)
    t.is_nil(why)
    t.eq(core.observe_now_seconds(observe_idle()), 1781830860)
  end,

  test_observe_predicate_fails_closed_on_missing_each_real_busy_dimension = function()
    for _, field in ipairs({ "depth", "pending", "in_flight", "retrying" }) do
      local facts = observe_idle()
      facts.queues[1][field] = nil
      t.raises(function() core.is_idle_observe(facts) end)
    end
  end,

  test_observe_predicate_fails_closed_on_non_dense_observe_lists = function()
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

  test_observe_predicate_fails_closed_on_malformed_real_shape = function()
    for _, mutate in ipairs({
      function(facts) facts.schema_version = nil end,
      function(facts) facts.schema_version = 2 end,
      function(facts) facts.generated_at_ms = "1781830860000" end,
      function(facts) facts.source = nil end,
      function(facts) facts.source = "bad" end,
      function(facts) facts.limits = nil end,
      function(facts) facts.limits.max_deliveries = 1.5 end,
      function(facts) facts.limits.max_dead_letters = "500" end,
      function(facts) facts.truncated = nil end,
      function(facts) facts.truncated.deliveries = "false" end,
      function(facts) facts.truncated.dead_letters = 0 end,
      function(facts) facts.queues[1] = "bad" end,
      function(facts) facts.queues[1].queue = "" end,
      function(facts) facts.queues[1].pending = -1 end,
    }) do
      local facts = observe_idle()
      mutate(facts)
      t.raises(function() core.is_idle_observe(facts) end)
    end
  end,

  test_observe_predicate_rejects_real_busy_dimensions_and_lists = function()
    for _, field in ipairs({ "depth", "pending", "in_flight", "retrying" }) do
      local facts = observe_idle()
      facts.queues[1][field] = 1
      local idle, why = core.is_idle_observe(facts)
      t.eq(idle, false)
      t.is_true(why:find(field, 1, true) ~= nil)
    end

    local facts = observe_idle()
    facts.deliveries = { { delivery_id = "d1", queue = "proposal", dept = "decide", status = "pending", attempt = 1 } }
    local idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("deliveries=1", 1, true) ~= nil)

    facts = observe_idle()
    facts.dead_letters = { { delivery_id = "dead", queue = "proposal", dept = "decide", attempts = 1, replayable = true, permanent = false } }
    idle, why = core.is_idle_observe(facts)
    t.eq(idle, false)
    t.is_true(why:find("dead_letters=1", 1, true) ~= nil)
  end,

  test_observe_predicate_rejects_truncated_observe_lists_as_not_idle = function()
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

  test_skip_fact_fields_are_pure_and_structured = function()
    for _, why in ipairs({
      "stale system_idle hint",
      "expired system_idle hint",
      "observe-unreadable: observe failed",
      "current observe busy queue=proposal pending=1",
      "current observe dead_letters=1",
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
