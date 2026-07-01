local devloop_base = require("devloop.base")
local convergence_shared = require("devloop.convergence.shared")
local h = require("tests.devloop_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_unresolved = h.review_unresolved
local run_review_loop = h.run_review_loop
local mock_bot_env = h.mock_bot_env
local mock_pr_origin = h.mock_pr_origin
local mock_issue_review = h.mock_issue_review
local find_raise = h.find_raise
local config = require("devloop.config")
local m_builders = require("devloop.markers.builders")

return {
  test_review_loop_reraised_proposal_dedup_follows_incoming_review_lineage = function()
    local event = review_unresolved({
      dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, reviewing().version, "def456") .. "/review/loop/1",
      round = 1,
      narrowed_question = "Is the named PR gap closed now?",
      angle_digests = {
        { angle = "minimal", verdict = "approve", digest = "still-disagrees" },
      },
    })
    local impl_version = reviewing().version
    local _, _, review_version = devloop_base.parse_pr_review_proposal_id(event.proposal_id)
    local origin_marker = m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev")
    mock_bot_env()
    mock_pr_origin({
      origin_marker,
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_loop(event, opts("review-loop-dedup-lineage"))
    t.eq(result.exit_code, 0)
    local proposal = find_raise(result.raises, "consensus.proposal").payload
    t.eq(proposal.dedup_key, conv_rounds.converge_proposal_base_dedup(core, event.dedup_key) .. "/loop/2")
    t.eq(proposal.round, 2)
    t.eq(proposal.convergence_question, event.narrowed_question)
    t.eq(proposal.source_ref.ref, "owner/repo#pr/7")

    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    t.is_true(comment.body:find('version="' .. review_version .. '"', 1, true) ~= nil)
    t.is_true(comment.body:find('round="1"', 1, true) ~= nil)
  end,

  test_review_loop_round_cap_records_round_and_raises_review_reconcile_even_when_question_varies = function()
    local cap = config.max_converge_rounds(core)
    local event = review_unresolved({
      dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, reviewing().version, "def456") .. "/review/loop/" .. tostring(cap),
      round = cap,
      narrowed_question = "Review question " .. tostring(cap),
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "review-digest-" .. tostring(cap) },
      },
    })
    local impl_version = reviewing().version
    local _, _, review_version = devloop_base.parse_pr_review_proposal_id(event.proposal_id)
    local origin_marker = m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev")
    local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
    local function varying_digest(round)
      return {
        { angle = "minimal", verdict = "abstain", digest = "review-digest-" .. tostring(round) },
      }
    end
    mock_bot_env()
    mock_pr_origin({
      origin_marker,
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      conv_rounds.review_converge_round_marker(core, event.proposal_id, "github-devloop/issue/owner/repo/42", review_version, "def456", sr_digest, cap - 2, "base", "Review question " .. tostring(cap - 2), varying_digest(cap - 2)),
      conv_rounds.review_converge_round_marker(core, event.proposal_id, "github-devloop/issue/owner/repo/42", review_version, "def456", sr_digest, cap - 1, "loop", "Review question " .. tostring(cap - 1), varying_digest(cap - 1)),
    }, "devloop-owner-repo-42-01HY", "def456")

    local result = run_review_loop(event, opts("review-loop-round-cap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.is_true(result.raises[1].payload.body:find("fkst:github-devloop:review-converge-round:v1", 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('round="' .. tostring(cap) .. '"', 1, true) ~= nil)
    t.eq(result.raises[2].queue, "devloop_review_reconcile")
    local reconcile_raise = find_raise(result.raises, "devloop_review_reconcile").payload
    t.eq(reconcile_raise.schema, "github-devloop.review-reconcile.v1")
    t.eq(reconcile_raise.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reconcile_raise.review_proposal_id, event.proposal_id)
    t.eq(reconcile_raise.issue_version, review_version)
    t.eq(reconcile_raise.head_sha, "def456")
    t.eq(reconcile_raise.round, cap)
    t.eq(reconcile_raise.dedup_key, "review-reconcile:" .. review_version .. "/review-loop/" .. tostring(cap))
  end,

  test_review_loop_round_cap_uses_review_budget_when_version_head_and_source_ref_drift = function()
    local cap = config.max_converge_rounds(core)
    local event = review_unresolved({
      dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, reviewing().version, "def456") .. "/review/loop/6",
      round = 6,
      narrowed_question = "Review question 6 current lineage",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "review-digest-6" },
      },
    })
    local impl_version = reviewing().version
    local _, _, review_version = devloop_base.parse_pr_review_proposal_id(event.proposal_id)
    local drift_version = review_version .. "-drift"
    local origin_marker = m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev")
    local current_digest = convergence_shared.source_ref_digest(event.source_ref)
    local drift_digest = convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7?drift=1" })
    mock_bot_env()
    mock_pr_origin({
      origin_marker,
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      conv_rounds.review_converge_round_marker(core, event.proposal_id, "github-devloop/issue/owner/repo/42", drift_version, "feedface", drift_digest, cap, "review/loop/" .. tostring(cap), "Review question " .. tostring(cap) .. " drifted", {
        { angle = "minimal", verdict = "abstain", digest = "review-digest-" .. tostring(cap) },
      }),
    }, "devloop-owner-repo-42-01HY", "def456")

    local result = run_review_loop(event, opts("review-loop-budget-drift-cap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.is_true(result.raises[1].payload.body:find("fkst:github-devloop:review-converge-round:v1", 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('round="6"', 1, true) ~= nil)
    t.eq(result.raises[2].queue, "devloop_review_reconcile")
    local reconcile_raise = find_raise(result.raises, "devloop_review_reconcile").payload
    t.eq(reconcile_raise.round, 6)
    t.eq(reconcile_raise.dedup_key, "review-reconcile:" .. review_version .. "/review-loop/6")
  end,
}
