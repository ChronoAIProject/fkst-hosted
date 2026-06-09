local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_ready" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  stall_window = "10m",
}

local function raise_impl_failed(repo, issue_number, ready, reason, detail)
  local comment_request = core.build_impl_failure_comment_request(repo, issue_number, ready, reason, detail)
  local label_request = core.build_impl_failed_label_request(repo, issue_number, ready, reason)
  local add_labels, remove_labels = core.state_label_changes("impl-failed")
  core.log_apply("implement", ready.proposal_id, "impl-failed", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, base_branch, base_sha)
  local comment_request = core.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha, base_branch, base_sha)
  local label_request = core.build_implementing_label_request(repo, issue_number, ready)
  local add_labels, remove_labels = core.state_label_changes("implementing")
  core.log_apply("implement", ready.proposal_id, "implementing", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function implemented_branch_head(base_head, branch)
  local ahead_result = exec_sync({ cmd = core.git_branch_ahead_count_cmd(base_head, branch), timeout = 30 })
  if ahead_result.exit_code ~= 0 then
    error("github-devloop: git branch ahead check failed: " .. tostring(ahead_result.stderr))
  end
  local ahead_count = tonumber(tostring(ahead_result.stdout or ""):match("%d+"))
  if ahead_count == nil or ahead_count <= 0 then
    return nil
  end

  local head_result = exec_sync({ cmd = core.git_branch_head_cmd(branch), timeout = 30 })
  if head_result.exit_code ~= 0 then
    error("github-devloop: git branch head failed: " .. tostring(head_result.stderr))
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not core.is_safe_head_sha(head_sha) then
    error("github-devloop: unsafe implementing branch head")
  end
  return head_sha
end

function pipeline(event)
  local ready = event.payload or {}
  if not core.is_supported_ready(ready) then
    core.log_entry("implement", event, "unknown", ready.dedup_key)
    core.log_cas_decision("implement", "unknown", { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("implement", event, ready.proposal_id, ready.dedup_key)
  local repo, issue_number = core.parse_proposal_id(ready.proposal_id)
  if repo == nil then
    core.log_cas_decision("implement", ready.proposal_id, { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.implement_lock_key(ready.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("implement", ready.proposal_id, { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()

    local view = exec_sync({ cmd = core.gh_issue_view_implement_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue implement view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_implement(view.stdout)
    core.log_forged_markers("implement", ready.proposal_id, current.comments)
    local state = core.current_state(current.comments, ready.proposal_id)
    if state.state == "implementing" or state.state == "impl-failed" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "skip-idempotent(already at to_state)", "implementation fact marker already visible")
      return
    end
    local transition = core.versioned_transition_status(state, { "ready" }, "implementing", ready.dedup_key)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready event cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready state marker not yet visible")
      error("github-devloop: ready state marker not yet visible for implement; retrying")
    end
    core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready marker visible; attempting implementation")

    local issue_slug = core.safe_issue_slug(repo, issue_number)
    local branch = core.implement_branch(repo, issue_number, ready.dedup_key)
    core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
      "issue_slug=" .. tostring(issue_slug),
      "branch=" .. tostring(branch),
      "reason=implementation fact marker absent for this version",
    })

    local fetch_result = exec_sync({ cmd = core.git_fetch_branch_cmd("origin", branches.integration), timeout = 60 })
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git integration branch fetch failed: " .. tostring(fetch_result.stderr))
    end
    local base_result = exec_sync({ cmd = core.git_remote_branch_head_cmd("origin", branches.integration), timeout = 30 })
    if base_result.exit_code ~= 0 then
      error("github-devloop: git integration branch head failed: " .. tostring(base_result.stderr))
    end
    local base_head = tostring(base_result.stdout or ""):gsub("%s+$", "")
    if not core.is_safe_head_sha(base_head) then
      error("github-devloop: unsafe base head")
    end

    local branch_ref = exec_sync({ cmd = core.git_show_ref_branch_cmd(branch), timeout = 30 })
    local branch_exists = branch_ref.exit_code == 0
    if branch_exists then
      local head_sha = implemented_branch_head(base_head, branch)
      if head_sha ~= nil then
        core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
          "branch=" .. tostring(branch),
          "head_sha=" .. tostring(head_sha),
          "reason=reusing existing implementation branch",
        })
        raise_implementing(repo, issue_number, ready, "(existing deterministic branch)", branch, head_sha, branches.integration, base_head)
        return
      end
    elseif branch_ref.exit_code ~= 1 then
      error("github-devloop: git branch ref check failed: " .. tostring(branch_ref.stderr))
    end

    local runtime_result = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
    if runtime_result.exit_code ~= 0 then
      error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
    end
    local worktree = core.implement_worktree_path(runtime_result.stdout, repo, issue_number, ready.dedup_key)
    if branch_exists then
      local list_result = exec_sync({ cmd = core.git_worktree_list_cmd(), timeout = 30 })
      if list_result.exit_code ~= 0 then
        error("github-devloop: git worktree list failed: " .. tostring(list_result.stderr))
      end
      local existing_worktree = core.find_worktree_for_branch(list_result.stdout, branch)
      if existing_worktree ~= nil then
        worktree = existing_worktree
        core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
          "branch=" .. tostring(branch),
          "worktree=" .. tostring(worktree),
          "reason=reusing existing deterministic worktree",
        })
      else
        local worktree_result = exec_sync({ cmd = core.git_worktree_add_existing_branch_cmd(worktree, branch), timeout = 60 })
        if worktree_result.exit_code ~= 0 then
          error("github-devloop: git worktree add failed: " .. tostring(worktree_result.stderr))
        end
      end
    else
      local worktree_result = exec_sync({ cmd = core.git_worktree_add_new_branch_cmd(worktree, branch, base_head), timeout = 60 })
      if worktree_result.exit_code ~= 0 then
        error("github-devloop: git worktree add failed: " .. tostring(worktree_result.stderr))
      end
    end

    core.log_codex_start("implement", ready.proposal_id, "implement")
    local result = spawn_codex_sync({
      prompt = core.build_implement_prompt(ready.proposal_id, current, ready.framing),
      worktree = worktree,
      stall_window = M.spec.stall_window,
    })

    if type(result) ~= "table" or result.exit_code ~= 0 then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, stderr)
      raise_impl_failed(repo, issue_number, ready, "codex-failed", stderr)
      return
    end
    core.log_codex_result("implement", ready.proposal_id, "implement", result, "result=completed", nil)

    local status = exec_sync({ cmd = core.git_status_cmd(worktree), timeout = 30 })
    if status.exit_code ~= 0 then
      error("github-devloop: git status failed: " .. tostring(status.stderr))
    end

    if tostring(status.stdout or "") == "" then
      local head_sha = implemented_branch_head(base_head, branch)
      if head_sha ~= nil then
        core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
          "branch=" .. tostring(branch),
          "head_sha=" .. tostring(head_sha),
          "reason=reusing clean ahead implementation branch",
        })
        raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, branches.integration, base_head)
        return
      end

      local detail = tostring(result.stdout or "")
      if detail == "" then
        detail = tostring(result.stderr or "")
      end
      core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, "no-changes")
      raise_impl_failed(repo, issue_number, ready, "no-changes", detail)
      return
    end

    local add_result = exec_sync({ cmd = core.git_add_all_cmd(worktree), timeout = 30 })
    if add_result.exit_code ~= 0 then
      error("github-devloop: git add failed: " .. tostring(add_result.stderr))
    end

    local commit_result = exec_sync({
      cmd = core.git_commit_cmd(worktree, "Implement github-devloop ready state"),
      timeout = 60,
    })
    if commit_result.exit_code ~= 0 then
      error("github-devloop: git commit failed: " .. tostring(commit_result.stderr))
    end

    local branch_result = exec_sync({ cmd = core.git_current_branch_cmd(worktree), timeout = 30 })
    if branch_result.exit_code ~= 0 then
      error("github-devloop: git branch fact failed: " .. tostring(branch_result.stderr))
    end
    local actual_branch = tostring(branch_result.stdout or ""):gsub("%s+$", "")
    if actual_branch ~= branch then
      error("github-devloop: deterministic implementing branch mismatch")
    end
    if not core.is_safe_branch(branch) then
      error("github-devloop: unsafe implementing branch")
    end

    local head_result = exec_sync({ cmd = core.git_head_sha_cmd(worktree), timeout = 30 })
    if head_result.exit_code ~= 0 then
      error("github-devloop: git head fact failed: " .. tostring(head_result.stderr))
    end
    local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
    if not core.is_safe_head_sha(head_sha) then
      error("github-devloop: unsafe implementing head_sha")
    end

    raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, branches.integration, base_head)
  end)
end

return M
