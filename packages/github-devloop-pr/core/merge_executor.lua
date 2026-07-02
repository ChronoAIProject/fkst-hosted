local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local m_claims = require("devloop.claims")
local requests_bodies = require("devloop.requests.bodies")
local requests_labels = require("devloop.requests.labels")
local requests_lifecycle = require("devloop.requests.lifecycle")
local requests_review = require("devloop.requests.review")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local core = require("core")
local runtime_files = require("core.merge_runtime_files")
local ci_wait = require("core.merge_ci_wait")
local high_risk_merge_gate = require("core.high_risk_merge_gate")
local check_runs = require("forge.github.check_runs")
local merge_batch = require("devloop.merge_batch")
local autonomy_ledger = require("devloop.autonomy_ledger")
local payloads_builders = require("devloop.payloads.builders")
local conv_reconcile = require("devloop.convergence.reconcile")
local v_merge_ready = require("devloop.validators.merge_ready")
local m_facts = require("devloop.markers.facts")
local m_mq = require("devloop.merge_queue")
local M = {}
local github = require("forge.github").production_handle
local config = require("devloop.config")
local devloop_entity_view = require("devloop.github_proxy_entity_view")

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
  local ok, reason = m_facts.review_result_approval_matches_event(core, comments, merge_ready)
  if ok then
    return true
  end
  log_gate(merge_ready, "dry-run", "merge requires trusted review-result approve: " .. tostring(reason))
  return false
end

local function gate_baseline_sha_from_pr(pr)
  local baseline_sha = tostring(pr and pr.base_ref_oid or "")
  if not require("devloop.pr_safety").is_safe_head_sha(baseline_sha) then
    error("github-devloop: unsafe merge-gate baseline sha")
  end
  return baseline_sha
end

local function gate_baseline_sha_for_reason(_proposal_id, _pr_number, pr, _reason)
  return gate_baseline_sha_from_pr(pr)
end

local function pr_head_contains_current_base(pr, branches)
  local base_head, base_reason = git_mechanics.current_base_head(core.git, branches.integration)
  if base_head == nil then
    return false, base_reason
  end
  local head_sha = tostring(pr and pr.head_sha or "")
  if not require("devloop.pr_safety").is_safe_head_sha(head_sha) then
    return false, "unsafe-pr-head"
  end
  local result = git_mechanics.git_is_ancestor(core.git, base_head, head_sha, 30)
  if result.exit_code == 0 then
    return true, "current-base-contained"
  end
  return false, "current-base-not-contained"
end

local function should_wait_for_stale_mergeability(pr, branches, mergeable_reason)
  if not check_runs.is_not_mergeable_reason(mergeable_reason) then
    return false, "not-stale-mergeability"
  end
  return pr_head_contains_current_base(pr, branches)
end

local function raise_decompose_for_max_fix_rounds(merge_ready, current_state, reason, source_ref)
  local fix_reconcile = conv_reconcile.build_devloop_fix_reconcile_payload(core, {
    proposal_id = merge_ready.proposal_id,
    review_proposal_id = merge_ready.review_proposal_id,
    review_dedup_key = merge_ready.review_dedup_key,
    reviewed_head_sha = merge_ready.reviewed_head_sha,
    pr_number = merge_ready.pr_number,
    source_ref = source_ref,
  }, current_state.version)
  local decompose = payloads_builders.build_devloop_decompose_payload(core, fix_reconcile)
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "blocked", "applied(fix-loop-max-rounds)", reason)
  core.log_raise("merge", merge_ready.proposal_id, "devloop_fix_reconcile", fix_reconcile)
  core.log_raise("merge", merge_ready.proposal_id, "github-devloop-decompose.devloop_decompose", decompose)
end

