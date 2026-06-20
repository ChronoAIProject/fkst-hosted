local S = {}

function S.install(M)
local terminal_set = {
  merged = true,
  ["closed-unmerged"] = true,
  blocked = true,
}

local function pr_source_ref_text(repo, pr_number)
  return tostring(repo) .. "#pr/" .. tostring(pr_number)
end

local function terminal_from_payload(payload)
  local terminal = payload and (payload.terminal or payload.child_state) or nil
  if terminal_set[terminal] ~= true then
    return nil
  end
  local repo, pr_number = M.parse_pr_source_ref(payload.source_ref)
  if repo == nil then
    return nil
  end
  local pr_proposal = payload.pr_proposal or payload.pr_proposal_id
  local terminal_fact = {
    terminal = terminal,
    pr_proposal = pr_proposal,
    pr_proposal_id = pr_proposal,
    repo = repo,
    pr_identity = tostring(payload.pr_identity or payload.pr_number or pr_number),
    pr_number = tonumber(payload.pr_number or pr_number),
    delegation_generation = payload.delegation_generation,
    head_sha = payload.head_sha,
    merge_commit_sha = payload.merge_commit_sha,
    terminal_marker_id = payload.terminal_marker_id,
  }
  if tostring(terminal_fact.pr_identity) ~= tostring(pr_number)
    or tostring(terminal_fact.pr_number) ~= tostring(pr_number) then
    return nil
  end
  return terminal_fact
end

function M.pr_terminal_event_fact(payload)
  if not M.is_supported_pr_terminal(payload) then
    return nil
  end
  return terminal_from_payload(payload)
end

function M.build_pr_terminal_comment_request(payload)
  local fact = M.pr_terminal_event_fact(payload)
  if fact == nil then
    error("github-devloop: invalid pr-terminal payload")
  end
  local marker = M.pr_terminal_marker(fact)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = fact.repo,
    number = fact.pr_number,
  }, "github-devloop PR terminal fact recorded"
    .. "\n\nTerminal: " .. tostring(fact.terminal)
    .. "\nDelegation generation: " .. tostring(fact.delegation_generation)
    .. "\n\n" .. marker, M._dedup_key({
    "pr-terminal",
    "comment",
    tostring(fact.repo),
    tostring(fact.pr_identity),
    tostring(fact.delegation_generation),
    tostring(fact.terminal_marker_id),
  }), payload.source_ref)
end

function M.pr_terminal_child_completed_key(parent_proposal_id, terminal)
  return M._dedup_key({
    tostring(parent_proposal_id),
    pr_source_ref_text(terminal.repo, terminal.pr_number),
    tostring(terminal.delegation_generation),
    tostring(terminal.terminal_marker_id),
  })
end

local function next_generation_version(version)
  local base = tostring(version or "")
  local next_n = M.version_reimplement_round(base) + 1
  return base .. "/reimplement/" .. tostring(next_n)
end

local function build_parent_comment_request(repo, issue_number, state, terminal, delegation)
  local source_ref = M.issue_source_ref(repo, issue_number)
  local child_key = M.pr_terminal_child_completed_key(delegation.proposal_id, terminal)
  local completed_marker = M.child_completed_marker({
    proposal_id = delegation.proposal_id,
    pr_proposal = terminal.pr_proposal,
    pr_source_ref = pr_source_ref_text(terminal.repo, terminal.pr_number),
    delegation_generation = terminal.delegation_generation,
    terminal_marker_id = terminal.terminal_marker_id,
    terminal = terminal.terminal,
    idempotency_key = child_key,
  })
  local state_marker = M.state_marker(delegation.proposal_id, state.to_state, state.version)
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop resumed parent issue from delegated PR terminal"
    .. "\n\nPR terminal: " .. tostring(terminal.terminal)
    .. "\nPR: #" .. tostring(terminal.pr_number)
    .. "\nDelegation generation: " .. tostring(terminal.delegation_generation)
    .. "\n\n" .. state_marker
    .. "\n" .. completed_marker, M._dedup_key({
    "pr-terminal",
    "resume",
    tostring(delegation.proposal_id),
    tostring(terminal.repo),
    tostring(terminal.pr_number),
    tostring(terminal.delegation_generation),
    tostring(terminal.terminal_marker_id),
    tostring(state.to_state),
  }), source_ref)
end

local function read_pr_terminal(repo, pr_number)
  local pr_view = M.gh_pr_view_origin(repo, pr_number, 30)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr terminal view failed: " .. tostring(pr_view.stderr))
  end
  return M.parse_pr_view_origin(pr_view.stdout)
end

local function read_parent_issue(repo, issue_number)
  local issue_view = M.gh_issue_view_open_pr(repo, issue_number, 30)
  if issue_view.exit_code ~= 0 then
    error("github-devloop: gh issue terminal parent view failed: " .. tostring(issue_view.stderr))
  end
  return M.parse_issue_view_open_pr(issue_view.stdout)
end

local function child_terminal_matches_payload(terminal, payload_fact)
  return terminal ~= nil
    and payload_fact ~= nil
    and terminal.terminal == payload_fact.terminal
    and terminal.pr_proposal == payload_fact.pr_proposal
    and terminal.repo == payload_fact.repo
    and tostring(terminal.pr_identity) == tostring(payload_fact.pr_identity)
    and tostring(terminal.delegation_generation) == tostring(payload_fact.delegation_generation)
    and terminal.head_sha == payload_fact.head_sha
    and terminal.merge_commit_sha == payload_fact.merge_commit_sha
    and tostring(terminal.terminal_marker_id) == tostring(payload_fact.terminal_marker_id)
