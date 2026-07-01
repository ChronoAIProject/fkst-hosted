local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local merge_ready = h.merge_ready
local run_review_pr = h.run_review_pr
local run_merge = h.run_merge
local mock_issue_review = h.mock_issue_review
local mock_issue_merge = h.mock_issue_merge
local mock_pr_origin = h.mock_pr_origin
local mock_pr_merge = h.mock_pr_merge
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise

local function origin_marker(event, impl_version)
  return m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version or event.version, "dev")
end

return {
  test_review_pr_never_requests_ci_status_rollup = function()
    local event = reviewing()
    mock_bot_env()
    mock_pr_origin({
      origin_marker(event),
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })

    local result = run_review_pr(event, opts("review-pr-no-ci-gate"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal").payload.schema, "consensus.proposal.v1")
    t.eq(count_calls("statusCheckRollup"), 0)
  end,

  test_review_pr_review_loop_suffix_proceeds_against_base_reviewing_marker = function()
    local base_version = reviewing().version
    local event = reviewing({
      version = base_version .. "/review-loop/3",
    })
    mock_bot_env()
    mock_pr_origin({
      origin_marker(event, base_version),
      core.state_marker(event.proposal_id, "reviewing", base_version),
    })
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", base_version),
    })

    local result = run_review_pr(event, opts("review-pr-review-loop-suffix-proceeds"))

    t.eq(result.exit_code, 0)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.eq(proposal.payload.schema, "consensus.proposal.v1")
    t.eq(proposal.payload.proposal_id, devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, "def456"))
  end,

  test_review_pr_skips_after_canonical_state_advances = function()
    local event = reviewing()
    mock_bot_env()
    mock_pr_origin({
      origin_marker(event),
      core.state_marker(event.proposal_id, "merge-ready", event.version),
    })

    local result = run_review_pr(event, opts("review-pr-advanced-canonical-state-stale"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_pr_retries_when_canonical_state_is_before_reviewing = function()
    local event = reviewing()
    mock_bot_env()
    mock_pr_origin({
      origin_marker(event),
      core.state_marker(event.proposal_id, "pr-open", event.version),
    })

    local result = run_review_pr(event, opts("review-pr-pr-open-canonical-state-pending"))

    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_review_pr_retries_when_canonical_state_marker_is_absent = function()
    local event = reviewing()
    mock_bot_env()
    mock_pr_origin({
      origin_marker(event),
    })

    local result = run_review_pr(event, opts("review-pr-absent-canonical-state-pending"))

    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_merge_remains_the_ci_status_gate = function()
    local event = merge_ready()
    mock_bot_env()
    mock_write_env("")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) })

    local result = run_merge(event, opts("merge-is-ci-gate"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.is_true(count_calls("statusCheckRollup") >= 1)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
