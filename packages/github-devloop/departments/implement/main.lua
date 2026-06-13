local core = require("core")

local M = {}

local MAX_IMPLEMENT_ATTEMPTS = 2
local implemented_branch_head

M.spec = {
  consumes = { "devloop_ready", "devloop_ready_session" },
  ephemeral = { "devloop_ready_session" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_open_pr",
  },
  stall_window = "10m",
}

local function raise_impl_failed(repo, issue_number, ready, reason, detail, attempt)
  local comment_request = core.build_impl_failure_comment_request(repo, issue_number, ready, reason, detail, attempt)
  local label_request = core.build_impl_failed_label_request(repo, issue_number, ready, reason)
  local add_labels, remove_labels = core.state_label_changes("impl-failed")
  core.log_apply("implement", ready.proposal_id, "impl-failed", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, base_branch, base_sha, attempt, started_at)
  local comment_request = core.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha, base_branch, base_sha, attempt, started_at)
  local label_request = core.build_implementing_label_request(repo, issue_number, ready)
  local open_pr_payload = core.build_devloop_open_pr_payload(repo, issue_number, ready, branch, head_sha, base_branch)
  local add_labels, remove_labels = core.state_label_changes("implementing")
  core.log_apply("implement", ready.proposal_id, "implementing", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_open_pr",
  })
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  core.log_raise("implement", ready.proposal_id, "devloop_open_pr", open_pr_payload)
end

local function raise_implement_attempt(repo, issue_number, ready, attempt, started_at)
  local request = core.build_implement_attempt_comment_request(repo, issue_number, ready, attempt, started_at)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", request)
end

local function open_pr_payload_from_fact(repo, issue_number, ready, fact)
  return core.build_devloop_open_pr_payload(
    repo,
    issue_number,
    {
      proposal_id = ready.proposal_id,
      dedup_key = fact.dedup_key or ready.dedup_key,
      source_ref = ready.source_ref,
    },
    fact.branch,
    fact.head_sha,
    fact.base_branch
  )
end

local function raise_open_pr_from_fact(repo, issue_number, ready, fact, reason)
  core.log_cas_decision("implement", ready.proposal_id, {
    state = "implementing",
    version = fact.dedup_key or ready.dedup_key,
  }, "implementing", "pr-open", "applied(progress-derived)", reason)
  local payload = open_pr_payload_from_fact(repo, issue_number, ready, fact)
  core.log_apply("implement", ready.proposal_id, "implementing", fact.dedup_key or ready.dedup_key, { add = {}, remove = {} }, {
    "devloop_open_pr",
  })
  core.log_raise("implement", ready.proposal_id, "devloop_open_pr", payload)
end

local function remote_branch_fact(branch, base_branch, source_fact)
  local fetch_result = exec_sync({ cmd = core.git_fetch_branch_cmd("origin", branch), timeout = 60 })
  if fetch_result.exit_code ~= 0 then
    return nil
  end
  local head_result = exec_sync({ cmd = core.git_remote_branch_head_cmd("origin", branch), timeout = 30 })
  if head_result.exit_code ~= 0 then
    if head_result.exit_code == 1 then
      return nil
    end
    error("github-devloop: git implementing remote branch head failed: " .. tostring(head_result.stderr))
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not core.is_safe_head_sha(head_sha) then
    error("github-devloop: unsafe implementing remote branch head")
  end
  if source_fact ~= nil and source_fact.head_sha ~= nil and head_sha ~= source_fact.head_sha then
    local ancestry = exec_sync({ cmd = core.git_is_ancestor_cmd(source_fact.head_sha, head_sha), timeout = 30 })
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

local function local_branch_fact(base_head, branch, base_branch, dedup_key)
  local branch_ref = exec_sync({ cmd = core.git_show_ref_branch_cmd(branch), timeout = 30 })
  if branch_ref.exit_code ~= 0 then
    if branch_ref.exit_code == 1 then
      return nil
    end
    error("github-devloop: git branch ref check failed: " .. tostring(branch_ref.stderr))
  end
  local head_sha = implemented_branch_head(base_head, branch)
  if head_sha == nil then
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

local function ready_for_implementation_version(ready, version)
  local copy = {}
  for key, value in pairs(ready or {}) do
    copy[key] = value
  end
  copy.dedup_key = version
  return copy
end

