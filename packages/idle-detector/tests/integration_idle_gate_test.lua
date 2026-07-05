local testing = require("testkit.testing")
local github_fake = require("forge.github_fake")
local idle_gate = require("departments.idle_gate.main")
local t = fkst.test

local fixed_now = 1781830860

local function event(ts)
  local slot = ts or "2026-06-19T01:00:00Z"
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

local function issue(number, assignees, state)
  return {
    number = number,
    title = "issue " .. tostring(number),
    state = state or "OPEN",
    assignees = assignees or {},
  }
end

local function fake_department(seed, env_values, overrides)
  local model = github_fake.model(seed or {})
  local github = github_fake.new(model)
  for key, value in pairs(overrides or {}) do
    github[key] = value
  end
  local env = env_values or {
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot[bot]",
  }
  local dept = idle_gate.make_department({
    github = github,
    read_env = function(name)
      return env[name]
    end,
    now = function()
      return fixed_now
    end,
  })
  dept.model = model
  return dept
end

local function capture_warns(fn)
  local previous_warn = log.warn
  local warnings = {}
  log.warn = function(message)
    table.insert(warnings, tostring(message))
  end
  local ok, result = pcall(fn)
  log.warn = previous_warn
  if not ok then
    error(result, 0)
  end
  return result, warnings
end

local function run_with_logs(dept, evt)
  local result, warnings = capture_warns(function()
    return testing.run_fake(dept, evt or event())
  end)
  return result, warnings
end

return {
  test_idle_gate_raises_system_idle_when_no_open_issues_are_assigned_to_self = function()
    local dept = fake_department({
      issues = {
        ["owner/repo#issue/1"] = issue(1, { "other-bot" }),
        ["owner/repo#issue/2"] = issue(2, { "fkst-test-bot" }, "CLOSED"),
      },
    })

    local result = run_with_logs(dept)

    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "system_idle")
    t.eq(result.raises[1].payload.schema, "idle-detector.system-idle.v1")
    t.eq(result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
    t.eq(result.raises[1].payload.expires_at, "2026-06-19T01:10:00Z")
    t.eq(result.raises[1].payload.source_ref.kind, "github-assignee-query")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issues?state=open&assignee=fkst-test-bot")
  end,

  test_idle_gate_accepts_cron_slot_and_event_ts_fallbacks = function()
    local cron_event = event()
    cron_event.payload.slot = nil
    cron_event.payload.cron_slot = "2026-06-19T01:00:00Z"
    local cron_result = run_with_logs(fake_department(), cron_event)
    t.eq(cron_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")

    local ts_event = event()
    ts_event.payload.slot = nil
    ts_event.payload.cron_slot = nil
    ts_event.payload.detected_at = nil
    local ts_result = run_with_logs(fake_department(), ts_event)
    t.eq(ts_result.raises[1].payload.detected_at, "2026-06-19T01:00:00Z")
  end,

  test_idle_gate_skips_when_self_has_open_assigned_issue_and_logs_busy_count = function()
    local dept = fake_department({
      issues = {
        ["owner/repo#issue/42"] = issue(42, { "fkst-test-bot" }),
        ["owner/repo#issue/43"] = issue(43, { "other-bot" }),
      },
    })

    local result, warnings = run_with_logs(dept)

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(warnings[1]:find("busy self_assigned_open_issues=1", 1, true) ~= nil)
    t.is_true(warnings[1]:find("tag=SKIP", 1, true) ~= nil)
  end,

  test_idle_gate_query_failure_fails_closed_without_raising_idle = function()
    local dept = fake_department({}, nil, {
      issue_list_open_assigned = function()
        error("synthetic gh failure", 0)
      end,
    })

    local result, warnings = run_with_logs(dept)

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(warnings[1]:find("self-assigned issue query failed", 1, true) ~= nil)
  end,

  test_idle_gate_missing_bot_login_fails_closed_without_querying_github = function()
    local queried = false
    local dept = fake_department({}, {
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_BOT_LOGIN = "",
    }, {
      issue_list_open_assigned = function()
        queried = true
        return { stdout = "[]", stderr = "", exit_code = 0 }
      end,
    })

    local result, warnings = run_with_logs(dept)

    t.eq(#result.raises, 0)
    t.eq(queried, false)
    t.eq(#warnings, 1)
    t.is_true(warnings[1]:find("missing FKST_GITHUB_BOT_LOGIN", 1, true) ~= nil)
  end,

  test_idle_gate_does_not_use_observe_dead_letters_or_queues_as_busy_veto = function()
    local previous_observe = fkst.observe
    fkst.observe = function()
      return {
        schema_version = 1,
        generated_at_ms = fixed_now * 1000,
        source = {},
        limits = { max_deliveries = 500, max_dead_letters = 500 },
        truncated = { deliveries = false, dead_letters = false },
        queues = {
          { queue = "proposal", depth = 3, pending = 2, in_flight = 1, retrying = 0 },
        },
        deliveries = { { delivery_id = "d1" } },
        dead_letters = { { delivery_id = "dead" } },
      }
    end
    local ok, result = pcall(function()
      return run_with_logs(fake_department())
    end)
    fkst.observe = previous_observe
    if not ok then
      error(result, 0)
    end

    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "system_idle")
  end,

  test_idle_gate_skips_stale_and_malformed_slots_before_querying_github = function()
    for _, evt in ipairs({
      event("2026-06-19T00:40:00Z"),
      event("not-a-time"),
    }) do
      local queried = false
      local dept = fake_department({}, nil, {
        issue_list_open_assigned = function()
          queried = true
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
      })

      local result = run_with_logs(dept, evt)

      t.eq(#result.raises, 0)
      t.eq(queried, false)
    end
  end,
}
