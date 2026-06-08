local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local fix_reconcile = h.fix_reconcile
local run_review_result = h.run_review_result
local run_fix_reconcile = h.run_fix_reconcile
local mock_bot_env = h.mock_bot_env
local mock_pr_origin = h.mock_pr_origin
local mock_issue_result = h.mock_issue_result
local mock_issue_review = h.mock_issue_review
local find_raise = h.find_raise
local count_calls = h.count_calls

local function origin_marker(version)
  return core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", version, "dev")
end

local function fix_round_version(round)
  local version = reviewing().version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function reject_review_event(version)
  local proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, "feedface")
  return review_reached({
    decision = "reject",
    body = "Review consensus rejects the diff.",
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
  })
end

return {
  test_review_result_reject_within_fix_budget_marks_fixing = function()
    local base_version = reviewing().version
    local review_version = core.next_fix_version(base_version)
    local event = reject_review_event(review_version)
    local fix_version = core.fix_version_from_review_version(review_version)
    t.eq(core.version_fix_round(fix_version) <= core.fix_loop_budget(), true)
    mock_pr_origin({ origin_marker(base_version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
    })

    local result = run_review_result(event, opts("fix-budget-within"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local fixing = find_raise(result.raises, "devloop_fixing")
    t.eq(find_raise(result.raises, "devloop_fix_reconcile"), nil)
    t.eq(label.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(comment.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    t.eq(fixing.payload.version, fix_version)
  end,

  test_review_result_reject_over_fix_budget_raises_fix_reconcile = function()
    local review_version = fix_round_version(core.fix_loop_budget())
    local event = reject_review_event(review_version)
    t.eq(core.version_fix_round(review_version), core.fix_loop_budget())
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
    })

    local result = run_review_result(event, opts("fix-budget-over"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    local reconcile = find_raise(result.raises, "devloop_fix_reconcile").payload
    t.eq(reconcile.schema, "github-devloop.fix-reconcile.v1")
    t.eq(reconcile.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reconcile.review_proposal_id, event.proposal_id)
    t.eq(reconcile.review_dedup_key, event.dedup_key)
    t.eq(reconcile.issue_version, review_version)
    t.eq(reconcile.head_sha, "feedface")
    t.eq(reconcile.round, core.fix_loop_budget())
    t.eq(reconcile.pr_number, "7")
    t.eq(reconcile.dedup_key, "fix-reconcile:" .. review_version)
    t.eq(reconcile.source_ref.ref, "owner/repo#issue/42")
    t.eq(core.safe_version_segment(reconcile.issue_version), core.safe_version_segment(review_version))

    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(reconcile.proposal_id, "reviewing", review_version),
    })
    local accepted = run_fix_reconcile(reconcile, opts("fix-budget-over-accepted"))
    t.eq(accepted.exit_code, 0)
    t.eq(#accepted.raises, 2)
    local accepted_comment = find_raise(accepted.raises, "github-proxy.github_issue_comment_request").payload
    t.is_true(accepted_comment.body:find(core.state_marker(reconcile.proposal_id, "blocked", reconcile.issue_version), 1, true) ~= nil)
  end,

  test_review_result_reject_fix_budget_exact_boundary = function()
    local within_version = fix_round_version(core.fix_loop_budget() - 1)
    local within_event = reject_review_event(within_version)
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", within_version),
    })

    local within = run_review_result(within_event, opts("fix-budget-boundary-within"))
    t.eq(within.exit_code, 0)
    t.eq(find_raise(within.raises, "devloop_fix_reconcile"), nil)
    t.eq(find_raise(within.raises, "devloop_fixing").payload.version, core.next_fix_version(within_version))

    local over_version = fix_round_version(core.fix_loop_budget())
    local over_event = reject_review_event(over_version)
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", over_version),
    })

    local over = run_review_result(over_event, opts("fix-budget-boundary-over"))
    t.eq(over.exit_code, 0)
    t.eq(find_raise(over.raises, "devloop_fixing"), nil)
    local reconcile = find_raise(over.raises, "devloop_fix_reconcile").payload
    t.eq(reconcile.issue_version, over_version)
    t.eq(reconcile.round, core.fix_loop_budget())
  end,

  test_fix_reconcile_drop_blocks_reviewing_issue = function()
    local event = fix_reconcile()
    t.eq(core.safe_version_segment(event.issue_version) ~= event.issue_version, true)
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.issue_version),
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-drop"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.is_true(comment.body:find("github-devloop fix reconcile action: drop", 1, true) ~= nil)
    t.is_true(comment.body:find("fix-loop-budget-exhausted-after-3-rounds", 1, true) ~= nil)
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", event.issue_version), 1, true) ~= nil)
    t.is_true(comment.body:find(core.fix_reconcile_marker(event.proposal_id, event.issue_version, "drop"), 1, true) ~= nil)
    t.eq(label.add_labels[1], "fkst-dev:blocked")
    t.eq(label.remove_labels[1], "fkst-dev:thinking")
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_reconcile_visible_marker_is_idempotent = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:blocked" }, {
      core.build_fix_reconcile_comment_request("owner/repo", "42", event, "drop", "already done").body,
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-idempotent"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_reconcile_requires_visible_reviewing_marker = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:enabled" }, {})

    local result = run_fix_reconcile(event, opts("fix-reconcile-pending-reviewing"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_fix_reconcile_skips_when_already_terminal = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.issue_version),
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,
}
