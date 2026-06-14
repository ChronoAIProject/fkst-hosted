local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local review_unresolved = h.review_unresolved
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local run_review_loop = h.run_review_loop
local run_review_result = h.run_review_result
local mock_bot_env = h.mock_bot_env
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_issue_result = h.mock_issue_result
local mock_pr_origin = h.mock_pr_origin
local mock_pr_origin_sequence = h.mock_pr_origin_sequence
local count_calls = h.count_calls

local function pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function origin_marker(version)
  return core.pr_origin_marker(
    "github-devloop/issue/owner/repo/42",
    "42",
    "devloop-owner-repo-42-01HY",
    version,
    "dev"
  )
end

local function review_state_marker(version)
  return core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", version)
end

return {
  test_observe_pr_skips_issue_backed_review_when_claim_is_other = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    }, {
      assignees = { "other-bot" },
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 0)
    t.eq(count_calls("--json assignees"), 1)
  end,

  test_review_pr_skips_expensive_review_when_claim_is_other = function()
    local event = reviewing()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      review_state_marker(event.version),
    }, {
      assignees = { "other-bot" },
    })
    mock_pr_origin_sequence({
      { comments = { origin_marker(event.version) }, head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
    t.eq(count_calls("--json title,labels,comments,assignees"), 1)
  end,

  test_review_loop_skips_followup_review_when_claim_is_other = function()
    local event = review_unresolved()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_review({ "fkst-dev:reviewing" }, {
      review_state_marker(impl_version),
    }, {
      assignees = { "other-bot" },
    })

    local result = run_review_loop(event, opts("review-loop-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
    t.eq(count_calls("--json title,labels,comments,assignees"), 1)
  end,

  test_review_result_skips_external_write_when_claim_is_other = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      review_state_marker(impl_version),
    }, {
      assignees = { "other-bot" },
    })

    local result = run_review_result(event, opts("review-result-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json assignees"), 1)
  end,
}
