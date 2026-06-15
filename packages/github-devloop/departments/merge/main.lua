local core = require("core")
local runtime_files = require("departments.merge.runtime_files")

local M = {}
M.spec = {
  consumes = { "devloop_merge_ready", "devloop_merge_queue_tick" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_fix_reconcile",
    "devloop_decompose",
    "devloop_merge_queue_tick",
  },
  fanout = { "devloop_merge_queue_tick" },
  stall_window = "2m",
}

local function log_gate(merge_ready, outcome, reason)
  local pass = merge_ready and merge_ready._merge_pass
  local fields = {
    "pr=" .. tostring(merge_ready.pr_number),
    "version=" .. tostring(merge_ready.version),
    "outcome=" .. tostring(outcome),
    "reason=" .. tostring(reason or ""),
  }
  if pass ~= nil then
    table.insert(fields, "pass=" .. tostring(pass))
  end
  core.log_line("info", "merge", merge_ready.proposal_id, "GATE", fields)
end

local function require_consensus_review_approve(comments, merge_ready)
  local ok, reason = core.review_result_approval_matches_event(comments, merge_ready)
  if ok then
    return true
  end
  log_gate(merge_ready, "dry-run", "merge requires trusted review-result approve: " .. tostring(reason))
  return false
end

local function gate_baseline_sha_from_pr(pr)
  local baseline_sha = tostring(pr and pr.base_ref_oid or "")
  if not core.is_safe_head_sha(baseline_sha) then
    error("github-devloop: unsafe merge-gate baseline sha")
  end
  return baseline_sha
end

local function is_rollup_red_fix_reason(reason)
  return core.merge_gate_reason_class(reason) == "rollup-red"
end
local function fetch_pr_merge_product_sha(pr_number)
  local fetch_result = exec_sync({ cmd = core.git_fetch_pr_merge_ref_cmd("origin", pr_number), timeout = 60 })
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref fetch failed: " .. tostring(fetch_result.stderr))
  end
  local head_result = exec_sync({ cmd = core.git_fetch_head_commit_cmd(), timeout = 30 })
  if head_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref head failed: " .. tostring(head_result.stderr))
  end
  local merge_product_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not core.is_safe_head_sha(merge_product_sha) then
    error("github-devloop: unsafe PR merge product sha")
  end
  return merge_product_sha
end

local function gate_baseline_sha_for_reason(proposal_id, pr_number, pr, reason)
  if is_rollup_red_fix_reason(reason) then
    local gate_sha = tostring(core.rollup_failure_gate_sha(pr) or "")
    if not core.is_safe_head_sha(gate_sha) then
      core.log_line("info", "merge", proposal_id, "GATE", {
        "outcome=degrade",
        "reason=rollup gate sha underivable from statusCheckRollup; fix will merge current integration",
        "pr=" .. tostring(pr_number),
      })
      return nil
    end
    local merge_product_sha = fetch_pr_merge_product_sha(pr_number)
    if merge_product_sha ~= gate_sha then
      error("github-devloop: statusCheckRollup sha does not match PR merge product sha")
    end
    return merge_product_sha
  end
  return gate_baseline_sha_from_pr(pr)
end

local function pr_head_contains_current_base(pr, branches)
  local base_head, base_reason = core.current_base_head(branches.integration)
  if base_head == nil then
    return false, base_reason
  end
  local head_sha = tostring(pr and pr.head_sha or "")
  if not core.is_safe_head_sha(head_sha) then
    return false, "unsafe-pr-head"
  end
  local result = exec_sync({ cmd = core.git_is_ancestor_cmd(base_head, head_sha), timeout = 30 })
  if result.exit_code == 0 then
    return true, "current-base-contained"
  end
  return false, "current-base-not-contained"
end

local function should_wait_for_stale_mergeability(pr, branches, mergeable_reason)
  if not core.is_not_mergeable_reason(mergeable_reason) then
    return false, "not-stale-mergeability"
  end
  return pr_head_contains_current_base(pr, branches)
