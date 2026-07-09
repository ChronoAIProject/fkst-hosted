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
  return m_builders.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

local function mock_required_check_run(conclusion, id, status)
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":1,"check_runs":[{"id":' .. tostring(id or 101) .. ',"name":"test","status":"' .. tostring(status or "completed") .. '","conclusion":"' .. tostring(conclusion or "") .. '","head_sha":"def456"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_check_runs_json(json)
  t.mock_command(check_runs_cmd, {
    stdout = json,
    stderr = "",
    exit_code = 0,
  })
end

local function run_rollup_red_merge(name, check_conclusion, id, status)
  local event = merge_ready()
  local rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/shared","name":"shared-integration","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"integration"}]'
  mock_bot_env()
  mock_write_env("1")
  mock_write_env("1")
  mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
  mock_pr_merge_rollup({ origin_marker(event) }, rollup_json, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "UNSTABLE")
  mock_required_check_run(check_conclusion, id, status)
  return event, run_merge(event, opts(name, { FKST_GITHUB_WRITE = "1" }))
end

local function stable_ci_failure_key(value)
  return tostring(value or ""):match("^head:def456/checks:digest%-%d%d%d%d%d%d%d%d%d%d$") ~= nil
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
    local _, result = run_rollup_red_merge("merge-own-red-fixing", "failure", 101)
    t.eq(result.exit_code, 0)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local fixing = find_causal_raise(result, "devloop_fixing").payload
    t.is_true(stable_ci_failure_key(fixing.ci_failure_key))
    t.eq(comment.payload.handoff.ci_failure_key, fixing.ci_failure_key)
    t.is_true(comment.payload.body:find('ci_failure_key="' .. fixing.ci_failure_key .. '"', 1, true) ~= nil)
    t.eq(fixing.gate_failure_excerpt, "own-ci-red")
    t.eq(fixing.repair_input, "ci-failure")
    t.is_true(fixing.dedup_key:find("/head-def456/checks-digest-", 1, true) ~= nil)
    t.is_true(fixing.work_unit_key:find("ci-failure/def456/head-def456/checks-digest-", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_rerun_check_id_drift_keeps_same_structural_work_identity = function()
    local _, first = run_rollup_red_merge("merge-own-red-key-101", "failure", 101)
    local _, same = run_rollup_red_merge("merge-own-red-key-202-rerun", "failure", 202)
    local _, changed = run_rollup_red_merge("merge-own-red-key-conclusion-change", "timed_out", 303)
    local first_fix = find_causal_raise(first, "devloop_fixing").payload
    local same_fix = find_causal_raise(same, "devloop_fixing").payload
    local changed_fix = find_causal_raise(changed, "devloop_fixing").payload

    t.is_true(stable_ci_failure_key(first_fix.ci_failure_key))
    t.eq(same_fix.ci_failure_key, first_fix.ci_failure_key)
    t.eq(same_fix.dedup_key, first_fix.dedup_key)
    t.eq(same_fix.work_unit_key, first_fix.work_unit_key)
    t.is_true(stable_ci_failure_key(changed_fix.ci_failure_key))
    t.is_true(changed_fix.dedup_key ~= first_fix.dedup_key)
    t.is_true(changed_fix.work_unit_key ~= first_fix.work_unit_key)
  end,

  test_pending_required_check_does_not_route_to_fixing = function()
    local _, result = run_rollup_red_merge("merge-own-pending-holds", nil, 101, "in_progress")
    t.eq(result.exit_code, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise ~= nil)
    t.is_true(comment_raise.payload.body:find('reason="checks-pending"', 1, true) ~= nil)
  end,

  test_pending_required_with_failed_optional_check_does_not_route_to_fixing = function()
    local event = merge_ready()
    local rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/lint","name":"lint","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"lint"}]'
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker(event) }, rollup_json, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "UNSTABLE")
    mock_check_runs_json('{"total_count":2,"check_runs":[{"id":101,"name":"test","status":"in_progress","conclusion":null,"head_sha":"def456"},{"id":202,"name":"lint","status":"completed","conclusion":"failure","head_sha":"def456"}]}\n')

    local result = run_merge(event, opts("merge-required-pending-optional-failed", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
