local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local merge_ready = h.merge_ready
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_pr_origin = h.mock_pr_origin
local merge_comments = h.merge_comments
local find_raise = h.find_raise

local function pr_event(updated_at)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@" .. tostring(updated_at or "2026-06-04T03:00:00Z"),
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function trusted_command(id)
  return {
    id = id or "IC_rereview_1",
    body = "fkst: rereview\n\nCI was rerun.",
    author_login = "fkst-test-bot",
    created_at = "2026-06-04T03:00:00Z",
  }
end

return {
  test_trusted_rereview_command_reenters_reviewing = function()
    local impl_version = reviewing().version
    local command = trusted_command()
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview"))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.is_true(comment_raise.payload.body:find("operator command accepted: rereview", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:operator-command:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="reviewing"', 1, true) ~= nil)
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/3/review-loop/4/rereview/4/feedface")
    t.eq(reviewing_raise.payload.source_ref.ref, "owner/repo#pr/7")

    mock_issue_review({ "fkst-dev:reviewing" }, {
      comment_raise.payload.body,
    })
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      comment_raise.payload.body,
    }, "devloop-owner-repo-42-01HY", "feedface")
    local review = run_review_pr(reviewing_raise.payload, opts("operator-rereview-review"))
    t.eq(review.exit_code, 0)
    local proposal = find_raise(review.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, reviewing_raise.payload.version, "feedface"))
  end,

  test_untrusted_rereview_command_is_ignored = function()
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      {
        id = "IC_rereview_untrusted",
        body = "fkst: rereview",
        author_login = "ordinary-user",
        created_at = "2026-06-04T03:00:00Z",
      },
    })

    local result = run_observe_pr(pr_event(), opts("operator-rereview-untrusted"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_rereview_command_invalid_state_refuses_once = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_invalid")
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      command,
    })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(merge_ready()))

    local result = run_observe_pr(pr_event(), opts("operator-rereview-invalid"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find("operator command refused", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('outcome="refused"', 1, true) ~= nil)

    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      command,
      comment_raise.payload.body,
    })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(merge_ready()))
    local replay = run_observe_pr(pr_event("2026-06-04T03:01:00Z"), opts("operator-rereview-invalid-replay"))
    t.eq(replay.exit_code, 0)
    t.eq(find_raise(replay.raises, "github-proxy.github_pr_comment_request"), nil)
  end,

  test_rereview_command_active_reviewing_refuses = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_active_reviewing")
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-active-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find("operator command refused", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('outcome="refused"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("stalled reviewing state", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_rereview_command_stalled_reviewing_reenters_reviewing = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_stalled_reviewing")
    local review_proposal = core.pr_review_proposal_id("owner/repo", 7, impl_version, "feedface")
    local review_version = core.safe_version_segment(impl_version)
    local sr_digest = core.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7" })
    local angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "same-review-digest" },
    }
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      core.review_converge_round_marker(review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 1, "base", "Same review question", angle_digests),
      core.review_converge_round_marker(review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 2, "loop1", "Same review question", angle_digests),
      core.review_converge_round_marker(review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 3, "loop2", "Same review question", angle_digests),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-stalled-reviewing"))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.is_true(comment_raise.payload.body:find("operator command accepted: rereview", 1, true) ~= nil)
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/1/rereview/1/feedface")
  end,

  test_rereview_command_duplicate_response_is_idempotent = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_duplicate")
    local command_fact = core.operator_command_fact({ command }, "rereview")
    local response = core.build_operator_rereview_comment_request(
      "owner/repo",
      7,
      "github-devloop/issue/owner/repo/42",
      impl_version .. "/review-loop/4/rereview/4/feedface",
      command_fact,
      { kind = "external", ref = "owner/repo#pr/7" }
    ).body
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      command,
      response,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-duplicate"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, impl_version .. "/review-loop/4/rereview/4/feedface")
  end,
}
