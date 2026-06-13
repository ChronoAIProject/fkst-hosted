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

local function run_observability(name)
  return t.run_department("departments/observability/main.lua", {
    queue = "devloop_observe_tick",
    payload = { schema = "github-devloop.observe-tick.v1" },
  }, opts(name or "queue-starvation"))
end

local function json_string(value)
  return tostring(value or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
end

local function render_comment(body, author, created_at)
  return string.format(
    '{"body":"%s","author":{"login":"%s"},"createdAt":"%s"}',
    json_string(body),
    json_string(author or "fkst-test-bot"),
    json_string(created_at or "2026-06-13T01:02:03Z")
  )
end

local function version_minutes_ago(minutes)
  return os.date("!%Y-%m-%dT%H-%M-%SZ", now() - (tonumber(minutes) or 0) * 60)
end

local function closed_at_minutes_ago(minutes)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now() - (tonumber(minutes) or 0) * 60)
end

local function mock_env()
  for _ = 1, 12 do
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
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_DEVLOOP_CONFLICT_LOG_CMD"', {
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

local function mock_observe_lists(issue_number)
  t.mock_command(core.gh_issue_list_observe_cmd("owner/repo", core._enabled_label, 1, true), {
    stdout = '[{"number":' .. tostring(issue_number or 42) .. ',"state":"open"}]\n',
    stderr = "",
    exit_code = 0,
  })
  for _, state in ipairs(core._state_order) do
    t.mock_command(core.gh_issue_list_observe_cmd("owner/repo", core.state_label(state), 1, true), {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command(core.gh_pr_list_observe_cmd("owner/repo", 1, true), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_queue_head(age_minutes)
  local proposal_id = "github-devloop/issue/owner/repo/42"
  t.mock_command("--json title,comments,state,stateReason", {
    stdout = '{"title":"Merge-ready head","state":"OPEN","comments":['
      .. render_comment(core.state_marker(proposal_id, "merge-ready", version_minutes_ago(age_minutes or 90)))
      .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function recent_closed_item(number, closed_at, labels)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, '{"name":"' .. json_string(label) .. '"}')
  end
  return '{"number":' .. tostring(number)
    .. ',"title":"Closed issue ' .. tostring(number)
    .. '","closedAt":"' .. json_string(closed_at)
    .. '","labels":[' .. table.concat(rendered_labels, ",") .. "]}";
end

local function mock_recent_closed(stdout, exit_code, stderr)
  t.mock_command(core.gh_issue_list_recent_closed_cmd("owner/repo", 30), {
    stdout = stdout or "[]\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_closed_merged_issue(number, closed_minutes_ago, trusted)
  local proposal_id = "github-devloop/issue/owner/repo/" .. tostring(number)
  mock_recent_closed("[" .. recent_closed_item(number, closed_at_minutes_ago(closed_minutes_ago), { core._merged_label }) .. "]\n")
  t.mock_command("--json title,comments,state,stateReason", {
    stdout = '{"title":"Merged issue","state":"CLOSED","comments":['
      .. render_comment(core.merged_marker(proposal_id, 9, "v1", "abcdef123456"), trusted == false and "mallory" or "fkst-test-bot")
      .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
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

local function prepare_stale_head()
  mock_env()
  mock_observe_lists(42)
  mock_queue_head(90)
end

return {
  test_recent_closed_parser_requires_merge_signal_fields = function()
    local parsed = core.parse_issue_list_recent_closed("[" .. recent_closed_item(77, "2026-06-13T01:02:03Z", { core._merged_label }) .. "]")
    t.eq(parsed[1].number, 77)
    t.eq(parsed[1].title, "Closed issue 77")
    t.eq(parsed[1].closed_at, "2026-06-13T01:02:03Z")
    t.eq(parsed[1].closedAt, "2026-06-13T01:02:03Z")
    t.eq(parsed[1].labels[1], core._merged_label)
    t.raises(function()
      core.parse_issue_list_recent_closed('[{"number":77,"title":"bad","labels":[]}]')
    end)
  end,

  test_queue_starvation_fires_for_stale_merge_ready_with_no_recent_merge = function()
    prepare_stale_head()
    mock_recent_closed("[]\n")

    local result = run_observability("queue-starvation-fire")

    t.eq(result.exit_code, 0)
    local create = find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    local payload = create.payload
    t.eq(payload.schema, "github-proxy.issue-create.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.dedup_key, core.queue_starvation_dedup_key("owner/repo", "merge-ready", core.queue_starvation_window_key(now())))
    t.eq(payload.parent_comment_target.issue_number, "42")
    t.is_true(payload.body:find("Queue head: #42 Merge-ready head", 1, true) ~= nil)
    t.is_true(payload.body:find("Evidence snapshot: `/tmp/fkst-github-devloop-queue-starvation-owner-repo-", 1, true) ~= nil)
    local snapshot = payload.body:match("Evidence snapshot: `([^`]+)`")
    t.is_true(snapshot ~= nil)
    local written = file.read(snapshot)
    t.is_true(written:find('"detector":"queue-starvation"', 1, true) ~= nil)
    t.is_true(written:find('"age_minutes":90', 1, true) ~= nil)
  end,

  test_queue_starvation_suppresses_when_trusted_recent_merge_exists = function()
    prepare_stale_head()
    mock_closed_merged_issue(77, 30)

    local result = run_observability("queue-starvation-suppress")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,

  test_queue_starvation_fail_closed_when_recent_closed_command_fails = function()
    prepare_stale_head()
    mock_recent_closed("", 1, "gh failed")

    local result = run_observability("queue-starvation-fail-closed")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,

  test_queue_starvation_dedup_is_stable_for_detector_identity_and_window = function()
    local first = core.queue_starvation_dedup_key("owner/repo", "merge-ready", "window-123")
    local second = core.queue_starvation_dedup_key("owner/repo", "merge-ready", "window-123")
    local third = core.queue_starvation_dedup_key("owner/repo", "merge-ready", "window-124")

    t.eq(first, second)
    t.eq(first, "queue-starvation/owner/repo/merge-ready/window-123")
    t.is_true(first ~= third)
  end,

  test_queue_starvation_has_no_repair_side_effects = function()
    prepare_stale_head()
    mock_recent_closed("[]\n")

    local result = run_observability("queue-starvation-no-repair")

    t.eq(result.exit_code, 0)
    t.is_true(find_raise(result.raises, "github-proxy.github_issue_create_request") ~= nil)
    t.eq(count_calls("gh issue edit"), 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(count_calls("gh pr close"), 0)
    t.eq(count_calls("gh issue comment"), 0)
  end,
}
