local h = require("tests.devloop_ops_core_helpers")
local core = h.core
local t = h.t

local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function authoritative_transition_index()
  local rows = {}
  local body = file.read("packages/github-devloop/core/restart/transitions/index.lua")
  for module, key in body:gmatch('{%s*module%s*=%s*"([^"]+)"%s*,%s*key%s*=%s*"([^"]+)"%s*}') do
    rows[#rows + 1] = { module = module, key = key }
  end
  return rows
end

local authoritative_transition_modules = {}
local authoritative_transition_count = 0
for _, row in ipairs(authoritative_transition_index()) do
  authoritative_transition_count = authoritative_transition_count + 1
  authoritative_transition_modules[row.key] =
    "packages/github-devloop/core/restart/transitions/" .. row.module .. ".lua"
end

local function parse_minutes_expression(expr)
  if expr == nil then
    return nil
  end
  local product = 1
  local found = false
  for number in tostring(expr):gmatch("%d+") do
    product = product * tonumber(number)
    found = true
  end
  if not found then
    return nil
  end
  return product
end

local function authoritative_lifecycle_row(state)
  local body = file.read(assert(authoritative_transition_modules[state], "missing authoritative transition path"))
  local decompose_queue_name = body:match("local%s+decompose_queue%s*=%s*M%.decompose_package_queue%(%)") ~= nil
  local driving_queue = body:match('driving_queue%s*=%s*"([^"]+)"')
  if driving_queue == nil and decompose_queue_name then
    driving_queue = "github-devloop-decompose.devloop_decompose"
  end
  return {
    from_state = body:match('from_state%s*=%s*"([^"]+)"'),
    terminal = body:match("terminal%s*=%s*(%a+)") == "true",
    driving_queue = driving_queue,
    budget_minutes = parse_minutes_expression(body:match("budget%s*=%s*budget%(([%d%s%*]+),")),
  }
end

local function bot_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T01:02:03Z",
  }
end

local function entity(labels, comments, extra)
  local value = {
    kind = "issue",
    repo = "owner/repo",
    number = 42,
    proposal_id = proposal_id,
    labels = labels or { "fkst-dev:enabled" },
    comments = comments or {},
    open_state = "OPEN",
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  value.current_state = core.current_entity_state(value.comments, value.proposal_id)
  return value
end

local function state_comment(state, marker_version, created_at)
  return bot_comment(core.state_marker(proposal_id, state, marker_version or version), created_at)
end

local function classify(value, opts)
  local options = opts or {}
  options.now_seconds = options.now_seconds or core.iso_timestamp_epoch_seconds("2026-06-03T01:20:03Z")
  return core.saga_doctor_classify_entity(value, options)
end

return {
  test_core_doctor_classifies_ok_within_liveness_budget = function()
    local current = entity({ "fkst-dev:enabled", "fkst-dev:thinking" }, {
      state_comment("thinking", version, "2026-06-03T01:02:03Z"),
    })

    local result = classify(current)

    t.eq(result.verdict, "OK")
    t.eq(result.state, "thinking")
  end,

  test_core_doctor_lifecycle_rows_match_authoritative_restart_rows = function()
    local seen = 0
    for state, _ in pairs(authoritative_transition_modules) do
      seen = seen + 1
      local expected = authoritative_lifecycle_row(state)
      local actual = core.lifecycle_transition_row(state)
      t.is_true(actual ~= nil)
      t.eq(actual.from_state, expected.from_state)
      t.eq(actual.terminal, expected.terminal)
      t.eq(actual.driving_queue, expected.driving_queue)
      t.eq(actual.budget and tonumber(actual.budget.minutes) or nil, expected.budget_minutes)
    end
    t.eq(seen, authoritative_transition_count)
  end,

  test_core_doctor_classifies_stuck_past_budget = function()
    local current = entity({ "fkst-dev:enabled", "fkst-dev:thinking" }, {
      state_comment("thinking", version, "2026-06-02T22:00:03Z"),
    })

    local result = classify(current)

    t.eq(result.verdict, "STUCK")
    t.is_true(result.reason:find("exceeds", 1, true) ~= nil)
  end,

  test_core_doctor_classifies_seen_without_decision_for_enabled_open_issue = function()
    local current = entity({ "fkst-dev:enabled" }, {})

    local result = classify(current)

    t.eq(result.verdict, "SEEN-WITHOUT-DECISION")
  end,

  test_core_doctor_classifies_pr_open_orphan_when_linked_pr_absent = function()
    local current = entity({ "fkst-dev:enabled", "fkst-dev:pr-open" }, {
      state_comment("pr-open"),
      bot_comment(core.pr_link_marker(proposal_id, 7, "devloop/issue/owner/repo/42/v", version, "dev")),
    })
    t.eq(core.pr_link_fact(current.comments, proposal_id).pr_number, 7)

    local result = classify(current, {
      facts = {
        open_pr_numbers = {},
      },
    })

    t.eq(result.verdict, "ORPHANED")
    t.is_true(result.reason:find("PR #7", 1, true) ~= nil)
  end,

  test_core_doctor_classifies_blocked_orphan_when_decompose_children_absent = function()
    local current = entity({ "fkst-dev:enabled", "fkst-dev:blocked" }, {
      state_comment("blocked"),
      bot_comment(core.pr_link_marker(proposal_id, 7, "devloop/issue/owner/repo/42/v", version, "dev")),
      bot_comment(core.decomposed_marker(proposal_id, version, 7, 2)),
    })

    local result = classify(current, {
      facts = {
        decompose_children = {},
      },
    })

    t.eq(result.verdict, "ORPHANED")
    t.is_true(result.reason:find("2 decompose child", 1, true) ~= nil)
  end,

  test_core_doctor_department_is_read_only = function()
    local result = t.run_department("departments/doctor/main.lua", {
      queue = "devloop_doctor_tick",
      payload = {},
    }, {
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      },
      commands = {
        [core.gh_issue_list_observe_cmd("owner/repo")] = {
          stdout = "[[]]\n",
          stderr = "",
          exit_code = 0,
        },
        [core.gh_pr_list_observe_cmd("owner/repo")] = {
          stdout = "[[]]\n",
          stderr = "",
          exit_code = 0,
        },
      },
    })

    t.eq(#result.raises, 0)
    local calls = t.command_calls()
    for _, call in ipairs(calls) do
      local rendered = tostring(call.rendered or "")
      t.is_nil(rendered:find(" --method POST ", 1, true))
      t.is_nil(rendered:find(" --method PATCH ", 1, true))
      t.is_nil(rendered:find("gh issue comment", 1, true))
      t.is_nil(rendered:find("gh pr comment", 1, true))
    end
  end,
}
