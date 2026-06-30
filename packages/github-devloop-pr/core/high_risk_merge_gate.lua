local M = {}
local github_risk = require("devloop.github_risk")
local m_facts = require("devloop.markers.facts")

function M.require_evidence(core, repo, comments, merge_ready)
  local name_result = core.gh_pr_diff_name_only(repo, merge_ready.pr_number, 30)
  local risk = github_risk.github_diff_name_risk(name_result)
  if risk.high_risk ~= true then
    return true, "normal-risk"
  end
  if risk.known == false then
    return false, "retry-pending(high-risk-review-evidence:" .. tostring(risk.reason or "unknown") .. ")"
  end
  local paths_digest = nil
  paths_digest = github_risk.github_paths_digest(risk.paths)
  local fact = m_facts.high_risk_review_evidence_fact(core, 
    comments,
    merge_ready.proposal_id,
    merge_ready.version,
    merge_ready.pr_number,
    merge_ready.reviewed_head_sha,
    merge_ready.review_proposal_id,
    merge_ready.review_dedup_key,
    paths_digest
  )
  if fact ~= nil then
    return true, "high-risk-review-evidence"
  end
  return false, "retry-pending(high-risk-review-evidence:" .. tostring(risk.reason or "missing") .. ")"
end

function M.assert_evidence(core, log_gate, repo, comments, merge_ready)
  local ok, reason = M.require_evidence(core, repo, comments, merge_ready)
  if ok then
    return
  end
  log_gate(merge_ready, "dry-run", reason)
  error("github-devloop: high-risk review evidence marker not visible for merge; retrying")
end

return M
