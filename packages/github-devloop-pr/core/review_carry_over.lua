local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local requests_review = require("devloop.requests.review")
local m_facts = require("devloop.markers.facts")
local S = {}
local github_risk = require("devloop.github_risk")

function S.install(M)
local function carry_over_risk_gate(repo, pr_number, issue_proposal_id, version, comments, current_head_sha)
  local name_result = M.gh_pr_diff_name_only(repo, pr_number, 30)
  local risk = github_risk.github_diff_name_risk(name_result)
  if risk.known == false then
    return nil, "carry-over-risk-unknown: " .. tostring(risk.reason or "unknown")
  end
  if risk.high_risk ~= true then
    return true, "normal-risk"
  end
  local new_review_proposal = devloop_base.pr_review_proposal_id(repo, pr_number, version, current_head_sha)
  local new_review_dedup = "consensus:" .. new_review_proposal .. "/review"
  local fact = m_facts.high_risk_review_evidence_fact(M, 
    comments,
    issue_proposal_id,
    version,
    pr_number,
    current_head_sha,
    new_review_proposal,
    new_review_dedup,
    github_risk.github_paths_digest(risk.paths)
  )
  if fact == nil then
    return nil, "high-risk-carry-over-evidence-missing"
  end
  return true, "high-risk-evidence"
end

function M.approved_lineage_carry_over(repo, pr_number, issue_proposal_id, version, comments, base_branch, current_head_sha)
  if type(comments) ~= "table" or not require("devloop.pr_safety").is_safe_head_sha(current_head_sha) then
    return nil, "invalid-carry-over-input"
  end
  local fact = m_facts.merge_ready_fact(M, comments, issue_proposal_id, version, pr_number)
  if fact == nil then
    return nil, "missing-merge-ready-fact"
  end
  if tostring(fact.head_sha or "") == tostring(current_head_sha or "") then
    return nil, "head-unchanged"
  end
  local approved = {
    proposal_id = issue_proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    reviewed_head_sha = fact.head_sha,
  }
  local approval_ok = m_facts.review_result_approval_matches_event(M, comments, approved)
  if not approval_ok then
    return nil, "missing-review-result-approve"
  end
  local ancestry = git_mechanics.git_is_ancestor(M.git, fact.head_sha, current_head_sha, 30)
  if ancestry.exit_code ~= 0 then
    return nil, "approved-head-not-ancestor"
  end
  local base_head, base_error = git_mechanics.current_base_head(M.git, base_branch)
  if base_head == nil then
    return nil, "carry-over-proof-unavailable: " .. tostring(base_error)
  end
  local empty_delta, delta_reason = git_mechanics.has_empty_resolution_delta(M.git, fact.head_sha, base_head, current_head_sha)
  if not empty_delta then
    return nil, "non-empty-resolution-delta: " .. tostring(delta_reason)
  end
  local risk_ok, risk_reason = carry_over_risk_gate(repo, pr_number, issue_proposal_id, version, comments, current_head_sha)
  if not risk_ok then
    return nil, risk_reason
  end
  local new_review_proposal = devloop_base.pr_review_proposal_id(repo, pr_number, version, current_head_sha)
  local new_review_dedup = "consensus:" .. new_review_proposal .. "/review"
  return {
    version = version,
    old_review_proposal_id = fact.review_proposal_id,
    old_review_dedup_key = fact.review_dedup_key,
    approved_head_sha = fact.head_sha,
    new_review_proposal_id = new_review_proposal,
    new_review_dedup_key = new_review_dedup,
    new_head_sha = current_head_sha,
    base_head_sha = base_head,
  }, "approved-lineage-empty-delta"
end

function M.raise_review_carry_over(dept, repo, pr_number, issue_proposal_id, version, current_state, current_pr, base_branch)
  local carry, reason = M.approved_lineage_carry_over(
    repo,
    pr_number,
    issue_proposal_id,
    version,
    current_pr and current_pr.comments,
    base_branch,
    current_pr and current_pr.head_sha
  )
  if carry == nil then
    return nil, reason
  end
  local source_ref = entity_lib.pr_source_ref(repo, pr_number)
  local comment_request = requests_review.build_review_carry_over_comment_request(M, repo, pr_number, issue_proposal_id, version, carry, source_ref)
  M.log_cas_decision(dept, issue_proposal_id, current_state, "merge-ready", "merge-ready", "applied(review-carry-over)", "approved head is ancestor and resolution delta is empty")
  M.log_apply(dept, issue_proposal_id, "merge-ready", version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
  })
  M.log_raise(dept, issue_proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  return comment_request, "review-carry-over"
end
end

return S
