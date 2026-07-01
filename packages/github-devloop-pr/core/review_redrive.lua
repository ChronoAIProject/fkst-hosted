local devloop_base = require("devloop.base")
local S = {}
local transition_version = require("contract.transition_version")

function S.install(M)
local max_review_redrive_rounds = 3

function M.review_redrive_version(state, pr)
  local version = tostring(state and state.version or "")
  local state_name = tostring(state and state.state or "")
  if state_name ~= "pr-open" and state_name ~= "reviewing" then
    return version
  end
  local review_round = M.version_review_loop_round(version)
  if review_round > 0 or review_round >= max_review_redrive_rounds then
    return version
  end
  local escaped_state = state_name:gsub("%-", "%%-")
  local lineage_version = version
    :gsub("/timeout/" .. escaped_state .. "/%d+$", "")
    :gsub("%-timeout%-" .. escaped_state .. "%-%d+$", "")
  local next_version = M.next_review_loop_version(lineage_version)
  local current_review_id = devloop_base.pr_review_proposal_id(pr.repo, pr.number, lineage_version, pr.head_sha)
  local next_review_id = devloop_base.pr_review_proposal_id(pr.repo, pr.number, next_version, pr.head_sha)
  if current_review_id == next_review_id then
    return version
  end
  return next_version
end

function M.orphaned_pr_ready_version(state)
  local version = tostring(state and state.version or "")
  local lineage_version = transition_version.strip_suffixes(version)
  local next_n = M.version_reimplement_round(lineage_version) + 1
  return lineage_version .. "/reimplement/" .. tostring(next_n)
end

end

return S
