local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local mock_bot_env = h.mock_bot_env
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
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
  test_verify_pr_review_issue_claim_accepts_unassigned_self_author = function()
    mock_bot_env()
    local ok = core.verify_pr_review_issue_claim("claim-test", "owner/repo", 42, {
      assignees = {},
      author_login = "fkst-test-bot",
    }, "github-devloop/issue/owner/repo/42")
    t.eq(ok, true)
  end,

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
    t.eq(count_calls("--json assignees,author"), 1)
  end,

  test_observe_pr_processes_unassigned_self_authored_backing_issue = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    }, {
      assignees = {},
      author_login = "fkst-test-bot",
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-unassigned-self-author"))
    t.eq(result.exit_code, 0)
    t.is_true(h.find_raise(result.raises, "devloop_reviewing") ~= nil)
  end,

  test_observe_pr_skips_without_backing_issue = function()
    mock_bot_env()
    mock_pr_origin({}, "feature-branch", "def456")

    local result = run_observe_pr(pr_event(), opts("observe-pr-no-backing-issue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_observe_pr_claim_read_failure_fails_closed = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = "",
      stderr = "forced claim failure",
      exit_code = 1,
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-claim-read-fails"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
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
    t.eq(count_calls("--json title,labels,comments,assignees,author"), 1)
  end,

}
