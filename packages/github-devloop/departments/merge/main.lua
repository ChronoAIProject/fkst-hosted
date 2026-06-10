local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_merge_ready" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_fixing",
  },
  stall_window = "2m",
}

local MAX_RUNTIME_ID_LEN = 180

local function safe_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

local function runtime_identity(repo, issue_number)
  local id = "merge-" .. safe_segment(repo) .. "-issue-" .. safe_segment(issue_number)
  if #id > MAX_RUNTIME_ID_LEN then
    return id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return id
end

local function temp_body_file(repo, issue_number)
  return "/tmp/fkst-github-devloop-" .. runtime_identity(repo, issue_number) .. ".md"
end

local function log_gate(merge_ready, outcome, reason)
  core.log_line("info", "merge", merge_ready.proposal_id, "GATE", {
    "pr=" .. tostring(merge_ready.pr_number),
    "version=" .. tostring(merge_ready.version),
    "outcome=" .. tostring(outcome),
    "reason=" .. tostring(reason or ""),
  })
end

local function require_consensus_review_approve(comments, merge_ready)
  local ok, reason = core.review_result_approval_matches_event(comments, merge_ready)
  if ok then
    return true
  end
  log_gate(merge_ready, "dry-run", "merge requires trusted review-result approve: " .. tostring(reason))
  return false
end

local function raise_fixing(repo, issue_number, merge_ready, current_state, reason)
  local source_ref = core.pr_source_ref(repo, merge_ready.pr_number)
  local fix_version = core.fix_version_from_review_version(current_state.version)
  local comment_request = core.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, source_ref)
  local label_request = issue_number ~= nil and core.build_state_label_request(
    repo,
    issue_number,
    "fixing",
    merge_ready.dedup_key .. "/label/fixing",
    core.issue_source_ref(repo, issue_number)
  ) or nil
  local fix_payload = core.build_devloop_fixing_payload({
    proposal_id = merge_ready.proposal_id,
    impl_version = fix_version,
  }, merge_ready.pr_number, {
    review_proposal_id = merge_ready.review_proposal_id,
    review_dedup_key = merge_ready.review_dedup_key,
    reviewed_head_sha = merge_ready.reviewed_head_sha,
  }, source_ref)
  local add_labels, remove_labels = core.state_label_changes("fixing")
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "fixing", "applied", reason)
  core.log_apply("merge", merge_ready.proposal_id, "fixing", fix_version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_fixing",
  })
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  core.log_raise("merge", merge_ready.proposal_id, "devloop_fixing", fix_payload)
end

local function raise_reviewing_for_current_head(repo, issue_number, merge_ready, current_state, current_pr, reason)
  local source_ref = core.pr_source_ref(repo, merge_ready.pr_number)
  local review_version = core.next_review_loop_version(merge_ready.version)
  if core.has_state_marker(current_pr.comments, merge_ready.proposal_id, "reviewing", review_version) then
    core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "reviewing", "skip-idempotent(already at to_state)", reason)
    return
  end
  local current_head_sha = tostring(current_pr.head_sha or "")
  local comment_request = core.build_merge_head_reviewing_comment_request(
    repo,
    issue_number,
    merge_ready,
    merge_ready.reviewed_head_sha,
    current_head_sha,
    review_version,
    source_ref
  )
  local label_request = issue_number ~= nil and core.build_merge_head_reviewing_label_request(
    repo,
    issue_number,
    merge_ready,
    current_head_sha,
    review_version,
    core.issue_source_ref(repo, issue_number)
  ) or nil
  local reviewing_payload = core.build_devloop_reviewing_payload({
    proposal_id = merge_ready.proposal_id,
    impl_version = review_version,
  }, merge_ready.pr_number, source_ref, review_version)
  local add_labels, remove_labels = core.state_label_changes("reviewing")
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "reviewing", "applied", reason)
  core.log_apply("merge", merge_ready.proposal_id, "reviewing", review_version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_reviewing",
  })
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  core.log_raise("merge", merge_ready.proposal_id, "devloop_reviewing", reviewing_payload)
end

