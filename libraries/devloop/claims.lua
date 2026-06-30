local S = {}
local github_handle = nil
local error_facts = require("contract.error_facts")
local contract_time = require("contract.time")
local forks = require("devloop.forks")
local config = require("devloop.config")

function S.install(M)
local github_view = require("forge.github_view")
local function github()
  if github_handle ~= nil then
    return github_handle
  end
  if type(exec_argv) ~= "function" then
    error("github-devloop: GitHub adapter requires exec_argv")
  end
  github_handle = require("forge.github").new(exec_argv)
  return github_handle
end

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

local function issue_author_login(issue)
  if type(issue) ~= "table" then
    return nil
  end
  if issue.author_login ~= nil and tostring(issue.author_login) ~= "" then
    return tostring(issue.author_login)
  end
  if type(issue.author) == "table" and issue.author.login ~= nil and tostring(issue.author.login) ~= "" then
    return tostring(issue.author.login)
  end
  if type(issue.user) == "table" and issue.user.login ~= nil and tostring(issue.user.login) ~= "" then
    return tostring(issue.user.login)
  end
  return nil
end

function M.issue_author_login(issue)
  return issue_author_login(issue)
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

-- Single source for the claim owner: normalize the configured bot login so all
-- downstream comparisons get the bare slug regardless of whether the deployment
-- configured "<slug>" or "<slug>[bot]". No-op for ordinary user logins.
function M.claim_owner()
  return M.strip_bot_login_suffix(M.assert_trusted_bot_configured() or M.trusted_bot_login())
end

function M.managed_bot_logins(exec)
  local raw = M.read_env("FKST_DEVLOOP_MANAGED_BOT_LOGINS", exec)
  local logins = {}
  for entry in tostring(raw or ""):gmatch("[^,%s]+") do
    local login = M.strip_bot_login_suffix(M._trim(entry))
    if login ~= nil and login ~= "" then
      logins[login] = true
    end
  end
  return logins
end

function M.is_managed_bot_login(login, managed)
  local normalized = M.strip_bot_login_suffix(login)
  return normalized ~= nil and normalized ~= "" and type(managed) == "table" and managed[normalized] == true
end

local claimed_label = "fkst-dev:claimed"

function M.claimed_label()
  return claimed_label
end

-- assignee (default) ⇒ exactly today's behavior. label ⇒ opt-in GitHub App mode.
function M.claim_mode_active()
  return config.claim_mode(M)
end

-- assignee-mode (default): ownership is the current single self-assignee.
-- label-mode (opt-in): ownership is the presence of the fkst-dev:claimed label.
-- labels is optional/extra and ignored in assignee-mode, so existing 2-arg
-- callers keep byte-for-byte behavior.
function M.issue_claim_state(assignees, owner, labels)
  if config.claim_mode(M) == "label" then
    if M.has_label(labels, claimed_label) then
      return "self"
    end
    return "unassigned"
  end
  local logins = M.assignee_logins(assignees)
  if #logins == 0 then
    return "unassigned"
  end
  if #logins == 1 and M.strip_bot_login_suffix(logins[1]) == tostring(owner or "") then
    return "self"
  end
  return "other"
end

function M.is_self_owned_issue(ownership, owner)
  if type(ownership) ~= "table" then
    return false
  end
  local claim_state = M.issue_claim_state(ownership.assignees, owner, ownership.labels)
  if claim_state == "self" then
    return true
  end
  if claim_state ~= "unassigned" then
    return false
  end
  -- Unassigned+self-author is intentional for fork-and-block isolation: a different bot login sees author!=self and skips.
  local author = M.issue_author_login(ownership)
  if author == nil then
    return false
  end
  return M.strip_bot_login_suffix(author) == tostring(owner or "")
end

function M.read_current_issue_assignees(repo, issue_number)
  local ownership = M.read_current_issue_ownership(repo, issue_number)
  return M.assignee_logins(ownership and ownership.assignees)
