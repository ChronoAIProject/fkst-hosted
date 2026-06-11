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

local function mock_env(bot_login, write_mode)
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = bot_login == nil and "fkst-test-bot" or bot_login,
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = write_mode or "",
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

local function mock_all_issue_lists(items)
  local rendered = {}
  for _, item in ipairs(items or {}) do
    local number = type(item) == "table" and item.number or item
    local state = type(item) == "table" and item.state or "open"
    table.insert(rendered, string.format('{"number":%d,"state":"%s"}', number, json_string(state)))
  end
  local stdout = "[[" .. table.concat(rendered, ",") .. "]]\n"
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100'", {
    stdout = stdout,
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

local function mock_pr_list(items)
  local rendered = {}
  for _, item in ipairs(items or {}) do
    local number = type(item) == "table" and item.number or item
    local state = type(item) == "table" and item.state or "open"
    table.insert(rendered, string.format('{"number":%d,"state":"%s"}', number, json_string(state)))
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

local function first_call(needle)
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      return call.rendered
    end
  end
  return nil
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

local function try_capture_observability_logs(event)
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
  return ok, captured, err
end

local function summary_log(logs)
  for _, line in ipairs(logs or {}) do
    if line:find("tag=OBSERVE_SUMMARY", 1, true) ~= nil then
      return line
    end
  end
  return nil
end

local function stall_suspect_logs(logs)
  local matches = {}
  for _, line in ipairs(logs or {}) do
    if line:find("tag=STALL_SUSPECT", 1, true) ~= nil then
      table.insert(matches, line)
    end
  end
  return matches
end

local function version_minutes_ago(minutes)
  return os.date("!%Y-%m-%dT%H-%M-%SZ", now() - (tonumber(minutes) or 0) * 60)
end

