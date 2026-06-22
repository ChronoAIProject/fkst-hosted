local testing = require("contract.testing")
local github_fake = require("std.github_fake")
local core = require("core")
local audit_main = require("departments.audit.main")
local env_lib = require("contract.env")
local t = fkst.test

local function opts(name, env)
  local base = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/archaudit/" .. tostring(name),
    FKST_DURABLE_ROOT = "/tmp/fkst-packages-test/archaudit/durable-" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    ARCHAUDIT_MAX_ISSUES_PER_IDLE = "3",
    FKST_GITHUB_WRITE = "",
  }
  for key, value in pairs(env or {}) do
    base[key] = value
  end
  return { env = base }
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

local function stale_idle_event()
  return idle_event({
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:20:00Z",
  })
end

local function mock_env(repo, max_issues)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = max_issues or "3", stderr = "", exit_code = 0 })
end

local function mock_idle_observe()
  t.mock_command('fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', {
    stdout = '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[{"queue":"proposal","depth":0,"pending":0,"in_flight":0,"retrying":0,"oldest_pending_age_ms":null}],"deliveries":[],"dead_letters":[]}',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_busy_observe()
  t.mock_command('fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', {
    stdout = '{"schema_version":1,"generated_at_ms":1781830860000,"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[{"queue":"proposal","depth":1,"pending":1,"in_flight":0,"retrying":0,"oldest_pending_age_ms":1000}],"deliveries":[],"dead_letters":[]}',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_observe(stdout, exit_code)
  t.mock_command('fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "observe failed",
    exit_code = exit_code or 0,
  })
end

local function observe_json(opts)
  opts = opts or {}
  local parts = {
    '{"schema_version":' .. tostring(opts.schema_version or 1),
    ',"generated_at_ms":' .. tostring(opts.generated_at_ms or 1781830860000),
  }
  if not opts.omit_source then
    table.insert(parts, ',"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"}')
  end
  if not opts.omit_limits then
    table.insert(parts, ',"limits":' .. (opts.limits_json or '{"max_deliveries":500,"max_dead_letters":500}'))
  end
  if not opts.omit_truncated then
    table.insert(parts, ',"truncated":' .. (opts.truncated_json or '{"deliveries":false,"dead_letters":false}'))
  end
  if not opts.omit_queues then
    table.insert(parts, ',"queues":' .. (opts.queues_json or '[{"queue":"proposal","depth":0,"pending":0,"in_flight":0,"retrying":0,"oldest_pending_age_ms":null}]'))
  end
  table.insert(parts, ',"deliveries":' .. (opts.deliveries_json or "[]"))
  table.insert(parts, ',"dead_letters":' .. (opts.dead_letters_json or "[]"))
  table.insert(parts, "}")
  return table.concat(parts, "")
end

local function mock_stale_observe()
  mock_observe(observe_json({ generated_at_ms = 1781831461000 }), 0)
end

local function mock_codex_findings(stdout, exit_code)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "codex timeout",
    exit_code = exit_code or 0,
  })
end

local function finding_json(rule, why)
  return '{"file":"packages/archaudit/core.lua","line":1,"rule":"' .. rule .. '","why":"' .. why .. '","suggested_fix":"Fix ' .. rule .. '."}'
end

local function findings_json(count)
  local rows = {}
  for index = 1, count do
    table.insert(rows, finding_json("Rule" .. tostring(index), "Issue " .. tostring(index) .. "."))
  end
  return "[" .. table.concat(rows, ",") .. "]"
end

local function fake_audit_department(label_stdout)
  local model = github_fake.model()
  local label_calls = {}
  local github = github_fake.new(model)
  function github.label_list(repo, timeout)
    table.insert(label_calls, { repo = repo, timeout = timeout })
    return { stdout = label_stdout or "[]", stderr = "", exit_code = 0 }
  end
  t.eq(type(audit_main.make_department), "function")
  local dept = audit_main.make_department({ github = github, git = nil })
  dept.model = model
  return dept, model, label_calls
end

local function fake_audit_department_with_github(github)
  t.eq(type(audit_main.make_department), "function")
  return audit_main.make_department({ github = github, git = nil })
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

return {
  test_read_env_command_rejects_invalid_env_name = function()
    local allowed = {
      FKST_GITHUB_REPO = true,
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
    t.eq(#result.raises, 0)
  end,

  test_stale_idle_hint_skips_without_codex = function()
    mock_stale_observe()
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, stale_idle_event(), core.iso_timestamp_epoch_seconds("1970-01-01T00:00:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_expired_idle_hint_skips_without_codex = function()
    mock_observe(observe_json({ generated_at_ms = core.iso_timestamp_epoch_seconds("2026-06-19T01:03:00Z") * 1000 }), 0)
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

  test_fake_current_truncated_observe_skips_without_issue = function()
    for _, truncated_json in ipairs({
      '{"deliveries":true,"dead_letters":false}',
      '{"deliveries":false,"dead_letters":true}',
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_json({ truncated_json = truncated_json }), 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_missing_queues_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_json({ omit_queues = true }), 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_unknown_schema_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_json({ schema_version = 2, queues_json = "[]" }), 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_unreadable_skips_without_issue = function()
    mock_env("owner/repo", "3")
    mock_observe("", 1)
    local dept = fake_audit_department("[]")
    local result = run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_core_observe_unreadable_skips_without_issue = function()
    local result = with_core_patch({
      observe = function()
        error("archaudit: observe-unreadable: synthetic observe failure")
      end,
    }, function()
      mock_env("owner/repo", "3")
      local dept = fake_audit_department("[]")
      return run_fake_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_malformed_json_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe("{not json", 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_core_observe_malformed_is_structured_failure_no_issue = function()
    local result = with_core_patch({
      observe = function()
        error("archaudit: observe-malformed-json: synthetic malformed observe")
      end,
    }, function()
      mock_env("owner/repo", "3")
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_current_observe_mutates_between_observe_time_and_idle_checks = function()
    local original_observe = core.observe
    local original_observe_now_seconds = core.observe_now_seconds
    local original_is_idle_observe = core.is_idle_observe
    mock_env("owner/repo", "3")
    local facts = { schema_version = 1, generated_at_ms = 1781830860000 }
    core.observe = function()
      return facts
    end
    core.observe_now_seconds = function(_facts)
      return 1781830860
    end
    core.is_idle_observe = function(_facts)
      error("archaudit: observe-malformed-facts: mutated after time")
    end
    local ok, result_or_err = pcall(function()
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.observe = original_observe
    core.observe_now_seconds = original_observe_now_seconds
    core.is_idle_observe = original_is_idle_observe
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
  end,

  test_fake_current_observe_time_check_failure_is_structured_failure_no_issue = function()
    local original_observe = core.observe
    local original_observe_now_seconds = core.observe_now_seconds
    mock_env("owner/repo", "3")
    core.observe = function()
      return { schema_version = 1, generated_at_ms = 1781830860000 }
    end
    core.observe_now_seconds = function(_facts)
      error("archaudit: observe-malformed-facts: mutated before time")
    end
    local ok, result_or_err = pcall(function()
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    core.observe = original_observe
    core.observe_now_seconds = original_observe_now_seconds
    if not ok then
      error(result_or_err, 0)
    end
    t.eq(#result_or_err.raises, 0)
  end,

  test_fake_core_observe_time_check_failure_is_structured_failure_no_issue = function()
    local result = with_core_patch({
      observe = function()
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
      end,
      observe_now_seconds = function(_facts)
        error("archaudit: observe-malformed-facts: synthetic time failure")
      end,
    }, function()
      mock_env("owner/repo", "3")
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_core_idle_check_failure_is_structured_failure_no_issue = function()
    local result = with_core_patch({
      observe = function()
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
      end,
      observe_now_seconds = function(_facts)
        return 1781830860
      end,
      is_idle_observe = function(_facts)
        error("archaudit: observe-malformed-facts: synthetic idle predicate failure")
      end,
    }, function()
      mock_env("owner/repo", "3")
      local dept = fake_audit_department("[]")
      return run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    end)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.failure.error):find("observe-malformed", 1, true) ~= nil)
  end,

  test_fake_current_observe_malformed_top_level_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe('{"schema_version":1,"generated_at_ms":"1781830860000","source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"},"limits":{"max_deliveries":500,"max_dead_letters":500},"truncated":{"deliveries":false,"dead_letters":false},"queues":[],"deliveries":[],"dead_letters":[]}', 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_missing_or_malformed_source_limits_truncated_is_structured_failure_no_issue = function()
    for _, observe_stdout in ipairs({
      observe_json({ omit_source = true, queues_json = "[]" }),
      observe_json({ omit_limits = true, queues_json = "[]" }),
      observe_json({ omit_truncated = true, queues_json = "[]" }),
      observe_json({ truncated_json = '{"deliveries":"false","dead_letters":false}', queues_json = "[]" }),
      observe_json({ limits_json = '{"max_deliveries":1.5,"max_dead_letters":500}', queues_json = "[]" }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_stdout, 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_malformed_dead_letter_truncated_is_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_json({ truncated_json = '{"deliveries":false,"dead_letters":0}', queues_json = "[]" }), 0)
    local dept = fake_audit_department("[]")
    local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#result.raises, 0)
  end,

  test_fake_current_observe_keyed_lists_are_structured_failure_no_issue = function()
    for _, observe_json in ipairs({
      observe_json({ queues_json = '{"proposal":{"depth":0,"pending":0,"in_flight":0,"retrying":0}}' }),
      observe_json({ deliveries_json = '{"one":{}}' }),
      observe_json({ dead_letters_json = '{"one":{}}' }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_json, 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_missing_each_busy_dimension_is_structured_failure_no_issue = function()
    for _, observe_json in ipairs({
      observe_json({ queues_json = '[{"queue":"proposal","pending":0,"in_flight":0,"retrying":0}]' }),
      observe_json({ queues_json = '[{"queue":"proposal","depth":0,"in_flight":0,"retrying":0}]' }),
      observe_json({ queues_json = '[{"queue":"proposal","depth":0,"pending":0,"retrying":0}]' }),
      observe_json({ queues_json = '[{"queue":"proposal","depth":0,"pending":0,"in_flight":0}]' }),
    }) do
      mock_env("owner/repo", "3")
      mock_observe(observe_json, 0)
      local dept = fake_audit_department("[]")
      local result = run_fake_failure_at(dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
      t.eq(#result.raises, 0)
    end
  end,

  test_fake_current_observe_malformed_queue_rows_are_structured_failure_no_issue = function()
    mock_env("owner/repo", "3")
    mock_observe(observe_json({ queues_json = '[{"queue":"","depth":0,"pending":0,"in_flight":0,"retrying":0}]' }), 0)
    local bad_name_dept = fake_audit_department("[]")
    local bad_name = run_fake_failure_at(bad_name_dept, fresh_idle_event(), core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"))
    t.eq(#bad_name.raises, 0)

    mock_env("owner/repo", "3")
    mock_observe(observe_json({ queues_json = '[{"queue":"proposal","depth":0,"pending":-1,"in_flight":0,"retrying":0}]' }), 0)
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
      {},
      {
        label_list = function(_repo, _timeout)
          return { stdout = "[]", stderr = "no labels", exit_code = 1 }
        end,
      },
      {
        label_list = function(_repo, _timeout)
          return { stdout = "{not json", stderr = "", exit_code = 0 }
        end,
      },
      {
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
    }, opts("unknown-queue"))
    t.eq(bad_queue.exit_code, 1)
    t.eq(#bad_queue.raises, 0)

    local bad_schema = t.run_department("departments/audit/main.lua", idle_event({
      schema = "idle-detector.system-idle.v2",
    }), opts("unknown-schema"))
    t.eq(bad_schema.exit_code, 1)
    t.eq(#bad_schema.raises, 0)
  end,

  test_malformed_detected_at_is_structured_failure_no_issue = function()
    mock_idle_observe()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      detected_at = "not-a-time",
    }), opts("malformed-detected-at"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_malformed_expires_at_is_structured_failure_no_issue = function()
    mock_idle_observe()
    local result = t.run_department("departments/audit/main.lua", idle_event({
      expires_at = "not-a-time",
    }), opts("malformed-expires-at"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,
}
