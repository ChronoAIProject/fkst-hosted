local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")

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

local function encode_json_string(value)
  return tostring(value or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
end

local function render_comment(body, author, created_at)
  return string.format(
    '{"body":"%s","author":{"login":"%s"},"createdAt":"%s"}',
    encode_json_string(body),
    encode_json_string(author or "fkst-test-bot"),
    encode_json_string(created_at or "2026-06-13T01:02:03Z")
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
    t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
      stdout = "integration/dev",
      stderr = "",
      exit_code = 0,
    })
  end
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

local function mock_queue_head(age_minutes, version)
  local proposal_id = "github-devloop/issue/owner/repo/42"
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,comments,state,stateReason", {
    stdout = '{"title":"Merge-ready head","state":"OPEN","comments":['
      .. render_comment(core.state_marker(proposal_id, "merge-ready", version or version_minutes_ago(age_minutes or 90)))
      .. "]}\n",
  })
end

local function mock_merge_queue_list(pr_numbers)
  local items = {}
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(items, string.format('{"number":%d,"state":"open","base":{"ref":"integration/dev"},"head":{"ref":"devloop-owner-repo-%d","sha":"def%d"}}', number, number, number))
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=integration%2Fdev&per_page=100'", {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_queue_pr(pr_number, issue_number, age_minutes, head_sha)
  local proposal_id = "github-devloop/issue/owner/repo/" .. tostring(issue_number)
  local version = version_minutes_ago(age_minutes or 90)
  local review_proposal_id = core.pr_review_proposal_id("owner/repo", pr_number, version, head_sha or "abcdef123456")
  local comments = {
    core.state_marker(proposal_id, "merge-ready", version),
    core.merge_ready_marker(proposal_id, pr_number, version, review_proposal_id, "consensus:" .. review_proposal_id .. "/review", head_sha or "abcdef123456"),
  }
  local shaped_comments = {}
  for _, comment in ipairs(comments) do
    table.insert(shaped_comments, {
      body = comment,
      author_login = "fkst-test-bot",
      created_at = closed_at_minutes_ago(age_minutes or 90),
    })
  end
  entity_read_mocks.mock_pr_view_selector(t, {
    number = pr_number,
    head = "devloop-owner-repo-" .. tostring(pr_number),
    head_sha = head_sha or "abcdef123456",
    base_branch = "integration/dev",
    base_sha = "abc123",
    state = "OPEN",
    updated_at = "2026-06-03T02:03:04Z",
    comments = shaped_comments,
    head_repo = "owner/repo",
    status_check_rollup_json = '[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]',
  }, entity_read_mocks.pr_merge_selector)
end

