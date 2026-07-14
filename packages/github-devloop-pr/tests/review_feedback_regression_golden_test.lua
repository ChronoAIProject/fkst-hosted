local payloads_builders = require("devloop.payloads.builders")
local requests_review = require("devloop.requests.review")
local conv_reconcile = require("devloop.convergence.reconcile")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/test/fix/1"
local review_proposal_id = "review-proposal-1"
local review_dedup_key = "consensus-review-proposal-1-review"
local source_ref = {
  kind = "external",
  ref = "owner/repo#pr/7",
}

local function review_feedback_fixing_payload()
  return payloads_builders.build_devloop_fixing_payload({
    proposal_id = proposal_id,
    impl_version = version,
  }, 7, {
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
    reviewed_head_sha = "def456",
    blocking_gap = "missing regression guard",
  }, source_ref)
end

local function capture_raises(fn)
  local raised = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn)
  raise = original_raise
  if not ok then
    error(err)
  end
  return raised
end

return {
  test_review_reject_fix_reconcile_effect_keeps_origin_shape = function()
    local effect = conv_reconcile.build_devloop_fix_reconcile_payload({
      proposal_id = proposal_id,
      review_proposal_id = review_proposal_id,
      review_dedup_key = review_dedup_key,
      reviewed_head_sha = "def456",
      pr_number = 7,
      source_ref = source_ref,
    }, version)

    t.eq(effect.schema, "github-devloop.fix-reconcile.v1")
    t.eq(effect.proposal_id, proposal_id)
    t.eq(effect.review_proposal_id, review_proposal_id)
    t.eq(effect.review_dedup_key, review_dedup_key)
    t.eq(effect.issue_version, version)
    t.eq(effect.head_sha, "def456")
    t.eq(effect.round, 1)
    t.eq(effect.pr_number, 7)
    t.eq(effect.dedup_key, "fix-reconcile:" .. version)
    t.eq(effect.source_ref.kind, "external")
    t.eq(effect.source_ref.ref, "owner/repo#pr/7")
    t.eq(effect.termination_cause, nil)
    t.eq(effect.bound_head_sha, nil)
  end,

  test_review_reject_feedback_payload_keeps_origin_shape = function()
    local payload = review_feedback_fixing_payload()

    t.eq(payload.schema, "github-devloop.fixing.v1")
    t.eq(payload.proposal_id, proposal_id)
    t.eq(payload.pr_number, 7)
    t.eq(payload.version, version)
    t.eq(payload.review_proposal_id, review_proposal_id)
    t.eq(payload.review_dedup_key, review_dedup_key)
    t.eq(payload.reviewed_head_sha, "def456")
    t.eq(payload.repair_input, "review-feedback")
    t.eq(payload.ci_failure_key, nil)
    t.eq(payload.blocking_gap, "missing regression guard")
    t.eq(payload.dedup_key,
      "fixing/github-devloop/issue/owner/repo/42/ready/test/fix/1/7/consensus-review-proposal-1-review/noci")
    t.eq(payload.work_unit_key,
      "review-feedback/def456/consensus-review-proposal-1-review")
    t.eq(payload.source_ref.kind, "external")
    t.eq(payload.source_ref.ref, "owner/repo#pr/7")
  end,

  test_review_feedback_review_meta_output_keeps_origin_shape = function()
    local fix = review_feedback_fixing_payload()
    local request_core = {
      build_comment = function()
        return { kind = "review-meta-comment" }
      end,
      build_label = function()
        return { kind = "review-meta-label" }
      end,
    }
    local raised = capture_raises(function()
      requests_review.raise_fix_review_meta(request_core, "owner/repo", 42, fix, "no-fix", "No repaired revision was published.")
    end)

    t.eq(#raised, 2)
    t.eq(raised[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(raised[1].payload.kind, "review-meta-comment")
    t.eq(raised[2].queue, "github-proxy.github_issue_label_request")
    t.eq(raised[2].payload.kind, "review-meta-label")
    local handoff = raised[1].payload.handoff
    t.eq(handoff.kind, "github-devloop.review_meta")
    t.eq(handoff.proposal_id, proposal_id)
    t.eq(handoff.review_proposal_id, review_proposal_id)
    t.eq(handoff.review_dedup_key, review_dedup_key)
    t.eq(handoff.version, version)
    t.eq(handoff.pr_number, 7)
    t.eq(handoff.n, 0)
    t.eq(handoff.dedup_key, fix.dedup_key)
    t.eq(handoff.source_ref.kind, "external")
    t.eq(handoff.source_ref.ref, "owner/repo#pr/7")
  end,

  test_review_feedback_reviewing_output_keeps_origin_shape = function()
    local fix = review_feedback_fixing_payload()
    local raised = capture_raises(function()
      requests_review.raise_fix_reviewing(core, {
        dept = "fix",
        repo = "owner/repo",
        issue_number = 42,
        fix = fix,
        old_head_sha = "def456",
        new_head_sha = "feedface",
        reason = "fix-pushed",
        fix_summary = "fixed review feedback",
        clear_fix_summary = true,
      })
    end)

    t.eq(#raised, 2)
    t.eq(raised[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(raised[1].payload.dedup_key,
      "fix/comment/github-devloop/issue/owner/repo/42/consensus-review-proposal-1-review/feedface")
    t.eq(raised[1].payload.handoff.kind, "github-devloop.reviewing")
    t.eq(raised[1].payload.handoff.proposal_id, proposal_id)
    t.eq(raised[1].payload.handoff.pr_number, 7)
    t.eq(raised[1].payload.handoff.version, "ready/test/fix/1/fix/2")
    t.eq(raised[1].payload.handoff.source_ref.kind, "external")
    t.eq(raised[1].payload.handoff.source_ref.ref, "owner/repo#pr/7")
    t.is_true(raised[1].payload.body:find("fixed review feedback", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('state="reviewing" version="ready/test/fix/1/fix/2"', 1, true) ~= nil)
    t.eq(raised[2].queue, "github-proxy.github_issue_label_request")
    t.eq(raised[2].payload.add_labels[1], "fkst-dev:reviewing")
  end,
}
