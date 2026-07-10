local devloop_base = require("devloop.base")
local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local conv_reconcile = require("devloop.convergence.reconcile")
local m_builders = require("devloop.markers.builders")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

return {
  test_review_loop_true_stall_records_round_and_raises_review_reconcile = function()
    local event = h.review_unresolved({
      dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, h.reviewing().version, "def456") .. "/review/loop/3",
      round = 3,
      narrowed_question = "Same review framing",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "same" },
      },
    })
    local impl_version = h.reviewing().version
    local _, _, review_version = devloop_base.parse_pr_review_proposal_id(event.proposal_id)
    local origin_marker = m_builders.pr_origin_marker(
      "github-devloop/issue/owner/repo/42",
      "42",
      "devloop-owner-repo-42-01HY",
      impl_version,
      "dev"
    )
    local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
    h.mock_bot_env()
    h.mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")
    h.mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      conv_rounds.review_converge_round_marker(core, event.proposal_id, "github-devloop/issue/owner/repo/42", review_version, "def456", sr_digest, 1, "base", event.narrowed_question, event.angle_digests),
      conv_rounds.review_converge_round_marker(core, event.proposal_id, "github-devloop/issue/owner/repo/42", review_version, "def456", sr_digest, 2, "loop1", event.narrowed_question, event.angle_digests),
    })

    local result = h.run_review_loop(event, h.opts("review-loop-true-stall"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.is_true(result.raises[1].payload.body:find("fkst:github-devloop:review-converge-round:v1", 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('round="3"', 1, true) ~= nil)
    local reconcile = h.find_raise(result.raises, "devloop_review_reconcile").payload
    t.eq(reconcile.schema, "github-devloop.review-reconcile.v1")
    t.eq(reconcile.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reconcile.review_proposal_id, event.proposal_id)
    t.eq(reconcile.issue_version, review_version)
    t.eq(reconcile.head_sha, "def456")
    t.eq(reconcile.round, 3)
    t.eq(reconcile.dedup_key, "review-reconcile:" .. review_version .. "/review-loop/3")
    t.eq(reconcile.source_ref.ref, "owner/repo#pr/7")
  end,

  test_review_reconcile_drop_blocks_reviewing_issue = function()
    local event = h.review_reconcile()
    h.mock_bot_env()
    h.mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.issue_version),
    })

    local result = h.run_review_reconcile(event, h.opts("review-reconcile-drop"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    local label = h.find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    local version = conv_reconcile.review_reconcile_terminal_state_version(event.issue_version, event.round)
    t.is_true(comment.body:find("github-devloop review reconcile action: drop", 1, true) ~= nil)
    t.is_true(comment.body:find("no-actionable-framing-after-3-review-rounds", 1, true) ~= nil)
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", version), 1, true) ~= nil)
    t.is_true(comment.body:find(conv_reconcile.review_reconcile_marker(event.proposal_id, event.issue_version, event.round, "drop"), 1, true) ~= nil)
    t.eq(label.add_labels[1], "fkst-dev:blocked")
    t.eq(label.remove_labels[1], "fkst-dev:thinking")
    t.eq(h.count_calls("codex exec"), 0)
  end,

  test_review_reconcile_visible_marker_is_idempotent = function()
    local event = h.review_reconcile()
    local state_version = event.issue_version .. "/review-loop/9"
    h.mock_bot_env()
    h.mock_issue_review({ "fkst-dev:blocked" }, {
      core.build_review_reconcile_comment_request("owner/repo", "42", event, "drop", "already done", conv_reconcile.review_reconcile_terminal_state_version(state_version, event.round)).body,
    })

    local result = h.run_review_reconcile(event, h.opts("review-reconcile-idempotent"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("codex exec"), 0)
  end,

  test_review_reconcile_requires_visible_reviewing_marker = function()
    local event = h.review_reconcile()
    h.mock_bot_env()
    h.mock_issue_review({ "fkst-dev:enabled" }, {})

    local result = h.run_review_reconcile(event, h.opts("review-reconcile-pending-reviewing"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,
}
