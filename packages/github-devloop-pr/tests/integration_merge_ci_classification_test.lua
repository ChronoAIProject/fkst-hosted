local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

local check_runs_cmd = "gh api 'repos/owner/repo/commits/def456/check-runs'"

local function origin_marker(event)
  return m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

local function mock_required_check_run(conclusion)
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"' .. conclusion .. '","head_sha":"def456"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function run_rollup_red_merge(name, check_conclusion)
  local event = merge_ready()
  local rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/shared","name":"shared-integration","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"integration"}]'
  mock_bot_env()
  mock_write_env("1")
  mock_write_env("1")
  mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
  mock_pr_merge_rollup({ origin_marker(event) }, rollup_json, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "UNSTABLE")
  mock_required_check_run(check_conclusion)
  return event, run_merge(event, opts(name, { FKST_GITHUB_WRITE = "1" }))
end

return {
  test_red_shared_rollup_with_green_pr_head_required_checks_holds_without_fixing = function()
    local event, result = run_rollup_red_merge("merge-external-red-holds", "success")
    t.eq(result.exit_code, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(count_calls("gh pr merge"), 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise ~= nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:merge-gate-wait:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('reason="external-ci-red"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('proposal="' .. event.proposal_id .. '"', 1, true) ~= nil)
  end,

  test_red_pr_head_required_check_raises_fixing = function()
    local _, result = run_rollup_red_merge("merge-own-red-fixing", "failure")
    t.eq(result.exit_code, 0)
    t.eq(find_causal_raise(result, "devloop_fixing").payload.gate_failure_excerpt, "own-ci-red")
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