end

local function raise_decompose_for_max_fix_rounds(merge_ready, current_state, reason, source_ref)
  local fix_reconcile = core.build_devloop_fix_reconcile_payload({
    proposal_id = merge_ready.proposal_id,
    review_proposal_id = merge_ready.review_proposal_id,
    review_dedup_key = merge_ready.review_dedup_key,
    reviewed_head_sha = merge_ready.reviewed_head_sha,
    pr_number = merge_ready.pr_number,
    source_ref = source_ref,
  }, current_state.version)
  local decompose = core.build_devloop_decompose_payload(fix_reconcile)
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "blocked", "applied(fix-loop-max-rounds)", reason)
  core.log_raise("merge", merge_ready.proposal_id, "devloop_fix_reconcile", fix_reconcile)
  core.log_raise("merge", merge_ready.proposal_id, "devloop_decompose", decompose)
end

local function raise_fixing(repo, issue_number, merge_ready, current_state, current_pr, reason, queue_position)
  local source_ref = core.pr_source_ref(repo, merge_ready.pr_number)
  if core.version_fix_round(current_state.version) >= core.max_fix_rounds() then
    raise_decompose_for_max_fix_rounds(merge_ready, current_state, reason, source_ref)
    return
  end
  local fix_version = core.fix_version_from_review_version(current_state.version)
  local gate_baseline_sha = gate_baseline_sha_for_reason(merge_ready.proposal_id, merge_ready.pr_number, current_pr, reason)
  local predecessor_set = nil
  if queue_position ~= nil then
    predecessor_set = queue_position.predecessor_set
  else
    local branches = core.branch_config()
    local position, predecessor_reason = core.merge_queue_position(repo, branches.integration, {
      pr_number = merge_ready.pr_number,
      pr = current_pr,
    })
    if position == nil then
      error("github-devloop: merge queue predecessor derivation failed: " .. tostring(predecessor_reason))
    end
    predecessor_set = position.predecessor_set
  end
  local comment_request = core.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, gate_baseline_sha, source_ref, predecessor_set)
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
    gate_baseline_sha = gate_baseline_sha,
    predecessor_set = predecessor_set,
    gate_failure_excerpt = reason,
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
  local comment_request = core.build_merge_head_reviewing_comment_request(repo, issue_number, merge_ready, merge_ready.reviewed_head_sha, current_head_sha, review_version, source_ref)
  local label_request = issue_number ~= nil and core.build_merge_head_reviewing_label_request(repo, issue_number, merge_ready, current_head_sha, review_version, core.issue_source_ref(repo, issue_number)) or nil
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

  local fact = core.merge_ready_fact(pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number, merge_ready.reviewed_head_sha)
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

local function speculative_fix_fact_for_merge(comments, merge_ready)
  local fix_version = core._strip_latest_fix_version_suffix(merge_ready.version)
  if tostring(fix_version or "") == tostring(merge_ready.version or "") then
    return nil
  end
  local fact = core.merge_gate_fix_fact(comments, merge_ready.proposal_id, fix_version)
  if fact == nil or fact.predecessor_set == nil then
    return nil
  end
  if not core.has_fix_marker(
    comments,
    merge_ready.proposal_id,
    fact.review_proposal_id,
    fact.review_dedup_key,
    fact.reviewed_head_sha,
    merge_ready.reviewed_head_sha
  ) then
    return {
      predecessor_set = fact.predecessor_set,
      reason = "speculative-fix-head-binding-missing",
    }
  end
  return {
    predecessor_set = fact.predecessor_set,
    reason = "speculative-predecessor-set",
  }
end