end

local function parent_state_for_terminal(state, terminal)
  if terminal.terminal == "merged" then
    return { to_state = "merged", version = state.version, reason = "child-pr-merged" }
  end
  if terminal.terminal == "closed-unmerged" then
    if M.version_reimplement_round(state.version) >= M.max_fix_rounds() then
      return { to_state = "blocked", version = state.version .. "/blocked/replacement-budget-exhausted", reason = "replacement-budget-exhausted" }
    end
    return { to_state = "ready", version = next_generation_version(state.version), reason = "child-pr-closed-unmerged" }
  end
  return { to_state = "blocked", version = state.version .. "/blocked/child-pr-blocked", reason = "child-pr-blocked" }
end

function M.ensure_parent_resumed_from_pr_terminal(payload)
  local payload_fact = M.pr_terminal_event_fact(payload)
  if payload_fact == nil then
    error("github-devloop: invalid pr-terminal payload")
  end
  local pr = read_pr_terminal(payload_fact.repo, payload_fact.pr_number)
  local terminal = M.pr_terminal_fact(
    pr.comments,
    payload_fact.repo,
    payload_fact.pr_identity,
    payload_fact.delegation_generation
  )
  if not child_terminal_matches_payload(terminal, payload_fact) then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, { state = nil, version = payload.version }, "pr-terminal", "awaiting-pr", "retry-pending(terminal-marker-not-visible)", "trusted pr-terminal marker is not visible yet")
    error("github-devloop: pr-terminal-marker-not-visible retrying")
  end
  local parent_repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  if parent_repo == nil then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, { state = nil, version = payload.version }, "pr-terminal", "awaiting-pr", "skip-foreign(proposal_id)", "parent proposal is not an issue")
    return false
  end
  local issue = read_parent_issue(parent_repo, issue_number)
  if not M.verify_pr_review_issue_claim("on_pr_terminal", parent_repo, issue_number, issue, payload.proposal_id) then
    return false
  end
  local current = M.current_state(issue.comments, payload.proposal_id)
  local delegation = M.pr_delegation_fact(issue.comments, payload.proposal_id, current.version)
  if current.state ~= "awaiting-pr" then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", "parent-resume", "skip-pending(parent-not-awaiting-pr)", "terminal fact is durable and will be consumed by awaiting-pr entry")
    return false
  end
  if delegation == nil then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", "parent-resume", "retry-pending(pr-delegation-missing)", "awaiting-pr marker is visible without pr-delegation")
    return false
  end
  if tostring(delegation.pr_proposal) ~= tostring(terminal.pr_proposal)
    or tostring(delegation.delegation) ~= tostring(terminal.delegation_generation) then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", "parent-resume", "skip-stale(delegation-mismatch)", "terminal child id or generation does not match parent delegation")
    return false
  end
  local child_key = M.pr_terminal_child_completed_key(payload.proposal_id, terminal)
  if M.child_completed_fact(issue.comments, child_key) ~= nil then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", "parent-resume", "skip-idempotent(child-completed-visible)", "child-completed marker already visible")
    return false
  end
  local next_state = parent_state_for_terminal(current, terminal)
  local transition = M.versioned_transition_status(current, { "awaiting-pr" }, next_state.to_state, current.version)
  if transition ~= "apply" and transition ~= "idempotent" then
    M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", next_state.to_state, M.cas_outcome(current, transition, current.version), next_state.reason)
    return false
  end
  local comment_request = build_parent_comment_request(parent_repo, issue_number, next_state, terminal, delegation)
  local label_request = M.build_state_label_request(
    parent_repo,
    issue_number,
    next_state.to_state,
    M._dedup_key({
      "pr-terminal",
      "label",
      tostring(payload.proposal_id),
      tostring(terminal.delegation_generation),
      tostring(terminal.terminal_marker_id),
      tostring(next_state.to_state),
    }),
    M.issue_source_ref(parent_repo, issue_number)
  )
  local raised = {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  }
  local add_labels, remove_labels = M.state_label_changes(next_state.to_state)
  M.log_cas_decision("on_pr_terminal", payload.proposal_id, current, "awaiting-pr", next_state.to_state, "applied(" .. next_state.reason .. ")", "trusted child terminal matched parent delegation")
  M.log_apply("on_pr_terminal", payload.proposal_id, next_state.to_state, next_state.version, { add = add_labels, remove = remove_labels }, raised)
  M.log_raise("on_pr_terminal", payload.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  M.log_raise("on_pr_terminal", payload.proposal_id, "github-proxy.github_issue_label_request", label_request)
  if next_state.to_state == "merged" and M.write_mode() == "real" then
    local close_result = M.gh_issue_close(parent_repo, issue_number, 60)
    if close_result.exit_code ~= 0 then
      error("github-devloop: issue close failed after child PR terminal: " .. tostring(close_result.stderr))
    end
    M.invalidate_entity_after_write(parent_repo, "issue", issue_number)
  end
  return true
end

function M.on_pr_terminal(payload)
  local request = M.build_pr_terminal_comment_request(payload)
  M.log_raise("on_pr_terminal", payload.proposal_id, "github-proxy.github_pr_comment_request", request)
  return M.ensure_parent_resumed_from_pr_terminal(payload)
end
end

return S
