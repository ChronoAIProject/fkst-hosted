local h = require("tests.devloop_helpers")
require("tests.cache_seed_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_pr_merge = h.mock_pr_merge
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise

local check_runs_cmd = "gh api 'repos/owner/repo/commits/def456/check-runs'"
local rerequest_cmd = "gh api --method POST 'repos/owner/repo/check-runs/123/rerequest'"

local function mock_absent_check_runs()
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":0,"check_runs":[]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_rerunnable_check_runs()
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":1,"check_runs":[{"id":123,"name":"legacy-ci","status":"queued","conclusion":null,"head_sha":"def456"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_failing_required_check_runs()
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":1,"check_runs":[{"id":123,"name":"test","status":"completed","conclusion":"failure","head_sha":"def456"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_head_nudge_worktree(old_head, new_head)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git worktree remove --force", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git worktree add --detach", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("commit --allow-empty -m 'chore: nudge PR CI'", {
    stdout = "[detached " .. tostring(new_head or "fedcba") .. "] chore: nudge PR CI\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("--force-with-lease='refs/heads/devloop-owner-repo-42-01HY:" .. tostring(old_head or "def456") .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("rev-parse HEAD", {
    stdout = tostring(new_head or "fedcba") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function seed_cache(key, value, run_opts)
  return t.run_department("tests/cache_seed_helpers.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = tostring(value),
    },
  }, run_opts)
end

local function origin_marker(event)
  return core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

return {
  test_missing_status_rerequests_head_check_runs_after_first_observed_grace_and_still_waits = function()
    local event = merge_ready()
    local run_opts = opts("merge-missing-status-rerequest", { FKST_GITHUB_WRITE = "1" })
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    mock_absent_check_runs()

    local observed_key = core.ci_missing_status_first_observed_key("owner/repo", event.pr_number, event.reviewed_head_sha)
    local first = run_merge(event, run_opts)
    t.eq(first.exit_code, 1)
    t.eq(#first.raises, 0)
    t.eq(count_calls(rerequest_cmd), 0)
    t.eq(count_calls("gh pr merge"), 0)

    local seeded = seed_cache(observed_key, now() - 301, run_opts)
    t.eq(seeded.exit_code, 0)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    mock_rerunnable_check_runs()
    t.mock_command(rerequest_cmd, {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge(event, run_opts)
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls(rerequest_cmd), 1)
    t.eq(count_calls("gh workflow run"), 0)
    t.eq(count_calls("gh pr merge"), 0)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    mock_rerunnable_check_runs()

    local retry = run_merge(event, run_opts)
    t.eq(retry.exit_code, 1)
    t.eq(#retry.raises, 0)
    t.eq(count_calls(rerequest_cmd), 1)
    t.eq(count_calls(check_runs_cmd), 3)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_missing_status_head_nudges_when_no_head_check_run_exists = function()
    local event = merge_ready()
    local run_opts = opts("merge-missing-status-head-nudge", { FKST_GITHUB_WRITE = "1" })
    local observed_key = core.ci_missing_status_first_observed_key("owner/repo", event.pr_number, event.reviewed_head_sha)
    local seeded = seed_cache(observed_key, now() - 301, run_opts)
    t.eq(seeded.exit_code, 0)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    mock_absent_check_runs()
    mock_head_nudge_worktree("def456", "fedcba")

    local result = run_merge(event, run_opts)
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("commit --allow-empty -m 'chore: nudge PR CI'"), 1)
    t.eq(count_calls("--force-with-lease='refs/heads/devloop-owner-repo-42-01HY:def456'"), 1)
    t.eq(count_calls("gh workflow run"), 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_pending_checks_do_not_selfheal_ci = function()
    local event = merge_ready()
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "CLEAN", "IN_PROGRESS", "")

    local result = run_merge(event, opts("merge-pending-no-dispatch", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh workflow run"), 0)
    t.eq(count_calls(check_runs_cmd), 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_unstable_completed_failure_rollup_moves_back_to_fixing = function()
    local event = merge_ready()
    local rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/verify","name":"verify","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"ci"}]'
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, rollup_json, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "UNSTABLE")
    mock_failing_required_check_runs()

    local result = run_merge(event, opts("merge-unstable-failure-rollup", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(count_calls("gh pr merge"), 0)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:fixing")
    local fixing_payload = find_raise(result.raises, "devloop_fixing").payload
    t.eq(fixing_payload.gate_failure_excerpt, "own-ci-red")
    local comment_body = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(comment_body:find("own-ci-red", 1, true) ~= nil)
  end,

  test_missing_status_within_first_observed_grace_does_not_dispatch = function()
    local eligible, reason = core.ci_missing_status_dispatch_eligible({
      status_check_rollup = {},
      updated_at = "2026-06-03T02:02:00Z",
    }, 600, 420, 300)

    t.eq(eligible, false)
    t.eq(reason, "missing-status-grace")
    t.eq(count_calls("gh api"), 0)
  end,
}
