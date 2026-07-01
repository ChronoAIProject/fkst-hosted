local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local run_review_result = h.run_review_result
local mock_issue_result = h.mock_issue_result
local mock_pr_origin = h.mock_pr_origin
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

local ai_sentinel = string.char(226, 159, 166) .. "AI:FKST" .. string.char(226, 159, 167)
local verdict_summary_label = "Three-angle verdicts: "

local function mock_issue_claim(assignees, author_login)
  local rendered = {}
  for _, assignee in ipairs(assignees or {}) do
    table.insert(rendered, string.format('{"login":"%s"}', h.json_string(assignee)))
  end
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
    stdout = string.format(
      '{"assignees":[%s],"author":{"login":"%s"}}\n',
      table.concat(rendered, ","),
      h.json_string(author_login or "fkst-test-bot")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_claim_failure()
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
    stdout = "",
    stderr = "claim read failed",
    exit_code = 1,
  })
end

return {
  test_review_result_approve_marks_issue_merge_ready = function()
    local event = review_reached({
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "structural", verdict = "approve" },
        { angle = "delete", verdict = "approve" },
      },
    })
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_result(event, opts("review-result-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local expected_merge_ready = payloads_builders.build_devloop_merge_ready_payload(core, "github-devloop/issue/owner/repo/42", "7", impl_version, {
      review_proposal_id = event.proposal_id,
      review_dedup_key = event.dedup_key,
      reviewed_head_sha = "def456",
      current_head_sha = "def456",
    }, entity_lib.pr_source_ref("owner/repo", 7))
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:merge-ready")
    t.eq(#label_raise.payload.remove_labels, 12)
    t.is_true(comment_raise.payload.body:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find(verdict_summary_label .. "minimal=approve structural=approve delete=approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find(ai_sentinel, 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("state=\"merge-ready\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="merge-ready" version="' .. impl_version .. '"', 1, true) ~= nil)
    t.eq(core.current_state({ comment_raise.payload.body }, "github-devloop/issue/owner/repo/42").version, impl_version)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:review-result:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:merge-ready:v1", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.merge_ready")
    t.eq(comment_raise.payload.handoff.proposal_id, expected_merge_ready.proposal_id)
    t.eq(comment_raise.payload.handoff.pr_number, expected_merge_ready.pr_number)
    t.eq(comment_raise.payload.handoff.version, expected_merge_ready.version)
    t.eq(comment_raise.payload.handoff.review_proposal_id, expected_merge_ready.review_proposal_id)
    t.eq(comment_raise.payload.handoff.review_dedup_key, expected_merge_ready.review_dedup_key)
    t.eq(comment_raise.payload.handoff.reviewed_head_sha, expected_merge_ready.reviewed_head_sha)
    t.eq(comment_raise.payload.handoff.current_head_sha, "def456")
    t.eq(comment_raise.payload.handoff.source_ref.kind, expected_merge_ready.source_ref.kind)
    t.eq(comment_raise.payload.handoff.source_ref.ref, expected_merge_ready.source_ref.ref)
  end,

  test_review_result_reject_marks_issue_fixing = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_result(event, opts("review-result-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local fixing_raise = find_causal_raise(result, "devloop_fixing")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:fixing")
    t.eq(#label_raise.payload.remove_labels, 12)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.fixing")
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.is_true(comment_raise.payload.body:find("decision=\"reject\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("state=\"fixing\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    t.eq(fixing_raise.payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_raise.payload.version, fix_version)
    t.eq(fixing_raise.payload.reviewed_head_sha, "def456")
  end,

  test_review_result_skips_other_owned_backing_issue_before_raising = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_claim({ "human" })

    local result = run_review_result(event, opts("review-result-other-owned"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_accepts_unassigned_self_authored_backing_issue = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_claim({}, "fkst-test-bot")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_result(event, opts("review-result-unassigned-self-author"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.handoff.kind, "github-devloop.merge_ready")
  end,

  test_review_result_fails_closed_when_claim_read_fails = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_claim_failure()

    local result = run_review_result(event, opts("review-result-claim-fails"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_review_result_skips_without_backing_issue_before_raising = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({}, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")

    local result = run_review_result(event, opts("review-result-no-backing-issue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_skips_when_pr_head_advanced_since_review = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_review_result(event, opts("review-result-head-advanced"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_closed_pr_does_not_mark_merge_ready = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "def456", "CLOSED")

    local result = run_review_result(event, opts("review-result-closed"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_reject_new_fix_round_converges_over_same_review_version_merge_ready = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:merge-ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
    })

    local result = run_review_result(event, opts("review-result-conflict-fixing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(comment_raise.payload.body:find("decision=\"reject\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    local current = core.current_state({
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      comment_raise.payload.body,
    }, "github-devloop/issue/owner/repo/42")
    t.eq(current.state, "fixing")
    t.eq(current.version, fix_version)
  end,

  test_review_result_fix_round_approve_uses_safe_review_version_consistently = function()
    local old_version = reviewing().version
    local fix_round_version = core.next_fix_version(old_version)
    local event = review_reached({
      proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, fix_round_version, "feedface"),
      dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, fix_round_version, "feedface") .. "/review",
    })
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", old_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
    })

    local result = run_review_result(event, opts("review-result-fix-round-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find('state="merge-ready" version="' .. fix_round_version .. '"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.eq(comment_raise.payload.handoff.version, fix_round_version)
    t.eq(comment_raise.payload.handoff.reviewed_head_sha, "feedface")
    t.eq(comment_raise.payload.handoff.current_head_sha, "feedface")
    local current = core.current_state({
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
      comment_raise.payload.body,
    }, "github-devloop/issue/owner/repo/42")
    t.eq(current.state, "merge-ready")
    t.eq(current.version, fix_round_version)
  end,

  test_review_result_marker_lag_retries_then_visible_marker_applies = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:enabled" }, {})

    local pending = run_review_result(event, opts("review-result-marker-lag"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)

    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local visible = run_review_result(event, opts("review-result-marker-visible"))
    t.eq(visible.exit_code, 0)
    t.eq(#visible.raises, 2)
    t.eq(find_raise(visible.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
  end,

  test_review_result_same_version_approve_after_reject_stale_skips = function()
    local event = review_reached()
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
    })

    local result = run_review_result(event, opts("review-result-approve-after-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_stale_idempotent_forged_and_foreign_skip = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:merge-ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
    })
    local idempotent = run_review_result(event, opts("review-result-idempotent"))
    t.eq(idempotent.exit_code, 0)
    t.eq(#idempotent.raises, 0)

    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", core.next_fix_version(impl_version)),
    })
    local stale = run_review_result(event, opts("review-result-version-mismatch"))
    t.eq(stale.exit_code, 0)
    t.eq(#stale.raises, 0)

    mock_pr_origin({
      {
        body = m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
        author_login = "ordinary-user",
      },
    }, "devloop/issue/owner/repo/v1")
    local forged = run_review_result(event, opts("review-result-forged-origin"))
    t.eq(forged.exit_code, 0)
    t.eq(#forged.raises, 0)

    local foreign = run_review_result(review_reached({
      proposal_id = "autochrono/pr-review/owner/repo/7/v1",
      dedup_key = "consensus:autochrono/pr-review/owner/repo/7/v1",
    }), opts("review-result-foreign"))
    t.eq(foreign.exit_code, 0)
    t.eq(#foreign.raises, 0)
  end,
}
