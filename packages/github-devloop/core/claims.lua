local S = {}

function S.install(M)
local function assignee_login(assignee)
  if type(assignee) == "table" then
    if assignee.login ~= nil then
      return tostring(assignee.login)
    end
    if assignee.name ~= nil then
      return tostring(assignee.name)
    end
  elseif assignee ~= nil then
    return tostring(assignee)
  end
  return nil
end

function M.assignee_logins(value)
  local logins = {}
  if type(value) ~= "table" then
    return logins
  end
  for _, assignee in ipairs(value) do
    local login = assignee_login(assignee)
    if login ~= nil and login ~= "" then
      table.insert(logins, login)
    end
  end
  return logins
end

function M.claim_owner()
  return M.assert_trusted_bot_configured() or M.trusted_bot_login()
end

function M.issue_assigned_to_self_only(assignees, owner)
  local logins = M.assignee_logins(assignees)
  return #logins == 1 and logins[1] == tostring(owner or "")
end

function M.issue_claim_state(assignees, owner)
  local logins = M.assignee_logins(assignees)
  if #logins == 0 then
    return "unassigned"
  end
  if #logins == 1 and logins[1] == tostring(owner or "") then
    return "self"
  end
  return "other"
end

function M.is_self_owned_issue(ownership, owner)
  if type(ownership) ~= "table" then
    return false
  end
  local claim_state = M.issue_claim_state(ownership.assignees, owner)
  if claim_state == "self" then
    return true
  end
  if claim_state ~= "unassigned" then
    return false
  end
  -- Unassigned+self-author is intentional for fork-and-block isolation: a different bot login sees author!=self and skips.
  return M.issue_author_login(ownership) == tostring(owner or "")
end

function M.gh_issue_assign_cmd(repo, issue_number, login)
  return "gh issue edit " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --add-assignee " .. M._shell_single_quote(login)
end

function M.gh_issue_unassign_cmd(repo, issue_number, login)
  return "gh issue edit " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --remove-assignee " .. M._shell_single_quote(login)
end

function M.read_current_issue_assignees(repo, issue_number)
  local ownership = M.read_current_issue_ownership(repo, issue_number)
  return M.assignee_logins(ownership and ownership.assignees)
end

function M.read_current_issue_ownership(repo, issue_number)
  if issue_number == nil then
    return nil
  end
  local view = M.gh_exec({ cmd = M.gh_issue_view_claim_cmd(repo, issue_number), timeout = 30 })
  if view.exit_code ~= 0 then
    error("github-devloop: gh issue claim view failed: " .. tostring(view.stderr))
  end
  local decoded = json.decode(view.stdout or "{}")
  return {
    assignees = M.assignee_logins(decoded.assignees),
    author_login = M.issue_author_login(decoded),
  }
end

function M.verify_issue_claim(repo, issue_number, owner)
  return M.issue_claim_state(M.read_current_issue_assignees(repo, issue_number), owner) == "self"
end

local function log_claim(dept, proposal_id, action, reason)
  M.log_cas_decision(dept, proposal_id, { state = nil, version = nil }, "claim", "claim", action, reason)
end

function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  if issue_number == nil then
    log_claim(dept, proposal_id, "skip-not-owned", "backing issue is absent")
    return false
  end
  local owner = M.claim_owner()
  local ownership = nil
  if type(current_issue) == "table"
    and current_issue.assignees ~= nil
    and M.issue_author_login(current_issue) ~= nil then
    ownership = current_issue
  else
    ownership = M.read_current_issue_ownership(repo, issue_number)
  end
  if M.is_self_owned_issue(ownership, owner) then
    return true
  end
  local status = M.issue_claim_state(ownership and ownership.assignees, owner)
  if status == "other" then
    log_claim(dept, proposal_id, "skip-claimed-by-other", "backing issue assignee claim is held by another login")
  else
    log_claim(dept, proposal_id, "skip-not-owned", "backing issue is not self-owned")
  end
  return false
end

function M.fork_first_observed_key(repo, issue_number, progress_key)
  local parts = {
    "github-devloop",
    "fork-first-observed",
    M.safe_repo(repo),
    "issue",
    M.safe_issue(issue_number),
  }
  if progress_key ~= nil and tostring(progress_key) ~= "" then
    table.insert(parts, M.safe_updated_at(progress_key))
  end
  return M._dedup_key(parts)
end

function M.fork_grace_seconds(exec)
  local raw = M.read_env("FKST_DEVLOOP_FORK_GRACE_HOURS", exec)
  raw = M._trim(raw or "")
  if raw == "" then
    return 3 * 60 * 60
  end
  local hours = tonumber(raw)
  if hours == nil or hours <= 0 or hours > 168 then
    error("github-devloop: invalid FKST_DEVLOOP_FORK_GRACE_HOURS")
  end
  return math.floor(hours * 60 * 60)
end

