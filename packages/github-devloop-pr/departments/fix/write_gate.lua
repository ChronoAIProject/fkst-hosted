local devloop_logging = require("devloop.logging")
local merge_shared = require("forge.merge.shared")

local C = {}

function C.validate(repo, fix, branch, pr, state, reason_prefix, fail_closed)
  if tostring(pr.state or ""):lower() == "open"
    and tostring(pr.head_ref_name or "") == branch
    and tostring(pr.head_sha or "") == tostring(fix.reviewed_head_sha)
    and merge_shared.is_same_repo_pr_head(pr, repo) then
    return pr, state
  end

  local outcome = fail_closed and "fail-closed(write-gate)" or "skip-stale(write-gate)"
  local reason = table.concat({
    tostring(reason_prefix) .. " PR fact changed or head repository missing",
    "state=" .. tostring(pr.state),
    "head_ref=" .. tostring(pr.head_ref_name),
    "expected_head_ref=" .. tostring(branch),
    "head_sha=" .. tostring(pr.head_sha),
    "expected_head_sha=" .. tostring(fix.reviewed_head_sha),
    "head_repo=" .. tostring(pr.head_repository),
    "expected_repo=" .. tostring(repo),
  }, " ")
  devloop_logging.log_cas_decision(
    "fix",
    fix.proposal_id,
    state,
    "fixing",
    "reviewing|review-meta",
    outcome,
    reason
  )
  if fail_closed then
    error("github-devloop: write-time-pr-fact-changed: write-time PR fact changed or head repository missing")
  end
  return nil
end

return C
