local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function opts(name, extra)
  local env = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    FKST_GITHUB_WRITE = "",
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return { env = env }
end

local function run_observability(run_opts)
  return t.run_department("departments/observability/main.lua", {
    queue = "devloop_observe_tick",
    payload = { schema = "github-devloop.observe-tick.v1" },
  }, run_opts or opts("observability"))
end

local function mock_env(bot_login)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = bot_login == nil and "fkst-test-bot" or bot_login,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function json_string(value)
  return tostring(value or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
end

local function render_comment(body, author, created_at)
  return string.format(
    '{"body":"%s","author":{"login":"%s"},"createdAt":"%s"}',
    json_string(body),
    json_string(author or "fkst-test-bot"),
    json_string(created_at or "2026-06-03T01:02:03Z")
  )
end

local function mock_all_issue_lists(numbers)
  local rendered = {}
  for _, number in ipairs(numbers or {}) do
    table.insert(rendered, string.format('{"number":%d,"state":"open"}', number))
  end
  local stdout = "[[" .. table.concat(rendered, ",") .. "]]\n"
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=all&labels=fkst-dev%3Aenabled&per_page=100'", {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
  for _, state in ipairs(core._state_order) do
    t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=all&labels=" .. core.state_label(state):gsub(":", "%%3A") .. "&per_page=100'", {
      stdout = "[[]]\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_list(numbers)
  local rendered = {}
  for _, number in ipairs(numbers or {}) do
    table.insert(rendered, string.format('{"number":%d,"state":"open"}', number))
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=all&per_page=100'", {
    stdout = "[[" .. table.concat(rendered, ",") .. "]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view(comments)
  t.mock_command("--json comments,state", {
    stdout = '{"state":"OPEN","comments":[' .. table.concat(comments or {}, ",") .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_view(comments)
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
    stdout = '{"headRefName":"devloop-owner-repo-42","headRefOid":"def456","baseRefName":"integration/dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":['
      .. table.concat(comments or {}, ",") .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function has_call(needle)
  return count_calls(needle) > 0
end

local function package_root()
  local source = package.searchpath("tests.integration_observability_test", package.path)
  return source:match("(.+)/tests/integration_observability_test%.lua$")
end

local function capture_observability_logs(event)
  local captured = {}
  local old_log = log
  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }

  local ok, err = pcall(function()
    dofile(package_root() .. "/departments/observability/main.lua")
    pipeline(event or {
      queue = "devloop_observe_tick",
      payload = { schema = "github-devloop.observe-tick.v1" },
    })
  end)

  log = old_log
  if not ok then
    error(err)
  end
  return captured
end

local function summary_log(logs)
  for _, line in ipairs(logs or {}) do
    if line:find("tag=OBSERVE_SUMMARY", 1, true) ~= nil then
      return line
    end
  end
  return nil
end

local function call_contains_bad_limit()
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("observ", 1, true) == nil
      and (call.rendered:find("issues%?state=all", 1, false) ~= nil
        or call.rendered:find("pulls%?state=all", 1, false) ~= nil)
      and call.rendered:find("--limit 100", 1, true) ~= nil then
      return true
    end
  end
  return false
end

return {
  test_summary_logs_all_known_states_with_zero_defaults = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "ready", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })

    local summary = summary_log(capture_observability_logs())

    t.is_true(summary ~= nil)
    t.is_true(summary:find("total=1", 1, true) ~= nil)
    for _, state in ipairs(core._state_order) do
      local expected = state == "ready" and 1 or 0
      t.is_true(summary:find(state .. "=" .. tostring(expected), 1, true) ~= nil)
    end
    t.is_true(summary:find("unmanaged=", 1, true) == nil)
  end,

  test_logs_issue_phase_state_from_trusted_marker_and_ignores_forged_marker = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "blocked", "2099-01-01T00-00-00Z"), "mallory"),
      render_comment(core.state_marker(proposal_id, "ready", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })

    local result = run_observability()

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    local observed = core.observe_entity_log_line(proposal_id, {
      state = "ready",
      version = "2026-06-03T01-02-03Z",
      marker_source = "issue",
      marker_created_at = "2026-06-03T01:02:03Z",
    })
    t.is_true(observed:find("tag=OBSERVE_ENTITY", 1, true) ~= nil)
    t.is_true(observed:find("state=ready", 1, true) ~= nil)
    t.is_true(observed:find("marker_source=issue", 1, true) ~= nil)
  end,

  test_pr_phase_comment_stream_wins_over_stale_issue_pr_open = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local impl_version = "2026-06-03T01-02-03Z"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "pr-open", impl_version), "fkst-test-bot", "2026-06-03T01:02:03Z"),
      render_comment(core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42", impl_version, "integration/dev")),
    })
    mock_pr_view({
      render_comment(core.pr_origin_marker(proposal_id, "42", "devloop-owner-repo-42", impl_version, "integration/dev")),
      render_comment(core.state_marker(proposal_id, "reviewing", impl_version), "fkst-test-bot", "2026-06-03T02:03:04Z"),
    })

    local result = run_observability()

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    local observed = core.observe_entity_log_line(proposal_id, {
      state = "reviewing",
      version = impl_version,
      marker_source = "pr-comment",
      pr_number = 7,
      marker_created_at = "2026-06-03T02:03:04Z",
    })
    t.is_true(observed:find("state=reviewing", 1, true) ~= nil)
    t.is_true(observed:find("marker_source=pr-comment", 1, true) ~= nil)
    t.is_true(observed:find("pr=7", 1, true) ~= nil)
  end,

  test_pr_enumeration_reads_origin_fact_when_issue_side_is_absent = function()
    local proposal_id = "github-devloop/issue/owner/repo/43"
    mock_env()
    mock_all_issue_lists({})
    mock_pr_list({ 8 })
    mock_pr_view({
      render_comment(core.pr_origin_marker(proposal_id, "43", "devloop-owner-repo-43", "v1", "integration/dev")),
      render_comment(core.state_marker(proposal_id, "merge-ready", "v1"), "fkst-test-bot", "2026-06-03T03:03:04Z"),
    })

    local result = run_observability()

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr view"), 1)
  end,

  test_fail_closed_when_bot_login_is_unset = function()
    mock_env("")
    local result = run_observability(opts("observability-no-bot", { FKST_GITHUB_BOT_LOGIN = "" }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh api --paginate --slurp"), 0)
  end,

  test_enumeration_is_paginated_and_not_fixed_silent_100_cap = function()
    mock_env()
    mock_all_issue_lists({})
    mock_pr_list({})

    local result = run_observability()

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.is_true(has_call("gh api --paginate --slurp 'repos/owner/repo/issues?state=all&labels=fkst-dev%3Aenabled&per_page=100'"))
    t.is_true(has_call("gh api --paginate --slurp 'repos/owner/repo/pulls?state=all&per_page=100'"))
    t.eq(call_contains_bad_limit(), false)
  end,
}
