local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local gh_argv = require("testkit.gh_argv_mock")
local zh_summary = string.char(228, 184, 173, 230, 150, 135, 230, 145, 152, 232, 166, 129)

local function opts(name, extra)
  local env = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
    FKST_DEVLOOP_ROLLUP_MERGE = "auto",
    FKST_DEVLOOP_RELEASE_NOTES_FALLBACK = "",
    FKST_GITHUB_WRITE = "",
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return { env = env }
end

local function mock_env(write_mode, rollup_merge, integration, release_notes_fallback)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = write_mode or "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = rollup_merge or "auto", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_RELEASE_NOTES_FALLBACK"', {
    stdout = release_notes_fallback or "",
    stderr = "",
    exit_code = 0,
  })
end

local function run_scan(run_opts)
  return t.run_department("departments/rollup_scan/main.lua", {
    queue = "devloop_branch_tick",
    payload = { schema = "github-devloop.branch-tick.v1" },
  }, run_opts or opts("rollup-scan"))
end

local function mock_fetches()
  t.mock_command("git fetch 'origin' 'dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_fetches_for(integration)
  t.mock_command("git fetch 'origin' 'dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' '" .. tostring(integration) .. "'", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_ahead(count)
  t.mock_command("git rev-list --count refs/remotes/origin/'dev'..refs/remotes/origin/'integration/dev'", {
    stdout = tostring(count) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_ahead_for(integration, count)
  t.mock_command("git rev-list --count refs/remotes/origin/'dev'..refs/remotes/origin/'" .. tostring(integration) .. "'", {
    stdout = tostring(count) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_content_diff(has_diff)
  t.mock_command("git diff --quiet refs/remotes/origin/dev refs/remotes/origin/integration/dev", {
    stdout = "",
    stderr = "",
    exit_code = has_diff and 1 or 0,
  })
end

local function mock_content_diff_for(integration, has_diff)
  t.mock_command("git diff --quiet refs/remotes/origin/dev refs/remotes/origin/" .. tostring(integration), {
    stdout = "",
    stderr = "",
    exit_code = has_diff and 1 or 0,
  })
end

local function mock_pr_list(pr)
  local stdout = "[]\n"
  if pr ~= nil then
    stdout = string.format(
      '[[{"number":%d,"head":{"sha":"%s","ref":"integration/dev"},"base":{"ref":"dev"},"state":"open"}]]\n',
      pr.number or 9,
      h.json_string(pr.head_sha or "def456")
    )
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&head=owner%3Aintegration%2Fdev&per_page=100&base=dev'", {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_list_for(integration, pr)
  local stdout = "[]\n"
  if pr ~= nil then
    stdout = string.format(
      '[[{"number":%d,"head":{"sha":"%s","ref":"%s"},"base":{"ref":"dev"},"state":"open"}]]\n',
      pr.number or 9,
      h.json_string(pr.head_sha or "def456"),
      h.json_string(integration)
    )
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&head=owner%3A" .. tostring(integration) .. "&per_page=100&base=dev'", {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_integration_head(head)
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", {
    stdout = (head or "def456") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_integration_head_for(integration, head)
  t.mock_command("refs/remotes/'origin'/'" .. tostring(integration) .. "'^{commit}", {
    stdout = (head or "def456") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_rollup_pr_view(fields)
  fields = fields or {}
  local status = fields.status or "green"
  local state = "COMPLETED"
  local conclusion = "SUCCESS"
  if status == "red" then
    conclusion = "FAILURE"
  elseif status == "pending" then
    state = "IN_PROGRESS"
    conclusion = ""
  end
  local updated_at = fields.updated_at or "2026-06-14T01:02:03Z"
  local completed_at = fields.completed_at or updated_at
  t.mock_command("gh pr view '" .. tostring(fields.pr_number or 9) .. "'", {
    stdout = string.format(
      '{"number":%d,"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"OPEN","updatedAt":"%s","isDraft":false,"mergedAt":"","comments":[],"headRepository":{"nameWithOwner":"owner/repo"},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"test","state":"%s","conclusion":"%s","headSha":"%s","completedAt":"%s"}]}\n',
      fields.pr_number or 9,
      h.json_string(fields.head_ref or "integration/dev"),
      h.json_string(fields.head_sha or "def456"),
      h.json_string(updated_at),
      h.json_string(state),
      h.json_string(conclusion),
      h.json_string(fields.head_sha or "def456"),
      h.json_string(completed_at)
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_release_notes(body)
  t.mock_command("codex exec", {
    stdout = body or ("Release highlights\n\nZh: fa bu zhai yao.\n" .. core._release_notes_ai_sentinel),
    stderr = "",
    exit_code = 0,
  })
end

local function find_call(needle)
  for _, call in ipairs(t.command_calls()) do
    if gh_argv.call_contains(call, needle) then
      return call
    end
  end
  return nil
end

local function argv_option(call, name)
  local argv = { call.program }
  for _, arg in ipairs(call.args or {}) do
    table.insert(argv, arg)
  end
  for index, value in ipairs(argv) do
    if value == name then
      return argv[index + 1]
    end
  end
  return nil
end

return {
  test_rollup_scan_integration_equal_upstream_noops = function()
    mock_env("", "auto", "dev")
    local result = run_scan(opts("rollup-same", { FKST_DEVLOOP_INTEGRATION_BRANCH = "dev" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("git fetch"), 0)
  end,

  test_rollup_scan_not_ahead_noops = function()
    mock_env()
    mock_fetches()
    mock_ahead(0)
    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("gh api --paginate --slurp 'repos/owner/repo/pulls"), 0)
  end,

  test_rollup_scan_ahead_no_open_pr_real_creates_with_head_and_base = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(3)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    mock_release_notes("Release highlights\n\nZh: fa bu zhai yao.\n" .. core._release_notes_ai_sentinel)
    t.mock_command("gh pr create", { stdout = "https://github.example/owner/repo/pull/9\n", stderr = "", exit_code = 0 })
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view()
    local result = run_scan(opts("rollup-create", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr create"), 1)
    t.eq(h.count_calls("codex exec"), 1)
    local saw_prompt_range = false
    local saw_prompt_issue_fetch = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered:find("codex exec", 1, true) ~= nil then
        saw_prompt_range = call.stdin:find("git log --format=%H%x09%s refs/remotes/origin/dev..def456", 1, true) ~= nil
        saw_prompt_issue_fetch = call.stdin:find("gh issue view <referenced-number> --repo owner/repo --json title,body,comments,labels,state", 1, true) ~= nil
      end
    end
    t.is_true(saw_prompt_range)
    t.is_true(saw_prompt_issue_fetch)
    t.is_true(h.has_call("--head integration/dev"))
    t.is_true(h.has_call("--base dev"))
    local create_call = find_call("gh pr create")
    local body = argv_option(create_call, "--body")
    t.is_true(body:find("Release highlights", 1, true) ~= nil)
    t.is_true(body:find(core._release_notes_ai_sentinel, 1, true) ~= nil)
    t.eq(h.count_calls("mktemp '/tmp/fkst-github-devloop-rollup.XXXXXX'"), 0)
    t.eq(h.count_calls("rm -f --"), 0)
  end,

  test_rollup_scan_codex_failure_fails_closed_before_create = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(3)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    t.mock_command("codex exec", { stdout = "", stderr = "model unavailable", exit_code = 1 })
    local result = run_scan(opts("rollup-codex-fail", { FKST_GITHUB_WRITE = "1" }))
    t.is_true(result.exit_code ~= 0)
    t.eq(h.count_calls("gh pr create"), 0)
  end,

  test_rollup_scan_empty_codex_output_fails_closed_before_create = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(3)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    t.mock_command("codex exec", { stdout = "\n" .. core._release_notes_ai_sentinel .. "\n", stderr = "", exit_code = 0 })
    local result = run_scan(opts("rollup-codex-empty", { FKST_GITHUB_WRITE = "1" }))
    t.is_true(result.exit_code ~= 0)
    t.eq(h.count_calls("gh pr create"), 0)
  end,

  test_rollup_scan_explicit_release_notes_fallback_allows_create = function()
    mock_env("1", "auto", nil, "1")
    mock_fetches()
    mock_ahead(3)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    t.mock_command("codex exec", { stdout = "", stderr = "model unavailable", exit_code = 1 })
    t.mock_command("gh pr create", { stdout = "https://github.example/owner/repo/pull/9\n", stderr = "", exit_code = 0 })
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view()
    local result = run_scan(opts("rollup-fallback", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_RELEASE_NOTES_FALLBACK = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr create"), 1)
    local create_call = find_call("gh pr create")
    local body = argv_option(create_call, "--body")
    t.is_true(body:find("Automated rollup", 1, true) ~= nil)
    t.is_true(body:find("Zh: zi dong", 1, true) == nil)
    t.is_true(body:find(zh_summary, 1, true) ~= nil)
    t.is_true(body:find(core._release_notes_ai_sentinel, 1, true) ~= nil)
    t.eq(h.count_calls("rm -f --"), 0)
  end,

  test_rollup_scan_create_failure_has_no_release_notes_body_file = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(3)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    mock_release_notes("Release highlights\n\nZh: fa bu zhai yao.\n" .. core._release_notes_ai_sentinel)
    t.mock_command("gh pr create", { stdout = "", stderr = "create failed", exit_code = 1 })
    local result = run_scan(opts("rollup-create-fail", { FKST_GITHUB_WRITE = "1" }))
    t.is_true(result.exit_code ~= 0)
    t.eq(h.count_calls("gh pr create"), 1)
    t.eq(h.count_calls("mktemp '/tmp/fkst-github-devloop-rollup.XXXXXX'"), 0)
    t.eq(h.count_calls("rm -f --"), 0)
  end,

  test_rollup_scan_no_commits_between_create_failure_noops = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(1)
    mock_content_diff(true)
    mock_pr_list(nil)
    mock_integration_head("def456")
    mock_release_notes("Release highlights\n\nZh: fa bu zhai yao.\n" .. core._release_notes_ai_sentinel)
    t.mock_command("gh pr create", {
      stdout = "",
      stderr = "pull request create failed: GraphQL: No commits between dev and integration/dev",
      exit_code = 1,
    })
    local result = run_scan(opts("rollup-no-commits-between", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("gh pr create"), 1)
    t.eq(h.count_calls("gh api --paginate --slurp 'repos/owner/repo/pulls"), 1)
  end,

  test_rollup_scan_no_commits_between_unslashed_integration_noops = function()
    mock_env("1", "auto", "integration")
    mock_fetches_for("integration")
    mock_ahead_for("integration", 1)
    mock_content_diff_for("integration", true)
    mock_pr_list_for("integration", nil)
    mock_integration_head_for("integration", "def456")
    mock_release_notes("Release highlights\n\nZh: fa bu zhai yao.\n" .. core._release_notes_ai_sentinel)
    t.mock_command("gh pr create", {
      stdout = "",
      stderr = "pull request create failed: GraphQL: No commits between dev and integration",
      exit_code = 1,
    })
    local result = run_scan(opts("rollup-no-commits-between-unslashed", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_INTEGRATION_BRANCH = "integration",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("gh pr create"), 1)
    t.eq(h.count_calls("gh api --paginate --slurp 'repos/owner/repo/pulls"), 1)
  end,

  test_rollup_scan_ahead_without_content_diff_skips_pr = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(1)
    mock_content_diff(false)
    local result = run_scan(opts("rollup-empty-diff", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("gh api --paginate --slurp 'repos/owner/repo/pulls"), 0)
    t.eq(h.count_calls("gh pr create"), 0)
  end,

  test_rollup_scan_existing_pr_never_duplicates_create = function()
    mock_env("1")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view()
    local result = run_scan(opts("rollup-existing", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr create"), 0)
  end,

  test_rollup_scan_manual_posture_no_ready_event = function()
    mock_env("1", "manual")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view()
    local result = run_scan(opts("rollup-manual", { FKST_GITHUB_WRITE = "1", FKST_DEVLOOP_ROLLUP_MERGE = "manual" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_rollup_scan_auto_raises_ready_payload = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view()
    local result = run_scan(opts("rollup-auto", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local raised = h.find_raise(result.raises, "devloop_rollup_ready")
    t.eq(raised.payload.schema, "github-devloop.v1")
    t.eq(raised.payload.repo, "owner/repo")
    t.eq(raised.payload.pr_number, 9)
    t.eq(raised.payload.upstream_branch, "dev")
    t.eq(raised.payload.integration_branch, "integration/dev")
    t.eq(raised.payload.head_sha, "def456")
    t.eq(raised.payload.source_ref.ref, "owner/repo#pr/9")
    t.eq(raised.payload.dedup_key, core.rollup_dedup_key("owner/repo", "dev", "integration/dev", 9, "def456"))
  end,

  test_rollup_scan_surfaces_stale_red_rollup_as_deduped_issue = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view({
      status = "red",
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 45 * 60),
    })
    local result = run_scan(opts("rollup-red-health", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = "30",
    }))
    t.eq(result.exit_code, 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.eq(create.payload.schema, "github-proxy.issue-create.v1")
    t.eq(create.payload.repo, "owner/repo")
    t.eq(create.payload.dedup_key, core.rollup_health_dedup_key("owner/repo", "test: COMPLETED/FAILURE"))
    t.eq(create.payload.parent_comment_target.issue_number, "9")
    t.is_true(create.payload.body:find("Rollup PR: #9", 1, true) ~= nil)
    t.is_true(create.payload.body:find("Failing check: `test: COMPLETED/FAILURE`", 1, true) ~= nil)
    local snapshot = create.payload.body:match("Evidence snapshot: `([^`]+)`")
    t.is_true(snapshot ~= nil)
    local written = file.read(snapshot)
    t.is_true(written:find('"detector":"rollup-health"', 1, true) ~= nil)
    t.is_true(written:find('"failing_check":"test: COMPLETED/FAILURE"', 1, true) ~= nil)
    t.is_true(written:find('"red_started_at"', 1, true) ~= nil)
    t.is_true(h.find_raise(result.raises, "devloop_rollup_ready") ~= nil)
  end,

  test_rollup_scan_red_window_uses_failed_check_time_not_pr_update_time = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view({
      status = "red",
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 5 * 60),
      completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 45 * 60),
    })
    local result = run_scan(opts("rollup-red-check-age", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = "30",
    }))
    t.eq(result.exit_code, 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    local snapshot = create.payload.body:match("Evidence snapshot: `([^`]+)`")
    t.is_true(snapshot ~= nil)
    local written = file.read(snapshot)
    t.is_true(written:find('"age_minutes":45', 1, true) ~= nil)
  end,

  test_rollup_scan_suppresses_red_rollup_inside_window = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view({
      status = "red",
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 5 * 60),
    })
    local result = run_scan(opts("rollup-red-window", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = "30",
    }))
    t.eq(result.exit_code, 0)
    t.eq(h.find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.is_true(h.find_raise(result.raises, "devloop_rollup_ready") ~= nil)
  end,

  test_rollup_scan_pending_rollup_does_not_alert = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view({
      status = "pending",
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 120 * 60),
    })
    local result = run_scan(opts("rollup-pending-health", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(h.find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.is_true(h.find_raise(result.raises, "devloop_rollup_ready") ~= nil)
  end,

  test_rollup_health_has_no_repair_side_effects = function()
    mock_env("1", "auto")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list({ number = 9 })
    mock_integration_head("def456")
    mock_rollup_pr_view({
      status = "red",
      updated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 45 * 60),
    })
    local result = run_scan(opts("rollup-red-no-repair", {
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = "30",
    }))
    t.eq(result.exit_code, 0)
    t.is_true(h.find_raise(result.raises, "github-proxy.github_issue_create_request") ~= nil)
    t.eq(h.count_calls("gh issue edit"), 0)
    t.eq(h.count_calls("gh pr merge"), 0)
    t.eq(h.count_calls("gh pr close"), 0)
    t.eq(h.count_calls("gh issue comment"), 0)
  end,

  test_rollup_scan_dry_run_never_creates_pr = function()
    mock_env("")
    mock_fetches()
    mock_ahead(2)
    mock_content_diff(true)
    mock_pr_list(nil)
    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("gh pr create"), 0)
  end,
}
