local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_fixing" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_review_meta",
  },
  stall_window = "10m",
}

local function branch_worktree(repo, issue_number, version, branch)
  local runtime_result = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
  if runtime_result.exit_code ~= 0 then
    error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
  end
  local runtime_root = runtime_result.stdout
  local worktree = core.implement_worktree_path(runtime_root, repo, issue_number, version)
  local list_result = exec_sync({ cmd = core.git_worktree_list_cmd(), timeout = 30 })
  if list_result.exit_code ~= 0 then
    error("github-devloop: git worktree list failed: " .. tostring(list_result.stderr))
  end
  local existing = core.find_worktree_for_branch(list_result.stdout, branch)
  if existing ~= nil then
    local dir_result = exec_sync({ cmd = core.path_is_directory_cmd(existing), timeout = 30 })
    if dir_result.exit_code ~= 0 and dir_result.exit_code ~= 1 then
      error("github-devloop: git worktree path check failed: " .. tostring(dir_result.stderr))
    end
    if dir_result.exit_code == 0 and core.path_under_runtime_root(runtime_root, existing) then
      return existing
    end
    if dir_result.exit_code == 1 then
      local prune_result = exec_sync({ cmd = core.git_worktree_prune_cmd(), timeout = 60 })
      if prune_result.exit_code ~= 0 then
        error("github-devloop: git worktree prune failed: " .. tostring(prune_result.stderr))
      end
    else
      local remove_result = exec_sync({ cmd = core.git_worktree_remove_cmd(existing), timeout = 60 })
      if remove_result.exit_code ~= 0 then
        error("github-devloop: git worktree remove failed: " .. tostring(remove_result.stderr))
      end
    end
  end

  local fetch_result = exec_sync({ cmd = core.git_fetch_branch_cmd("origin", branch), timeout = 60 })
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR head branch fetch failed: " .. tostring(fetch_result.stderr))
  end
  local add_cmd = core.git_worktree_add_remote_branch_cmd(worktree, "origin", branch, false)
  if existing ~= nil then
    add_cmd = core.git_worktree_add_remote_branch_cmd(worktree, "origin", branch, true)
  end
  local add_result = exec_sync({ cmd = add_cmd, timeout = 60 })
  if add_result.exit_code ~= 0 then
    error("github-devloop: git worktree add failed: " .. tostring(add_result.stderr))
  end
  return worktree
end

local function fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
  if expected_baseline_sha == nil then
    return nil
  end
  local fetch_result = exec_sync({ cmd = core.git_fetch_pr_merge_ref_cmd("origin", pr_number), timeout = 60 })
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref fetch failed: " .. tostring(fetch_result.stderr))
  end
  local head_result = exec_sync({ cmd = core.git_fetch_head_commit_cmd(), timeout = 30 })
  if head_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref head failed: " .. tostring(head_result.stderr))
  end
  local merge_product_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if merge_product_sha ~= expected_baseline_sha then
    error("github-devloop: PR merge ref head does not match merge-gate baseline")
  end
  return merge_product_sha
end

