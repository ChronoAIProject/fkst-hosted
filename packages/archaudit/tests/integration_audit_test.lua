local core = require("core")
local audit_main = require("departments.audit.main")
local env_lib = require("workflow.env")
local helper = require("tests.audit_test_helpers")
local t = fkst.test

local run_department_opts = helper.run_department_opts
local idle_event = helper.idle_event
local fresh_idle_event = helper.fresh_idle_event
local stale_idle_event = helper.stale_idle_event
local stale_tick_event = helper.stale_tick_event
local mock_env = helper.mock_env
local observe_facts = helper.observe_facts
local mock_observe = helper.mock_observe
local mock_idle_observe = helper.mock_idle_observe
local mock_busy_observe = helper.mock_busy_observe
local mock_stale_observe = helper.mock_stale_observe
local mock_idle_observe_at = helper.mock_idle_observe_at
local mock_codex_findings = helper.mock_codex_findings
local findings_json = helper.findings_json
local fake_audit_department = helper.fake_audit_department
local fake_audit_department_with_search = helper.fake_audit_department_with_search
local fake_audit_department_with_github = helper.fake_audit_department_with_github
local fake_audit_department_with_observe = helper.fake_audit_department_with_observe
local run_fake_at = helper.run_fake_at
local run_fake_failure_at = helper.run_fake_failure_at

local function with_core_patch(patches, fn)
  local originals = {}
  for key, value in pairs(patches) do
    originals[key] = core[key]
    core[key] = value
  end
  local ok, result = pcall(fn)
  for key, value in pairs(originals) do
    core[key] = value
  end
  if not ok then
    error(result, 0)
  end
  return result
end

local function with_observe_port_error(message, fn)
  local observe_port = audit_main.observe_port
  local original = observe_port.facts
  observe_port.facts = function()
    error(message)
  end
  local ok, result = pcall(fn)
  observe_port.facts = original
  if not ok then
    error(result, 0)
  end
  return result
end