end

local function issue_labels(decoded)
  return github_view.label_names(decoded and decoded.labels)
end

function M.read_current_issue_ownership(repo, issue_number)
  if issue_number == nil then
    return nil
  end
  local fields = "assignees,author"
  if config.claim_mode(M) == "label" then
    fields = "assignees,author,labels"
  end
  local view = github().issue_view(repo, issue_number, fields, 30)
  local decoded = json.decode(view.stdout or "{}")
  return {
    assignees = M.assignee_logins(decoded.assignees),
    author_login = M.issue_author_login(decoded),
    labels = issue_labels(decoded),
  }
end

function M.verify_issue_claim(repo, issue_number, owner)
  local ownership = M.read_current_issue_ownership(repo, issue_number)
  return M.issue_claim_state(ownership and ownership.assignees, owner, ownership and ownership.labels) == "self"
end

local function log_claim(dept, proposal_id, action, reason)
  M.log_cas_decision(dept, proposal_id, { state = nil, version = nil }, "claim", "claim", action, reason)
end

local function log_terminal_skip(dept, proposal_id, queue, source_ref, error_class, why)
  local fields = error_facts.error_fact_fields(error_class, queue, dept, why, {
    source_ref = source_ref,
    terminal = true,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  M.log_line("warn", dept, proposal_id, "SKIP", fields)
end

local function is_assign_permission_denied(err)
  return type(err) == "table" and err.class == "gh-issue-assign-permission-denied"
end

local function issue_source_ref(repo, issue_number)
  return {
    kind = "external",
    ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
  }
end

function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  if issue_number == nil then
    log_claim(dept, proposal_id, "skip-not-owned", "backing issue is absent")
    return false
  end
  local owner = M.claim_owner()
  local ownership = nil
  local current_usable
  if config.claim_mode(M) == "label" then
    -- label-mode ownership is derived from the labels projection.
    current_usable = type(current_issue) == "table" and current_issue.labels ~= nil
  else
    current_usable = type(current_issue) == "table"
      and current_issue.assignees ~= nil
      and M.issue_author_login(current_issue) ~= nil
  end
  if current_usable then
    ownership = current_issue
  else
    ownership = M.read_current_issue_ownership(repo, issue_number)
  end
  if M.is_self_owned_issue(ownership, owner) then
    return true
  end
  local status = M.issue_claim_state(ownership and ownership.assignees, owner, ownership and ownership.labels)
  if status == "other" then
    log_claim(dept, proposal_id, "skip-claimed-by-other", "backing issue assignee claim is held by another login")
  else
    log_claim(dept, proposal_id, "skip-not-owned", "backing issue is not self-owned")
  end
  return false
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
    return false, "fork-grace-age-unknown", nil
  end

  local created_seconds = contract_time.iso_timestamp_epoch_seconds(current and (current.created_at or current.createdAt))
  if created_seconds == nil then
    return false, "fork-grace-age-unknown", nil
  end

  local age_seconds = current_seconds - created_seconds
  if age_seconds < 0 then
    age_seconds = 0
  end

  if age_seconds < grace then
    return false, "fork-grace-pending", age_seconds
  end
  return true, "fork-grace-elapsed", age_seconds
end

function M.claim_issue_for_management(dept, repo, issue_number, current, proposal_id)
  local owner = M.claim_owner()
  local status = M.issue_claim_state(current and current.assignees, owner, current and current.labels)
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
  author = M.strip_bot_login_suffix(author)
  -- Fork-and-block isolation (grace + fork of other-authored issues) is an
  -- assignee-mode policy: it keeps an assignee-claim bot from intruding on a
  -- human's issue. In label-mode the loop is single-tenant and explicitly
  -- opts issues in via the fkst-dev:enabled label, so it claims directly
  -- (matching the label-claim fork). Assignee-mode keeps the original behavior.
  if config.claim_mode(M) ~= "label" and author ~= owner then
    local managed = M.managed_bot_logins()
    if M.is_managed_bot_login(author, managed) then
      log_claim(dept, proposal_id, "skip-fork-peer-bot", "other-authored unassigned issue belongs to a managed bot login")
      return false
    end
    local dedup_key = forks.fork_issue_dedup_key(repo, issue_number)
    if forks.has_trusted_issue_create_parent_marker(M, current and current.comments, dedup_key, owner, managed) then
      log_claim(dept, proposal_id, "fork-present", "trusted fork issue-create ledger marker already exists")
      return false
    end
    local grace_seconds = M.fork_grace_seconds()
    local elapsed, grace_reason, age_seconds = M.fork_grace_elapsed(repo, issue_number, current, now(), grace_seconds)
    if not elapsed then
      local reason = "other-authored unassigned issue is inside fork grace window"
        .. " reason=" .. tostring(grace_reason)
        .. " age_seconds=" .. tostring(age_seconds or "unknown")
        .. " grace_seconds=" .. tostring(grace_seconds)
      log_claim(dept, proposal_id, "skip-fork-grace", reason)
      return false
    end
    current = forks.rederive_issue_state(M, repo, issue_number)
    local request, request_reason = forks.build_fork_issue_create_request(M, repo, issue_number, current, M.issue_source_ref(repo, issue_number))
    if request == nil then
      log_claim(dept, proposal_id, "skip-fork-" .. tostring(request_reason or "invalid"), "fork request could not be built from current issue")
      return false
    end
    if forks.has_trusted_issue_create_parent_marker(M, current and current.comments, request.dedup_key, owner, managed) then
      log_claim(dept, proposal_id, "fork-present", "trusted fork issue-create ledger marker already exists")
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

  if config.claim_mode(M) == "label" then
    github().issue_add_label(repo, issue_number, claimed_label, 30)
    M.invalidate_entity_after_write(repo, "issue", issue_number)
    if M.verify_issue_claim(repo, issue_number, owner) then
      log_claim(dept, proposal_id, "claim-won", "label claim verified after add-label")
      return true
    end

    github().issue_remove_label(repo, issue_number, claimed_label, 30)
    M.invalidate_entity_after_write(repo, "issue", issue_number)
    log_claim(dept, proposal_id, "claim-lost", "label claim lost after add-label verification")
    return false
  end

  local assigned, assign_error = pcall(function()
    return github().issue_assign(repo, issue_number, owner, 30)
  end)
  if not assigned then
    if is_assign_permission_denied(assign_error) then
      local why = "assign permission-denied is permanent"
      log_terminal_skip(dept, proposal_id, "claim", issue_source_ref(repo, issue_number), "intake-skip-unclaimable", why)
      log_claim(dept, proposal_id, "skip-claim-permission-denied", why)
      return false
    end
    error(assign_error, 0)
  end
  M.invalidate_entity_after_write(repo, "issue", issue_number)
  if M.verify_issue_claim(repo, issue_number, owner) then
    log_claim(dept, proposal_id, "claim-won", "assignee claim verified after assign")
    return true
  end

  github().issue_unassign(repo, issue_number, owner, 30)
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
  -- github-proxy's pre-write guard verifies the attached claim against the
  -- issue's ASSIGNEES. In label-mode the owner is a GitHub App, which holds the
  -- fkst-dev:claimed label but is never an assignee, so an attached assignee
  -- claim would always read as "lost" and block every write. Ownership in
  -- label-mode is instead verified at claim time (claim_issue_for_management),
  -- so skip attaching the assignee claim and let github-proxy's no-claim path
  -- proceed. Assignee-mode is unchanged.
  if config.claim_mode(M) == "label" then
    return payload
  end
  payload.claim = M.claim_required_payload(source_ref or payload.source_ref)
  return payload
end

end

return S
