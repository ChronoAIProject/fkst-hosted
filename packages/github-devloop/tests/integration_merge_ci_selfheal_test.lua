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

local dispatch_cmd = "gh workflow run 'ci.yml' --repo 'owner/repo' --ref 'devloop-owner-repo-42-01HY'"

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
  test_missing_status_dispatches_after_first_observed_grace_and_still_waits = function()
    local event = merge_ready()
    local run_opts = opts("merge-missing-status-dispatch", { FKST_GITHUB_WRITE = "1" })
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")

    local observed_key = core.ci_missing_status_first_observed_key("owner/repo", event.pr_number, event.reviewed_head_sha)
    local first = run_merge(event, run_opts)
    t.eq(first.exit_code, 1)
    t.eq(#first.raises, 0)
    t.eq(count_calls(dispatch_cmd), 0)
    t.eq(count_calls("gh pr merge"), 0)

    local seeded = seed_cache(observed_key, now() - 301, run_opts)
    t.eq(seeded.exit_code, 0)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    t.mock_command(dispatch_cmd, {
      stdout = "dispatched\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge(event, run_opts)
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls(dispatch_cmd), 1)
    t.eq(count_calls("gh pr merge"), 0)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, "[]")
    t.mock_command(dispatch_cmd, {
      stdout = "dispatched\n",
      stderr = "",
      exit_code = 0,
    })

    local retry = run_merge(event, run_opts)
    t.eq(retry.exit_code, 1)
    t.eq(#retry.raises, 0)
    t.eq(count_calls(dispatch_cmd), 1)
    t.eq(count_calls("gh api"), 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_pending_checks_do_not_dispatch_ci_selfheal = function()
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
    t.eq(count_calls("gh api"), 0)
    t.eq(count_calls("gh pr merge"), 0)
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