function M.fork_grace_elapsed(repo, issue_number, current, now_seconds, grace_seconds)
  local current_seconds = tonumber(now_seconds)
  local grace = tonumber(grace_seconds)
  if current_seconds == nil or grace == nil then
    return false, "fork-grace-age-unknown"
  end
  local progress_key = current and (current.updated_at or current.updatedAt)
  local observed_key = M.fork_first_observed_key(repo, issue_number, progress_key)
  local first_observed_seconds = tonumber(cache_get(observed_key) or "")
  if first_observed_seconds == nil then
    cache_set(observed_key, tostring(current_seconds))
    return false, "fork-grace-started"
  end
  if current_seconds - first_observed_seconds < grace then
    return false, "fork-grace-pending"
  end
  return true, "fork-grace-elapsed", current_seconds - first_observed_seconds
end

function M.claim_issue_for_management(dept, repo, issue_number, current, proposal_id)
  local owner = M.claim_owner()
  local status = M.issue_claim_state(current and current.assignees, owner)
  if status == "self" then
    return true
  end
  if status == "other" then
    log_claim(dept, proposal_id, "skip-claimed-by-other", "issue assignee claim is held by another login")
    return false
  end

  local author = M.issue_author_login(current)
  if author == nil or author == "" then
    log_claim(dept, proposal_id, "skip-fork-author-unknown", "issue author is missing or unknown")
    return false
  end
  if author ~= owner then
    local request = M.build_fork_issue_create_request(repo, issue_number, current, M.issue_source_ref(repo, issue_number))
    if request == nil then
      log_claim(dept, proposal_id, "skip-fork-author-unknown", "fork request could not be built from current issue author")
      return false
    end
    if M.has_trusted_issue_create_parent_marker(current and current.comments, request.dedup_key, owner) then
      log_claim(dept, proposal_id, "fork-present", "trusted fork issue-create ledger marker already exists")
      return false
    end
    local elapsed = M.fork_grace_elapsed(repo, issue_number, current, now(), M.fork_grace_seconds())
    if not elapsed then
      log_claim(dept, proposal_id, "skip-fork-grace", "other-authored unassigned issue is inside fork grace window")
      return false
    end
    log_claim(dept, proposal_id, "fork-raised", "other-authored unassigned issue is forked before management")
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_create_request", request)
    return false
  end

  if M.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log_claim(dept, proposal_id, "dry-run-claim", "FKST_GITHUB_WRITE!=1")
    return true
  end

  local assigned = M.gh_exec({ cmd = M.gh_issue_assign_cmd(repo, issue_number, owner), timeout = 30 })
  if assigned.exit_code ~= 0 then
    error("github-devloop: gh issue edit assign failed: " .. tostring(assigned.stderr))
  end
  M.invalidate_entity_after_write(repo, "issue", issue_number)
  if M.verify_issue_claim(repo, issue_number, owner) then
    log_claim(dept, proposal_id, "claim-won", "assignee claim verified after assign")
    return true
  end

  local unassigned = M.gh_exec({ cmd = M.gh_issue_unassign_cmd(repo, issue_number, owner), timeout = 30 })
  if unassigned.exit_code ~= 0 then
    error("github-devloop: gh issue edit unassign failed: " .. tostring(unassigned.stderr))
  end
  M.invalidate_entity_after_write(repo, "issue", issue_number)
  log_claim(dept, proposal_id, "claim-lost", "assignee claim lost after assign verification")
  return false
end

function M.claim_required_payload(source_ref)
  local normalized = M.normalize_source_ref(source_ref)
  local repo, issue_number = M.parse_issue_source_ref(normalized)
  if repo == nil or issue_number == nil then
    return nil
  end
  return {
    owner = M.claim_owner(),
    source_ref = normalized,
  }
end

function M.attach_issue_claim(payload, source_ref)
  if type(payload) ~= "table" then
    return payload
  end
  payload.claim = M.claim_required_payload(source_ref or payload.source_ref)
  return payload
end

function M.claim_timeout_due(state)
  if type(state) ~= "table" or state.state == nil then
    return false
  end
  local row = M.restart_transition_row(state.state)
  if row ~= nil and row.terminal == true then
    return false
  end
  local threshold = M.liveness_budget_minutes(state.state) or M.stall_suspect_threshold_minutes(state.state)
  local age = M.liveness_state_age_minutes(state, now())
  return threshold ~= nil and age ~= nil and age >= threshold
end

function M.maybe_release_stale_self_claim(dept, repo, issue_number, current, proposal_id, state)
  if not M.claim_timeout_due(state) then
    return false
  end
  local owner = M.claim_owner()
  if not M.issue_assigned_to_self_only(current and current.assignees, owner) then
    return false
  end
  if M.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log_claim(dept, proposal_id, "dry-run-timeout-release", "stale self claim would be released")
    return false
  end
  local fresh = M.read_current_issue_assignees(repo, issue_number)
  if not M.issue_assigned_to_self_only(fresh, owner) then
    log_claim(dept, proposal_id, "skip-timeout-release", "fresh assignee read is not self-only")
    return false
  end
  local unassigned = M.gh_exec({ cmd = M.gh_issue_unassign_cmd(repo, issue_number, owner), timeout = 30 })
  if unassigned.exit_code ~= 0 then
    error("github-devloop: gh issue edit unassign failed: " .. tostring(unassigned.stderr))
  end
  M.invalidate_entity_after_write(repo, "issue", issue_number)
  log_claim(dept, proposal_id, "timeout-release", "stale self claim released after fresh self-only verification")
  return true
end

end

return S