local function merge_integration_for_fix(worktree, pr_number, integration_branch, expected_baseline_sha, merge_gate_reason)
  if core.merge_gate_reason_requires_pr_merge_product(merge_gate_reason) then
    fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
  end
  local base_head = expected_baseline_sha
  if base_head == nil then
    local fetch_result = exec_sync({ cmd = core.git_fetch_branch_cmd("origin", integration_branch), timeout = 60 })
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git integration branch fetch failed: " .. tostring(fetch_result.stderr))
    end
    local base_result = exec_sync({ cmd = core.git_remote_branch_head_cmd("origin", integration_branch), timeout = 30 })
    if base_result.exit_code ~= 0 then
      error("github-devloop: git integration branch head failed: " .. tostring(base_result.stderr))
    end
    base_head = tostring(base_result.stdout or ""):gsub("%s+$", "")
  end
  if not core.is_safe_head_sha(base_head) then
    error("github-devloop: unsafe integration head")
  end
  local result = {
    target_branch = integration_branch,
    target_sha = base_head,
    conflicted = false,
    unmerged_paths = "",
  }
  local merge_result = exec_sync({ cmd = core.git_worktree_merge_no_edit_cmd(worktree, base_head), timeout = 120 })
  if merge_result.exit_code ~= 0 then
    local unmerged_result = exec_sync({ cmd = core.git_unmerged_paths_cmd(worktree), timeout = 30 })
    if unmerged_result.exit_code ~= 0 then
      error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
    end
    if tostring(unmerged_result.stdout or "") == "" then
      error("github-devloop: git integration merge failed: " .. tostring(merge_result.stderr))
    end
    result.conflicted = true
    result.unmerged_paths = tostring(unmerged_result.stdout or "")
    core.log_line("info", "fix", "merge-target", "MERGE_SKEW", {
      "integration_branch=" .. tostring(integration_branch),
      "integration_sha=" .. tostring(base_head),
      "reason=integration merge requires codex conflict resolution",
    })
  end
  return result
end

local function assert_no_unmerged_paths(worktree)
  local unmerged_result = exec_sync({ cmd = core.git_unmerged_paths_cmd(worktree), timeout = 30 })
  if unmerged_result.exit_code ~= 0 then
    error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
  end
  if tostring(unmerged_result.stdout or "") ~= "" then
    error("github-devloop: fix left target merge conflicts unresolved")
  end
end

local function assert_no_conflict_markers(worktree)
  local markers_result = exec_sync({ cmd = core.git_conflict_markers_cmd(worktree), timeout = 30 })
  if markers_result.exit_code == 1 then
    return
  end
  if markers_result.exit_code == 0 then
    error("github-devloop: fix left conflict markers unresolved")
  end
  error("github-devloop: git conflict marker check failed: " .. tostring(markers_result.stderr))
end

