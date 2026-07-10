local devloop_base = require("devloop.base")
local m_mq = require("devloop.merge_queue")
local devloop_logging = require("devloop.logging")
local devloop_commands = require("devloop.commands")
local git_adapter = require("forge.git")

local M = {}

function M.make(core)
  local git = git_adapter.production_handle
  local function branch_worktree(repo, issue_number, version, branch)
    local runtime_result = exec_sync({ cmd = devloop_commands.read_runtime_root_cmd(), timeout = 30 })
    if runtime_result.exit_code ~= 0 then
      error("github-devloop: runtime-root-read-failed: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
    end
    local runtime_root = runtime_result.stdout
    local worktree = devloop_base.implement_worktree_path(runtime_root, repo, issue_number, version)
    local list_result = devloop_commands.git_worktree_list(30)
    if list_result.exit_code ~= 0 then
      error("github-devloop: git-worktree-list-failed: git worktree list failed: " .. tostring(list_result.stderr))
    end
    local existing = devloop_commands.find_worktree_for_branch(list_result.stdout, branch)
    if existing ~= nil then
      local dir_result = exec_sync({ cmd = devloop_commands.path_is_directory_cmd(existing), timeout = 30 })
      if dir_result.exit_code ~= 0 and dir_result.exit_code ~= 1 then
        error("github-devloop: worktree-path-check-failed: git worktree path check failed: " .. tostring(dir_result.stderr))
      end
      if dir_result.exit_code == 0 and devloop_base.path_under_runtime_root(runtime_root, existing) then
        return existing
      end
      if dir_result.exit_code == 1 then
        local prune_result = devloop_commands.git_worktree_prune(60)
        if prune_result.exit_code ~= 0 then
          error("github-devloop: git-worktree-prune-failed: git worktree prune failed: " .. tostring(prune_result.stderr))
        end
      else
        local remove_result = git("github-devloop").worktree_remove(existing, 60)
        if remove_result.exit_code ~= 0 then
          error("github-devloop: git-worktree-remove-failed: git worktree remove failed: " .. tostring(remove_result.stderr))
        end
      end
    end

    local fetch_result = devloop_commands.git_fetch_branch("origin", branch, 60)
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git-pr-head-branch-fetch-failed: git PR head branch fetch failed: " .. tostring(fetch_result.stderr))
    end
    local add_result = devloop_commands.git_worktree_add_remote_branch(worktree, "origin", branch, existing ~= nil, 60)
    if add_result.exit_code ~= 0 then
      error("github-devloop: git-worktree-add-failed: git worktree add failed: " .. tostring(add_result.stderr))
    end
    return worktree
  end

  local function fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
    if expected_baseline_sha == nil then
      return nil
    end
    local fetch_result = devloop_commands.git_fetch_pr_merge_ref("origin", pr_number, 60)
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git-pr-merge-ref-fetch-failed: git PR merge ref fetch failed: " .. tostring(fetch_result.stderr))
    end
    local head_result = devloop_commands.git_fetch_head_commit(30)
    if head_result.exit_code ~= 0 then
      error("github-devloop: git-pr-merge-ref-head-failed: git PR merge ref head failed: " .. tostring(head_result.stderr))
    end
    local merge_product_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
    if merge_product_sha ~= expected_baseline_sha then
      error("github-devloop: merge-ref-baseline-mismatch: PR merge ref head does not match merge-gate baseline")
    end
    return merge_product_sha
  end

  local function merge_integration_for_fix(worktree, pr_number, integration_branch, expected_baseline_sha, merge_gate_reason)
    if core.merge_gate_reason_requires_pr_merge_product(merge_gate_reason) then
      fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
    end
    local base_head = expected_baseline_sha
    if base_head == nil then
      local fetch_result = devloop_commands.git_fetch_branch("origin", integration_branch, 60)
      if fetch_result.exit_code ~= 0 then
        error("github-devloop: git-integration-branch-fetch-failed: git integration branch fetch failed: " .. tostring(fetch_result.stderr))
      end
      local base_result = devloop_commands.git_remote_branch_head("origin", integration_branch, 30)
      if base_result.exit_code ~= 0 then
        error("github-devloop: git-integration-branch-head-failed: git integration branch head failed: " .. tostring(base_result.stderr))
      end
      base_head = tostring(base_result.stdout or ""):gsub("%s+$", "")
    end
    if not require("devloop.pr_safety").is_safe_head_sha(base_head) then
      error("github-devloop: integration-head-unsafe: unsafe integration head")
    end
    local result = {
      target_branch = integration_branch,
      target_sha = base_head,
      conflicted = false,
      unmerged_paths = "",
    }
    local merge_result = devloop_commands.git_worktree_merge_no_edit(worktree, base_head, 120)
    if merge_result.exit_code ~= 0 then
      local unmerged_result = git("github-devloop").unmerged_paths(worktree, 30)
      if unmerged_result.exit_code ~= 0 then
        error("github-devloop: git-unmerged-path-check-failed: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
      end
      if tostring(unmerged_result.stdout or "") == "" then
        error("github-devloop: git-integration-merge-failed: git integration merge failed: " .. tostring(merge_result.stderr))
      end
      result.conflicted = true
      result.unmerged_paths = tostring(unmerged_result.stdout or "")
      devloop_logging.log_line("info", "fix", "merge-target", "MERGE_SKEW", {
        "integration_branch=" .. tostring(integration_branch),
        "integration_sha=" .. tostring(base_head),
        "reason=integration merge requires codex conflict resolution",
      })
    end
    return result
  end

  local function merge_result_context(target_branch, target_sha)
    return {
      target_branch = target_branch,
      target_sha = target_sha,
      conflicted = false,
      unmerged_paths = "",
    }
  end

  local function append_unmerged_paths(left, right)
    if tostring(left or "") == "" then
      return tostring(right or "")
    end
    if tostring(right or "") == "" then
      return tostring(left or "")
    end
    return tostring(left) .. "\n" .. tostring(right)
  end

  local function merge_sha_for_fix(worktree, sha, context, log_values)
    local merge_result = devloop_commands.git_worktree_merge_no_edit(worktree, sha, 120)
    if merge_result.exit_code == 0 then
      return context
    end
    local unmerged_result = git("github-devloop").unmerged_paths(worktree, 30)
    if unmerged_result.exit_code ~= 0 then
      error("github-devloop: git-unmerged-path-check-failed: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
    end
    if tostring(unmerged_result.stdout or "") == "" then
      error("github-devloop: git-target-merge-failed: git target merge failed: " .. tostring(merge_result.stderr))
    end
    context.conflicted = true
    context.unmerged_paths = append_unmerged_paths(context.unmerged_paths, unmerged_result.stdout)
    devloop_logging.log_line("info", "fix", "merge-target", "MERGE_SKEW", log_values)
    return context
  end

  local function fetch_verified_pr_head(pr_number, expected_head_sha)
    local fetch_result = devloop_commands.git_fetch_pr_head_ref("origin", pr_number, 60)
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git-pr-head-ref-fetch-failed: git PR head ref fetch failed: " .. tostring(fetch_result.stderr))
    end
    local head_result = devloop_commands.git_fetch_head_commit(30)
    if head_result.exit_code ~= 0 then
      error("github-devloop: git-pr-head-ref-head-failed: git PR head ref head failed: " .. tostring(head_result.stderr))
    end
    local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
    if head_sha ~= tostring(expected_head_sha) then
      error("github-devloop: pr-head-predecessor-mismatch: PR head ref does not match merge queue predecessor")
    end
    return head_sha
  end

  local function current_predecessors_for_fix(repo, integration_branch, fix, current_pr)
    local predecessors, reason = m_mq.merge_queue_predecessors(core, repo, integration_branch, {
      pr_number = fix.pr_number,
      pr = current_pr,
    })
    if predecessors == nil then
      return nil, reason
    end
    return predecessors, m_mq.merge_queue_predecessor_set(predecessors)
  end

  local function merge_predecessor_entries_for_fix(worktree, integration_branch, predecessors, current_set)
    if #predecessors == 0 then
      return nil, "not-speculative"
    end
    local context = merge_result_context("speculative:" .. integration_branch, current_set)
    for _, predecessor in ipairs(predecessors) do
      if not require("devloop.pr_safety").is_safe_head_sha(predecessor.head_sha) then
        error("github-devloop: speculative-predecessor-head-unsafe: unsafe speculative predecessor head")
      end
      local predecessor_head = fetch_verified_pr_head(predecessor.pr_number, predecessor.head_sha)
      context = merge_sha_for_fix(worktree, predecessor_head, context, {
        "integration_branch=" .. tostring(integration_branch),
        "predecessor_pr=" .. tostring(predecessor.pr_number),
        "predecessor_head=" .. tostring(predecessor_head),
        "reason=speculative predecessor merge requires codex conflict resolution",
      })
    end
    return context, "ok"
  end

  local function merge_speculative_predecessors_for_fix(worktree, repo, integration_branch, fix, current_pr)
    if fix.predecessor_set == nil then
      return nil, "not-speculative"
    end
    local predecessors, current_set = current_predecessors_for_fix(repo, integration_branch, fix, current_pr)
    if predecessors == nil then
      return nil, current_set
    end
    if tostring(current_set) ~= tostring(fix.predecessor_set) then
      return nil, "predecessor-set-mismatch", current_set
    end
    return merge_predecessor_entries_for_fix(worktree, integration_branch, predecessors, current_set)
  end

  local function assert_no_unmerged_paths(worktree)
    local unmerged_result = git("github-devloop").unmerged_paths(worktree, 30)
    if unmerged_result.exit_code ~= 0 then
      error("github-devloop: git-unmerged-path-check-failed: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
    end
    if tostring(unmerged_result.stdout or "") ~= "" then
      error("github-devloop: unresolved-merge-conflicts: fix left target merge conflicts unresolved")
    end
  end

  local function assert_no_conflict_markers(worktree)
    local markers_result = git("github-devloop").conflict_markers(worktree, 30)
    if markers_result.exit_code == 1 then
      return
    end
    if markers_result.exit_code == 0 then
      error("github-devloop: unresolved-conflict-markers: fix left conflict markers unresolved")
    end
    error("github-devloop: git-conflict-marker-check-failed: git conflict marker check failed: " .. tostring(markers_result.stderr))
  end

  local function bounded_fix_summary(value)
    local text = tostring(value or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
    if #text > 600 then
      text = text:sub(1, 600)
    end
    return text
  end

  return {
    branch_worktree = branch_worktree,
    merge_integration_for_fix = merge_integration_for_fix,
    current_predecessors_for_fix = current_predecessors_for_fix,
    merge_predecessor_entries_for_fix = merge_predecessor_entries_for_fix,
    merge_speculative_predecessors_for_fix = merge_speculative_predecessors_for_fix,
    assert_no_unmerged_paths = assert_no_unmerged_paths,
    assert_no_conflict_markers = assert_no_conflict_markers,
    bounded_fix_summary = bounded_fix_summary,
  }
end

return M
