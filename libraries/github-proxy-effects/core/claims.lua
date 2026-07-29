local content_filter = require("forge.github.content_filter")
local devloop_config = require("devloop.config")

local S = {}

local function same_login(left, right)
  local canonical_left = content_filter.canon_login(left)
  local canonical_right = content_filter.canon_login(right)
  return canonical_left ~= nil and canonical_right ~= nil and canonical_left == canonical_right
end

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

function M.gh_issue_view_assignees_cmd(repo, issue_number)
  return M.gh_issue_rest_view_cmd(repo, issue_number)
end

function M.github_issue_assign(repo, issue_number, login, timeout)
  return M.github().issue_assign(repo, issue_number, login, timeout or 30)
end

function M.github_issue_unassign(repo, issue_number, login, timeout)
  return M.github().issue_unassign(repo, issue_number, login, timeout or 30)
end

function M.parse_issue_assignees(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.assignee_logins(decoded.assignees)
end

local function issue_label_names(labels)
  local names = {}
  for _, label in ipairs(labels or {}) do
    local name = type(label) == "table" and label.name or label
    if name ~= nil and tostring(name) ~= "" then
      names[#names + 1] = tostring(name)
    end
  end
  return names
end

function M.parse_issue_claim_snapshot(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    assignees = M.assignee_logins(decoded.assignees),
    labels = issue_label_names(decoded.labels),
  }
end

function M.issue_claim_snapshot(repo, issue_number)
  local view = M.gh_exec(M.gh_issue_view_assignees_cmd(repo, issue_number), 30, "GitHub issue claim")
  return M.parse_issue_claim_snapshot(view.stdout)
end

function M.issue_claim_held_by_self(repo, issue_number, login)
  local logins = M.issue_claim_snapshot(repo, issue_number).assignees
  return #logins == 1 and same_login(logins[1], login)
end

local function snapshot_in_session_scope(snapshot)
  if not devloop_config.work_label_family_isolation_active() then
    return true
  end
  local scope = devloop_config.session_work_label_scope()
  return devloop_config.matches_session_work_label(snapshot and snapshot.labels, nil, scope)
end

local function snapshot_holds_claim(snapshot, claim)
  if tostring(claim and claim.mode or "assignee") == "label" then
    local expected = tostring(claim.label or "")
    for _, label in ipairs(snapshot and snapshot.labels or {}) do
      if tostring(label) == expected then
        return true
      end
    end
    return false
  end
  local logins = snapshot and snapshot.assignees or {}
  return #logins == 1 and same_login(logins[1], claim.owner)
end

local function claim_source_ref_matches(payload, repo, issue_number)
  local claim = payload and payload.claim
  local source_ref = claim and claim.source_ref
  if type(source_ref) ~= "table" or source_ref.kind ~= "external" then
    return false
  end
  return tostring(source_ref.ref or "") == tostring(repo) .. "#issue/" .. tostring(issue_number)
end

local function verify_claim_log(dept, reason, repo, issue_number, owner)
  local fields = {
    "outcome=lost",
    "reason=" .. tostring(reason),
    "repo=" .. tostring(repo),
    "issue=" .. tostring(issue_number),
  }
  if owner ~= nil and tostring(owner) ~= "" then
    table.insert(fields, "owner=" .. tostring(owner))
  end
  M.log_line("info", dept, "CLAIM", fields)
end

function M.verify_issue_claim_before_write(payload, repo, issue_number, dept)
  local claim = payload and payload.claim
  if type(claim) ~= "table" or claim.owner == nil or tostring(claim.owner) == "" then
    return true
  end
  if not claim_source_ref_matches(payload, repo, issue_number) then
    verify_claim_log(dept, "source-ref-mismatch", repo, issue_number, claim.owner)
    return false
  end
  local owner = tostring(claim.owner)
  local snapshot = M.issue_claim_snapshot(repo, issue_number)
  if not snapshot_in_session_scope(snapshot) then
    verify_claim_log(dept, "work-label-scope-lost", repo, issue_number, owner)
    return false
  end
  if snapshot_holds_claim(snapshot, claim) then
    return true
  end
  local reason = tostring(claim.mode or "assignee") == "label" and "label-claim-lost" or "assignee-claim-lost"
  verify_claim_log(dept, reason, repo, issue_number, owner)
  return false
end

function M.verify_issue_claim_in_issue(issue, payload, repo, issue_number, dept)
  local claim = payload and payload.claim
  if type(claim) ~= "table" or claim.owner == nil or tostring(claim.owner) == "" then
    return true
  end
  if not claim_source_ref_matches(payload, repo, issue_number) then
    verify_claim_log(dept, "source-ref-mismatch", repo, issue_number, claim.owner)
    return false
  end
  local snapshot = {
    assignees = M.assignee_logins(issue and issue.assignees),
    labels = issue_label_names(issue and issue.labels),
  }
  if not snapshot_in_session_scope(snapshot) then
    verify_claim_log(dept, "work-label-scope-lost", repo, issue_number, claim.owner)
    return false
  end
  if snapshot_holds_claim(snapshot, claim) then
    return true
  end
  local reason = tostring(claim.mode or "assignee") == "label" and "label-claim-lost" or "assignee-claim-lost"
  verify_claim_log(dept, reason, repo, issue_number, claim.owner)
  return false
end

end

return S