local function raise_review_meta(repo, issue_number, fix, reason, detail)
  local comment_request = core.build_fix_review_meta_comment_request(repo, issue_number, fix, reason, detail)
  local label_request = core.build_fix_review_meta_label_request(repo, issue_number, fix, reason)
  local add_labels, remove_labels = core.state_label_changes("review-meta")
  core.log_apply("fix", fix.proposal_id, "review-meta", fix.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_review_meta",
  })
  core.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if issue_number ~= nil then
    core.log_raise("fix", fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  core.log_raise("fix", fix.proposal_id, "devloop_review_meta", {
    schema = "github-devloop.review-meta.v1",
    proposal_id = fix.proposal_id,
    review_proposal_id = fix.review_proposal_id,
    review_dedup_key = fix.review_dedup_key,
    version = fix.version,
    pr_number = fix.pr_number,
    n = 0,
    dedup_key = fix.dedup_key,
    source_ref = fix.source_ref,
  })
end

local function bounded_fix_summary(value)
  local text = tostring(value or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  if #text > 600 then
    text = text:sub(1, 600)
  end
  return text
end

local function raise_reviewing(repo, issue_number, fix, old_head_sha, new_head_sha, reason, summary)
  core.raise_fix_reviewing({
    dept = "fix",
    repo = repo,
    issue_number = issue_number,
    fix = fix,
    old_head_sha = old_head_sha,
    new_head_sha = new_head_sha,
    reason = reason,
    fix_summary = bounded_fix_summary(summary),
    clear_fix_summary = true,
  })
end

local function raise_work_card(repo, fix, card)
  local request = core.build_work_card_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, {
    proposal_id = fix.proposal_id,
    role = "fix",
    version = fix.version,
    round = core.version_fix_round(fix.version),
    started_at = card.started_at,
    finished_at = card.finished_at,
    outcome = card.outcome,
    gate_baseline_sha = fix.gate_baseline_sha,
    last_stage = fix.blocking_gap or fix.gate_failure_excerpt,
    source_ref = fix.source_ref,
  })
  core.log_work_card("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", request)
end

local function assert_fix_write_gate(fix, repo, issue_number)
  local write_enabled = core.write_mode() == "real"
  if write_enabled then
    return true
  end
  core.log_line("info", "fix", fix.proposal_id, "OUTBOUND", {
    "mode=dry-run",
    "repo=" .. tostring(repo),
    "issue=" .. tostring(issue_number),
    "pr=" .. tostring(fix.pr_number),
    "reason=PR fix requires FKST_GITHUB_WRITE=1 before codex",
  })
  return false
end

local function branch_head_if_ahead(base_head_sha, branch)
  local ahead_result = exec_sync({ cmd = core.git_branch_ahead_count_cmd(base_head_sha, branch), timeout = 30 })
  if ahead_result.exit_code ~= 0 then
    error("github-devloop: git branch ahead check failed: " .. tostring(ahead_result.stderr))
  end
  local ahead_count = tonumber(tostring(ahead_result.stdout or ""):match("%d+"))
  if ahead_count == nil or ahead_count <= 0 then
    return nil
  end
  local head_result = exec_sync({ cmd = core.git_branch_head_cmd(branch), timeout = 30 })
  if head_result.exit_code ~= 0 then
    error("github-devloop: git branch head check failed: " .. tostring(head_result.stderr))
  end
  local branch_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not core.is_safe_head_sha(branch_head_sha) then
    error("github-devloop: unsafe deterministic branch head sha")
  end
  if branch_head_sha == base_head_sha then
    return nil
  end
  return branch_head_sha
end

function pipeline(event)
  local fix = event.payload or {}
  if not core.is_supported_fixing(fix) then
    core.log_entry("fix", event, "unknown", core.payload_field(fix, "dedup_key"))
    core.log_cas_decision("fix", "unknown", { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("fix", event, fix.proposal_id, fix.dedup_key)
  local entity = core.parse_entity_proposal_id(fix.proposal_id)
  if entity == nil then
    core.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number

  local lock_key = core.transition_lock_key(fix.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()

    local pr_view = core.gh_exec({ cmd = core.gh_pr_view_fix_cmd(repo, fix.pr_number), timeout = 30 })
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr fix view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = core.parse_pr_view_fix(pr_view.stdout)
    core.log_forged_markers("fix", fix.proposal_id, current_pr.comments)
    local reviewing_version = core.next_fix_version(fix.version)
    if core.has_state_marker(current_pr.comments, fix.proposal_id, "reviewing", reviewing_version) then
      core.log_cas_decision("fix", fix.proposal_id, { state = "reviewing", version = reviewing_version }, "fixing", "reviewing", "skip-idempotent(already at to_state)", "reviewing state marker for fix already visible")
      return
    end
    local state = core.current_entity_state(current_pr.comments, fix.proposal_id)
    local transition = core.cyclic_transition_status(state, { "fixing" }, "reviewing", fix.version, reviewing_version)
    if transition == "pending" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "fixing state marker not yet visible")
      error("github-devloop: fixing state marker not yet visible for fix; retrying")
    end
    if transition == "idempotent" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "reviewing state marker for fix already visible")
      return
    end
    if state.state ~= "fixing" or transition == "stale" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "issue is not currently fixing")
      return
    end
    if tostring(state.version or "") ~= tostring(fix.version) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(version-mismatch)", "fix event version does not match canonical issue marker")
      return
    end
    local reject_fact = core.review_reject_fact(current_pr.comments, fix.proposal_id, fix.version)
    local meta_fix_fact = nil
    if reject_fact == nil then
      meta_fix_fact = core.review_meta_fix_fact(current_pr.comments, fix.proposal_id, fix.version)
    end
    local merge_gate_fact = nil
    if reject_fact == nil and meta_fix_fact == nil then
      local merge_gate_candidate = core.merge_gate_fix_fact(current_pr.comments, fix.proposal_id, fix.version)
      merge_gate_fact = core.merge_gate_fix_fact(current_pr.comments, fix.proposal_id, fix.version, {
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        gate_baseline_sha = fix.gate_baseline_sha,
        match_gate_baseline_sha = true,
      })
      if merge_gate_fact == nil then
        merge_gate_fact = merge_gate_candidate
      end
    end
    if reject_fact == nil and meta_fix_fact == nil and merge_gate_fact == nil then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(fix feedback marker not visible)", "reject review marker or review-meta fix marker missing")
      error("github-devloop: fix feedback marker not visible for fix; retrying")
    end
    local feedback_reason = nil
    if reject_fact ~= nil then
      if reject_fact.review_proposal_id ~= fix.review_proposal_id
        or reject_fact.review_dedup_key ~= fix.review_dedup_key
        or reject_fact.reviewed_head_sha ~= fix.reviewed_head_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(review-fact-mismatch)", "fix event does not match canonical reject review marker")
        return
      end
      feedback_reason = reject_fact.review_reason
    elseif meta_fix_fact ~= nil then
      if meta_fix_fact.review_dedup_key ~= fix.review_dedup_key then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(review-meta-fact-mismatch)", "fix event does not match canonical review-meta fix marker")
        return
      end
      feedback_reason = meta_fix_fact.review_reason
    else
      if merge_gate_fact.review_proposal_id ~= fix.review_proposal_id
        or merge_gate_fact.review_dedup_key ~= fix.review_dedup_key
        or merge_gate_fact.reviewed_head_sha ~= fix.reviewed_head_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(merge-gate-fact-mismatch)", "fix event does not match canonical merge-gate marker")
        return
      end
      if merge_gate_fact.gate_baseline_sha ~= fix.gate_baseline_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(merge-gate-baseline-mismatch)", "fix event does not match canonical merge-gate baseline")
        return
      end
      feedback_reason = merge_gate_fact.review_reason
    end

    local origin = core.pr_origin_fact(current_pr.comments)
    if origin == nil then
      origin = core.pr_native_origin(repo, fix.pr_number, current_pr)
    end
    if origin.proposal_id ~= fix.proposal_id
      or origin.repo ~= repo
      or tostring(origin.base_branch) ~= tostring(branches.integration)
      or tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
      or tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-foreign(pr-origin)", "PR origin/link does not match immutable PR branch")
      return
    end
    local branch = origin.branch
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    if not core.is_same_repo_pr_head(current_pr, repo) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(head-repository)", "PR head repository is missing or not the target repository")
      error("github-devloop: PR head repository is missing or not the target repository")
    end
    if tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
      local branch_head = exec_sync({ cmd = core.git_branch_head_cmd(branch), timeout = 30 })
      if branch_head.exit_code ~= 0 then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
        error("github-devloop: PR head changed before fix marker and deterministic branch head is not readable")
      end
      local intended_head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
      if not core.is_safe_head_sha(intended_head_sha) then
        error("github-devloop: unsafe PR origin branch head sha")
      end
      if tostring(current_pr.head_sha or "") == intended_head_sha
        and tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
        raise_reviewing(repo, issue_number, fix, fix.reviewed_head_sha, intended_head_sha, "push already visible; self-healing missing reviewing marker")
        return
      end
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(head-advanced)", "PR head changed since rejected review")
      return
    end

    if not assert_fix_write_gate(fix, repo, issue_number) then
      return
    end

    local current_issue = {
      title = "PR #" .. tostring(fix.pr_number),
      body = "(PR-only fix context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_exec({ cmd = core.gh_issue_view_fix_cmd(repo, issue_number), timeout = 30 })
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue fix view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = core.parse_issue_view_fix(issue_view.stdout)
    end

    local worktree = branch_worktree(repo, issue_number, fix.version, branch)
    local merge_context = merge_integration_for_fix(
      worktree,
      fix.pr_number,
      branches.integration,
      merge_gate_fact and merge_gate_fact.gate_baseline_sha or nil,
      merge_gate_fact and merge_gate_fact.reason or nil
    )
    if merge_context.conflicted then
      core.log_conflict_files("fix", fix.proposal_id, fix.pr_number, merge_context.unmerged_paths)
    end
    local codex_started_at = now()
    raise_work_card(repo, fix, {
      started_at = codex_started_at,
    })
    core.log_codex_start("fix", fix.proposal_id, "fix")
    local content_fetch = core.context_fetch_from_bundle({
      dept = "fix",
      repo = repo,
      issue_number = issue_number,
      pr_number = fix.pr_number,
      proposal_id = fix.proposal_id,
      version = fix.dedup_key,
      tick = event.ts,
    })
    local result = spawn_codex_sync({
      prompt = core.build_fix_prompt(fix, current_issue, feedback_reason, fix.framing, content_fetch, merge_context),
      worktree = worktree,
    })
    if type(result) ~= "table" or result.exit_code ~= 0 then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("fix", fix.proposal_id, "fix", result, nil, stderr)
      raise_work_card(repo, fix, {
        started_at = codex_started_at,
        finished_at = now(),
        outcome = "failed: codex-failed",
      })
      error("github-devloop: fix codex failed: " .. tostring(stderr))
    end
    core.log_codex_result("fix", fix.proposal_id, "fix", result, "result=completed", nil)
    assert_no_unmerged_paths(worktree)
    assert_no_conflict_markers(worktree)

    local status = exec_sync({ cmd = core.git_status_cmd(worktree), timeout = 30 })
    if status.exit_code ~= 0 then
      error("github-devloop: git status failed: " .. tostring(status.stderr))
    end
    if tostring(status.stdout or "") == "" then
      local existing_head_sha = branch_head_if_ahead(fix.reviewed_head_sha, branch)
      if existing_head_sha ~= nil then
        core.log_codex_result("fix", fix.proposal_id, "fix", result, "result=reusing-existing-head", nil)
        local pr_recheck = core.gh_exec({ cmd = core.gh_pr_view_fix_cmd(repo, fix.pr_number), timeout = 30 })
        if pr_recheck.exit_code ~= 0 then
          error("github-devloop: gh pr fix recheck failed: " .. tostring(pr_recheck.stderr))
        end
        local rechecked_pr = core.parse_pr_view_fix(pr_recheck.stdout)
        local rechecked_state = core.current_entity_state(rechecked_pr.comments, fix.proposal_id)
        if rechecked_state.state ~= "fixing" or tostring(rechecked_state.version or "") ~= tostring(fix.version) then
          core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing", "skip-stale(write-gate)", "write-time issue state changed")
          return
        end
        if tostring(rechecked_pr.state or ""):lower() ~= "open"
          or tostring(rechecked_pr.head_ref_name or "") ~= branch
          or tostring(rechecked_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha)
          or not core.is_same_repo_pr_head(rechecked_pr, repo) then
          core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing", "fail-closed(write-gate)", "write-time PR fact changed or head repository missing")
          error("github-devloop: write-time PR fact changed or head repository missing")
        end

        local push = exec_sync({ cmd = core.git_push_branch_cmd(branch), timeout = 120 })
        if push.exit_code ~= 0 then
          error("github-devloop: git push failed: " .. tostring(push.stderr))
        end
        local pushed_view = core.gh_exec({ cmd = core.gh_pr_view_fix_cmd(repo, fix.pr_number), timeout = 30 })
        if pushed_view.exit_code ~= 0 then
          error("github-devloop: gh pr pushed head view failed: " .. tostring(pushed_view.stderr))
        end
        local pushed_pr = core.parse_pr_view_fix(pushed_view.stdout)
        if tostring(pushed_pr.state or ""):lower() ~= "open"
          or tostring(pushed_pr.head_ref_name or "") ~= branch
          or tostring(pushed_pr.head_sha or "") ~= existing_head_sha
          or not core.is_same_repo_pr_head(pushed_pr, repo) then
          error("github-devloop: pushed PR head verification failed")
        end

        raise_work_card(repo, fix, {
          started_at = codex_started_at,
          finished_at = now(),
          outcome = "completed: existing head pushed",
        })
        raise_reviewing(repo, issue_number, fix, fix.reviewed_head_sha, existing_head_sha, "existing fix commit pushed and PR head verified", result.stdout or result.stderr)
        return
      end
      core.log_codex_result("fix", fix.proposal_id, "fix", result, nil, "no-changes")
      raise_work_card(repo, fix, {
        started_at = codex_started_at,
        finished_at = now(),
        outcome = "escalated: no-fix",
      })
      raise_review_meta(repo, issue_number, fix, "no-fix", result.stdout or result.stderr)
      return
    end

    local add_result = exec_sync({ cmd = core.git_add_all_cmd(worktree), timeout = 30 })
    if add_result.exit_code ~= 0 then
      error("github-devloop: git add failed: " .. tostring(add_result.stderr))
    end
    local commit_result = exec_sync({
      cmd = core.git_commit_cmd(worktree, core.fix_commit_subject(
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
    if tostring(branch_result.stdout or ""):gsub("%s+$", "") ~= branch then
      error("github-devloop: PR origin fix branch mismatch")
    end
    local head_result = exec_sync({ cmd = core.git_head_sha_cmd(worktree), timeout = 30 })
    if head_result.exit_code ~= 0 then
      error("github-devloop: git head fact failed: " .. tostring(head_result.stderr))
    end
    local new_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
    if not core.is_safe_head_sha(new_head_sha) then
      error("github-devloop: unsafe fix head_sha")
    end
    if new_head_sha == fix.reviewed_head_sha then
      raise_work_card(repo, fix, {
        started_at = codex_started_at,
        finished_at = now(),
        outcome = "escalated: no-new-head",
      })
      raise_review_meta(repo, issue_number, fix, "no-new-head", result.stdout or result.stderr)
      return
    end

    local pr_recheck = core.gh_exec({ cmd = core.gh_pr_view_fix_cmd(repo, fix.pr_number), timeout = 30 })
    if pr_recheck.exit_code ~= 0 then
      error("github-devloop: gh pr fix recheck failed: " .. tostring(pr_recheck.stderr))
    end
    local rechecked_pr = core.parse_pr_view_fix(pr_recheck.stdout)
    local rechecked_state = core.current_entity_state(rechecked_pr.comments, fix.proposal_id)
    if rechecked_state.state ~= "fixing" or tostring(rechecked_state.version or "") ~= tostring(fix.version) then
      core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing", "skip-stale(write-gate)", "write-time issue state changed")
      return
    end
    if tostring(rechecked_pr.state or ""):lower() ~= "open"
      or tostring(rechecked_pr.head_ref_name or "") ~= branch
      or tostring(rechecked_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha)
      or not core.is_same_repo_pr_head(rechecked_pr, repo) then
      core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing", "fail-closed(write-gate)", "write-time PR fact changed or head repository missing")
      error("github-devloop: write-time PR fact changed or head repository missing")
    end

    local push = exec_sync({ cmd = core.git_push_branch_cmd(branch), timeout = 120 })
    if push.exit_code ~= 0 then
      error("github-devloop: git push failed: " .. tostring(push.stderr))
    end
    local pushed_view = core.gh_exec({ cmd = core.gh_pr_view_fix_cmd(repo, fix.pr_number), timeout = 30 })
    if pushed_view.exit_code ~= 0 then
      error("github-devloop: gh pr pushed head view failed: " .. tostring(pushed_view.stderr))
    end
    local pushed_pr = core.parse_pr_view_fix(pushed_view.stdout)
    if tostring(pushed_pr.state or ""):lower() ~= "open"
      or tostring(pushed_pr.head_ref_name or "") ~= branch
      or tostring(pushed_pr.head_sha or "") ~= new_head_sha
      or not core.is_same_repo_pr_head(pushed_pr, repo) then
      error("github-devloop: pushed PR head verification failed")
    end

    raise_work_card(repo, fix, {
      started_at = codex_started_at,
      finished_at = now(),
      outcome = "completed: pushed for re-review",
    })
    raise_reviewing(repo, issue_number, fix, fix.reviewed_head_sha, new_head_sha, "fix pushed and PR head verified", result.stdout or result.stderr)
  end)
end

return M
