local h = require("tests.proxy_integration_helpers")
local t = h.t

local function mock_session_work_scope(label, map_json, namespace)
  for _ = 1, 2 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
      stdout = label,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
      stdout = map_json or "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
      stdout = namespace or "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_poll(issues, prs, work_scope)
  local scope = work_scope or {}
  mock_session_work_scope(scope.label or "fkst-dev", scope.map_json, scope.namespace)
  h.mock_repo_env()
  h.mock_poll_label_prefix_env("adapter-")
  h.mock_proxy_replay_budget_env("10")
  h.mock_issue_list(issues)
  h.mock_pr_list(prs or "[]\n")
end

local function run_poll(run_opts, token)
  return t.run_department("departments/github_poll/main.lua", {
    queue = "github_poll_tick",
    payload = {},
    ts = token,
  }, run_opts)
end

local function issue(number, updated_at, labels, assignees_json)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    rendered_labels[#rendered_labels + 1] = string.format('{"name":"%s"}', label)
  end
  local assignees = ""
  if assignees_json ~= nil then
    assignees = ',"assignees":' .. assignees_json
  end
  return string.format(
    '{"number":%d,"title":"Issue %d","html_url":"https://github.example/owner/x/issues/%d","updated_at":"%s","state":"open","author":{"login":"trusted-human"},"labels":[%s]%s}',
    number,
    number,
    number,
    updated_at,
    table.concat(rendered_labels, ","),
    assignees
  )
end

local function creator_opts(name, extra_env)
  local env = {
    FKST_GITHUB_PROXY_REPLAY_BUDGET = "10",
    FKST_SESSION_CREATOR = "Creator-Login",
    FKST_SESSION_WORK_LABEL = "fkst-dev",
  }
  for key, value in pairs(extra_env or {}) do
    env[key] = value
  end
  return h.opts(name, env)
end

local function issue_delivery_count(raises, number)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    if (raised.queue == "github_issue_changed" or raised.queue == "github_issue_observed")
      and tonumber(raised.payload and raised.payload.number) == tonumber(number)
    then
      count = count + 1
    end
  end
  return count
end

return {
  test_creator_metadata_read_failure_fails_closed = function()
    local issues = h.poll_issue_list_from({
      issue(49, "2026-06-03T00:00:00Z", { "fkst-dev", "bug" }, "[]"),
    })
    mock_poll(issues)
    t.mock_command('printf %s "$FKST_SESSION_CREATOR"', {
      stdout = "",
      stderr = "forced creator metadata read failure",
      exit_code = 1,
    })

    local result = run_poll(creator_opts("creator-read-failure"), "creator-read-failure")
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_creator_scope_accepts_any_exact_namespaced_work_label_only = function()
    local namespace = "chronoai-fkst-cloud-test"
    local dev = "fkst-dev-" .. namespace
    local security = "fkst-security-" .. namespace
    local labels = dev .. "," .. security
    local map_json = string.format('{"fkst-dev":"%s","fkst-security":"%s"}', dev, security)
    local run_opts = creator_opts("creator-namespaced-multi-label", {
      FKST_SESSION_WORK_LABEL = labels,
      FKST_SESSION_WORK_LABEL_MAP_JSON = map_json,
      FKST_WORK_LABEL_NAMESPACE = namespace,
    })
    local issues = h.poll_issue_list_from({
      issue(70, "2026-06-03T00:01:00Z", { dev, "bug" }, '[{"login":"creator-login"}]'),
      issue(71, "2026-06-03T00:02:00Z", { security, "bug" }, '[{"login":"CREATOR-LOGIN"}]'),
      issue(72, "2026-06-03T00:03:00Z", { "fkst-dev", "bug" }, '[{"login":"creator-login"}]'),
      issue(73, "2026-06-03T00:04:00Z", { "fkst-dev-other-provider", "bug" }, '[{"login":"creator-login"}]'),
      issue(74, "2026-06-03T00:05:00Z", { dev, "fkst-dev" }, '[{"login":"creator-login"}]'),
    })

    mock_poll(issues, nil, {
      label = labels,
      map_json = map_json,
      namespace = namespace,
    })
    local result = run_poll(run_opts, "creator-namespaced-poll")
    t.eq(result.exit_code, 0)
    local changed = h.changed_raises(result.raises)
    t.eq(#changed, 2)
    t.eq(changed[1].payload.number, 70)
    t.eq(changed[2].payload.number, 71)
  end,

  test_creator_scope_is_shared_by_cold_replay_level_replay_and_observation = function()
    local run_opts = creator_opts("creator-routed-admission")
    local issues = h.poll_issue_list_from({
      issue(50, "2026-06-03T01:01:00Z", { "fkst-dev", "bug" }, '[{"login":"creator-login"}]'),
      issue(51, "2026-06-03T01:02:00Z", { "fkst-dev", "bug" }, nil),
      issue(82, "2026-06-03T01:03:00Z", { "fkst-dev", "fkst-unrouted", "fkst-session-retired", "bug" }, "[]"),
      issue(53, "2026-06-03T01:04:00Z", { "fkst-dev", "bug" }, '[{"login":"other-login"}]'),
      issue(54, "2026-06-03T01:05:00Z", { "fkst-dev", "bug" }, '[{"login":"creator-login"},{"login":"other-login"}]'),
      issue(55, "2026-06-03T01:06:00Z", { "fkst-dev", "bug" }, '[{"login":"creator-login"},{"name":"missing-login"}]'),
      issue(56, "2026-06-03T01:07:00Z", { "bug" }, '[{"login":"creator-login"}]'),
      issue(57, "2026-06-03T01:08:00Z", { "fkst-dev", "adapter-thinking" }, '[{"login":"CREATOR-LOGIN"}]'),
      issue(58, "2026-06-03T01:09:00Z", { "fkst-dev", "bug" }, '["creator-login"]'),
      issue(59, "2026-06-03T01:10:00Z", { "fkst-dev", "bug" }, '[{"login":" creator-login "}]'),
    })
    local prs = h.poll_pr_list_from({
      h.poll_pr_json(7, "2026-06-03T02:01:00Z", "OPEN"),
    })

    mock_poll(issues, prs)
    local first = run_poll(run_opts, "creator-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 3)
    t.eq(h.find_entity_raise(first.raises, "issue", 50).payload.dedup_key,
      "owner/x#issue#50@2026-06-03T01:01:00Z/poll/creator-poll-1")
    t.is_true(h.find_entity_raise(first.raises, "issue", 57) ~= nil)
    t.is_true(h.find_entity_raise(first.raises, "pr", 7) ~= nil)
    t.eq(issue_delivery_count(first.raises, 82), 0)
    t.eq(issue_delivery_count(first.raises, 58), 0)
    t.eq(issue_delivery_count(first.raises, 59), 0)

    mock_poll(issues, prs)
    local second = run_poll(run_opts, "creator-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 1)
    t.eq(h.changed_raises(second.raises)[1].payload.number, 50)
    t.eq(h.changed_raises(second.raises)[1].payload.dedup_key,
      "owner/x#issue#50@2026-06-03T01:01:00Z/poll/creator-poll-2")
    t.eq(#h.observed_issue_raises(second.raises), 1)
    t.eq(h.observed_issue_raises(second.raises)[1].payload.number, 57)
    t.eq(issue_delivery_count(second.raises, 82), 0)
    t.eq(issue_delivery_count(second.raises, 58), 0)
    t.eq(issue_delivery_count(second.raises, 59), 0)
  end,

  test_assignment_changes_leave_and_reenter_creator_scope = function()
    local run_opts = creator_opts("creator-assignment-change")
    local unassigned = h.poll_issue_list_from({
      issue(60, "2026-06-03T03:01:00Z", { "fkst-dev", "bug" }, "[]"),
    })

    mock_poll(unassigned)
    local first = run_poll(run_opts, "creator-assignment-1")
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 0)

    local assigned = h.poll_issue_list_from({
      issue(60, "2026-06-03T03:02:00Z", { "fkst-dev", "bug" }, '[{"login":"creator-login"}]'),
    })
    mock_poll(assigned)
    local second = run_poll(run_opts, "creator-assignment-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 1)
    t.eq(h.changed_raises(second.raises)[1].payload.number, 60)

    local removed = h.poll_issue_list_from({
      issue(60, "2026-06-03T03:03:00Z", { "fkst-dev", "bug" }, "[]"),
    })
    mock_poll(removed)
    local third = run_poll(run_opts, "creator-assignment-3")
    t.eq(third.exit_code, 0)
    t.eq(issue_delivery_count(third.raises, 60), 0)

    local foreign = h.poll_issue_list_from({
      issue(60, "2026-06-03T03:04:00Z", { "fkst-dev", "bug" }, '[{"login":"other-login"}]'),
    })
    mock_poll(foreign)
    local fourth = run_poll(run_opts, "creator-assignment-4")
    t.eq(fourth.exit_code, 0)
    t.eq(issue_delivery_count(fourth.raises, 60), 0)

    local reassigned = h.poll_issue_list_from({
      issue(60, "2026-06-03T03:05:00Z", { "fkst-dev", "bug" }, '[{"login":"CREATOR-LOGIN"}]'),
    })
    mock_poll(reassigned)
    local fifth = run_poll(run_opts, "creator-assignment-5")
    t.eq(fifth.exit_code, 0)
    t.eq(#h.changed_raises(fifth.raises), 1)
    t.eq(h.changed_raises(fifth.raises)[1].payload.number, 60)
  end,
}
