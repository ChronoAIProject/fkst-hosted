local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local graph = require("testkit.graph")
local author_policy = require("testkit.github_author_policy")

local repo = "owner/repo"
local pr_number = 9
local comments_cmd = "gh api --paginate --slurp 'repos/owner/repo/issues/9/comments?per_page=100'"

local function issue_create_marker(dedup_key)
  return "<!-- fkst:github-proxy:issue-create:" .. tostring(dedup_key) .. " -->"
end

local function issue_create_intent_marker(dedup_key)
  return '<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. tostring(dedup_key) .. '" -->'
end

local function issue_created_marker(dedup_key, issue_number)
  return '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. tostring(dedup_key)
    .. '" issue="' .. tostring(issue_number) .. '" -->'
end

local function comments_json(body)
  if body == nil then
    return "[[]]\n"
  end
  return string.format(
    '[[{"id":1,"body":"%s","user":{"login":"%s"}}]]\n',
    h.json_string(body),
    h.json_string(core._test_bot_login)
  )
end

local function rollup_request(head_sha)
  return core.build_rollup_health_issue_create_request(repo, {
    pr_number = pr_number,
    upstream_branch = "dev",
    integration_branch = "integration/dev",
    head_sha = head_sha,
    updated_at = "2026-07-13T01:00:00Z",
    red_started_at = "2026-07-13T01:00:00Z",
    age_minutes = 45,
    threshold_minutes = 30,
    failing_check = "test: COMPLETED/FAILURE",
    rollup_autofix = false,
  }, "/tmp/rollup-health-evidence.json")
end

local function run_opts(name)
  return {
    max_steps = 2,
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop-integration/" .. tostring(now()) .. "/" .. name,
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_BOT_LOGIN = core._test_bot_login,
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = core._test_bot_login,
      FKST_GITHUB_AUTHORIZED_LOGINS = "trusted-human",
    },
  }
end

local function deliver(request, name, comments, confirm, search, created_issue)
  local opts = run_opts(name)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
  author_policy.mock_env(t, opts)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = core._test_bot_login, stderr = "", exit_code = 0,
  })
  t.mock_command(comments_cmd, { stdout = comments_json(comments), stderr = "", exit_code = 0 })
  if confirm ~= nil then
    t.mock_command("gh issue comment 9 --repo owner/repo --body-file /tmp/fkst-github-proxy-intent-", {
      stdout = "", stderr = "", exit_code = 0,
    })
    t.mock_command(comments_cmd, { stdout = comments_json(confirm), stderr = "", exit_code = 0 })
  end
  t.mock_command("gh issue list", { stdout = search, stderr = "", exit_code = 0 })
  if created_issue ~= nil then
    t.mock_command("gh issue create", {
      stdout = "https://github.example/owner/repo/issues/" .. tostring(created_issue) .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("gh issue comment 9 --repo owner/repo --body-file /tmp/fkst-github-proxy-created-", {
    stdout = "", stderr = "", exit_code = 0,
  })

  local trace = graph.require_quiescent(graph.run({
    queue = "github-proxy.github_issue_create_request",
    payload = request,
    source_ref = { kind = request.source_ref.kind, reference = request.source_ref.ref },
  }, opts))
  graph.assert_covers(trace, {
    "github-proxy.github_issue_create_request -> github-proxy.github_issue_create",
  })
end

return {
  test_rollup_alert_replay_creates_once_and_new_head_creates_after_closed_alert = function()
    local request_a = rollup_request("aaaa1111")
    local replay_a = rollup_request("aaaa1111")
    local request_b = rollup_request("bbbb2222")
    local key_a = request_a.dedup_key
    local key_b = request_b.dedup_key
    local closed_a = string.format(
      '[{"number":101,"title":"Existing A","state":"CLOSED","body":"%s","author":{"login":"%s"}}]\n',
      h.json_string(issue_create_marker(key_a)),
      h.json_string(core._test_bot_login)
    )

    t.eq(key_a, replay_a.dedup_key)
    t.is_true(key_a ~= key_b)
    deliver(request_a, "head-a", nil, issue_create_intent_marker(key_a), "[]\n", 101)
    t.eq(h.count_calls("gh issue create"), 1)
    deliver(replay_a, "head-a-replay", issue_create_intent_marker(key_a), nil, closed_a, nil)
    t.eq(h.count_calls("gh issue create"), 1)
    deliver(request_b, "head-b", issue_created_marker(key_a, 101),
      issue_created_marker(key_a, 101) .. "\n" .. issue_create_intent_marker(key_b), closed_a, 102)
    t.eq(h.count_calls("gh issue create"), 2)
  end,
}
