local devloop_base = require("devloop.base")
local default_git = require("forge.git").new(function(...) return exec_argv(...) end)
local devloop_logging = require("devloop.logging")
local devloop_commands = require("devloop.commands")
local exec_sync = exec_sync

local M = {}

local function git_or_default(git)
  return git or default_git
end

local function ensure_worktree_parent(worktree)
  local parent = devloop_commands.worktree_parent_dir(worktree)
  local result = exec_sync({ cmd = devloop_base.mkdir_p_cmd(parent), timeout = 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: worktree-parent-setup-failed: directory setup failed: " .. tostring(result.stderr))
  end
end

local function force_clean(git, worktree, timeout)
  git.worktree_remove(worktree, timeout)
  return git.worktree_prune(timeout)
end

local function has_recoverable_changes(git, worktree)
  local status = git.status_porcelain(worktree, 30)
  if status.exit_code ~= 0 then
    error("github-devloop: worktree-status-failed: git worktree status failed: " .. tostring(status.stderr))
  end
  return tostring(status.stdout or "") ~= ""
end

function M.prepare_base(branches, scoped_git)
  local git = git_or_default(scoped_git)
  local fetch_result = git.fetch_branch("origin", branches.integration, 60)
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: integration-branch-fetch-failed: git integration branch fetch failed: " .. tostring(fetch_result.stderr))
  end
  local base_result = git.remote_branch_head("origin", branches.integration, 30)
  if base_result.exit_code ~= 0 then
    error("github-devloop: git-head-read-failed: git integration branch head failed: " .. tostring(base_result.stderr))
  end
  local base_head = tostring(base_result.stdout or ""):gsub("%s+$", "")
  if not require("devloop.pr_safety").is_safe_head_sha(base_head) then
    error("github-devloop: unsafe-head-sha: unsafe base head")
  end
  return base_head
end

function M.reconcile_worktree_to_branch(worktree, branch, scoped_git)
  local git = git_or_default(scoped_git)
  local reset_result = git.reset_hard_branch(worktree, branch, 60)
  if reset_result.exit_code ~= 0 then
    error("github-devloop: worktree-reset-failed: git worktree reset failed: " .. tostring(reset_result.stderr))
  end
  local clean_result = git.clean_fd(worktree, 60)
  if clean_result.exit_code ~= 0 then
    error("github-devloop: worktree-clean-failed: git worktree clean failed: " .. tostring(clean_result.stderr))
  end
end

function M.remove_stale_worktree(path, scoped_git)
  local git = git_or_default(scoped_git)
  local dir_result = exec_sync({ cmd = devloop_commands.path_is_directory_cmd(path), timeout = 30 })
  if dir_result.exit_code ~= 0 and dir_result.exit_code ~= 1 then
    error("github-devloop: worktree-path-check-failed: git worktree path check failed: " .. tostring(dir_result.stderr))
  end
  if dir_result.exit_code == 1 then
    local prune_result = git.worktree_prune(60)
    if prune_result.exit_code ~= 0 then
      error("github-devloop: worktree-prune-failed: git worktree prune failed: " .. tostring(prune_result.stderr))
    end
    return
  end
  local remove_result = git.worktree_remove(path, 60)
  if remove_result.exit_code ~= 0 then
    error("github-devloop: worktree-remove-failed: git worktree remove failed: " .. tostring(remove_result.stderr))
  end
end

function M.prepare_worktree(repo, issue_number, ready, branch, base_head, scoped_git)
  local git = git_or_default(scoped_git)
  local branch_ref = git.show_ref_branch_quiet(branch, 30)
  local branch_exists = branch_ref.exit_code == 0
  if branch_ref.exit_code ~= 0 and branch_ref.exit_code ~= 1 then
    error("github-devloop: branch-ref-check-failed: git branch ref check failed: " .. tostring(branch_ref.stderr))
  end

  local runtime_result = exec_sync({ cmd = devloop_commands.read_runtime_root_cmd(), timeout = 30 })
  if runtime_result.exit_code ~= 0 then
    error("github-devloop: runtime-root-read-failed: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
  end
  local worktree = devloop_base.implement_worktree_path(runtime_result.stdout, repo, issue_number, ready.dedup_key)
  if branch_exists then
    local list_result = git.worktree_list(30)
    if list_result.exit_code ~= 0 then
      error("github-devloop: worktree-list-failed: git worktree list failed: " .. tostring(list_result.stderr))
    end
    local existing_worktree = devloop_commands.find_worktree_for_branch_under_runtime(list_result.stdout, branch, runtime_result.stdout)
    if existing_worktree ~= nil and has_recoverable_changes(git, existing_worktree) then
      devloop_logging.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
        "branch=" .. tostring(branch),
        "worktree=" .. tostring(existing_worktree),
        "reason=recovering dirty current-runtime deterministic worktree",
      })
      return existing_worktree, true
    end
    for _, stale_worktree in ipairs(devloop_commands.find_worktrees_for_branch(list_result.stdout, branch)) do
      if not devloop_base.path_under_runtime_root(runtime_result.stdout, stale_worktree) then
        devloop_logging.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
          "branch=" .. tostring(branch),
          "worktree=" .. tostring(stale_worktree),
          "reason=removing non-current-runtime deterministic worktree",
        })
        M.remove_stale_worktree(stale_worktree, git)
      end
    end
    if existing_worktree ~= nil then
      worktree = existing_worktree
      devloop_logging.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
        "branch=" .. tostring(branch),
        "worktree=" .. tostring(worktree),
        "reason=reusing current-runtime deterministic worktree",
      })
    else
      local clean_result = force_clean(git, worktree, 60)
      if clean_result.exit_code ~= 0 then
        error("github-devloop: worktree-cleanup-failed: git worktree cleanup failed: " .. tostring(clean_result.stderr))
      end
      ensure_worktree_parent(worktree)
      local worktree_result = git.worktree_add_existing_branch(worktree, branch, 60)
      if worktree_result.exit_code ~= 0 then
        error("github-devloop: git-worktree-add-failed: git worktree add failed: " .. tostring(worktree_result.stderr))
      end
    end
  else
    local clean_result = force_clean(git, worktree, 60)
    if clean_result.exit_code ~= 0 then
      error("github-devloop: worktree-cleanup-failed: git worktree cleanup failed: " .. tostring(clean_result.stderr))
    end
    ensure_worktree_parent(worktree)
    local worktree_result = git.worktree_add_new_branch(worktree, branch, base_head, 60)
    if worktree_result.exit_code ~= 0 then
      error("github-devloop: git-worktree-add-failed: git worktree add failed: " .. tostring(worktree_result.stderr))
    end
  end
  M.reconcile_worktree_to_branch(worktree, branch, git)
  return worktree, false
