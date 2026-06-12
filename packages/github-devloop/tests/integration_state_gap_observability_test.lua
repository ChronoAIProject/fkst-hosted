local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      FKST_GITHUB_WRITE = "",
      FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
      FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
    },
  }
end

local function run_observability()
  return t.run_department("departments/observability/main.lua", {
    queue = "devloop_observe_tick",
    payload = { schema = "github-devloop.observe-tick.v1" },
  }, opts("state-gap-observability"))
end

local function mock_env()
  for _ = 1, 8 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 8 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
  for _, name in ipairs({ "GH_TOKEN", "GITHUB_TOKEN" }) do
    t.mock_command('if [ -n "${' .. name .. ':-}" ]; then printf present; fi', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
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
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100'", {
    stdout = "[[" .. table.concat(rendered, ",") .. "]]\n",
    stderr = "",
    exit_code = 0,
  })
  for _, state in ipairs(core._state_order) do
    t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=" .. core.state_label(state):gsub(":", "%%3A") .. "&per_page=100'", {
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
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&per_page=100'", {
    stdout = "[[" .. table.concat(rendered, ",") .. "]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view(comments)
  t.mock_command("--json title,comments,state", {
    stdout = '{"title":"Observed issue","state":"OPEN","comments":[' .. table.concat(comments or {}, ",") .. "]}\n",
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

local function gap_logs()
  local logs = {}
  local old_log = log
  log = {
    info = function(message)
      table.insert(logs, tostring(message))
    end,
    warn = function(message)
      table.insert(logs, tostring(message))
    end,
    error = function(message)
      table.insert(logs, tostring(message))
    end,
  }

  local ok, err = pcall(function()
    local package_root = package.searchpath("tests.integration_state_gap_observability_test", package.path)
      :match("(.+)/tests/integration_state_gap_observability_test%.lua$")
    dofile(package_root .. "/departments/observability/main.lua")
    pipeline({
      queue = "devloop_observe_tick",
      payload = { schema = "github-devloop.observe-tick.v1" },
    })
  end)

  log = old_log
  if not ok then
    error(err)
  end
  return logs
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

return {
  test_logs_state_gap_edges_from_trusted_marker_stream = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "ready", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.state_marker(proposal_id, "blocked", "v1"), "mallory", "2026-06-03T01:01:00Z"),
      render_comment(core.state_marker(proposal_id, "implementing", "v1"), "fkst-test-bot", "2026-06-03T01:50:00Z"),
    })

    local logs = table.concat(gap_logs(), "\n")

    t.is_true(logs:find("tag=GAP_EDGE", 1, true) ~= nil)
    t.is_true(logs:find("proposal_id=" .. proposal_id, 1, true) ~= nil)
    t.is_true(logs:find("gap_edge=ready->implementing", 1, true) ~= nil)
    t.is_true(logs:find("gap_seconds=3000", 1, true) ~= nil)
    t.is_true(logs:find("budget_seconds=2700", 1, true) ~= nil)
    t.is_true(logs:find("budget_status=over-budget", 1, true) ~= nil)
    t.is_true(logs:find("wait_class=unattributed", 1, true) ~= nil)
    t.is_true(logs:find("ready->blocked", 1, true) == nil)
  end,

  test_attributes_dependency_gate_wait_from_trusted_marker_stream = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "ready", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.dependency_wait_marker(proposal_id, "v1", { 7 }), "fkst-test-bot", "2026-06-03T01:05:00Z"),
      render_comment(core.dependency_wait_marker(proposal_id, "v1", { 8 }), "mallory", "2026-06-03T01:06:00Z"),
      render_comment(core.state_marker(proposal_id, "implementing", "v1"), "fkst-test-bot", "2026-06-03T01:50:00Z"),
    })

    local logs = table.concat(gap_logs(), "\n")

    t.is_true(logs:find("gap_edge=ready->implementing", 1, true) ~= nil)
    t.is_true(logs:find("wait_class=dependency-gate", 1, true) ~= nil)
    t.is_true(logs:find("classes dependency-gate 1", 1, true) ~= nil)
  end,

  test_attributes_codex_runtime_from_work_card_marker = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "implementing", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.work_card_marker(proposal_id), "fkst-test-bot", "2026-06-03T01:02:00Z"),
      render_comment(core.state_marker(proposal_id, "pr-open", "v1"), "fkst-test-bot", "2026-06-03T01:20:00Z"),
    })

    local logs = table.concat(gap_logs(), "\n")

    t.is_true(logs:find("gap_edge=implementing->pr-open", 1, true) ~= nil)
    t.is_true(logs:find("wait_class=codex-runtime", 1, true) ~= nil)
    t.is_true(logs:find("classes codex-runtime 1", 1, true) ~= nil)
  end,

  test_dashboard_renders_p50_p95_max_and_worst_offenders = function()
    local proposal_42 = "github-devloop/issue/owner/repo/42"
    local proposal_43 = "github-devloop/issue/owner/repo/43"
    mock_env()
    mock_all_issue_lists({ 42, 43 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_42, "ready", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.state_marker(proposal_42, "implementing", "v1"), "fkst-test-bot", "2026-06-03T01:50:00Z"),
    })
    mock_issue_view({
      render_comment(core.state_marker(proposal_43, "ready", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.state_marker(proposal_43, "implementing", "v1"), "fkst-test-bot", "2026-06-03T01:10:00Z"),
    })

    local logs = table.concat(gap_logs(), "\n")

    t.is_true(logs:find("## State-gap latency", 1, true) ~= nil)
    t.is_true(logs:find("ready->implementing: count 2, P50 10m 0s, P95 50m 0s, max 50m 0s, budget 45m 0s, near 0, over 1, classes unattributed 2", 1, true) ~= nil)
    t.is_true(logs:find("worst #42 50m 0s, #43 10m 0s", 1, true) ~= nil)
  end,

  test_state_gap_stream_spans_issue_and_pr_marker_comments = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "pr-open", "v1"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42", "v1", "integration/dev"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
    })
    mock_pr_view({
      render_comment(core.pr_origin_marker(proposal_id, "42", "devloop-owner-repo-42", "v1", "integration/dev"), "fkst-test-bot", "2026-06-03T01:00:00Z"),
      render_comment(core.state_marker(proposal_id, "reviewing", "v1"), "fkst-test-bot", "2026-06-03T01:03:00Z"),
    })

    local logs = table.concat(gap_logs(), "\n")

    t.eq(count_calls("gh pr view"), 1)
    t.is_true(logs:find("gap_edge=pr-open->reviewing", 1, true) ~= nil)
    t.is_true(logs:find("gap_seconds=180", 1, true) ~= nil)
  end,
}