local function raise_work_card(repo, issue_number, ready, card)
  local run_id = core.work_card_run_id({
    "implement",
    tostring(ready.dedup_key),
    "attempt",
    tostring(card.attempt or 1),
  })
  local request = core.build_work_card_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, {
    proposal_id = ready.proposal_id,
    role = "implement",
    run_id = run_id,
    version = ready.dedup_key,
    started_at = card.started_at,
    finished_at = card.finished_at,
    outcome = card.outcome,
    base_sha = card.base_sha,
    source_ref = ready.source_ref,
  })
  core.log_work_card("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", request)
end

implemented_branch_head = function(base_head, branch)
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

local function merge_integration_for_implementation(worktree, integration_branch, base_head)
  local merge_result = exec_sync({ cmd = core.git_worktree_merge_no_edit_cmd(worktree, base_head), timeout = 120 })
  if merge_result.exit_code ~= 0 then
    local unmerged_result = exec_sync({ cmd = core.git_unmerged_paths_cmd(worktree), timeout = 30 })
    if unmerged_result.exit_code ~= 0 then
      error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
    end
    if tostring(unmerged_result.stdout or "") == "" then
      error("github-devloop: git integration merge failed: " .. tostring(merge_result.stderr))
    end
    core.log_line("info", "implement", "merge-target", "MERGE_SKEW", {
      "integration_branch=" .. tostring(integration_branch),
      "integration_sha=" .. tostring(base_head),
      "reason=integration merge requires codex conflict resolution",
    })
  end
end

local function prepare_base(branches)
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
  return base_head
end

local function reconcile_worktree_to_branch(worktree, branch)
  local reset_result = exec_sync({ cmd = core.git_worktree_reset_hard_cmd(worktree, branch), timeout = 60 })
  if reset_result.exit_code ~= 0 then
    error("github-devloop: git worktree reset failed: " .. tostring(reset_result.stderr))
  end
  local clean_result = exec_sync({ cmd = core.git_worktree_clean_cmd(worktree), timeout = 60 })
  if clean_result.exit_code ~= 0 then
    error("github-devloop: git worktree clean failed: " .. tostring(clean_result.stderr))
  end
end

local function remove_stale_worktree(path)
  local dir_result = exec_sync({ cmd = core.path_is_directory_cmd(path), timeout = 30 })
  if dir_result.exit_code ~= 0 and dir_result.exit_code ~= 1 then
    error("github-devloop: git worktree path check failed: " .. tostring(dir_result.stderr))
  end
  if dir_result.exit_code == 1 then
    local prune_result = exec_sync({ cmd = core.git_worktree_prune_cmd(), timeout = 60 })
    if prune_result.exit_code ~= 0 then
      error("github-devloop: git worktree prune failed: " .. tostring(prune_result.stderr))
    end
    return
  end
  local remove_result = exec_sync({ cmd = core.git_worktree_remove_cmd(path), timeout = 60 })
  if remove_result.exit_code ~= 0 then
    error("github-devloop: git worktree remove failed: " .. tostring(remove_result.stderr))
  end
end

local function prepare_worktree(repo, issue_number, ready, branch, base_head)
  local branch_ref = exec_sync({ cmd = core.git_show_ref_branch_cmd(branch), timeout = 30 })
  local branch_exists = branch_ref.exit_code == 0
  if branch_ref.exit_code ~= 0 and branch_ref.exit_code ~= 1 then
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
    local existing_worktree = core.find_worktree_for_branch_under_runtime(list_result.stdout, branch, runtime_result.stdout)
    for _, stale_worktree in ipairs(core.find_worktrees_for_branch(list_result.stdout, branch)) do
      if not core.path_under_runtime_root(runtime_result.stdout, stale_worktree) then
        core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
          "branch=" .. tostring(branch),
          "worktree=" .. tostring(stale_worktree),
          "reason=removing non-current-runtime deterministic worktree",
        })
        remove_stale_worktree(stale_worktree)
      end
    end
    if existing_worktree ~= nil then
      worktree = existing_worktree
      core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
        "branch=" .. tostring(branch),
        "worktree=" .. tostring(worktree),
        "reason=reusing current-runtime deterministic worktree",
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
  reconcile_worktree_to_branch(worktree, branch)
  return worktree
end

