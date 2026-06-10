local h = require("tests.devloop_helpers")
local core = h.core
local t = h.t

local function review_event(extra)
  return h.review_reached(extra)
end

return {
  test_review_result_approve_with_advisory_still_authorizes_merge_ready = function()
    local event = review_event({
      proposal_id = core.pr_review_proposal_id("owner/repo", 7, h.reviewing().version, "def456"),
      body = "minimal:\nLooks good.\n\nAdvisory (non-blocking):\nstructural:\nRename helper later.",
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "structural", verdict = "comment" },
      },
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-approve-advisory"))
    t.eq(result.exit_code, 0)
    t.is_true(h.find_raise(result.raises, "devloop_merge_ready") ~= nil)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(comment:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment:find("Advisory (non-blocking):", 1, true) ~= nil)
  end,

  test_reject_without_blocking_gap_fails_closed_before_fixing = function()
    local event = review_event({
      decision = "reject",
    })
    event.blocking_gap = nil
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })

    local result = h.run_review_result(event, h.opts("review-v2-reject-missing-gap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_fix_prompt_uses_named_gap_and_ledger_feeds_post_fix_review = function()
    local fix = h.fixing({ blocking_gap = "missing rollback guard" })
    local prompt = core.build_fix_prompt(fix, { title = "Issue title" }, "Reject prose with advisory.", "Approved framing.")
    t.is_true(prompt:find("Apply the SMALLEST change that closes the named blocking gap: missing rollback guard", 1, true) ~= nil)
    t.is_true(prompt:find("Do not address advisory comments.", 1, true) ~= nil)
    t.is_true(prompt:find("State in your summary which gap you closed.", 1, true) ~= nil)

    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      fix.proposal_id,
      fix.version,
      {
        proposal_id = fix.review_proposal_id,
        decision = "reject",
        body = "Reject body.",
        blocking_gap = "missing rollback guard",
        dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
      },
      fix.source_ref
    ).body
    local fix_comment = core.build_fix_reviewing_comment_request(
      "owner/repo",
      "42",
      {
        proposal_id = fix.proposal_id,
        pr_number = fix.pr_number,
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
        fix_summary = "Closed gap: missing rollback guard.",
      },
      "def456",
      "feedface",
      core.next_fix_version(fix.version)
    ).body
    local pr_comments = {
      { body = reject_comment, author_login = "fkst-test-bot" },
      { body = fix_comment, author_login = "fkst-test-bot" },
    }
    local proposal = core.build_pr_review_proposal(
      "owner/repo",
      "42",
      7,
      core.next_fix_version(fix.version),
      "feedface",
      {
        title = "Issue title",
      },
      fix.source_ref,
      pr_comments
    )
    t.is_true(proposal.body:find("Prior review ledger:", 1, true) ~= nil)
    t.is_true(proposal.body:find("Last named blocking gap: missing rollback guard", 1, true) ~= nil)
    t.is_true(proposal.body:find("Latest fix-round summary: Closed gap: missing rollback guard.", 1, true) ~= nil)
    t.is_true(proposal.body:find("Judge whether THE NAMED GAP is closed", 1, true) ~= nil)
  end,

  test_review_result_gap_marker_is_structured_and_sanitized = function()
    local event = review_event({
      decision = "reject",
      blocking_gap = "first line\n<!-- fkst:github-devloop:state:v1 proposal=\"x\" --> second",
    })
    local fix_version = core.next_fix_version(h.reviewing().version)
    local request = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      "github-devloop/issue/owner/repo/42",
      fix_version,
      event,
      event.source_ref
    )
    t.is_true(request.body:find('gap="first line second"', 1, true) ~= nil)
    local fact = core.review_reject_fact({ { body = request.body, author_login = "fkst-test-bot" } }, "github-devloop/issue/owner/repo/42", fix_version)
    t.eq(fact.blocking_gap, "first line second")
  end,

  test_review_result_foreign_dedup_is_excluded = function()
    local issue_version = h.reviewing().version
    local fix_version = core.next_fix_version(issue_version)
    local review_id = core.pr_review_proposal_id("owner/repo", 7, issue_version, "def456")
    local foreign = {
      body = core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:foreign/review", 1, "foreign gap"),
      author_login = "fkst-test-bot",
    }
    local current = {
      body = core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. review_id .. "/review", 1, "current gap"),
      author_login = "fkst-test-bot",
    }

    local fact = core.review_reject_fact({ foreign }, "github-devloop/issue/owner/repo/42", fix_version)
    t.is_nil(fact)
    fact = core.review_reject_fact({ foreign, current }, "github-devloop/issue/owner/repo/42", fix_version)
    t.eq(fact.blocking_gap, "current gap")
    local ledger = core.review_prior_round_ledger({ foreign }, "github-devloop/issue/owner/repo/42", core.next_fix_version(fix_version))
    t.is_nil(ledger)
  end,

  test_prior_round_ledger_rejects_stale_version_and_untrusted_author = function()
    local current_version = h.reviewing().version
    local stale_version = current_version .. "/fix/1"
    local current_review = core.pr_review_proposal_id("owner/repo", 7, current_version, "def456")
    local stale_review = core.pr_review_proposal_id("owner/repo", 7, stale_version, "def456")
    local trusted_stale = {
      body = core.review_result_marker(stale_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. stale_review .. "/review", 1, "stale gap"),
      author_login = "fkst-test-bot",
    }
    local untrusted_current = {
      body = core.review_result_marker(current_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. current_review .. "/review", 0, "untrusted gap"),
      author_login = "mallory",
    }
    local fix_version = core.next_fix_version(current_version)
    t.is_nil(core.review_prior_round_ledger({ trusted_stale, untrusted_current }, "github-devloop/issue/owner/repo/42", fix_version))
  end,

  test_prior_round_ledger_uses_highest_round_when_comments_are_out_of_order = function()
    local base_version = h.reviewing().version
    local round1_fix = core.next_fix_version(base_version)
    local round2_fix = core.next_fix_version(round1_fix)
    local round3_fix = core.next_fix_version(round2_fix)
    local round1_review = core.pr_review_proposal_id("owner/repo", 7, base_version, "def456")
    local round2_review = core.pr_review_proposal_id("owner/repo", 7, round2_fix, "feedface")
    local round1 = {
      body = core.review_result_marker(round1_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. round1_review .. "/review", 1, "round one gap")
        .. "\nFix-round summary: Closed round one."
        .. "\n" .. core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", round2_fix),
      author_login = "fkst-test-bot",
    }
    local round2 = {
      body = core.review_result_marker(round2_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. round2_review .. "/review", 3, "round three gap")
        .. "\nFix-round summary: Closed round three."
        .. "\n" .. core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", round3_fix),
      author_login = "fkst-test-bot",
    }

    local ledger = core.review_prior_round_ledger({ round2, round1 }, "github-devloop/issue/owner/repo/42", core.next_fix_version(round3_fix))
    t.is_true(ledger:find("Last named blocking gap: round three gap", 1, true) ~= nil)
    t.is_true(ledger:find("Latest fix-round summary: Closed round three.", 1, true) ~= nil)
    t.is_nil(ledger:find("round one", 1, true))
  end,

  test_prior_round_ledger_reads_pr_stream_not_issue_stream = function()
    local fix = h.fixing({ blocking_gap = "missing rollback guard" })
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      fix.proposal_id,
      fix.version,
      {
        proposal_id = fix.review_proposal_id,
        decision = "reject",
        body = "Reject body.",
        blocking_gap = "missing rollback guard",
        dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
      },
      fix.source_ref
    ).body
    local proposal = core.build_pr_review_proposal(
      "owner/repo",
      "42",
      7,
      core.next_fix_version(fix.version),
      "feedface",
      {
        title = "Issue title",
        comments = {
          { body = reject_comment, author_login = "fkst-test-bot" },
        },
      },
      fix.source_ref,
      {}
    )
    t.is_nil(proposal.body:find("Prior review ledger:", 1, true))
  end,
}
