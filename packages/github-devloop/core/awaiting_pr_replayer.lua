-- `awaiting-pr` is the issue-side `dependency_wait` twin: poll-reconcile the delegated PR's trusted `state:v1` marker and never drive `github-devloop-pr` internal lifecycle queues; the PR package owns those queues.
local S = {}
function S.install(M)
local child_terminal_states = {
  merged = true,
  ["closed-unmerged"] = true,
  blocked = true,
}

local function log_skip(dept, proposal_id, state, from_state, to_state, outcome, reason)
  if type(M.replay_log_skip) == "function" then
    return M.replay_log_skip(dept, proposal_id, state, from_state, to_state, outcome, reason)
  end
  M.log_cas_decision(dept, proposal_id, state, from_state, to_state, outcome, reason)
  return false
end

local function raise_effects(dept, proposal_id, apply_state, version, label_changes, effects)
  if type(M.replay_raise_effects) == "function" then
    return M.replay_raise_effects(dept, proposal_id, apply_state, version, label_changes, effects)
  end
  local queues = {}
  for _, effect in ipairs(effects or {}) do
    table.insert(queues, effect.queue)
  end
  M.log_apply(dept, proposal_id, apply_state, version, label_changes or { add = {}, remove = {} }, queues)
  for _, effect in ipairs(effects or {}) do
    M.log_raise(dept, proposal_id, effect.queue, effect.payload)
  end
  return true
end

local function next_reimplementation_version(version)
  local base = tostring(version or "")
  local next_n = M.version_reimplement_round(base) + 1
  return base .. "/reimplement/" .. tostring(next_n)
end

local function parent_state_for_child_terminal(state, child_state)
  if child_state.state == "merged" then
    return {
      to_state = "merged",
      version = state.version,
      reason = "child-pr-merged",
    }
  end
  if child_state.state == "closed-unmerged" then
    if M.version_reimplement_round(state.version) >= M.max_fix_rounds() then
      return {
        to_state = "blocked",
        version = tostring(state.version or "") .. "/blocked/replacement-budget-exhausted",
        reason = "replacement-budget-exhausted",
      }
    end
    return {
      to_state = "ready",
      version = next_reimplementation_version(state.version),
      reason = "child-pr-closed-unmerged",
    }
  end
  return {
    to_state = "blocked",
    version = tostring(state.version or "") .. "/blocked/child-pr-blocked",
    reason = "child-pr-blocked",
  }
end

local function read_delegated_child_pr(dept, issue, delegation)
  local pr_view = M.fetch_pr_view_origin(issue.repo, delegation.pr_number, nil, {
    force_fresh = true,
    consumer = dept,
  })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: awaiting-pr-child-view-failed: " .. tostring(pr_view.stderr))
  end
  local current_pr = M.parse_pr_view_origin(pr_view.stdout)
  current_pr.number = delegation.pr_number
  return current_pr
end

local function child_lineage_matches_delegation(state, delegation, child_state)
  return tostring(delegation.version or "") == tostring(state.version or "")
    and M.strip_transition_version_suffixes(child_state.version) == M.strip_transition_version_suffixes(delegation.version)
end

local function build_resume_comment_request(issue, state, next_state, child_state, delegation)
  local source_ref = issue.source_ref or M.issue_source_ref(issue.repo, issue.number)
  local state_marker = M.state_marker(delegation.proposal_id, next_state.to_state, next_state.version)
  return M.build_entity_comment_request({
    kind = "issue",
    repo = issue.repo,
    number = issue.number,
  }, "github-devloop resumed parent issue from delegated PR child state"
    .. "\n\nChild PR: #" .. tostring(delegation.pr_number)
    .. "\nChild state: " .. tostring(child_state.state)
    .. "\nReason: " .. tostring(next_state.reason)
    .. "\n\n" .. state_marker, M._dedup_key({
    "awaiting-pr",
    "resume",
    tostring(delegation.proposal_id),
    tostring(state.version),
    tostring(delegation.pr_number),
    tostring(delegation.delegation),
    tostring(child_state.state),
    tostring(next_state.to_state),
    tostring(next_state.version),
  }), source_ref)
