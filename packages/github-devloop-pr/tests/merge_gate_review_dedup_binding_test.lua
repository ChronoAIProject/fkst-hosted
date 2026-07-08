local devloop_base = require("devloop.base")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local h = require("tests.devloop_helpers")
local t = h.t

local issue_proposal = "github-devloop/issue/owner/repo/42"
local review_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function trusted_comment(body)
  return {
    body = body,
    author_login = devloop_base._test_bot_login,
    created_at = "2026-06-03T02:00:00Z",
  }
end

local function review_identity(version, head_sha)
  local review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, version, head_sha)
  return review_proposal, devloop_base.pr_review_consensus_dedup_key(review_proposal)
end

local function merge_ready_for(review_proposal, review_dedup, head_sha)
  return {
    proposal_id = issue_proposal,
    pr_number = 7,
    version = review_version,
    review_proposal_id = review_proposal,
    review_dedup_key = review_dedup,
    reviewed_head_sha = head_sha,
  }
end

local function merge_gate_marker_for(review_proposal, review_dedup, head_sha)
  return m_builders.merge_gate_marker(
    issue_proposal,
    7,
    review_version,
    review_proposal,
    review_dedup,
    head_sha,
    nil,
    "gate-ok"
  )
end

return {
  test_loop_form_review_approval_does_not_bind_different_review_merge_ready = function()
    local review_a, dedup_a = review_identity(review_version, "def456")
    local review_b, dedup_b = review_identity(review_version, "feedface")
    local merge_ready_b = merge_ready_for(review_b, dedup_b .. "/loop/1", "feedface")
    local approval_a = {
      proposal_id = issue_proposal,
      pr_number = 7,
      version = review_version,
      head_sha = "def456",
      review_proposal_id = review_a,
      review_dedup_key = dedup_a,
    }
    local comments = {
      trusted_comment(m_builders.review_result_marker(
        review_a,
        issue_proposal,
        "approve",
        dedup_a,
        nil,
        nil
      )),
    }

    local ready_ok = m_facts.merge_ready_approval_matches_event(approval_a, merge_ready_b)
    t.eq(ready_ok, false)
    local review_ok = m_facts.review_result_approval_matches_event(comments, merge_ready_b)
    t.eq(review_ok, false)
  end,

  test_loop_form_review_approval_binds_same_review_base_merge_ready = function()
    local review_a, dedup_a = review_identity(review_version, "def456")
    local merge_ready_a = merge_ready_for(review_a, dedup_a, "def456")
    local approval_a = {
      proposal_id = issue_proposal,
      pr_number = 7,
      version = review_version,
      head_sha = "def456",
      review_proposal_id = review_a,
      review_dedup_key = dedup_a .. "/loop/1",
    }
    local comments = {
      trusted_comment(m_builders.review_result_marker(
        review_a,
        issue_proposal,
        "approve",
        dedup_a .. "/loop/1",
        nil,
        nil
      )),
    }

    local ready_ok = m_facts.merge_ready_approval_matches_event(approval_a, merge_ready_a)
    t.eq(ready_ok, true)
    local review_ok = m_facts.review_result_approval_matches_event(comments, merge_ready_a)
    t.eq(review_ok, true)
  end,

  test_base_review_dedup_option_matches_without_review_proposal_option = function()
    local review_a, dedup_a = review_identity(review_version, "def456")
    local comments = {
      trusted_comment(merge_gate_marker_for(review_a, dedup_a, "def456")),
    }

    local fact = m_facts.merge_gate_fix_fact(comments, issue_proposal, review_version, {
      review_dedup_key = dedup_a,
    })
    t.is_true(fact ~= nil)
    t.eq(fact.review_proposal_id, review_a)
  end,
}