local function recent_closed_item(number, closed_at, labels)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, '{"name":"' .. encode_json_string(label) .. '"}')
  end
  return '{"number":' .. tostring(number)
    .. ',"title":"Closed issue ' .. tostring(number)
    .. '","closedAt":"' .. encode_json_string(closed_at)
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
  entity_read_mocks.mock_issue_view_raw_selector(t, { number = number }, "title,comments,state,stateReason", {
    stdout = '{"title":"Merged issue","state":"CLOSED","comments":['
      .. render_comment(core.merged_marker(proposal_id, 9, "v1", "abcdef123456"), trusted == false and "mallory" or "fkst-test-bot")
      .. "]}\n",
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

local function prepare_stale_head(version)
  mock_env()
  mock_merge_queue_list({})
  mock_observe_lists(42)
  mock_queue_head(90, version)
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
    local expected_version = version_minutes_ago(90)
    prepare_stale_head(expected_version)
    mock_recent_closed("[]\n")

    local result = run_observability("queue-starvation-fire")

    t.eq(result.exit_code, 0)
    local create = find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    local payload = create.payload
    t.eq(payload.schema, "github-proxy.issue-create.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.dedup_key, core.queue_starvation_dedup_key("owner/repo", "merge-ready/proposal/github-devloop/issue/owner/repo/42/version/" .. expected_version))
    t.eq(payload.parent_comment_target.issue_number, "42")
    t.is_true(payload.body:find("Queue head: #42 Merge-ready head", 1, true) ~= nil)
    t.is_true(payload.body:find("Evidence snapshot: `/tmp/fkst-github-devloop-queue-starvation-owner-repo-", 1, true) ~= nil)
    local snapshot = payload.body:match("Evidence snapshot: `([^`]+)`")
    t.is_true(snapshot ~= nil)
    local written = file.read(snapshot)
    t.is_true(written:find('"detector":"queue-starvation"', 1, true) ~= nil)
    t.is_true(written:find('"age_minutes":90', 1, true) ~= nil)
  end,

  test_queue_starvation_raises_bounded_merge_queue_redrive = function()
    mock_env()
    mock_merge_queue_list({ 459 })
    mock_merge_queue_pr(459, 459, 120, "abcdef123456")
    mock_observe_lists(42)
    mock_queue_head(90)
    mock_recent_closed("[]\n")

    local result = run_observability("queue-starvation-redrive")

    t.eq(result.exit_code, 0)
    local redrive = find_raise(result.raises, "devloop_merge_queue_tick")
    t.is_true(redrive ~= nil)
    t.eq(redrive.payload.schema, "github-devloop.merge-queue-tick.v1")
    t.eq(redrive.payload.source_ref.ref, "owner/repo#pr/459")
    t.eq(redrive.payload.cause.kind, "queue-starvation")
    t.eq(redrive.payload.cause.attempt_key, core.queue_starvation_window_key(now()))
    t.eq(redrive.payload.cause.head_pr_number, 459)
    t.eq(redrive.payload.cause.head_sha, "abcdef123456")
    t.is_true(redrive.payload.cause.incident_identity:find("merge-ready/pr/459/proposal/github-devloop/issue/owner/repo/459", 1, true) ~= nil)
    t.is_true(redrive.payload.dedup_key:find("/window-", 1, true) ~= nil)
  end,

  test_queue_starvation_suppresses_when_trusted_recent_merge_exists = function()
    prepare_stale_head()
    mock_closed_merged_issue(77, 30)

    local result = run_observability("queue-starvation-suppress")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,

  test_queue_starvation_redrives_stale_head_when_recent_unrelated_merge_suppresses_alert = function()
    mock_env()
    mock_merge_queue_list({ 459 })
    mock_merge_queue_pr(459, 459, 120, "abcdef123456")
    mock_observe_lists(42)
    mock_queue_head(90)
    mock_closed_merged_issue(77, 30)

    local result = run_observability("queue-starvation-recent-merge-redrive")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    local redrive = find_raise(result.raises, "devloop_merge_queue_tick")
    t.is_true(redrive ~= nil)
    t.eq(redrive.payload.cause.kind, "queue-starvation")
    t.eq(redrive.payload.cause.head_pr_number, 459)
    t.eq(redrive.payload.cause.attempt_key, core.queue_starvation_window_key(now()))
  end,

  test_queue_starvation_redrive_payload_uses_wrapped_queue_head_entity = function()
    local version = version_minutes_ago(120)
    local redrive = core.queue_starvation_redrive_payload("owner/repo", {
      incident_identity = "merge-ready/pr/459/proposal/github-devloop/issue/owner/repo/459",
      window_key = "window-queue-head-shape",
      queue_head = {
        entity = {
          proposal_id = "github-devloop/issue/owner/repo/459",
          pr_number = 459,
          state = {
            state = "merge-ready",
            version = version,
          },
          head_sha = "abcdef123456",
        },
        age_minutes = 120,
      },
    })

    t.is_true(redrive ~= nil)
    t.eq(redrive.source_ref.ref, "owner/repo#pr/459")
    t.eq(redrive.cause.kind, "queue-starvation")
    t.eq(redrive.cause.head_pr_number, 459)
    t.eq(redrive.cause.head_sha, "abcdef123456")
    t.eq(redrive.cause.proposal_id, "github-devloop/issue/owner/repo/459")
    t.eq(redrive.cause.version, version)
    t.eq(redrive.cause.attempt_key, "window-queue-head-shape")
  end,

  test_queue_starvation_fail_closed_when_recent_closed_command_fails = function()
    prepare_stale_head()
    mock_recent_closed("", 1, "gh failed")

    local result = run_observability("queue-starvation-fail-closed")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,

  test_queue_starvation_dedup_is_stable_for_detector_identity_and_window = function()
    local identity = "merge-ready/pr/9/proposal/github-devloop/issue/owner/repo/42/version/v1/head/abcdef123456"
    local first = core.queue_starvation_dedup_key("owner/repo", identity, "window-123")
    local second = core.queue_starvation_dedup_key("owner/repo", identity, "window-124")
    local third = core.queue_starvation_dedup_key("owner/repo", identity .. "/next", "window-123")

    t.eq(first, second)
    t.eq(first, "queue-starvation/owner/repo/" .. identity)
    t.is_true(first ~= third)
  end,

  test_queue_starvation_derives_actual_head_from_merge_queue_when_sample_misses_it = function()
    mock_env()
    mock_merge_queue_list({ 459 })
    mock_merge_queue_pr(459, 459, 120, "abcdef123456")
    mock_observe_lists(42)
    mock_queue_head(90)
    mock_recent_closed("[]\n")

    local result = run_observability("queue-starvation-merge-queue-head")

    t.eq(result.exit_code, 0)
    local create = find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.eq(create.payload.parent_comment_target.issue_number, "459")
    t.is_true(create.payload.body:find("Queue head: #459 PR #459", 1, true) ~= nil)
    t.is_true(create.payload.body:find("Queue head PR: #459", 1, true) ~= nil)
    t.is_true(create.payload.body:find("Head source: `merge-queue`", 1, true) ~= nil)
    t.is_true(create.payload.dedup_key:find("merge-ready/pr/459/proposal/github-devloop/issue/owner/repo/459", 1, true) ~= nil)
    t.is_true(find_raise(result.raises, "devloop_merge_queue_tick") ~= nil)
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