local function call_contains_bad_limit()
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("observ", 1, true) == nil
      and (call.rendered:find("issues%?state=open", 1, false) ~= nil
        or call.rendered:find("pulls%?state=open", 1, false) ~= nil)
      and call.rendered:find("--limit 100", 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function dashboard_hash(body)
  return tostring(body or ""):match("<!%-%- fkst:dashboard:v1[^>]-hash=\"([^\"]+)\"[^>]*%-%->")
end

local function command_input_path(command)
  return tostring(command or ""):match("%-%-input '([^']+)'")
end

local function dashboard_issue_list_command()
  return "gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dashboard&per_page=100'"
end

local function dashboard_label_get_command()
  return "gh api --method GET 'repos/owner/repo/labels/fkst-dashboard'"
end

local function dashboard_label_create_command()
  return "gh api --method POST 'repos/owner/repo/labels' -f 'name=fkst-dashboard' -f 'color=ededed' -f 'description=fkst observability dashboard singleton'"
end

local function mock_dashboard_label_exists()
  t.mock_command(dashboard_label_get_command(), {
    stdout = '{"name":"fkst-dashboard"}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_dashboard_issue_list(stdout, exit_code, stderr)
  mock_dashboard_label_exists()
  t.mock_command(dashboard_issue_list_command(), {
    stdout = stdout or "[[]]\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
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

  test_observe_summary_counts_only_open_list_entities = function()
    local open_proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({
      { number = 42, state = "open" },
      { number = 43, state = "closed" },
    })
    mock_pr_list({
      { number = 8, state = "closed" },
    })
    mock_issue_view({
      render_comment(core.state_marker(open_proposal_id, "ready", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })

    local summary = summary_log(capture_observability_logs())

    t.is_true(summary ~= nil)
    t.is_true(summary:find("total=1", 1, true) ~= nil)
    t.is_true(summary:find("ready=1", 1, true) ~= nil)
    t.eq(count_calls("gh issue view"), 1)
    t.eq(count_calls("gh pr view"), 0)
    t.is_true(has_call("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100'"))
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

  test_stall_suspect_logs_once_when_entity_exceeds_state_threshold = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = version_minutes_ago(31)
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "thinking", version), "fkst-test-bot"),
    })

    local logs = stall_suspect_logs(capture_observability_logs())

    t.eq(#logs, 1)
    t.is_true(logs[1]:find("github-devloop", 1, true) ~= nil)
    t.is_true(logs[1]:find("dept=observability", 1, true) ~= nil)
    t.is_true(logs[1]:find("proposal=" .. proposal_id, 1, true) ~= nil)
    t.is_true(logs[1]:find("state=thinking", 1, true) ~= nil)
    t.is_true(logs[1]:find("threshold_minutes=30", 1, true) ~= nil)
  end,

  test_stall_suspect_does_not_log_under_threshold = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "reviewing", version_minutes_ago(60)), "fkst-test-bot"),
    })

    local logs = stall_suspect_logs(capture_observability_logs())

    t.eq(#logs, 0)
  end,

  test_stall_suspect_excludes_dependency_held_ready_entities = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = version_minutes_ago(31)
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "ready", version), "fkst-test-bot"),
      render_comment(core.dependency_wait_marker(proposal_id, version, { 7 }), "fkst-test-bot"),
    })

    local logs = stall_suspect_logs(capture_observability_logs())

    t.eq(#logs, 0)
  end,

  test_stall_suspect_never_logs_terminal_states = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "blocked", version_minutes_ago(1000)), "fkst-test-bot"),
    })

    local logs = stall_suspect_logs(capture_observability_logs())

    t.eq(#logs, 0)
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
    t.is_true(has_call("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100'"))
    t.is_true(has_call("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&per_page=100'"))
    t.eq(call_contains_bad_limit(), false)
  end,

  test_dashboard_dry_run_renders_board_without_github_write = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env()
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "ready", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })

    local logs = capture_observability_logs()
    local body = table.concat(logs, "\n")

    t.is_true(body:find("tag=DASHBOARD_DRY_RUN", 1, true) ~= nil)
    t.is_true(body:find("# fkst-dev board", 1, true) ~= nil)
    t.is_true(body:find("## Now working", 1, true) ~= nil)
    t.is_true(body:find("## Board by state", 1, true) ~= nil)
    t.is_true(body:find("#42 Observed issue - ready", 1, true) ~= nil)
    t.is_true(body:find("fkst:dashboard:v1", 1, true) ~= nil)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH"), 0)
  end,

  test_dashboard_write_creates_single_marker_issue_when_absent = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "implementing", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })
    mock_dashboard_issue_list()
    t.mock_command("gh api --method POST 'repos/owner/repo/issues' --input '/tmp/fkst-github-devloop-dashboard-owner-repo-", {
      stdout = '{"number":99}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observability(opts("observability-dashboard-create", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(dashboard_label_get_command()), 1)
    t.eq(count_calls("gh api --method POST 'repos/owner/repo/issues'"), 1)
    t.eq(count_calls("gh api --method PATCH"), 0)
    local input_path = command_input_path(first_call("gh api --method POST 'repos/owner/repo/issues'"))
    t.is_true(input_path ~= nil)
    t.is_true(input_path:find("/tmp/fkst-github-devloop-dashboard-owner-repo-", 1, true) == 1)
    t.is_true(input_path ~= "/tmp/fkst-github-devloop-dashboard-owner-repo.json")
    local written = file.read(input_path)
    t.is_true(written:find('"title":"fkst-dev board"', 1, true) ~= nil)
    t.is_true(written:find('"labels":["fkst-dashboard"]', 1, true) ~= nil)
    t.is_true(written:find("fkst:dashboard:v1", 1, true) ~= nil)
    t.is_true(written:find("implementing", 1, true) ~= nil)
    t.eq(count_calls("--search"), 0)
  end,

  test_dashboard_write_updates_existing_trusted_issue_when_hash_changes = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "reviewing", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })
    mock_dashboard_issue_list('[[{"number":99,"title":"fkst-dev board","user":{"login":"fkst-test-bot"},"body":"old\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"old\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}]]\n')
    t.mock_command("gh api --method GET --include 'repos/owner/repo/issues/99'", {
      stdout = 'HTTP/2.0 200 OK\netag: "dashboard-old-etag"\n\n{"number":99,"title":"fkst-dev board","author":{"login":"fkst-test-bot"},"body":"old\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"old\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api --method PATCH 'repos/owner/repo/issues/99' --header 'If-Match: \"dashboard-old-etag\"' --input '/tmp/fkst-github-devloop-dashboard-owner-repo-", {
      stdout = '{"number":99}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observability(opts("observability-dashboard-update", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH 'repos/owner/repo/issues/99' --header 'If-Match: \"dashboard-old-etag\"'"), 1)
    local input_path = command_input_path(first_call("gh api --method PATCH 'repos/owner/repo/issues/99' --header 'If-Match: \"dashboard-old-etag\"'"))
    t.is_true(input_path ~= nil)
    t.is_true(input_path:find("/tmp/fkst-github-devloop-dashboard-owner-repo-", 1, true) == 1)
    t.is_true(input_path ~= "/tmp/fkst-github-devloop-dashboard-owner-repo.json")
  end,

  test_dashboard_write_skips_update_when_version_cas_mismatches = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "reviewing", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })
    mock_dashboard_issue_list('[[{"number":99,"title":"fkst-dev board","user":{"login":"fkst-test-bot"},"body":"old\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"old\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}]]\n')
    t.mock_command("gh api --method GET --include 'repos/owner/repo/issues/99'", {
      stdout = 'HTTP/2.0 200 OK\netag: "dashboard-newer-etag"\n\n{"number":99,"title":"fkst-dev board","author":{"login":"fkst-test-bot"},"body":"newer\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:01:00Z\\" hash=\\"newer\\" generated_at=\\"2026-06-01T00:01:00Z\\" -->"}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observability(opts("observability-dashboard-cas-mismatch", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH"), 0)
    t.eq(count_calls(dashboard_issue_list_command()), 1)
    t.eq(count_calls("gh api --method GET --include 'repos/owner/repo/issues/99'"), 1)
  end,

  test_dashboard_write_bootstraps_missing_dashboard_label_before_create = function()
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({})
    mock_pr_list({})
    t.mock_command(dashboard_label_get_command(), {
      stdout = "",
      stderr = "HTTP 404: Not Found\n",
      exit_code = 1,
    })
    t.mock_command(dashboard_label_create_command(), {
      stdout = '{"name":"fkst-dashboard"}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(dashboard_issue_list_command(), {
      stdout = "[[]]\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api --method POST 'repos/owner/repo/issues' --input '/tmp/fkst-github-devloop-dashboard-owner-repo-", {
      stdout = '{"number":99}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observability(opts("observability-dashboard-label-bootstrap", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(dashboard_label_get_command()), 1)
    t.eq(count_calls(dashboard_label_create_command()), 1)
    t.eq(count_calls("gh api --method POST 'repos/owner/repo/issues'"), 1)
  end,

  test_dashboard_write_skips_update_when_etag_precondition_fails = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "reviewing", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })
    mock_dashboard_issue_list('[[{"number":99,"title":"fkst-dev board","user":{"login":"fkst-test-bot"},"body":"old\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"old\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}]]\n')
    t.mock_command("gh api --method GET --include 'repos/owner/repo/issues/99'", {
      stdout = 'HTTP/2.0 200 OK\netag: "dashboard-old-etag"\n\n{"number":99,"title":"fkst-dev board","author":{"login":"fkst-test-bot"},"body":"old\\n<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"old\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api --method PATCH 'repos/owner/repo/issues/99' --header 'If-Match: \"dashboard-old-etag\"' --input '/tmp/fkst-github-devloop-dashboard-owner-repo-", {
      stdout = "",
      stderr = "HTTP 412: Precondition Failed\n",
      exit_code = 1,
    })

    local result = run_observability(opts("observability-dashboard-etag-precondition", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH 'repos/owner/repo/issues/99' --header 'If-Match: \"dashboard-old-etag\"'"), 1)
  end,

  test_dashboard_write_skips_stale_snapshot_when_current_version_is_newer = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({ 42 })
    mock_pr_list({})
    mock_issue_view({
      render_comment(core.state_marker(proposal_id, "reviewing", "2026-06-03T01-02-03Z"), "fkst-test-bot", "2026-06-03T01:02:03Z"),
    })
    mock_dashboard_issue_list('[[{"number":99,"title":"fkst-dev board","user":{"login":"fkst-test-bot"},"body":"newer\\n<!-- fkst:dashboard:v1 version=\\"2099-01-01T00:00:00Z\\" hash=\\"newer\\" generated_at=\\"2099-01-01T00:00:00Z\\" -->"}]]\n')

    local result = run_observability(opts("observability-dashboard-stale", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH"), 0)
    t.eq(count_calls(dashboard_issue_list_command()), 1)
  end,

  test_dashboard_write_skips_existing_trusted_issue_when_hash_matches = function()
    mock_env("fkst-test-bot", "1")
    local rendered = core.render_observability_dashboard({
      entities = {},
      counts = {},
      stalls = {},
      now_seconds = now(),
    })
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({})
    mock_pr_list({})
    mock_dashboard_issue_list('[[{"number":99,"title":"fkst-dev board","user":{"login":"fkst-test-bot"},"body":"<!-- fkst:dashboard:v1 version=\\"2026-06-01T00:00:00Z\\" hash=\\"' .. dashboard_hash(rendered.body) .. '\\" generated_at=\\"2026-06-01T00:00:00Z\\" -->"}]]\n')

    local result = run_observability(opts("observability-dashboard-unchanged", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --method POST"), 0)
    t.eq(count_calls("gh api --method PATCH"), 0)
    t.is_true(first_call(dashboard_issue_list_command()) ~= nil)
  end,

  test_dashboard_locator_failure_logs_auth_mode_and_http_status = function()
    mock_env("fkst-test-bot", "1")
    mock_all_issue_lists({})
    mock_pr_list({})
    mock_dashboard_issue_list("", 1, "GraphQL: API rate limit already exceeded (HTTP 403)\n")

    local ok, logs = try_capture_observability_logs()

    t.eq(ok, false)
    t.eq(count_calls(dashboard_issue_list_command()), 1)
    t.eq(count_calls("--search"), 0)
    local body = table.concat(logs, "\n")
    t.is_true(body:find("tag=DASHBOARD_LOCATOR_FAILED", 1, true) ~= nil)
    t.is_true(body:find("locator=label-list", 1, true) ~= nil)
    t.is_true(body:find("auth_mode=gh-auth", 1, true) ~= nil)
    t.is_true(body:find("http_status=403", 1, true) ~= nil)
  end,
}