end

function M.replay_awaiting_pr_state(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  if state.state ~= "awaiting-pr" then
    return log_skip(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-foreign(state)", "awaiting-pr replay requires awaiting-pr state")
  end
  local delegation = facts["pr-delegation"] or facts.pr_delegation
  if delegation == nil then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-foreign(pr-delegation-missing)", "awaiting-pr marker is visible without matching pr-delegation")
  end
  if tostring(delegation.proposal_id or "") ~= tostring(proposal_id or "")
    or tostring(delegation.version or "") ~= tostring(state.version or "") then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(pr-delegation-version)", "pr-delegation proposal or version does not match awaiting-pr state")
  end
  local pr_repo, pr_number = M.parse_pr_proposal_id(delegation.pr_proposal_id or delegation.pr_proposal)
  if pr_repo ~= issue.repo or tostring(pr_number or "") ~= tostring(delegation.pr_number or "") then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(pr-delegation-child)", "pr-delegation child identity is malformed or cross-repo")
  end
  local current_pr = facts.current_pr or read_delegated_child_pr(dept, issue, delegation)
  local child_state = facts.child_state or M.current_entity_state(current_pr.comments, delegation.proposal_id)
  if child_state == nil or child_state.state == nil then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-pending(child-state-missing)", "delegated child PR has no trusted state marker")
  end
  if child_terminal_states[child_state.state] ~= true then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-pending(child-nonterminal)", "delegated child PR is not terminal")
  end
  if not child_lineage_matches_delegation(state, delegation, child_state) then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(child-state-lineage)", "child terminal state does not match parent delegation lineage")
  end
  local next_state = parent_state_for_child_terminal(state, child_state)
  local transition = M.versioned_transition_status(state, { "awaiting-pr" }, next_state.to_state, state.version)
  if transition ~= "apply" and transition ~= "idempotent" then
    return log_skip(dept, proposal_id, state, "awaiting-pr", next_state.to_state, M.cas_outcome(state, transition, state.version), next_state.reason)
  end
  if transition == "idempotent" then
    return log_skip(dept, proposal_id, state, "awaiting-pr", next_state.to_state, "skip-idempotent(already at to_state)", "parent issue already reflects delegated child terminal")
  end

  local comment_request = build_resume_comment_request(issue, state, next_state, child_state, delegation)
  local label_request = M.build_state_label_request(
    issue.repo,
    issue.number,
    next_state.to_state,
    M._dedup_key({
      "awaiting-pr",
      "label",
      tostring(proposal_id),
      tostring(delegation.pr_number),
      tostring(delegation.delegation),
      tostring(next_state.to_state),
      tostring(next_state.version),
    }),
    issue.source_ref or M.issue_source_ref(issue.repo, issue.number)
  )
  local add_labels, remove_labels = M.state_label_changes(next_state.to_state)
  M.log_cas_decision(dept, proposal_id, state, "awaiting-pr", next_state.to_state, "applied(" .. next_state.reason .. ")", "trusted child state marker matched parent delegation")
  local effects = {
    { queue = "github-proxy.github_issue_comment_request", payload = comment_request },
    { queue = "github-proxy.github_issue_label_request", payload = label_request },
  }
  if next_state.to_state == "merged" and M.write_mode() == "real" then
    local close_result = M.gh_issue_close(issue.repo, issue.number, 60)
    if close_result.exit_code ~= 0 then
      error("github-devloop: awaiting-pr-issue-close-failed: " .. tostring(close_result.stderr))
    end
    M.invalidate_entity_after_write(issue.repo, "issue", issue.number)
  end
  return raise_effects(dept, proposal_id, next_state.to_state, next_state.version, { add = add_labels, remove = remove_labels }, effects)
end

end

return S