end

function M.prepare_worktree_from_base(repo, issue_number, ready, branch, base_head, scoped_git)
  local git = git_or_default(scoped_git)
  local runtime_result = exec_sync({ cmd = devloop_commands.read_runtime_root_cmd(), timeout = 30 })
  if runtime_result.exit_code ~= 0 then
    error("github-devloop: runtime-root-read-failed: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
  end
  local runtime_root = runtime_result.stdout
  local worktree = devloop_base.implement_worktree_path(runtime_root, repo, issue_number, ready.dedup_key)
  local list_result = git.worktree_list(30)
  if list_result.exit_code ~= 0 then
    error("github-devloop: worktree-list-failed: git worktree list failed: " .. tostring(list_result.stderr))
  end
  local existing_worktree = devloop_commands.find_worktree_for_branch_under_runtime(list_result.stdout, branch, runtime_root)
  if existing_worktree ~= nil and has_recoverable_changes(git, existing_worktree) then
    devloop_logging.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
      "branch=" .. tostring(branch),
      "worktree=" .. tostring(existing_worktree),
      "reason=recovering dirty current-runtime deterministic worktree before external PR reprovisioning",
    })
    return existing_worktree, true
  end
  for _, stale_worktree in ipairs(devloop_commands.find_worktrees_for_branch(list_result.stdout, branch)) do
    devloop_logging.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
      "branch=" .. tostring(branch),
      "worktree=" .. tostring(stale_worktree),
      "reason=removing existing deterministic worktree before external PR provisioning",
    })
    M.remove_stale_worktree(stale_worktree, git)
  end
  local clean_result = force_clean(git, worktree, 60)
  if clean_result.exit_code ~= 0 then
    error("github-devloop: worktree-cleanup-failed: git worktree cleanup failed: " .. tostring(clean_result.stderr))
  end
  ensure_worktree_parent(worktree)
  local worktree_result = git.worktree_add_reset_branch(worktree, branch, base_head, 60)
  if worktree_result.exit_code ~= 0 then
    error("github-devloop: git-worktree-add-failed: git worktree reset add failed: " .. tostring(worktree_result.stderr))
  end
  return worktree, false
end

return M
