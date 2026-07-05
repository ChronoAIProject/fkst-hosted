local devloop_base = require("devloop.base")
local devloop_claims = require("devloop.claims")
local devloop_entity_view = require("devloop.github_proxy_entity_view")
local devloop_logging = require("devloop.logging")

local M = {}

M.DEPT = "workflow_materialize_next"
M.RELEASE_TIMEOUT_SECONDS = 30

local function owner()
  return devloop_claims.claim_owner()
end

local function is_self_only_assignee(core, ownership, claim_owner)
  local logins = devloop_claims.assignee_logins(ownership and ownership.assignees)
  return #logins == 1 and devloop_base.strip_bot_login_suffix(logins[1]) == tostring(claim_owner or "")
end

local function log(origin, action, reason)
  devloop_logging.log_cas_decision(M.DEPT, origin, { state = nil, version = nil }, "claim", "claim", action, reason)
end

local function github()
  if type(exec_argv) ~= "function" then
    error("github-devloop-workflow: github-adapter-missing-exec-argv: GitHub adapter requires exec_argv")
  end
  return require("forge.github").new(exec_argv)
end

local function read_ownership(core, deps, repo, issue_number)
  if type(deps) == "table" and type(deps.read_current_issue_ownership) == "function" then
    return deps.read_current_issue_ownership(core, repo, issue_number)
  end
  return devloop_claims.read_current_issue_ownership(repo, issue_number)
end

local function write_enabled(deps)
  if type(deps) == "table" and type(deps.write_enabled) == "function" then
    return deps.write_enabled()
  end
  return devloop_base.read_env("FKST_GITHUB_WRITE") == "1"
end

local function unassign(deps, repo, issue_number, claim_owner)
  if type(deps) == "table" and type(deps.issue_unassign) == "function" then
    return deps.issue_unassign(repo, issue_number, claim_owner, M.RELEASE_TIMEOUT_SECONDS)
  end
  return github().issue_unassign(repo, issue_number, claim_owner, M.RELEASE_TIMEOUT_SECONDS)
end

function M.release_done_claim(core, deps, repo, issue_number, origin)
  if type(deps) == "table" and type(deps.release_done_claim) == "function" then
    return deps.release_done_claim(core, repo, issue_number, origin)
  end

  local claim_owner = owner()
  local ownership = read_ownership(core, deps, repo, issue_number)
  if not is_self_only_assignee(core, ownership, claim_owner) then
    log(origin, "skip-release-claim-not-self", "terminal done does not remove a non-self assignee claim")
    return false
  end

  if not write_enabled(deps) then
    log(origin, "dry-run-release-claim", "terminal done would release self-only claim but FKST_GITHUB_WRITE!=1")
    return true
  end

  unassign(deps, repo, issue_number, claim_owner)
  devloop_entity_view.invalidate_entity_after_write(repo, "issue", issue_number)
  log(origin, "released-claim", "terminal done released the self-only assignee claim")
  return true
end

local function issue_close(deps, repo, issue_number)
  if type(deps) == "table" and type(deps.issue_close) == "function" then
    return deps.issue_close(repo, issue_number, M.RELEASE_TIMEOUT_SECONDS)
  end
  return github().issue_close(repo, issue_number, M.RELEASE_TIMEOUT_SECONDS)
end

-- A workflow whose every slot genuinely merged is fully implemented, so its origin
-- idea issue is closed: leaving completed origins open clutters the board. Only the
-- "done" terminal reaches here (a "blocked" terminal keeps the issue open for human
-- follow-up). The reconcile skips a non-OPEN origin (discovery skip-closed), so this
-- runs when the issue is open and is not re-attempted once the close is visible.
function M.close_done_origin(core, deps, repo, issue_number, origin)
  if type(deps) == "table" and type(deps.close_done_origin) == "function" then
    return deps.close_done_origin(core, repo, issue_number, origin)
  end

  if not write_enabled(deps) then
    log(origin, "dry-run-close-origin", "terminal done would close the completed origin issue but FKST_GITHUB_WRITE!=1")
    return true
  end

  local result = issue_close(deps, repo, issue_number)
  if type(result) == "table" and result.exit_code ~= nil and result.exit_code ~= 0 then
    error("github-devloop-workflow: workflow-origin-close-failed: workflow origin issue close failed: " .. tostring(result.stderr))
  end
  devloop_entity_view.invalidate_entity_after_write(repo, "issue", issue_number)
  log(origin, "closed-origin", "terminal done closed the completed workflow origin issue")
  return true
end

return M