local function raise_fixing(repo, issue_number, merge_ready, current_state, current_pr, reason, queue_position)
  local source_ref = entity_lib.pr_source_ref(repo, merge_ready.pr_number)
  if core.version_fix_round(current_state.version) >= config.max_fix_rounds(core) then
    raise_decompose_for_max_fix_rounds(merge_ready, current_state, reason, source_ref)
    return
  end
  local fix_version = core.fix_version_from_review_version(current_state.version)
  local gate_baseline_sha = gate_baseline_sha_for_reason(merge_ready.proposal_id, merge_ready.pr_number, current_pr, reason)
  local predecessor_set = nil
  if queue_position ~= nil then
    predecessor_set = queue_position.predecessor_set
  else
    local branches = config.branch_config(core)
    local position, predecessor_reason = m_mq.merge_queue_position(core, repo, branches.integration, {
      pr_number = merge_ready.pr_number,
      pr = current_pr,
    })
    if position == nil then
      error("github-devloop: merge queue predecessor derivation failed: " .. tostring(predecessor_reason))
    end
    predecessor_set = position.predecessor_set
  end
  local comment_request = requests_review.build_merge_gate_fix_comment_request(core, repo, issue_number, merge_ready, fix_version, reason, gate_baseline_sha, source_ref, predecessor_set)
  local label_request = issue_number ~= nil and requests_labels.build_state_label_request(core,
    repo,
    issue_number,
    "fixing",
    merge_ready.dedup_key .. "/label/fixing",
    entity_lib.issue_source_ref(repo, issue_number)
  ) or nil
  local add_labels, remove_labels = core.state_label_changes("fixing")
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "fixing", "applied", reason)
  local raised = {
    "github-proxy.github_pr_comment_request",
  }
  if label_request ~= nil then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  core.log_apply("merge", merge_ready.proposal_id, "fixing", fix_version, { add = add_labels, remove = remove_labels }, raised)
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
end