local function run_attempt(repo, issue_number, ready, current, branches, branch, base_head, attempt, event_ts, event_queue)
  local worktree = prepare_worktree(repo, issue_number, ready, branch, base_head)
  merge_integration_for_implementation(worktree, branches.integration, base_head)

  local codex_started_at = now()
  raise_implement_attempt(repo, issue_number, ready, attempt, codex_started_at)
  raise_work_card(repo, issue_number, ready, {
    started_at = codex_started_at,
    base_sha = base_head,
    attempt = attempt,
  })
  core.log_codex_start("implement", ready.proposal_id, "implement")
  local content_fetch = core.context_fetch_from_bundle({
    dept = "implement",
    repo = repo,
    issue_number = issue_number,
    proposal_id = ready.proposal_id,
    version = ready.dedup_key,
    tick = event_ts,
  })
  local result = spawn_codex_sync({
    prompt = core.build_implement_prompt(ready.proposal_id, current, ready.framing, content_fetch),
    worktree = worktree,
  })

  if type(result) ~= "table" or result.exit_code ~= 0 then
    local stderr = type(result) == "table" and result.stderr or "nil result"
    core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, stderr, {
      queue = event_queue,
      source_ref = ready.source_ref,
      terminal = false,
    })
    raise_work_card(repo, issue_number, ready, {
      started_at = codex_started_at,
      finished_at = now(),
      outcome = "failed: codex-failed",
      base_sha = base_head,
      attempt = attempt,
    })
    raise_impl_failed(repo, issue_number, ready, "codex-failed", stderr, attempt)
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
      raise_work_card(repo, issue_number, ready, {
        started_at = codex_started_at,
        finished_at = now(),
        outcome = "completed",
        base_sha = base_head,
        attempt = attempt,
      })
      raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, branches.integration, base_head, attempt, codex_started_at)
      return
    end

    local detail = tostring(result.stdout or "")
    if detail == "" then
      detail = tostring(result.stderr or "")
    end
    core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, "no-changes", {
      queue = event_queue,
      source_ref = ready.source_ref,
      terminal = false,
    })
    raise_work_card(repo, issue_number, ready, {
      started_at = codex_started_at,
      finished_at = now(),
      outcome = "failed: no-changes",
      base_sha = base_head,
      attempt = attempt,
    })
    raise_impl_failed(repo, issue_number, ready, "no-changes", detail, attempt)
    return
  end

  local add_result = exec_sync({ cmd = core.git_add_all_cmd(worktree), timeout = 30 })
  if add_result.exit_code ~= 0 then
    error("github-devloop: git add failed: " .. tostring(add_result.stderr))
  end

  local commit_result = exec_sync({
    cmd = core.git_commit_cmd(worktree, core.implement_commit_subject(
      issue_number,
      core.commit_issue_subject_snapshot(repo, issue_number)
    )),
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

  raise_work_card(repo, issue_number, ready, {
    started_at = codex_started_at,
    finished_at = now(),
    outcome = "completed",
    base_sha = base_head,
    attempt = attempt,
  })
  raise_implementing(repo, issue_number, ready, worktree, branch, head_sha, branches.integration, base_head, attempt, codex_started_at)
end

function pipeline(event)
  local ready = event.payload or {}
  if not core.is_supported_ready(ready) then
    core.log_entry("implement", event, "unknown", core.payload_field(ready, "dedup_key"))
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

    local view = core.gh_exec({ cmd = core.gh_issue_view_implement_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue implement view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_implement(view.stdout)
    core.log_forged_markers("implement", ready.proposal_id, current.comments)
    local state = core.current_state(current.comments, ready.proposal_id)
    local gate = core.dependency_gate(repo, issue_number, {
      proposal_id = ready.proposal_id,
      version = ready.dedup_key,
      comments = current.comments,
    })
    if not gate.ok then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "hold-dependency-backstop", gate.reason)
      return
    end

    local branches = core.branch_config()
    local implementation_version = core.implementation_attempt_version(ready.dedup_key, ready.impl_retry_attempt)
    local branch_version = core.implementation_base_version(ready.dedup_key)
    local marker_ready = ready_for_implementation_version(ready, implementation_version)
    local branch = core.implement_branch(repo, issue_number, branch_version)

    if state.state == "implementing" then
      if tostring(state.version or "") ~= tostring(marker_ready.dedup_key or "") then
        core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "skip-stale(version-mismatch)", "ready event does not match current implementing version")
        return
      end
      local link = core.pr_link_fact(current.comments, ready.proposal_id)
      if link ~= nil and tostring(link.impl_version or "") == tostring(marker_ready.dedup_key) then
        core.log_cas_decision("implement", ready.proposal_id, state, "implementing", "pr-open", "skip-idempotent(pr-link-visible)", "linked PR fact is already visible")
        return
      end
      local fact = core.implementing_fact(current.comments, ready.proposal_id, marker_ready.dedup_key)
      local progress = nil
      if fact ~= nil then
        progress = remote_branch_fact(fact.branch, fact.base_branch, fact)
      else
        progress = remote_branch_fact(branch, branches.integration, {
          proposal_id = ready.proposal_id,
          dedup_key = marker_ready.dedup_key,
        })
      end
      if progress ~= nil then
        progress.proposal_id = ready.proposal_id
        progress.dedup_key = marker_ready.dedup_key
        raise_open_pr_from_fact(repo, issue_number, marker_ready, progress, "implementing remote branch progress is visible")
        return
      end
      local base_head = prepare_base(branches)
      local local_progress = local_branch_fact(base_head, branch, branches.integration, marker_ready.dedup_key)
      if local_progress ~= nil then
        local_progress.proposal_id = ready.proposal_id
        raise_open_pr_from_fact(repo, issue_number, marker_ready, local_progress, "local implementation branch progress is visible")
        return
      end
      local attempts = core.implement_attempt_count(current.comments, ready.proposal_id, marker_ready.dedup_key)
      if attempts >= MAX_IMPLEMENT_ATTEMPTS then
        core.log_cas_decision("implement", ready.proposal_id, state, "implementing", "impl-failed", "applied(attempts-exhausted)", "implementation attempts exhausted with no PR or branch progress")
        raise_impl_failed(repo, issue_number, marker_ready, "retry-exhausted", "No linked PR, remote branch, or local branch progress was visible after " .. tostring(attempts) .. " attempts.", attempts)
        return
      end
      core.log_cas_decision("implement", ready.proposal_id, state, "implementing", "implementing", "applied(retry-no-progress)", "no PR or branch progress is visible; retrying implementation attempt")
      return run_attempt(repo, issue_number, marker_ready, current, branches, branch, base_head, attempts + 1, event.ts, event.queue)
    end

    local retry_failure = nil
    if state.state == "impl-failed" and ready.impl_retry_attempt ~= nil and state.version == ready.dedup_key then
      retry_failure = core.impl_failure_fact(current.comments, ready.proposal_id, ready.dedup_key)
      if retry_failure ~= nil and tonumber(ready.impl_retry_attempt) <= tonumber(retry_failure.attempt or 1) then
        core.log_cas_decision("implement", ready.proposal_id, state, "impl-failed", "implementing", "skip-idempotent(retry-not-advanced)", "implementation retry event does not advance the failure attempt")
        return
      end
    elseif state.state == "implementing" or state.state == "impl-failed" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "skip-idempotent(already at to_state)", "implementation fact marker already visible")
      return
    end
    local expected_states = retry_failure ~= nil and { "impl-failed" } or { "ready" }
    local transition = core.versioned_transition_status(state, expected_states, "implementing", ready.dedup_key)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready event cannot advance current marker")
      return
    end
    local accepts_ready_hand_off = event.queue == "devloop_ready_session"
    if transition == "pending" then
      if accepts_ready_hand_off and retry_failure == nil and ready.impl_retry_attempt == nil and core.is_ready_hand_off(ready.ready_hand_off, ready) then
        core.log_cas_decision("implement", ready.proposal_id, {
          state = "ready",
          version = ready.dedup_key,
          stage_rank = core.stage_rank("ready"),
        }, "ready", "implementing", "apply(own-ready-hand-off)", "ready marker was written by the same in-band generation")
      else
        core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready state marker not yet visible")
        error("github-devloop: ready state marker not yet visible for implement; retrying")
      end
    else
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready marker visible; attempting implementation")
    end

    local wip_ok, wip_reason, wip_count, wip_max = core.wip_capacity_allows_start(repo, issue_number)
    if not wip_ok then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "hold-wip-cap", wip_reason .. ": " .. tostring(wip_count) .. "/" .. tostring(wip_max))
      return
    end

    local issue_slug = core.safe_issue_slug(repo, issue_number)
    core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
      "issue_slug=" .. tostring(issue_slug),
      "branch=" .. tostring(branch),
      "reason=implementation fact marker absent for this version",
    })

    run_attempt(repo, issue_number, marker_ready, current, branches, branch, prepare_base(branches), ready.impl_retry_attempt or 1, event.ts, event.queue)
  end)
end

pipeline = core.wrap_pipeline_failure("implement", pipeline)

return M
