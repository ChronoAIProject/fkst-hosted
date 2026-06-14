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
  local view = M.gh_exec({ cmd = M.gh_issue_view_claim_cmd(repo, issue_number), timeout = 30 })
  if view.exit_code ~= 0 then
    error("github-devloop: gh issue claim view failed: " .. tostring(view.stderr))
  end
  local decoded = json.decode(view.stdout or "{}")
  return M.assignee_logins(decoded.assignees)
end

function M.verify_issue_claim(repo, issue_number, owner)
  return M.issue_claim_state(M.read_current_issue_assignees(repo, issue_number), owner) == "self"
end

local function log_claim(dept, proposal_id, action, reason)
  M.log_cas_decision(dept, proposal_id, { state = nil, version = nil }, "claim", "claim", action, reason)
end

function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  if issue_number == nil then
    return true
  end
  local owner = M.trusted_bot_login()
  if current_issue ~= nil and current_issue.assignees ~= nil then
    local status = M.issue_claim_state(current_issue.assignees, owner)
    if status == "self" then
      return true
    end
    if status == "other" then
      log_claim(dept, proposal_id, "skip-claimed-by-other", "backing issue assignee claim is held by another login")
      return false
    end
  end
  local fresh = M.read_current_issue_assignees(repo, issue_number)
  if M.issue_claim_state(fresh, owner) == "self" then
    return true
  end
  log_claim(dept, proposal_id, "skip-claim-missing", "backing issue assignee claim is not self-only")
  return false
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