local function revalidate_speculative_predecessors(repo, issue_number, merge_ready, state, current_pr, queue_position, speculative_fact)
  if speculative_fact == nil then
    return true
  end
  local current_position = queue_position
  if current_position == nil then
    local branches = core.branch_config()
    local position, reason = core.merge_queue_position(repo, branches.integration, {
      pr_number = merge_ready.pr_number,
      pr = current_pr,
    })
    if position == nil then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "hold-merge-queue", reason)
      log_gate(merge_ready, "dry-run", reason)
      return false
    end
    current_position = position
  end
  local branches = core.branch_config()
  local matches, match_reason = core.merge_queue_predecessor_set_matches_current_base(
    speculative_fact.predecessor_set,
    current_position.predecessor_set,
    branches.integration
  )
  if matches then
    return true
  end
  log_gate(merge_ready, "fixing", match_reason)
  raise_fixing(repo, issue_number, merge_ready, state, current_pr, match_reason, current_position)
  return false
end

local function ensure_pr_ready_for_merge(repo, merge_ready, current_pr)
  if current_pr.is_draft ~= true then
    return current_pr
  end
  local ready_result = core.gh_exec({ cmd = core.gh_pr_ready_cmd(repo, merge_ready.pr_number), timeout = 60 })
  if ready_result.exit_code ~= 0 then
    error("github-devloop: gh pr ready failed: " .. tostring(ready_result.stderr))
  end

  local pr_view = core.gh_exec({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr ready recheck failed: " .. tostring(pr_view.stderr))
  end
  return core.parse_pr_view_merge(pr_view.stdout)
end

local function build_merging_body(merge_ready)
  return core.build_merging_comment_body(merge_ready)
end
local function write_merging_marker(repo, merge_ready, comments)
  if core.merging_fact(comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) ~= nil then
    return
  end
  local path = runtime_files.temp_body_file(repo, merge_ready.pr_number)
  local body = core.with_github_debug_stamp(build_merging_body(merge_ready), {
    emitter = "github-devloop.merge.merging",
    target = "pr:" .. tostring(repo) .. "#" .. tostring(merge_ready.pr_number),
    dedup_key = merge_ready.dedup_key,
    context = merge_ready.reviewed_head_sha,
  })
  file.write(path, body)
  local result = core.gh_exec({ cmd = core.gh_pr_comment_cmd(repo, merge_ready.pr_number, path), timeout = 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: gh pr merging marker comment failed: " .. tostring(result.stderr))
  end
  core.invalidate_entity_after_write(repo, "pr", merge_ready.pr_number)
end

local function build_merged_requests(repo, issue_number, merge_ready)
  local merged_source_ref = core.pr_source_ref(repo, merge_ready.pr_number)
  local merged_body = core.build_merged_comment_body(merge_ready)
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
    local close_result = core.gh_exec({ cmd = core.gh_issue_close_cmd(repo, issue_number), timeout = 60 })
    if close_result.exit_code ~= 0 then
      error("github-devloop: gh issue close failed: " .. tostring(close_result.stderr))
    end
    core.invalidate_entity_after_write(repo, "issue", issue_number)
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

local function process_merge_ready_locked(repo, issue_number, merge_ready, branches, initial_pr, options)
  local enforce_queue = options == nil or options.enforce_queue ~= false
  local write_mode = options and options.write_mode or nil
  local entity = core.parse_entity_proposal_id(merge_ready.proposal_id)
  if entity == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  if tostring(entity.repo or "") ~= tostring(repo or "")
    or tostring(entity.issue_number or "") ~= tostring(issue_number or "") then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end
  if not core.verify_pr_review_issue_claim("merge", repo, issue_number, nil, merge_ready.proposal_id) then
    return
  end
  if options ~= nil and type(options.queue_starvation_cause) == "table" then
    local comment_request = core.build_queue_starvation_reconcile_comment_request(repo, merge_ready, options.queue_starvation_cause)
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    raise("github-proxy.github_pr_comment_request", comment_request)
  end
  local current_pr = initial_pr
  if current_pr == nil then
    local pr_view = core.gh_exec({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr merge view failed: " .. tostring(pr_view.stderr))
    end
    current_pr = core.parse_pr_view_merge(pr_view.stdout)
  end
  core.log_forged_markers("merge", merge_ready.proposal_id, current_pr.comments)
  local state = core.current_entity_state(current_pr.comments, merge_ready.proposal_id)
  if state.state == "merged" and core.has_merged_marker(current_pr.comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merged", "skip-idempotent(already at to_state)", "merged marker already visible")
    return
  end
  local transition = core.cyclic_transition_status(state, { "merge-ready", "merging" }, "merging", merge_ready.version)
  if state.state ~= "merge-ready" and state.state ~= "merging" and state.state ~= "merged" then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(from-state-mismatch)", "issue is not currently merge-ready or merging")
    return
  end
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
  local fact = core.merge_ready_fact(current_pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number, merge_ready.reviewed_head_sha)
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
  local write_enabled = (write_mode or core.write_mode()) == "real"
  local pr_ok, pr_reason = assert_open_same_repo_pr(merge_ready, current_pr, repo, origin.branch, merge_ready.reviewed_head_sha)
  if not pr_ok then
    if core.is_merged_pr(current_pr)
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
      return { status = "merged", pr_number = merge_ready.pr_number, merge_ready = merge_ready }
    end
    if pr_reason == "head-sha-mismatch" and state.state == "merging" then
      log_gate(merge_ready, "fixing", "head-sha-mismatch")
      raise_fixing(repo, issue_number, merge_ready, state, current_pr, "head-sha-mismatch", {
        is_head = true,
        predecessors = {},
        predecessor_set = "none",
      })
      return
    end
    if pr_reason == "head-sha-mismatch" and state.state == "merge-ready" then
      local carried = core.raise_review_carry_over("merge", repo, merge_ready.pr_number, merge_ready.proposal_id, merge_ready.version, state, current_pr, origin.base_branch)
      if carried ~= nil then
        return
      end
      log_gate(merge_ready, "reviewing", "head-sha-mismatch")
      raise_reviewing_for_current_head(repo, issue_number, merge_ready, state, current_pr, "head-sha-mismatch")
      return
    end
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(" .. pr_reason .. ")", "write-time PR fact failed")
    return
  end

  local queue_entries = nil
  local queue_position = nil
  local speculative_fact = speculative_fix_fact_for_merge(current_pr.comments, merge_ready)
  if enforce_queue and tostring(current_pr.state or ""):upper() == "OPEN" then
    local queue_head
    queue_head, queue_entries = core.merge_queue_head(repo, branches.integration, {
      pr_number = merge_ready.pr_number,
      pr = current_pr,
    })
    if queue_head == nil then
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "hold-merge-queue", "merge-queue-empty")
      log_gate(merge_ready, "dry-run", "merge-queue-empty")
      return
    end
    local queue_ok = tostring(queue_head.proposal_id or "") == tostring(merge_ready.proposal_id or "")
      and tostring(queue_head.version or "") == tostring(merge_ready.version or "")
      and tostring(queue_head.pr_number or "") == tostring(merge_ready.pr_number or "")
      and tostring(queue_head.head_sha or "") == tostring(merge_ready.reviewed_head_sha or "")
    if not queue_ok then
      local queue_reason
      queue_position, queue_reason = core.merge_queue_position(repo, branches.integration, {
        pr_number = merge_ready.pr_number,
        pr = current_pr,
      })
      if queue_position == nil then
        core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "hold-merge-queue", queue_reason)
        log_gate(merge_ready, "dry-run", queue_reason)
        return
      end
      if not revalidate_speculative_predecessors(repo, issue_number, merge_ready, state, current_pr, queue_position, speculative_fact) then
        return
      end
      local mergeable, mergeable_reason = core.pr_mergeable(current_pr)
      if not mergeable and core.is_not_mergeable_reason(mergeable_reason) then
        local stale_mergeability, stale_reason = should_wait_for_stale_mergeability(current_pr, branches, mergeable_reason)
        if stale_mergeability then
          log_gate(merge_ready, "dry-run", stale_reason)
          error("github-devloop: merge wait on stale " .. tostring(mergeable_reason) .. "; retrying")
        end
        if not write_enabled then
          log_gate(merge_ready, "dry-run", "speculative fix requires FKST_GITHUB_WRITE=1")
          return
        end
        local capacity_ok, capacity_reason = core.wip_capacity_allows_start(repo, issue_number)
        if not capacity_ok then
          core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "fixing", "hold-wip-cap", capacity_reason)
          log_gate(merge_ready, "dry-run", capacity_reason)
          return
        end
        log_gate(merge_ready, "fixing", "speculative-" .. tostring(mergeable_reason))
        raise_fixing(repo, issue_number, merge_ready, state, current_pr, mergeable_reason, queue_position)
        return
      end
      core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "hold-merge-queue", "merge-queue-non-head")
      log_gate(merge_ready, "dry-run", "merge-queue-non-head")
      return
    end
    queue_position = {
      is_head = true,
      predecessors = {},
      predecessor_set = "none",
    }
  end
  if not revalidate_speculative_predecessors(repo, issue_number, merge_ready, state, current_pr, queue_position, speculative_fact) then
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

  local mergeable, mergeable_reason = core.pr_mergeable(current_pr)
  if not mergeable then
    if not core.is_not_mergeable_reason(mergeable_reason) then
      log_gate(merge_ready, "dry-run", mergeable_reason)
      error("github-devloop: merge wait on " .. tostring(mergeable_reason) .. "; retrying")
    end
    local stale_mergeability, stale_reason = should_wait_for_stale_mergeability(current_pr, branches, mergeable_reason)
    if stale_mergeability then
      log_gate(merge_ready, "dry-run", stale_reason)
      error("github-devloop: merge wait on stale " .. tostring(mergeable_reason) .. "; retrying")
    end
    log_gate(merge_ready, "fixing", mergeable_reason)
    raise_fixing(repo, issue_number, merge_ready, state, current_pr, mergeable_reason, queue_position)
    return
  end

  local rollup_green, rollup_reason = core.evaluate_ci_status_gate(current_pr, {
    repo = repo,
    dept = "merge",
    proposal_id = merge_ready.proposal_id,
  })
  if not rollup_green then
    if not core.is_ci_red_reason(rollup_reason) then
      if rollup_reason == "missing-status-rollup" then
        local dispatched, dispatch_reason = core.dispatch_ci_selfheal_once(
          repo,
          merge_ready.pr_number,
          current_pr,
          merge_ready.proposal_id
        )
        if dispatched then
          log_gate(merge_ready, "dry-run", "ci-dispatch-selfheal-dispatched; waiting for checks")
        else
          log_gate(merge_ready, "dry-run", dispatch_reason)
        end
      else
        log_gate(merge_ready, "dry-run", rollup_reason)
      end
      error("github-devloop: merge wait on " .. tostring(rollup_reason) .. "; retrying")
    end
    local fix_reason = core.rollup_red_fix_reason(current_pr, rollup_reason)
    log_gate(merge_ready, "fixing", fix_reason)
    raise_fixing(repo, issue_number, merge_ready, state, current_pr, fix_reason, queue_position)
    return
  end

  local pr_recheck = core.gh_exec({ cmd = core.gh_pr_view_merge_cmd(repo, merge_ready.pr_number), timeout = 30 })
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
  local rechecked_speculative_fact = speculative_fix_fact_for_merge(rechecked_pr_for_gate.comments, merge_ready)
  if not revalidate_speculative_predecessors(repo, issue_number, merge_ready, rechecked_state, rechecked_pr_for_gate, nil, rechecked_speculative_fact) then
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
    dept = "merge",
    proposal_id = merge_ready.proposal_id,
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
    raise_fixing(repo, issue_number, merge_ready, rechecked_state, merge_rechecked_pr, fix_reason, queue_position)
    return
  end
  if not merge_ok and core.is_not_mergeable_reason(merge_reason) then
    local stale_mergeability, stale_reason = should_wait_for_stale_mergeability(merge_rechecked_pr, branches, merge_reason)
    if stale_mergeability then
      log_gate(merge_ready, "dry-run", stale_reason)
      error("github-devloop: merge wait on write-time stale " .. tostring(merge_reason) .. "; retrying")
    end
    log_gate(merge_ready, "fixing", merge_reason)
    raise_fixing(repo, issue_number, merge_ready, rechecked_state, merge_rechecked_pr, merge_reason, queue_position)
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
  return { status = "merged", pr_number = merge_ready.pr_number, merge_ready = merge_ready, queue_entries = queue_entries }
end

local function synthesize_merge_ready_from_queue_head(repo, head)
  if type(head) ~= "table"
    or head.proposal_id == nil
    or head.pr_number == nil
    or head.version == nil
    or head.review_proposal_id == nil
    or head.review_dedup_key == nil
    or head.head_sha == nil then
    return nil
  end
  return core.build_devloop_merge_ready_payload(head.proposal_id, head.pr_number, head.version, {
    review_proposal_id = head.review_proposal_id,
    review_dedup_key = head.review_dedup_key,
    reviewed_head_sha = head.head_sha,
  }, core.pr_source_ref(repo, head.pr_number))
end

local function merge_queue_head_all(repo, base_branch)
  local head, entries = core.merge_queue_head(repo, base_branch)
  return head, entries or {}
end

local function chain_merge_queue_if_non_empty(repo, branches, merged_pr_number)
  local next_head = merge_queue_head_all(repo, branches.integration)
  if next_head == nil then
    core.log_line("info", "merge", "merge", "GATE", { "outcome=quiescent", "reason=merge-queue-empty-after-progress", "pass=poll" })
  else
    local payload = core.merge_queue_tick_payload(repo, merged_pr_number, next_head)
    core.log_raise("merge", tostring(next_head.proposal_id or "merge"), "devloop_merge_queue_tick", payload)
    raise("devloop_merge_queue_tick", payload)
  end
end

local function queue_starvation_cause_matches_entry(cause, entry)
  local cause_pr = tonumber(cause and cause.head_pr_number)
  if cause_pr == nil or type(entry) ~= "table" then
    return false
  end
  return tostring(entry.pr_number or "") == tostring(cause_pr)
    and tostring(entry.head_sha or "") == tostring(cause.head_sha or "")
    and tostring(entry.proposal_id or "") == tostring(cause.proposal_id or "")
    and tostring(entry.version or "") == tostring(cause.version or "")
end

local function queue_starvation_target_entry(cause, entries)
  for _, entry in ipairs(entries or {}) do
    if queue_starvation_cause_matches_entry(cause, entry) then
      return entry
    end
  end
  return nil
end

local function raise_queue_starvation_reconcile_only(repo, merge_ready, cause)
  local entity = core.parse_entity_proposal_id(merge_ready and merge_ready.proposal_id)
  if entity == nil then
    core.log_cas_decision("merge", merge_ready and merge_ready.proposal_id or "unknown", { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  if not core.verify_pr_review_issue_claim("merge", repo, entity.issue_number, nil, merge_ready.proposal_id) then
    return
  end
  local comment_request = core.build_queue_starvation_reconcile_comment_request(repo, merge_ready, cause)
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  raise("github-proxy.github_pr_comment_request", comment_request)
end

local function process_merge_queue_tick(event)
  local cause = type(event and event.payload) == "table" and event.payload.cause or nil
  local cause_kind = type(cause) == "table" and tostring(cause.kind or "") or ""
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or repo == "" then
    core.log_entry("merge", event, "unknown", "")
    core.log_line("info", "merge", "unknown", "GATE", {
      "outcome=skip",
      "reason=missing-repo-config",
      "pass=poll",
    })
    return
  end

  local lock_key = core.merge_lane_lock_key(repo)
  if lock_key == nil then
    core.log_entry("merge", event, "unknown", "")
    core.log_line("info", "merge", "unknown", "GATE", {
      "outcome=skip",
      "reason=no-transition-lock-key",
      "pass=poll",
    })
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()
    local head, entries = merge_queue_head_all(repo, branches.integration)
    if head == nil then
      core.log_line("info", "merge", "unknown", "GATE", {
        "outcome=skip",
        "reason=merge-queue-empty",
        "pass=poll",
      })
      return
    end
    local selected = head
    if cause_kind == "queue-starvation" then
      local cause_proposal = tostring(cause and cause.proposal_id or "")
      selected = queue_starvation_target_entry(cause, entries)
      if selected == nil then
        core.log_line("info", "merge", tostring(cause_proposal ~= "" and cause_proposal or head.proposal_id), "GATE", {
          "pr=" .. tostring(head.pr_number),
          "reported_pr=" .. tostring(cause and cause.head_pr_number or ""),
          "version=" .. tostring(head.version),
          "outcome=hold",
          "reason=queue-starvation-target-not-current",
          "incident=" .. tostring(cause and cause.incident_identity or ""),
          "pass=poll",
        })
        return
      end
      core.log_line("info", "merge", selected.proposal_id, "GATE", {
        "pr=" .. tostring(selected.pr_number),
        "version=" .. tostring(selected.version),
        "outcome=reconcile",
        "reason=queue-starvation-redrive",
        "incident=" .. tostring(cause.incident_identity or ""),
        "pass=poll",
      })
    end
    if selected.state == "merging" then
      core.log_line("info", "merge", selected.proposal_id, "GATE", {
        "pr=" .. tostring(selected.pr_number),
        "version=" .. tostring(selected.version),
        "outcome=skip",
        "reason=merge-queue-head-merging",
        "pass=poll",
      })
      return
    end
    local merge_ready = synthesize_merge_ready_from_queue_head(repo, selected)
    if merge_ready == nil or not core.is_supported_merge_ready(merge_ready) then
      core.log_line("info", "merge", selected.proposal_id, "GATE", {
        "pr=" .. tostring(selected.pr_number),
        "version=" .. tostring(selected.version),
        "outcome=skip",
        "reason=merge-queue-head-missing-merge-ready-fact",
        "pass=poll",
      })
      return
    end
    local entity = core.parse_entity_proposal_id(merge_ready.proposal_id)
    if entity == nil then
      core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
      return
    end
    merge_ready._merge_pass = "poll"
    core.log_entry("merge", event, merge_ready.proposal_id, merge_ready.dedup_key)
    if cause_kind == "queue-starvation" and not queue_starvation_cause_matches_entry(cause, head) then
      raise_queue_starvation_reconcile_only(repo, merge_ready, cause)
      return
    end
    local write_mode = core.write_mode()
    local outcome = process_merge_ready_locked(repo, entity.issue_number, merge_ready, branches, nil, {
      enforce_queue = false,
      write_mode = write_mode,
      queue_starvation_cause = cause_kind == "queue-starvation" and cause or nil,
    })
    if outcome ~= nil and outcome.status == "merged" then
      local last_merged_pr_number = core.run_merge_batch_window(repo, branches, merge_ready, entries, { write_mode = write_mode }, process_merge_ready_locked)
      chain_merge_queue_if_non_empty(repo, branches, last_merged_pr_number or outcome.pr_number)
    end
  end)
end
local function process_merge_ready_event(event)
  local merge_ready = type(event and event.payload) == "table" and event.payload or {}
  if not core.is_supported_merge_ready(merge_ready) then
    core.log_entry("merge", event, "unknown", core.payload_field(merge_ready, "dedup_key"))
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
  local lock_key = core.merge_lane_lock_key(repo)
  if lock_key == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()
    process_merge_ready_locked(repo, issue_number, merge_ready, branches)
  end)
end

function pipeline(event)
  core.dispatch_consumed_queue("merge", M.spec, event, {
    devloop_merge_queue_tick = function()
      process_merge_queue_tick(event)
    end,
    devloop_merge_ready = process_merge_ready_event,
  })
end
pipeline = core.wrap_pipeline_failure("merge", pipeline)
return M
