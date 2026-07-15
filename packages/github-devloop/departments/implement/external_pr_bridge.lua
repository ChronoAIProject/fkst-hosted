local bridge = require("contract.external_pr_bridge")
local devloop_base = require("devloop.base")
local devloop_commands = require("devloop.commands")
local devloop_logging = require("devloop.logging")
local git_adapter = require("forge.git")
local m_claims = require("devloop.claims")
local pr_safety = require("devloop.pr_safety")

local M = {}
local git_handle = nil

local function git()
  if git_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: external-pr-bridge-git-adapter-unavailable: exec_argv is required")
    end
    git_handle = git_adapter.new(exec_argv)
  end
  return git_handle
end

local function trim(value)
  return tostring(value or ""):gsub("%s+$", "")
end

local function trusted_issue_author(current, managed)
  local author = current and current.author_login
  if m_claims.is_managed_bot_login(author, managed) then
    return true
  end
  local trusted = devloop_base.trusted_bot_login()
  return trusted ~= nil
    and trusted ~= ""
    and devloop_base.strip_bot_login_suffix(author) == tostring(trusted)
end

function M.detect(current, repo, managed)
  local body = current and current.body
  if not bridge.has_marker(body) then
    return nil
  end
  if not trusted_issue_author(current, managed) then
    error("github-devloop: external-pr-bridge-untrusted: bridge issue body marker was not authored by a trusted bot")
  end
  local marker = bridge.find_marker(body)
  if marker == nil then
    error("github-devloop: external-pr-bridge-invalid: bridge issue body marker could not be parsed")
  end
  if tostring(marker.repo or "") ~= tostring(repo or "") then
    error("github-devloop: external-pr-bridge-mismatch: bridge marker repo does not match implementation repo")
  end
  return marker
end

function M.provision(worktree, marker, proposal_id)
  if marker == nil then
    return true
  end
  local fetch = devloop_commands.git_fetch_pr_head_ref("origin", marker.pr_number, 60)
  if fetch.exit_code ~= 0 then
    error("github-devloop: external-pr-bridge-fetch-failed: git fetch external PR head failed: " .. tostring(fetch.stderr))
  end
  local head = devloop_commands.git_fetch_head_commit(30)
  if head.exit_code ~= 0 then
    error("github-devloop: external-pr-bridge-head-resolve-failed: git FETCH_HEAD resolve failed: " .. tostring(head.stderr))
  end
  local head_sha = trim(head.stdout)
  if not pr_safety.is_safe_head_sha(head_sha) then
    error("github-devloop: external-pr-bridge-head-unsafe: unsafe external PR head sha")
  end
  local merge = devloop_commands.git_worktree_merge_no_edit(worktree, head_sha, 120)
  if merge.exit_code == 0 then
    devloop_logging.log_line("info", "implement", proposal_id, "EXTERNAL_PR_BRIDGE", {
      "repo=" .. tostring(marker.repo),
      "pr=" .. tostring(marker.pr_number),
      "head_sha=" .. tostring(head_sha),
      "outcome=provisioned",
    })
    return true
  end
  local unmerged = git().unmerged_paths(worktree, 30)
  if unmerged.exit_code ~= 0 then
    error("github-devloop: external-pr-bridge-unmerged-check-failed: git unmerged path check failed: " .. tostring(unmerged.stderr))
  end
  if tostring(unmerged.stdout or "") == "" then
    error("github-devloop: external-pr-bridge-merge-failed: git external PR merge failed without unmerged paths: " .. tostring(merge.stderr))
  end
  devloop_logging.log_line("info", "implement", proposal_id, "EXTERNAL_PR_BRIDGE", {
    "repo=" .. tostring(marker.repo),
    "pr=" .. tostring(marker.pr_number),
    "head_sha=" .. tostring(head_sha),
    "outcome=conflicted",
    "reason=external PR merge requires codex conflict resolution",
  })
  return false
end

return M
