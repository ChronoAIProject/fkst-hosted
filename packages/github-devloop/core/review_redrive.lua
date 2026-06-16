local S = {}

function S.install(M)
local max_review_redrive_rounds = 3

function M.review_redrive_version(state, pr)
  local version = tostring(state and state.version or "")
  local state_name = tostring(state and state.state or "")
  if state_name ~= "pr-open" and state_name ~= "reviewing" then
    return version
  end
  if M.version_timeout_round(version, state_name) <= 0 then
    return version
  end
  if M.liveness_timeout_due(M.restart_transition_row(state_name), state, now()) ~= true then
    return version
  end
  local lineage_version = M.strip_transition_version_suffixes(version)
  if M.version_review_loop_round(version) >= max_review_redrive_rounds then
    return version
  end
  local next_version = M.next_review_loop_version(lineage_version)
  local current_review_id = M.pr_review_proposal_id(pr.repo, pr.number, lineage_version, pr.head_sha)
  local next_review_id = M.pr_review_proposal_id(pr.repo, pr.number, next_version, pr.head_sha)
  if current_review_id == next_review_id then
    return version
  end
  return next_version
end

function M.orphaned_pr_ready_version(state)
  local version = tostring(state and state.version or "")
  local lineage_version = M.strip_transition_version_suffixes(version)
  local next_n = M.version_reimplement_round(lineage_version) + 1
  return lineage_version .. "/reimplement/" .. tostring(next_n)
end

end

return S