local function assert_open_same_repo_pr(merge_ready, pr, repo, branch, head_sha)
  return core.pr_identity_matches(pr, {
    repo = repo,
    head_sha = head_sha,
    head_branch = branch,
    base_branch = pr.base_ref_name,
  })
end

local function assert_merge_pr_authority(merge_ready, pr, repo, issue_number, origin, branches)
  local state = core.current_entity_state(pr.comments, merge_ready.proposal_id)
  if (state.state ~= "merge-ready" and state.state ~= "merging")
    or tostring(state.version or "") ~= tostring(merge_ready.version) then
    return false, "write-time PR state changed", state
  end

  if not require_consensus_review_approve(pr.comments, merge_ready) then
    return false, "trusted review-result approve missing", state
  end

  local fact = core.merge_ready_fact(pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number)
  local approval_ok, approval_reason = core.merge_ready_approval_matches_event(fact, merge_ready)
  if not approval_ok then
    return false, "merge-ready fact changed: " .. tostring(approval_reason), state
  end

  local current_origin = core.pr_origin_fact(pr.comments)
  if current_origin == nil then
    current_origin = core.pr_native_origin(repo, merge_ready.pr_number, pr)
  end
  if current_origin.proposal_id ~= merge_ready.proposal_id
    or current_origin.repo ~= repo
    or tostring(current_origin.issue_number) ~= tostring(issue_number)
    or tostring(current_origin.branch) ~= tostring(origin.branch)
    or tostring(current_origin.impl_version) ~= tostring(origin.impl_version)
    or tostring(current_origin.base_branch) ~= tostring(origin.base_branch)
    or tostring(pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch) ~= tostring(branches.integration) then
    return false, "pr-origin-changed", state
  end

  local pr_ok, pr_reason = assert_open_same_repo_pr(merge_ready, pr, repo, origin.branch, merge_ready.reviewed_head_sha)
  if not pr_ok then
    return false, pr_reason, state
  end

  return true, "merge-authority-ok", state
end