return {
  test_read_env_command_rejects_invalid_env_name = function()
    local allowed = {
      FKST_GITHUB_REPO = true,
      FKST_GITHUB_BOT_LOGIN = true,
      ARCHAUDIT_MAX_ISSUES_PER_IDLE = true,
    }
    local function read_env_command(name)
      if not allowed[name] then
        error("archaudit: invalid-env-name: env name is not allowed")
      end
      return 'printf %s "$' .. name .. '"'
    end
    local read_env = env_lib.read_env(read_env_command)
    t.raises(function() read_env("NOT_ALLOWED", function() return { stdout = "bad", stderr = "", exit_code = 0 } end) end)
    t.eq(read_env("FKST_GITHUB_REPO", function(command)
      t.eq(command, 'printf %s "$FKST_GITHUB_REPO"')
      return { stdout = "owner/repo", stderr = "", exit_code = 0 }
    end), "owner/repo")
  end,

  test_fake_fresh_idle_codex_finding_raises_issue_create_request = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Core has one concrete issue.","suggested_fix":"Move the local helper."}]', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
    t.eq(result.raises[1].payload.schema, "github-proxy.issue-create.v1")
    t.eq(result.raises[1].payload.repo, "owner/repo")
    t.eq(#result.raises[1].payload.labels, 0)
    t.eq(result.raises[1].payload.source_ref.kind, "repo-site")
    t.is_true(result.raises[1].payload.body:find("archaudit-dedup: " .. result.raises[1].payload.dedup_key, 1, true) ~= nil)
  end,

  test_fake_audit_codex_uses_engine_default_timeout = function()
    local captured_timeout = nil
    local previous_spawn_codex_sync = spawn_codex_sync
    spawn_codex_sync = function(opts)
      captured_timeout = opts.timeout
      return { stdout = "[]", stderr = "", exit_code = 0 }
    end
    local ok, err = pcall(function()
      mock_env("owner/repo", "3")
      mock_idle_observe()
      local dept = fake_audit_department("[]")
      run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    spawn_codex_sync = previous_spawn_codex_sync
    if not ok then
      error(err, 0)
    end
    t.eq(captured_timeout, 3600)
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

  test_fake_honors_large_positive_max_issues_without_upper_clamp = function()
    mock_env("owner/repo", "50")
    mock_idle_observe()
    mock_codex_findings(findings_json(25), 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 25)
    t.eq(result.raises[1].payload.title, "Archaudit: packages/archaudit/core.lua:1 Rule1")
    t.eq(result.raises[25].payload.title, "Archaudit: packages/archaudit/core.lua:1 Rule25")
  end,

  test_fake_invalid_max_issues_values_default_to_three = function()
    for _, max_issues in ipairs({ "", "not-a-number", "0", "-1" }) do
      mock_env("owner/repo", max_issues)
      mock_idle_observe()
      mock_codex_findings(findings_json(4), 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 3)
    end
  end,

  test_fake_mixed_valid_plus_invalid_batch_is_all_or_nothing_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Valid issue.","suggested_fix":"Fix valid."},{"file":"packages/archaudit/core.lua","line":999999,"rule":"DIP","why":"Invalid line.","suggested_fix":"Fix invalid."}]', 0)
    local dept = fake_audit_department("[]")
    local event = fresh_idle_event()
    local result = run_fake_failure_at(dept, event, core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.is_true(tostring(result.failure.error):find("invalid file or line", 1, true) ~= nil)
    t.eq(table.concat({ dept.git_calls[1].ref, dept.git_calls[1].path, tostring(dept.git_calls[1].timeout) }, "|"), "HEAD|packages/archaudit/core.lua|30")
    t.eq(table.concat({ dept.git_calls[2].ref, dept.git_calls[2].path, tostring(dept.git_calls[2].timeout) }, "|"), "HEAD|packages/archaudit/core.lua|30")
    t.eq(#result.raises, 0)
  end,

  test_stale_idle_hint_skips_without_codex = function()
    mock_stale_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, stale_idle_event(), core.iso_timestamp_epoch_seconds("1970-01-01T00:00:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_expired_idle_hint_skips_without_codex = function()
    mock_observe(observe_facts({ generated_at_ms = core.iso_timestamp_epoch_seconds("2026-06-19T01:03:00Z") * 1000 }))
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, idle_event({
      detected_at = "2026-06-19T01:00:00Z",
      expires_at = "2026-06-19T01:02:00Z",
    }), core.iso_timestamp_epoch_seconds("2026-06-19T01:03:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_busy_skips_without_codex = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_stale_tick_runs_when_system_is_busy_and_no_durable_audit_exists = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete stale audit issue.","suggested_fix":"Small local fix."}]', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
    t.is_true(result.raises[1].payload.body:find("Audit trigger: stale", 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('fkst:archaudit:audit-run:v1 reason="stale"', 1, true) ~= nil)
    t.eq(#dept.search_calls, 1)
    t.eq(dept.search_calls[1].query, "fkst:archaudit:audit-run:v1")
  end,

  test_stale_tick_zero_findings_records_durable_audit_run_marker = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    mock_codex_findings("[]", 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
    t.eq(result.raises[1].payload.title, "Archaudit: audit completed with zero findings")
    t.is_true(result.raises[1].payload.dedup_key:find("archaudit-run/owner/repo/", 1, true) == 1)
    t.is_true(result.raises[1].payload.body:find("Architecture audit completed with zero findings.", 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('fkst:archaudit:audit-run:v1 reason="stale"', 1, true) ~= nil)
  end,

  test_stale_tick_malformed_payload_fails_before_durable_search_or_codex = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    local dept = fake_audit_department("[]")
    local event = stale_tick_event()
    event.payload.raiser = "wrong"
    local result = run_fake_failure_at(dept, event, core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 0)
    t.eq(#dept.search_calls, 0)
    t.is_true(tostring(result.failure.error):find("unknown archaudit_tick producer", 1, true) ~= nil)
  end,

  test_stale_tick_fails_closed_without_durable_search_port = function()
    mock_env("owner/repo", "3")
    mock_busy_observe()
    local dept = fake_audit_department_with_github({})
    local result = run_fake_failure_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("audit-search-failed", 1, true) ~= nil)
  end,

  test_stale_tick_is_bounded_by_recent_durable_audit_issue = function()
    local search_stdout = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-20T00:30:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_busy_observe()
    local dept = fake_audit_department_with_search(search_stdout, "[]")
    local result = run_fake_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 0)
    t.eq(#dept.search_calls, 1)
  end,

  test_idle_trigger_is_also_bounded_by_recent_durable_audit_issue = function()
    local search_stdout = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"idle\\" -->","createdAt":"2026-06-19T00:30:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_idle_observe()
    local dept = fake_audit_department_with_search(search_stdout, "[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_idle_trigger_can_run_early_once_per_staleness_window = function()
    local search_stdout = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"idle\\" -->","createdAt":"2026-06-19T23:30:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_idle_observe_at(1781917260000)
    mock_codex_findings("[]", 0)
    local dept = fake_audit_department_with_search(search_stdout, "[]")
    local result = run_fake_at(dept, idle_event({
      detected_at = "2026-06-20T01:00:00Z",
      expires_at = "2026-06-20T01:10:00Z",
    }), core.iso_timestamp_epoch_seconds("2026-06-20T01:01:00Z"))
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.title, "Archaudit: audit completed with zero findings")
    t.is_true(result.raises[1].payload.body:find('fkst:archaudit:audit-run:v1 reason="idle"', 1, true) ~= nil)
  end,

  test_stale_tick_not_overdue_can_run_early_only_when_idle = function()
    local search_stdout = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T02:00:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_idle_observe_at(1781917260000)
    mock_codex_findings("[]", 0)
    local dept = fake_audit_department_with_search(search_stdout, "[]")
    local result = run_fake_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:01:00Z"))
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.title, "Archaudit: audit completed with zero findings")
    t.is_true(result.raises[1].payload.body:find('fkst:archaudit:audit-run:v1 reason="stale"', 1, true) ~= nil)
  end,

  test_stale_tick_ignores_untrusted_durable_audit_issue = function()
    local search_stdout = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-20T00:30:00Z","author":{"login":"human"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_busy_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete stale audit issue.","suggested_fix":"Small local fix."}]', 0)
    local dept = fake_audit_department_with_search(search_stdout, "[]")
    local result = run_fake_at(dept, stale_tick_event(), core.iso_timestamp_epoch_seconds("2026-06-20T01:00:00Z"))
    t.eq(#result.raises, 1)
  end,

  test_fake_current_truncated_observe_skips_without_issue = function()
    for _, truncated_json in ipairs({
      { deliveries = true, dead_letters = false },
      { deliveries = false, dead_letters = true },
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_facts({ truncated = truncated_json }))
      local dept = fake_audit_department_with_search("[]", "[]")
      local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_missing_queues_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ omit_queues = true }))
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_unknown_schema_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ schema_version = 2, queues = {} }))
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_unreadable_skips_without_issue = function()
    mock_env("owner/repo", "3")
    local result = with_observe_port_error("archaudit: observe-unreadable: synthetic observe failure", function()
      local dept = fake_audit_department("[]")
      return run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
  end,

  test_fake_observe_port_unreadable_skips_without_issue = function()
    mock_env("owner/repo", "3")
    local dept = fake_audit_department_with_observe(function()
      error("archaudit: observe-unreadable: synthetic observe failure")
    end)
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_observe_durable_root_unresolved_is_loud_failure_not_terminal_skip = function()
    mock_env("owner/repo", "3")
    local result = with_observe_port_error("archaudit: observe-durable-root-unresolved: FKST_DURABLE_ROOT must be set for fkst.observe", function()
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-durable-root-unresolved", 1, true) ~= nil)
    t.is_true(tostring(result.failure.error):find("terminal-skip", 1, true) == nil)
  end,

  test_fake_current_observe_malformed_snapshot_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe("not facts")
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_observe_port_malformed_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    local dept = fake_audit_department_with_observe(function()
      error("archaudit: observe-malformed-json: synthetic malformed observe")
    end)
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_current_observe_mutates_between_observe_time_and_idle_checks = function()
    local original_observe_now_seconds = core.observe_now_seconds
    local original_is_idle_observe = core.is_idle_observe
    mock_env("owner/repo", "3")
    local facts = { schema_version = 1, generated_at_ms = 1781830860000 }
    core.observe_now_seconds = function(_facts)
      return 1781830860
    end
    core.is_idle_observe = function(_facts)
      error("archaudit: observe-malformed-facts: mutated after time")
    end
    local ok, result_or_err = pcall(function()
      local dept = fake_audit_department_with_observe(function()
        return facts
      end)
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.observe_now_seconds = original_observe_now_seconds
    core.is_idle_observe = original_is_idle_observe
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
  end,

  test_fake_current_observe_time_check_failure_is_structured_failure_no_issue = function()
    local original_observe_now_seconds = core.observe_now_seconds
    mock_env("owner/repo", "3")
    core.observe_now_seconds = function(_facts)
      error("archaudit: observe-malformed-facts: mutated before time")
    end
    local ok, result_or_err = pcall(function()
      local dept = fake_audit_department_with_observe(function()
        return { schema_version = 1, generated_at_ms = 1781830860000 }
      end)
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.observe_now_seconds = original_observe_now_seconds
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
  end,

  test_fake_observe_port_time_check_failure_is_structured_failure_no_issue = function()
    local result = with_core_patch({
      observe_now_seconds = function(_facts)
        error("archaudit: observe-malformed-facts: synthetic time failure")
      end,
    }, function()
      mock_env("owner/repo", "3")
      local dept = fake_audit_department_with_observe(function()
        return {
          schema_version = 1,
          generated_at_ms = 1781830860000,
          source = {},
          limits = { max_deliveries = 500, max_dead_letters = 500 },
          truncated = { deliveries = false, dead_letters = false },
          queues = {},
          deliveries = {},
          dead_letters = {},
        }
      end)
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_current_observe_idle_check_malformed_queue_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ queues = { { queue = "proposal", depth = "bad", pending = 0, in_flight = 0, retrying = 0 } } }))
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_current_observe_malformed_top_level_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ generated_at_ms = "1781830860000", queues = {} }))
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_missing_or_malformed_source_limits_truncated_is_structured_failure_no_issue = function()
    for _, snapshot in ipairs({
      observe_facts({ omit_source = true, queues = {} }),
      observe_facts({ omit_limits = true, queues = {} }),
      observe_facts({ omit_truncated = true, queues = {} }),
      observe_facts({ truncated = { deliveries = "false", dead_letters = false }, queues = {} }),
      observe_facts({ limits = "bad", queues = {} }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(snapshot)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_malformed_dead_letter_truncated_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ truncated = { deliveries = false, dead_letters = 0 }, queues = {} }))
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_keyed_lists_are_structured_failure_no_issue = function()
    for _, snapshot in ipairs({
      observe_facts({ queues = { proposal = { depth = 0, pending = 0, in_flight = 0, retrying = 0 } } }),
      observe_facts({ deliveries = { one = {} } }),
      observe_facts({ dead_letters = { one = {} } }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(snapshot)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_missing_each_busy_dimension_is_structured_failure_no_issue = function()
    for _, snapshot in ipairs({
      observe_facts({ queues = { { queue = "proposal", pending = 0, in_flight = 0, retrying = 0 } } }),
      observe_facts({ queues = { { queue = "proposal", depth = 0, in_flight = 0, retrying = 0 } } }),
      observe_facts({ queues = { { queue = "proposal", depth = 0, pending = 0, retrying = 0 } } }),
      observe_facts({ queues = { { queue = "proposal", depth = 0, pending = 0, in_flight = 0 } } }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(snapshot)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_malformed_queue_rows_are_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ queues = { { queue = "", depth = 0, pending = 0, in_flight = 0, retrying = 0 } } }))
    local bad_name_dept = fake_audit_department("[]")
    local bad_name = run_fake_failure_at(bad_name_dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#bad_name.raises, 0)

    mock_env("owner/repo", "3")
    mock_observe(observe_facts({ queues = { { queue = "proposal", depth = 0, pending = -1, in_flight = 0, retrying = 0 } } }))
    local negative_dept = fake_audit_department("[]")
    local negative = run_fake_failure_at(negative_dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#negative.raises, 0)
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

  test_fake_codex_parser_classifies_malformed_non_array_and_validation_failures = function()
    for _, stdout in ipairs({
      "[{]",
      '[{"file":"packages/archaudit/core.lua","line":1,"rule":"","why":"Bad shape.","suggested_fix":"Fix."}]',
    }) do
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings(stdout, 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end

    local original_parse = core.parse_findings_json
    core.parse_findings_json = function(_stdout)
      error("archaudit: non-array-json: fake parser classification")
    end
    local ok, result_or_err = pcall(function()
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings("[]", 0)
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.parse_findings_json = original_parse
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
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

  test_fake_unclassified_codex_error_uses_parser_error_class_no_issue = function()
    local result = with_core_patch({
      parse_findings_json = function(_stdout)
        error("archaudit: validation-failure: synthetic parser fallback")
      end,
    }, function()
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings("[]", 0)
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("validation-failure", 1, true) ~= nil)
  end,

  test_fake_codex_validation_failure_is_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":999999,"rule":"SRP","why":"Bad line.","suggested_fix":"Fix."}]', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

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

  test_run_fake_label_probe_failures_raise_unlabeled_issue = function()
    for _, github in ipairs({
      {
        issue_search = function(_repo, _query, _fields, _timeout)
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
      },
      {
        issue_search = function(_repo, _query, _fields, _timeout)
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
        label_list = function(_repo, _timeout)
          return { stdout = "[]", stderr = "no labels", exit_code = 1 }
        end,
      },
      {
        issue_search = function(_repo, _query, _fields, _timeout)
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
        label_list = function(_repo, _timeout)
          return { stdout = "{not json", stderr = "", exit_code = 0 }
        end,
      },
      {
        issue_search = function(_repo, _query, _fields, _timeout)
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
        label_list = function(_repo, _timeout)
          return { stdout = '"not labels"', stderr = "", exit_code = 0 }
        end,
      },
    }) do
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete issue.","suggested_fix":"Small local fix."}]', 0)
      local dept = fake_audit_department_with_github(github)
      local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 1)
      t.eq(#result.raises[1].payload.labels, 0)
    end
  end,

  test_request_build_failure_is_structured_validation_failure_no_issue = function()
    local original_build_issue_create_request = core.build_issue_create_request
    core.build_issue_create_request = function(_repo, _finding, _label_available)
      error("archaudit: invalid-issue-create-field: fake request")
    end
    local ok, result_or_err = pcall(function()
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete issue.","suggested_fix":"Small local fix."}]', 0)
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.build_issue_create_request = original_build_issue_create_request
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
  end,

  test_fake_request_build_failure_is_structured_validation_failure_no_issue = function()
    local result = with_core_patch({
      build_issue_create_request = function(_repo, _finding, _label_available)
        error("archaudit: invalid-issue-create-field: synthetic request")
      end,
    }, function()
      mock_env("owner/repo", "3")
      mock_idle_observe()
      mock_codex_findings('[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Concrete issue.","suggested_fix":"Small local fix."}]', 0)
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("validation-failure", 1, true) ~= nil)
  end,

  test_unknown_queue_and_schema_are_structured_failures_no_issue = function()
    local bad_queue = t.run_department("departments/audit/main.lua", {
      queue = "foreign_queue",
      payload = {
        schema = "idle-detector.system-idle.v1",
        source_ref = { kind = "host-observe", ref = "idle_tick/foreign" },
      },
    }, run_department_opts())
    t.eq(bad_queue.exit_code, 1)
    t.eq(#bad_queue.raises, 0)

    local bad_schema = t.run_department("departments/audit/main.lua", idle_event({
      schema = "idle-detector.system-idle.v2",
    }), run_department_opts())
    t.eq(bad_schema.exit_code, 1)
    t.eq(#bad_schema.raises, 0)
  end,

  test_malformed_detected_at_is_structured_failure_no_issue = function()
    mock_idle_observe()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      detected_at = "not-a-time",
    }), run_department_opts())
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_malformed_expires_at_is_structured_failure_no_issue = function()
    mock_idle_observe()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      expires_at = "not-a-time",
    }), run_department_opts())
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,
}