local function raise_reviewing_for_current_head(repo, issue_number, merge_ready, current_state, current_pr, reason)
  local source_ref = entity_lib.pr_source_ref(repo, merge_ready.pr_number)
  local review_version = core.next_review_loop_version(merge_ready.version)
  if core.has_state_marker(current_pr.comments, merge_ready.proposal_id, "reviewing", review_version) then
    core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "reviewing", "skip-idempotent(already at to_state)", reason)
    return
  end
  local current_head_sha = tostring(current_pr.head_sha or "")
  local comment_request = requests_review.build_merge_head_reviewing_comment_request(core, repo, issue_number, merge_ready, merge_ready.reviewed_head_sha, current_head_sha, review_version, source_ref)
  local label_request = issue_number ~= nil and requests_labels.build_merge_head_reviewing_label_request(core, repo, issue_number, merge_ready, current_head_sha, review_version, entity_lib.issue_source_ref(repo, issue_number)) or nil
  local add_labels, remove_labels = core.state_label_changes("reviewing")
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "reviewing", "applied", reason)
  local raised = {
    "github-proxy.github_pr_comment_request",
  }
  if label_request ~= nil then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  core.log_apply("merge", merge_ready.proposal_id, "reviewing", review_version, { add = add_labels, remove = remove_labels }, raised)
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
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
  local state = require("devloop.entity").current_entity_state(core, pr.comments, merge_ready.proposal_id)
  if (state.state ~= "merge-ready" and state.state ~= "merging")
    or tostring(state.version or "") ~= tostring(merge_ready.version) then
    return false, "write-time PR state changed", state
  end

  if not require_consensus_review_approve(pr.comments, merge_ready) then
    return false, "trusted review-result approve missing", state
  end

  local fact = m_facts.merge_ready_fact(core, pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number, merge_ready.reviewed_head_sha)
  local approval_ok, approval_reason = m_facts.merge_ready_approval_matches_event(core, fact, merge_ready)
  if not approval_ok then
    return false, "merge-ready fact changed: " .. tostring(approval_reason), state
  end

  local current_origin = m_facts.pr_origin_fact(core, pr.comments)
  if current_origin == nil then
    current_origin = entity_lib.pr_native_origin(repo, merge_ready.pr_number, pr)
  end
  local origin_issue_matches = (issue_number == nil and current_origin.pr_native == true)
    or tostring(current_origin.issue_number) == tostring(issue_number)
  if current_origin.proposal_id ~= merge_ready.proposal_id
    or current_origin.repo ~= repo
    or not origin_issue_matches
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
  local fact = m_facts.merge_gate_fix_fact(core, comments, merge_ready.proposal_id, fix_version)
  if fact == nil or fact.predecessor_set == nil then
    return nil
  end
  if not m_facts.has_fix_marker(core,
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
    local branches = config.branch_config(core)
    local position, reason = m_mq.merge_queue_position(core, repo, branches.integration, {
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
  local branches = config.branch_config(core)
  local matches, match_reason = m_mq.merge_queue_predecessor_set_matches_current_base(core,
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
  local ready_result = core.gh_pr_ready(repo, merge_ready.pr_number, 60)
  if ready_result.exit_code ~= 0 then
    error("github-devloop: PR ready failed: " .. tostring(ready_result.stderr))
  end

  local pr_view = github("github-devloop-pr.merge_executor").gh_pr_view_merge(repo, merge_ready.pr_number, 30)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: PR ready recheck failed: " .. tostring(pr_view.stderr))
  end
  return parsers_pr.parse_pr_view_merge(core, pr_view.stdout)
end

local function build_merging_body(merge_ready)
  return requests_bodies.build_merging_comment_body(core, merge_ready)
end
local function write_merging_marker(repo, merge_ready, comments)
  if m_facts.merging_fact(core, comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) ~= nil then
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
  local result = core.gh_pr_comment(repo, merge_ready.pr_number, path, 30)
  if result.exit_code ~= 0 then
    error("github-devloop: PR merging marker comment failed: " .. tostring(result.stderr))
  end
  devloop_entity_view.invalidate_entity_after_write(repo, "pr", merge_ready.pr_number)
end

local function build_merged_requests(repo, issue_number, merge_ready, merged_pr)
  local merged_source_ref = entity_lib.pr_source_ref(repo, merge_ready.pr_number)
  local autonomy_record = issue_number ~= nil and autonomy_ledger.autonomy_result_record(core, repo, issue_number, merge_ready, nil, merged_pr) or nil
  local merged_body = requests_bodies.build_merged_comment_body(core, merge_ready, autonomy_record)
  local comment_request = entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, merged_body, merge_ready.dedup_key .. "/comment/merged", merged_source_ref)
  return comment_request
end

local function finalize_merged(repo, issue_number, merge_ready, current_state, reason, merged_pr)
  local comment_request = build_merged_requests(repo, issue_number, merge_ready, merged_pr)
  core.log_cas_decision("merge", merge_ready.proposal_id, current_state, "merge-ready", "merged", "applied", reason)
  core.log_apply("merge", merge_ready.proposal_id, "merged", merge_ready.version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
  })
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
end

local function process_merge_ready_locked(repo, issue_number, merge_ready, branches, initial_pr, options)
  local enforce_queue = options == nil or options.enforce_queue ~= false
  local write_mode = options and options.write_mode or nil
  local entity = entity_lib.parse_entity_proposal_id(merge_ready.proposal_id)
  if entity == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local entity_matches = tostring(entity.repo or "") == tostring(repo or "")
    and ((entity.kind == "issue" and tostring(entity.issue_number or "") == tostring(issue_number or ""))
      or (entity.kind == "pr" and issue_number == nil and tostring(entity.pr_number or "") == tostring(merge_ready.pr_number or "")))
  if not entity_matches then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end
  if issue_number == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "claim", "claim", "skip-not-owned", "backing issue is absent")
    return
  end
  if issue_number ~= nil and not m_claims.verify_pr_review_issue_claim(core, "merge", repo, issue_number, nil, merge_ready.proposal_id) then
    return
  end
  if options ~= nil and type(options.queue_starvation_cause) == "table" then
    local comment_request = requests_lifecycle.build_queue_starvation_reconcile_comment_request(core, repo, merge_ready, options.queue_starvation_cause)
    core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    raise("github-proxy.github_pr_comment_request", comment_request)
  end
  local current_pr = initial_pr
  if current_pr == nil then
    local pr_view = github("github-devloop-pr.merge_executor").gh_pr_view_merge(repo, merge_ready.pr_number, 30)
    if pr_view.exit_code ~= 0 then
      error("github-devloop: PR merge view failed: " .. tostring(pr_view.stderr))
    end
    current_pr = parsers_pr.parse_pr_view_merge(core, pr_view.stdout)
  end
  core.log_forged_markers("merge", merge_ready.proposal_id, current_pr.comments)
  local state = require("devloop.entity").current_entity_state(core, current_pr.comments, merge_ready.proposal_id)
  if state.state == "merged" and m_facts.has_merged_marker(core, current_pr.comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha) then
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
  local fact = m_facts.merge_ready_fact(core, current_pr.comments, merge_ready.proposal_id, merge_ready.version, merge_ready.pr_number, merge_ready.reviewed_head_sha)
  if fact == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "retry-pending(merge-ready fact marker not visible)", "trusted merge-ready fact marker missing")
    error("github-devloop: merge-ready fact marker not visible for merge; retrying")
  end
  local approval_ok, approval_reason = m_facts.merge_ready_approval_matches_event(core, fact, merge_ready)
  if not approval_ok then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-stale(" .. tostring(approval_reason) .. ")", "merge-ready event does not match canonical approval fact marker")
    return
  end
  local origin = m_facts.pr_origin_fact(core, current_pr.comments)
  if origin == nil then
    origin = entity_lib.pr_native_origin(repo, merge_ready.pr_number, current_pr)
  end
  if origin.proposal_id ~= merge_ready.proposal_id
    or origin.repo ~= repo
    or tostring(origin.base_branch) ~= tostring(branches.integration)
    or tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch) then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "skip-foreign(pr-origin)", "PR origin/link does not match immutable PR branch")
    return
  end
  local write_enabled = (write_mode or config.write_mode(core)) == "real"
  local pr_ok, pr_reason = assert_open_same_repo_pr(merge_ready, current_pr, repo, origin.branch, merge_ready.reviewed_head_sha)
  if not pr_ok then
    if core.is_merged_pr(current_pr)
      and tostring(current_pr.head_ref_name or "") == tostring(origin.branch)
      and tostring(current_pr.head_sha or "") == tostring(merge_ready.reviewed_head_sha)
      and require("forge.merge.shared").is_same_repo_pr_head(current_pr, repo) then
      local merging_fact = m_facts.merging_fact(core, current_pr.comments, merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
      if merging_fact == nil then
        core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merged", "skip-external-merge(no-bot-merging-marker)", "PR is already merged without a prior trusted bot merging marker")
        return
      end
      if not write_enabled then
        log_gate(merge_ready, "dry-run", "PR already merged; finalization requires FKST_GITHUB_WRITE=1")
        return
      end
      finalize_merged(repo, issue_number, merge_ready, state, "PR already merged; self-healing finalization", current_pr)
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
    queue_head, queue_entries = m_mq.merge_queue_head(core, repo, branches.integration, {
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
      queue_position, queue_reason = m_mq.merge_queue_position(core, repo, branches.integration, {
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
      local mergeable, mergeable_reason = check_runs.pr_mergeable(current_pr)
      if not mergeable and check_runs.is_not_mergeable_reason(mergeable_reason) then
        local stale_mergeability, stale_reason = should_wait_for_stale_mergeability(current_pr, branches, mergeable_reason)
        if stale_mergeability then
          log_gate(merge_ready, "dry-run", stale_reason)
          error("github-devloop: merge wait on stale " .. tostring(mergeable_reason) .. "; retrying")
        end
        if not write_enabled then
          log_gate(merge_ready, "dry-run", "speculative fix requires FKST_GITHUB_WRITE=1")
          return
        end
        local capacity_ok, capacity_reason = m_mq.wip_capacity_allows_start(core, repo, issue_number)
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
  high_risk_merge_gate.assert_evidence(core, log_gate, repo, current_pr.comments, merge_ready)
  log_gate(merge_ready, "write-ready", "FKST_GITHUB_WRITE=1 and trusted review-result approve")

  current_pr = ensure_pr_ready_for_merge(repo, merge_ready, current_pr)
  local ready_ok, ready_reason = assert_merge_pr_authority(merge_ready, current_pr, repo, issue_number, origin, branches)
  if not ready_ok then
    core.log_cas_decision("merge", merge_ready.proposal_id, state, "merge-ready", "merging", "fail-closed(ready-recheck)", ready_reason)
    error("github-devloop: PR fact changed after ready conversion")
  end

  local mergeable, mergeable_reason = check_runs.pr_mergeable(current_pr)
  if not mergeable then
    if not check_runs.is_not_mergeable_reason(mergeable_reason) then
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

  local rollup_green, rollup_reason, check_runs = core.evaluate_ci_status_gate(current_pr, {
    repo = repo,
    dept = "merge",
    proposal_id = merge_ready.proposal_id,
  })
  if not rollup_green then
    if rollup_reason == "rollup-red" then
      local classification = core.classify_pr_ci_gate(current_pr, {
        repo = repo,
        dept = "merge",
        proposal_id = merge_ready.proposal_id,
      })
      if classification.kind ~= "OWN_CI_RED" then
        log_gate(merge_ready, "hold", classification.reason)
        ci_wait.hold(core, merge_ready, repo, current_pr, classification)
        return
      end
      log_gate(merge_ready, "fixing", classification.reason)
      raise_fixing(repo, issue_number, merge_ready, state, current_pr, classification.reason, queue_position)
      return
    end
    if not parsers_misc.is_ci_red_reason(core, rollup_reason) then
      if rollup_reason == "missing-status-rollup" then
        local healed, heal_reason = core.ci_selfheal_once(
          repo,
          merge_ready.pr_number,
          current_pr,
          merge_ready.proposal_id,
          nil,
          check_runs
        )
        if healed then
          log_gate(merge_ready, "dry-run", "ci-selfheal-triggered; waiting for checks")
        else
          log_gate(merge_ready, "dry-run", heal_reason)
        end
      else
        log_gate(merge_ready, "dry-run", rollup_reason)
      end
      error("github-devloop: merge wait on " .. tostring(rollup_reason) .. "; retrying")
    end
    log_gate(merge_ready, "fixing", rollup_reason)
    raise_fixing(repo, issue_number, merge_ready, state, current_pr, rollup_reason, queue_position)
    return
  end

  local pr_recheck = github("github-devloop-pr.merge_executor").gh_pr_view_merge(repo, merge_ready.pr_number, 30)
  if pr_recheck.exit_code ~= 0 then
    error("github-devloop: PR merge recheck failed: " .. tostring(pr_recheck.stderr))
  end
  local rechecked_pr_for_gate = parsers_pr.parse_pr_view_merge(core, pr_recheck.stdout)
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
  core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merging", "applied", "all merge gates satisfied; invoking PR merge")
  local merge_ok, merge_reason, merge_rechecked_pr = core.run_verified_pr_merge({
    repo = repo,
    pr_number = merge_ready.pr_number,
    head_sha = merge_ready.reviewed_head_sha,
    head_branch = origin.branch,
    base_branch = branches.integration,
    dept = "merge",
    proposal_id = merge_ready.proposal_id,
    validate_rechecked_pr = function(rechecked_pr)
      local recheck_origin = m_facts.pr_origin_fact(core, rechecked_pr.comments)
      if recheck_origin == nil then
        recheck_origin = entity_lib.pr_native_origin(repo, merge_ready.pr_number, rechecked_pr)
      end
      local recheck_origin_issue_matches = (issue_number == nil and recheck_origin.pr_native == true)
        or tostring(recheck_origin.issue_number) == tostring(issue_number)
      if recheck_origin.proposal_id ~= merge_ready.proposal_id
        or recheck_origin.repo ~= repo
        or not recheck_origin_issue_matches
        or tostring(recheck_origin.branch) ~= tostring(origin.branch)
        or tostring(recheck_origin.impl_version) ~= tostring(origin.impl_version)
        or tostring(recheck_origin.base_branch) ~= tostring(origin.base_branch)
        or tostring(rechecked_pr.base_ref_name or "") ~= tostring(origin.base_branch)
        or tostring(origin.base_branch) ~= tostring(branches.integration) then
        return false, "pr-origin-changed"
      end
      local evidence_ok, evidence_reason = high_risk_merge_gate.require_evidence(core, repo, rechecked_pr.comments, merge_ready)
      if not evidence_ok then
        return false, evidence_reason
      end
      return true, "pr-origin-ok"
    end,
    before_merge = function()
      write_merging_marker(repo, merge_ready, rechecked_pr_for_gate.comments)
    end,
  })
  if not merge_ok and merge_reason == "merge-confirmation-pending" then
    core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merged", "retry-pending(merge-confirmation)", "PR merge returned without a merged PR fact")
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
  if not merge_ok and parsers_misc.is_ci_red_reason(core, merge_reason) then
    log_gate(merge_ready, "fixing", merge_reason)
    raise_fixing(repo, issue_number, merge_ready, rechecked_state, merge_rechecked_pr, merge_reason, queue_position)
    return
  end
  if not merge_ok and parsers_misc.is_ci_wait_reason(core, merge_reason) then
    log_gate(merge_ready, "hold", merge_reason)
    ci_wait.hold(core, merge_ready, repo, merge_rechecked_pr or rechecked_pr_for_gate, {
      kind = "CI_WAIT",
      reason = merge_reason,
    })
    return
  end
  if not merge_ok and check_runs.is_not_mergeable_reason(merge_reason) then
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
  if not merge_ok and tostring(merge_reason or ""):find("retry%-pending%(high%-risk%-review%-evidence", 1) ~= nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merging", merge_reason, "trusted high-risk review evidence marker is not visible")
    error("github-devloop: high-risk review evidence marker not visible at write-time; retrying")
  end
  if not merge_ok then
    core.log_cas_decision("merge", merge_ready.proposal_id, rechecked_state, "merge-ready", "merging", "fail-closed(write-gate)", "write-time PR fact failed: " .. tostring(merge_reason))
    error("github-devloop: write-time PR fact changed before merge")
  end

  finalize_merged(repo, issue_number, merge_ready, rechecked_state, "PR merge confirmed merged", merge_rechecked_pr)
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
  return payloads_builders.build_devloop_merge_ready_payload(core, head.proposal_id, head.pr_number, head.version, {
    review_proposal_id = head.review_proposal_id,
    review_dedup_key = head.review_dedup_key,
    reviewed_head_sha = head.head_sha,
  }, entity_lib.pr_source_ref(repo, head.pr_number))
end
local function merge_queue_head_all(repo, base_branch)
  local head, entries = m_mq.merge_queue_head(core, repo, base_branch); return head, entries or {}
end
local function chain_merge_queue_if_non_empty(repo, branches, merged_pr_number)
  local next_head = merge_queue_head_all(repo, branches.integration)
  if next_head == nil then
    core.log_line("info", "merge", "merge", "GATE", { "outcome=quiescent", "reason=merge-queue-empty-after-progress", "pass=poll" })
  else
    local payload = m_mq.merge_queue_tick_payload(core, repo, merged_pr_number, next_head)
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
  local target = nil
  for _, entry in ipairs(entries or {}) do
    if queue_starvation_cause_matches_entry(cause, entry) then
      target = entry
      break
    end
  end
  if target == nil then
    return nil, nil, "target-not-current"
  end
  local candidate, age_minutes = m_mq.merge_queue_starvation_candidate(core, entries, m_mq._merge_ready_starvation_threshold_minutes, now())
  if not queue_starvation_cause_matches_entry(cause, candidate) then
    return nil, age_minutes, "target-not-aged-candidate"
  end
  return target, age_minutes, "aged-candidate"
end
local function process_merge_queue_tick(event)
  local cause = type(event and event.payload) == "table" and event.payload.cause or nil
  local cause_kind = type(cause) == "table" and tostring(cause.kind or "") or ""
  local repo = devloop_base.read_env("FKST_GITHUB_REPO")
  if repo == nil or repo == "" then
    core.log_entry("merge", event, "unknown", "")
    core.log_line("info", "merge", "unknown", "GATE", {
      "outcome=skip",
      "reason=missing-repo-config",
      "pass=poll",
    })
    return
  end
  local lock_key = entity_lib.merge_lane_lock_key(repo)
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
    devloop_base.assert_trusted_bot_configured()
    local branches = config.branch_config(core)
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
      local selected_age
      local selected_reason
      selected, selected_age, selected_reason = queue_starvation_target_entry(cause, entries)
      if selected == nil then
        core.log_line("info", "merge", tostring(cause_proposal ~= "" and cause_proposal or head.proposal_id), "GATE", {
          "pr=" .. tostring(head.pr_number),
          "reported_pr=" .. tostring(cause and cause.head_pr_number or ""),
          "version=" .. tostring(head.version),
          "outcome=hold",
          "reason=queue-starvation-" .. tostring(selected_reason or "target-not-current"),
          "age_minutes=" .. tostring(selected_age or ""),
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
        "age_minutes=" .. tostring(selected_age or ""),
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
    if merge_ready == nil or not v_merge_ready.is_supported_merge_ready(core, merge_ready) then
      core.log_line("info", "merge", selected.proposal_id, "GATE", {
        "pr=" .. tostring(selected.pr_number),
        "version=" .. tostring(selected.version),
        "outcome=skip",
        "reason=merge-queue-head-missing-merge-ready-fact",
        "pass=poll",
      })
      return
    end
    local entity = entity_lib.parse_entity_proposal_id(merge_ready.proposal_id)
    if entity == nil then
      core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
      return
    end
    merge_ready._merge_pass = "poll"
    core.log_entry("merge", event, merge_ready.proposal_id, merge_ready.dedup_key)
    local selected_is_fifo_head = queue_starvation_cause_matches_entry(cause, head)
    local write_mode = config.write_mode(core)
    local outcome = process_merge_ready_locked(repo, entity.issue_number, merge_ready, branches, nil, {
      enforce_queue = false,
      write_mode = write_mode,
      queue_starvation_cause = cause_kind == "queue-starvation" and cause or nil,
    })
    if outcome ~= nil and outcome.status == "merged" then
      local last_merged_pr_number = outcome.pr_number
      if cause_kind ~= "queue-starvation" or selected_is_fifo_head then
        last_merged_pr_number = merge_batch.run_merge_batch_window(core, repo, branches, merge_ready, entries, { write_mode = write_mode }, process_merge_ready_locked)
      end
      chain_merge_queue_if_non_empty(repo, branches, last_merged_pr_number or outcome.pr_number)
    end
  end)
end
local function process_merge_ready_event(event)
  local merge_ready = type(event and event.payload) == "table" and event.payload or {}
  if not v_merge_ready.is_supported_merge_ready(core, merge_ready) then
    core.log_entry("merge", event, "unknown", core.payload_field(merge_ready, "dedup_key"))
    core.log_cas_decision("merge", "unknown", { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("merge", event, merge_ready.proposal_id, merge_ready.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(merge_ready.proposal_id)
  if entity == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local lock_key = entity_lib.merge_lane_lock_key(repo)
  if lock_key == nil then
    core.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()
    local branches = config.branch_config(core)
    process_merge_ready_locked(repo, issue_number, merge_ready, branches)
  end)
end

M.process_merge_queue_tick = process_merge_queue_tick
M.process_merge_ready_event = process_merge_ready_event

return M
