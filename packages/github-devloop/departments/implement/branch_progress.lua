local devloop_commands = require("devloop.commands")
local git_mechanics = require("devloop.git_mechanics")
local pr_safety = require("devloop.pr_safety")
local substrate_pin = require("departments.implement.substrate_pin")

local M = {}

function M.implemented_branch_head(base_head, branch)
  local ahead_result = devloop_commands.git_branch_ahead_count(base_head, branch, 30)
  if ahead_result.exit_code ~= 0 then
    error("github-devloop: branch-fact-read-failed: git branch ahead check failed: " .. tostring(ahead_result.stderr))
  end
  local ahead_count = tonumber(tostring(ahead_result.stdout or ""):match("%d+"))
  if ahead_count == nil or ahead_count <= 0 then
    return nil
  end

  local head_result = devloop_commands.git_branch_head(branch, 30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git-head-read-failed: git branch head failed: " .. tostring(head_result.stderr))
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not pr_safety.is_safe_head_sha(head_sha) then
    error("github-devloop: unsafe-head-sha: unsafe implementing branch head")
  end
  return head_sha
end

function M.remote_branch_fact(core_git, branch, base_branch, source_fact)
  local fetch_result = devloop_commands.git_fetch_branch("origin", branch, 60)
  if fetch_result.exit_code ~= 0 then
    return nil
  end
  local head_result = devloop_commands.git_remote_branch_head("origin", branch, 30)
  if head_result.exit_code ~= 0 then
    if head_result.exit_code == 1 then
      return nil
    end
    error("github-devloop: git-head-read-failed: git implementing remote branch head failed: " .. tostring(head_result.stderr))
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not pr_safety.is_safe_head_sha(head_sha) then
    error("github-devloop: unsafe-head-sha: unsafe implementing remote branch head")
  end
  if source_fact ~= nil and source_fact.head_sha ~= nil and head_sha ~= source_fact.head_sha then
    local ancestry = git_mechanics.git_is_ancestor(core_git, source_fact.head_sha, head_sha, 30)
    if ancestry.exit_code ~= 0 then
      return nil
    end
  end
  return {
    proposal_id = source_fact and source_fact.proposal_id,
    dedup_key = source_fact and source_fact.dedup_key,
    branch = branch,
    head_sha = head_sha,
    base_branch = (source_fact and source_fact.base_branch) or base_branch,
    base_sha = source_fact and source_fact.base_sha,
  }
end

function M.local_branch_fact(base_head, branch, base_branch, dedup_key)
  local branch_ref = devloop_commands.git_show_ref_branch(branch, 30)
  if branch_ref.exit_code ~= 0 then
    if branch_ref.exit_code == 1 then
      return nil
    end
    error("github-devloop: branch-ref-check-failed: git branch ref check failed: " .. tostring(branch_ref.stderr))
  end
  local head_sha = M.implemented_branch_head(base_head, branch)
  if head_sha == nil or substrate_pin.is_only_pin_delta(base_head, branch) then
    return nil
  end
  return {
    dedup_key = dedup_key,
    branch = branch,
    head_sha = head_sha,
    base_branch = base_branch,
    base_sha = base_head,
  }
end

return M