local function ensure_pr_ready_for_merge(repo, merge_ready, current_pr)
  if current_pr.is_draft ~= true then
    return current_pr
  end
  local ready_result = exec_sync({ cmd = core.gh_pr_ready_cmd(repo, merge_ready.pr_number), timeout = 60 })
  if ready_result.exit_code ~= 0 then
    error("github-devloop: gh pr ready failed: " .. tostring(ready_result.stderr))
  end

  local pr_view = exec_sync({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr ready recheck failed: " .. tostring(pr_view.stderr))
  end
  return core.parse_pr_view_merge(pr_view.stdout)
end

local function is_merged_pr(pr)
  return core.is_merged_pr(pr)
end

local function build_merging_body(merge_ready)
  return "github-devloop is merging PR #" .. tostring(merge_ready.pr_number)
    .. "\n\n" .. core.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. core.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
end

local function write_merging_marker(repo, merge_ready, comments)
  if core.merging_fact(comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) ~= nil then
    return
  end
  local path = temp_body_file(repo, merge_ready.pr_number)
  file.write(path, build_merging_body(merge_ready))
  local result = exec_sync({ cmd = core.gh_pr_comment_cmd(repo, merge_ready.pr_number, path), timeout = 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: gh pr merging marker comment failed: " .. tostring(result.stderr))
  end
end

local function build_merged_requests(repo, issue_number, merge_ready)
  local merged_source_ref = core.pr_source_ref(repo, merge_ready.pr_number)
  local merged_body = "github-devloop merged PR #" .. tostring(merge_ready.pr_number)
    .. "\n\n" .. core.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. core.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
    .. "\n" .. core.state_marker(merge_ready.proposal_id, "merged", merge_ready.version)
    .. "\n" .. core.merged_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
  local comment_request = core.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, merged_body, merge_ready.dedup_key .. "/comment/merged", merged_source_ref)
  local label_request = issue_number ~= nil and core.build_state_label_request(
    repo,
    issue_number,
    "merged",
    merge_ready.dedup_key .. "/label/merged",
    core.issue_source_ref(repo, issue_number)
  ) or nil
  return comment_request, label_request
end

local function finalize_merged(repo, issue_number, merge_ready, current_state, reason)
  if issue_number ~= nil then
    local close_result = exec_sync({ cmd = core.gh_issue_close_cmd(repo, issue_number), timeout = 60 })
    if close_result.exit_code ~= 0 then
      error("github-devloop: gh issue close failed: " .. tostring(close_result.stderr))
    end
  end

  local comment_request, label_request = build_merged_requests(repo, issue_number, merge_ready)
  local add_labels, remove_labels = core.state_label_changes("merged")
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "merged", "applied", reason)
  core.log_apply("merge", merge_ready.proposal_id, "merged", merge_ready.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
end

function pipeline(event)
  local merge_ready = event.payload or {}
  if not core.is_supported_merge_ready(merge_ready) then
    core.log_entry("merge", event, "unknown", merge_ready.dedup_key)
    core.log_cas_decision("merge", "unknown", { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("merge", event, merge_ready.proposal_id, merge_ready.dedup_key)
  local entity = core.parse_entity_proposal_id(merge_ready.proposal_id)
  if entity == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number

  local lock_key = core.transition_lock_key(merge_ready.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()

    local pr_view = exec_sync({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr merge view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = core.parse_pr_view_merge(pr_view.stdout)
    core.log_forged_markers("merge", merge_ready.proposal_id, current_pr.comments)
    local state = core.current_entity_state(current_pr.comments, merge_ready.proposal_id)
    if state.state == "merged" and core.has_merged_marker(current_pr.comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merged", "skip-idempotent(already at to_state)", "merged marker already visible")
      return
    end
    local transition = core.cyclic_transition_status(state, { "merge-ready", "merging" }, "merging", merge_ready.version)
    if transition == "pending" then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", core.cas_outcome(state, transition, merge_ready.version), "merge-ready state marker not yet visible")
      error("github-devloop: merge-ready state marker not yet visible for merge; retrying")
    end
    if transition == "stale" then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", core.cas_outcome(state, transition, merge_ready.version), "issue is not currently merge-ready")
      return
    end
    if transition == "idempotent" and state.state ~= "merging" then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", core.cas_outcome(state, transition, merge_ready.version), "issue is not currently merge-ready or merging")
      return
    end
    if transition == "apply" and state.state ~= "merge-ready" then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(from-state-mismatch)", "issue is not currently merge-ready")
      return
    end
    if transition ~= "apply" and transition ~= "idempotent" then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", core.cas_outcome(state, transition, merge_ready.version), "issue is not currently merge-ready or merging")
      return
    end
    if tostring(state.version or "") ~= tostring(merge_ready.version) then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(version-mismatch)", "merge-ready event version does not match canonical issue marker")
      return
    end
    local fact = core.merge_ready_fact(current_pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number)
    if fact == nil then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "retry-pending(merge-ready fact marker not visible)", "trusted merge-ready fact marker missing")
      error("github-devloop: merge-ready fact marker not visible for merge; retrying")
    end
    local approval_ok, approval_reason = core.merge_ready_approval_matches_event(fact, merge_ready)
    if not approval_ok then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(" .. tostring(approval_reason) .. ")", "merge-ready event does not match canonical approval fact marker")
      return
    end
    local origin = core.pr_origin_fact(current_pr.comments)
    if origin == nil then
      origin = core.pr_native_origin(repo, merge_ready.pr_number, current_pr)
    end
    if origin.proposal_id ~= merge_ready.proposal_id
      or origin.repo ~= repo
      or tostring(origin.base_branch) ~= tostring(branches.integration)
      or tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch) then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-foreign(pr-origin)", "PR origin/link does not match immutable PR branch")
      return
    end
    local write_enabled = core.write_mode() == "real"
    local pr_ok, pr_reason = assert_open_same_repo_pr(merge_ready, current_pr, repo, origin.branch, merge_ready.reviewed_head_sha)
    if not pr_ok then
      if is_merged_pr(current_pr)
        and tostring(current_pr.head_ref_name or "") == tostring(origin.branch)
        and tostring(current_pr.head_sha or "") == tostring(merge_ready.reviewed_head_sha)
        and core.is_same_repo_pr_head(current_pr, repo) then
        local merging_fact = core.merging_fact(current_pr.comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
        if merging_fact == nil then
          core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merged", "skip-external-merge(no-bot-merging-marker)", "PR is already merged without a prior trusted bot merging marker")
          return
        end
        if not write_enabled then
          log_gate(merge_ready, "dry-run", "PR already merged; finalization requires FKST_GITHUB_WRITE=1")
          return
        end
        finalize_merged(repo, issue_number, merge_ready, state, "PR already merged; self-healing finalization")
        return
      end
      if pr_reason == "head-sha-mismatch" and state.state == "merging" then
        log_gate(merge_ready, "fixing", "head-sha-mismatch")
        raise_fixing(repo, issue_number, merge_ready, state, "head-sha-mismatch")
        return
      end
      if pr_reason == "head-sha-mismatch" and state.state == "merge-ready" then
        log_gate(merge_ready, "reviewing", "head-sha-mismatch")
        raise_reviewing_for_current_head(repo, issue_number, merge_ready, state, current_pr, "head-sha-mismatch")
        return
      end
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(" .. pr_reason .. ")", "write-time PR fact failed")
      return
    end

    if not write_enabled then
      log_gate(merge_ready, "dry-run", "merge requires FKST_GITHUB_WRITE=1")
      return
    end
    if not require_consensus_review_approve(current_pr.comments, merge_ready) then
      return
    end
    log_gate(merge_ready, "write-ready", "FKST_GITHUB_WRITE=1 and trusted review-result approve")

    current_pr = ensure_pr_ready_for_merge(repo, merge_ready, current_pr)
    local ready_ok, ready_reason = assert_merge_pr_authority(merge_ready, current_pr, repo, issue_number, origin, branches)
    if not ready_ok then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "fail-closed(ready-recheck)", ready_reason)
      error("github-devloop: PR fact changed after ready conversion")
    end

    local rollup_green, rollup_reason = core.pr_rollup_green(current_pr)
    if not rollup_green then
      if not core.is_ci_red_reason(rollup_reason) then
        log_gate(merge_ready, "dry-run", rollup_reason)
        error("github-devloop: merge wait on " .. tostring(rollup_reason) .. "; retrying")
      end
      local fix_reason = core.rollup_red_fix_reason(current_pr, rollup_reason)
      log_gate(merge_ready, "fixing", fix_reason)
      raise_fixing(repo, issue_number, merge_ready, state, fix_reason)
      return
    end
    local mergeable, mergeable_reason = core.pr_mergeable(current_pr)
    if not mergeable then
      if not core.is_not_mergeable_reason(mergeable_reason) then
        log_gate(merge_ready, "dry-run", mergeable_reason)
        error("github-devloop: merge wait on " .. tostring(mergeable_reason) .. "; retrying")
      end
      log_gate(merge_ready, "fixing", mergeable_reason)
      raise_fixing(repo, issue_number, merge_ready, state, mergeable_reason)
      return
    end

    local pr_recheck = exec_sync({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
    if pr_recheck.exit_code ~= 0 then
      error("github-devloop: gh pr merge recheck failed: " .. tostring(pr_recheck.stderr))
    end
    local rechecked_pr_for_gate = core.parse_pr_view_merge(pr_recheck.stdout)
    local recheck_ok, recheck_reason, rechecked_state = assert_merge_pr_authority(merge_ready, rechecked_pr_for_gate, repo, issue_number, origin, branches)
    if not recheck_ok then
      if recheck_reason == "head-sha-mismatch" then
        log_gate(merge_ready, "reviewing", "head-sha-mismatch")
        raise_reviewing_for_current_head(repo, issue_number, merge_ready, rechecked_state, rechecked_pr_for_gate, "head-sha-mismatch")
        return
      end
      core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready|merging", "merging", "skip-stale(write-gate)", "write-time PR state changed")
      return
    end
    if not write_enabled then
      log_gate(merge_ready, "dry-run", "write-time FKST_GITHUB_WRITE missing")
      return
    end
    log_gate(merge_ready, "write-ready", "write-time FKST_GITHUB_WRITE=1 and trusted review-result approve")
    core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merging", "applied", "all merge gates satisfied; invoking gh pr merge")
    local merge_ok, merge_reason, merge_rechecked_pr = core.run_verified_pr_merge({
      repo = repo,
      pr_number = merge_ready.pr_number,
      head_sha = merge_ready.reviewed_head_sha,
      head_branch = origin.branch,
      base_branch = branches.integration,
      validate_rechecked_pr = function(rechecked_pr)
        local recheck_origin = core.pr_origin_fact(rechecked_pr.comments)
        if recheck_origin == nil then
          recheck_origin = core.pr_native_origin(repo, merge_ready.pr_number, rechecked_pr)
        end
        if recheck_origin.proposal_id ~= merge_ready.proposal_id
          or recheck_origin.repo ~= repo
          or tostring(recheck_origin.issue_number) ~= tostring(issue_number)
          or tostring(recheck_origin.branch) ~= tostring(origin.branch)
          or tostring(recheck_origin.impl_version) ~= tostring(origin.impl_version)
          or tostring(recheck_origin.base_branch) ~= tostring(origin.base_branch)
          or tostring(rechecked_pr.base_ref_name or "") ~= tostring(origin.base_branch)
          or tostring(origin.base_branch) ~= tostring(branches.integration) then
          return false, "pr-origin-changed"
        end
        return true, "pr-origin-ok"
      end,
      before_merge = function()
        write_merging_marker(repo, merge_ready, rechecked_pr_for_gate.comments)
      end,
    })
    if not merge_ok and merge_reason == "merge-confirmation-pending" then
      core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merged", "retry-pending(merge-confirmation)", "gh pr merge returned without a merged PR fact")
      error("github-devloop: merge confirmation pending; retrying")
    end
    if not merge_ok and merge_reason == "merge-confirmation-mismatch" then
      core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merged", "fail-closed(merge-confirmation)", "merged PR fact does not match reviewed head")
      error("github-devloop: merged PR fact changed before finalization")
    end
    if not merge_ok and merge_reason == "head-sha-mismatch" then
      log_gate(merge_ready, "reviewing", "head-sha-mismatch")
      raise_reviewing_for_current_head(repo, issue_number, merge_ready, rechecked_state, merge_rechecked_pr or rechecked_pr_for_gate, "head-sha-mismatch")
      return
    end
    if not merge_ok and core.is_ci_red_reason(merge_reason) then
      local fix_reason = core.rollup_red_fix_reason(merge_rechecked_pr, merge_reason)
      log_gate(merge_ready, "fixing", fix_reason)
      raise_fixing(repo, issue_number, merge_ready, rechecked_state, fix_reason)
      return
    end
    if not merge_ok and core.is_not_mergeable_reason(merge_reason) then
      log_gate(merge_ready, "fixing", merge_reason)
      raise_fixing(repo, issue_number, merge_ready, rechecked_state, merge_reason)
      return
    end
    if not merge_ok and (merge_reason == "rollup-pending" or merge_reason == "mergeable-unknown") then
      log_gate(merge_ready, "dry-run", merge_reason)
      error("github-devloop: merge wait on write-time " .. tostring(merge_reason) .. "; retrying")
    end
    if not merge_ok then
      core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merging", "fail-closed(write-gate)", "write-time PR fact failed: " .. tostring(merge_reason))
      error("github-devloop: write-time PR fact changed before merge")
    end

    finalize_merged(repo, issue_number, merge_ready, rechecked_state, "gh pr merge confirmed merged")
  end)
end

return M
