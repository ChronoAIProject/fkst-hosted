local core = require("core")

local M = {}
local ai_sentinel = "⟦AI:FKST⟧"

M.spec = {
  consumes = { "github-proxy.github_entity_changed", "github-proxy.github_pr_opened" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_decompose",
    "devloop_merge_ready",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function pr_source_ref(repo, pr_number)
  return core.pr_source_ref(repo, pr_number)
end

local function pr_context(event)
  local payload = event.payload or {}
  if core.is_supported_pr(payload) then
    return {
      repo = payload.repo,
      number = payload.number,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  if core.is_supported_pr_opened(payload) then
    return {
      repo = payload.repo,
      number = payload.pr_number,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  return nil
end

local function origin_from_pr(repo, pr_number, current_pr)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin ~= nil then
    return origin, true
  end
  return core.pr_native_origin(repo, pr_number, current_pr), false
end

local function origin_matches_pr(origin, current_pr, repo, branches, require_issue_backing)
  if origin.repo ~= repo then
    return false, "repo"
  end
  if require_issue_backing and origin.issue_number == nil then
    return false, "issue"
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    return false, "head"
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch) then
    return false, "base"
  end
  if origin.base_branch ~= nil
    and tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    return false, "base"
  end
  return true, "ok"
end

local function maybe_label_hint(origin, state, source_ref)
  if origin.issue_number == nil then
    return
  end
  local label_request = core.build_reconcile_state_label_request(origin.repo, origin.issue_number, origin.proposal_id, state.state, state.version, source_ref)
  local add_labels, remove_labels = core.state_label_changes(state.state)
  core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function issue_comments_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  local issue_view = core.gh_exec({ cmd = core.gh_issue_view_result_cmd(origin.repo, origin.issue_number), timeout = 30 })
  if issue_view.exit_code ~= 0 then
    error("github-devloop: gh issue result view failed: " .. tostring(issue_view.stderr))
  end
  return core.parse_issue_view_result(issue_view.stdout).comments
end

local function has_reviewing_marker_for_comments(comments, proposal_id, version)
  return core.has_state_marker(comments, proposal_id, "reviewing", version)
end

local function has_reviewing_marker(issue_comments, pr_comments, proposal_id, version)
  return has_reviewing_marker_for_comments(issue_comments, proposal_id, version)
    or has_reviewing_marker_for_comments(pr_comments, proposal_id, version)
end

local function run_git(cmd, timeout, label)
  local result = exec_sync({ cmd = cmd, timeout = timeout })
  if result.exit_code ~= 0 then
    return nil, tostring(label or "git command") .. " failed: " .. tostring(result.stderr)
  end
  return result
end

local function current_base_head(origin)
  local fetch_result, fetch_error = run_git(core.git_fetch_branch_cmd("origin", origin.base_branch), 60, "git base fetch")
  if fetch_result == nil then
    return nil, fetch_error
  end
  local head_result, head_error = run_git(core.git_remote_branch_head_cmd("origin", origin.base_branch), 30, "git base head")
  if head_result == nil then
    return nil, head_error
  end
  local base_head = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not core.is_safe_head_sha(base_head) then
    return nil, "unsafe base head"
  end
  return base_head
end

local function has_empty_resolution_delta(approved_head_sha, base_head_sha, new_head_sha)
  local result = exec_sync({
    cmd = core.git_merge_tree_empty_delta_cmd(approved_head_sha, base_head_sha, new_head_sha),
    timeout = 120,
  })
  if result.exit_code == 0 then
    return true, "empty"
  end
  return false, tostring(result.stderr or "")
end

local function current_head_review_payload(origin, pr_number, current_pr, state, source_ref)
  local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
  if core.has_any_review_result_marker(current_pr.comments, review_proposal_id, origin.proposal_id) then
    return nil
  end
  return core.build_devloop_reviewing_payload({
    proposal_id = origin.proposal_id,
    impl_version = state.version,
  }, pr_number, source_ref, state.version)
end

local function build_review_carry_over_comment_request(origin, pr_number, version, carry, source_ref)
  local state_marker = core.state_marker(origin.proposal_id, "merge-ready", version)
  local review_marker = core.review_result_marker(carry.new_review_proposal_id, origin.proposal_id, "approve", carry.new_review_dedup_key)
  local merge_marker = core.merge_ready_marker(origin.proposal_id, pr_number, version, carry.new_review_proposal_id, carry.new_review_dedup_key, carry.new_head_sha)
  local carry_marker = core.review_carry_over_marker(
    origin.proposal_id,
    version,
    carry.old_review_proposal_id,
    carry.old_review_dedup_key,
    carry.approved_head_sha,
    carry.new_review_proposal_id,
    carry.new_review_dedup_key,
    carry.new_head_sha,
    carry.base_head_sha
  )
  return core.build_entity_comment_request({
    kind = "pr",
    repo = origin.repo,
    number = pr_number,
  }, "github-devloop PR review approval carried over"
    .. "\nResolution delta proof: merge-tree-empty-delta"
    .. "\nApproved head: " .. tostring(carry.approved_head_sha)
    .. "\nNew head: " .. tostring(carry.new_head_sha)
    .. "\nBase head: " .. tostring(carry.base_head_sha)
    .. "\n\n" .. state_marker
    .. "\n" .. review_marker
    .. "\n" .. merge_marker
    .. "\n" .. carry_marker
    .. "\n" .. ai_sentinel, core._dedup_key({
    "review-carry-over",
    "comment",
    tostring(origin.proposal_id),
    tostring(version),
    tostring(carry.approved_head_sha),
    tostring(carry.new_head_sha),
  }), source_ref)
end

local function maybe_carry_over_approved_head(origin, pr_number, current_pr, state, source_ref)
  if state.state ~= "merge-ready" then
    return false
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return false
  end
  if not core.is_safe_head_sha(current_pr.head_sha) then
    return false
  end
  local fact = core.merge_ready_fact(current_pr.comments, origin.proposal_id, state.version, pr_number)
  if fact == nil or tostring(fact.head_sha or "") == tostring(current_pr.head_sha or "") then
    return false
  end
  local approved = {
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = state.version,
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    reviewed_head_sha = fact.head_sha,
  }
  local approval_ok = core.review_result_approval_matches_event(current_pr.comments, approved)
  if not approval_ok then
    return false
  end
  local base_head, base_error = current_base_head(origin)
  if base_head == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "merge-ready", "reviewing", "skip-stale(carry-over-proof-unavailable)", base_error)
    local reviewing_payload = current_head_review_payload(origin, pr_number, current_pr, state, source_ref)
    if reviewing_payload ~= nil then
      core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    end
    return true
  end
  local empty_delta, delta_reason = has_empty_resolution_delta(fact.head_sha, base_head, current_pr.head_sha)
  if not empty_delta then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "merge-ready", "reviewing", "skip-stale(non-empty-resolution-delta)", delta_reason)
    local reviewing_payload = current_head_review_payload(origin, pr_number, current_pr, state, source_ref)
    if reviewing_payload ~= nil then
      core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    end
    return true
  end
  local new_review_proposal = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
  local new_review_dedup = "consensus:" .. new_review_proposal .. "/review"
  if core.has_any_review_result_marker(current_pr.comments, new_review_proposal, origin.proposal_id) then
    return false
  end
  local carry = {
    old_review_proposal_id = fact.review_proposal_id,
    old_review_dedup_key = fact.review_dedup_key,
    approved_head_sha = fact.head_sha,
    new_review_proposal_id = new_review_proposal,
    new_review_dedup_key = new_review_dedup,
    new_head_sha = current_pr.head_sha,
    base_head_sha = base_head,
  }
  local comment_request = build_review_carry_over_comment_request(origin, pr_number, state.version, carry, source_ref)
  local merge_payload = core.build_devloop_merge_ready_payload(origin.proposal_id, pr_number, state.version, {
    review_proposal_id = new_review_proposal,
    review_dedup_key = new_review_dedup,
    reviewed_head_sha = current_pr.head_sha,
  }, source_ref)
  core.log_cas_decision("observe_pr", origin.proposal_id, state, "merge-ready", "merge-ready", "applied(review-carry-over)", "resolution delta is empty")
  core.log_apply("observe_pr", origin.proposal_id, "merge-ready", state.version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
    "devloop_merge_ready",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  core.log_raise("observe_pr", origin.proposal_id, "devloop_merge_ready", merge_payload)
  return true
end

local function raise_current_state(origin, pr_number, current_pr, state, source_ref, known_issue_comments)
  if maybe_carry_over_approved_head(origin, pr_number, current_pr, state, source_ref) then
    return
  end
  if state.state == "reviewing" then
    local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
    if not core.has_any_review_result_marker(current_pr.comments, review_proposal_id, origin.proposal_id) then
      local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr_number, source_ref, state.version)
      core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "devloop_reviewing",
      })
      core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    end
    return
  end
  if state.state == "fixing" then
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "fixing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    local issue_comments = known_issue_comments or issue_comments_for_origin(origin)
    local fact_comments = issue_comments or current_pr.comments
    local feedback = core.fixing_replay_feedback_fact(fact_comments, origin.proposal_id, state.version)
    if feedback == nil and issue_comments ~= nil then
      feedback = core.fixing_replay_feedback_fact(current_pr.comments, origin.proposal_id, state.version)
    end
    if feedback == nil then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "fixing", "skip-stale(no-trusted-fix-feedback)", "trusted fix feedback marker is not visible")
      return
    end
    if feedback.review_proposal_id ~= nil and feedback.reviewed_head_sha ~= nil then
      if tostring(current_pr.head_sha or "") ~= tostring(feedback.reviewed_head_sha or "") then
        local fetch_result = exec_sync({ cmd = core.git_fetch_branch_cmd("origin", origin.branch), timeout = 60 })
        if fetch_result.exit_code ~= 0 then
          core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
          error("github-devloop: PR head changed before fix replay and deterministic branch head is not readable")
        end
        local branch_head = exec_sync({ cmd = core.git_fetch_head_commit_cmd(), timeout = 30 })
        if branch_head.exit_code ~= 0 then
          core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
          error("github-devloop: PR head changed before fix replay and deterministic branch head is not readable")
        end
        local intended_head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
        if not core.is_safe_head_sha(intended_head_sha) then
          error("github-devloop: unsafe PR origin branch head sha")
        end
        if tostring(current_pr.head_sha or "") == intended_head_sha
          and tostring(current_pr.head_sha or "") ~= tostring(feedback.reviewed_head_sha or "") then
          local reviewing_version = core.next_fix_version(state.version)
          if has_reviewing_marker(fact_comments, current_pr.comments, origin.proposal_id, reviewing_version) then
            core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "reviewing", "skip-idempotent(reviewing marker already visible)", "reviewing state marker for recovered head is already visible")
            return
          end
          local fix = {
            proposal_id = origin.proposal_id,
            pr_number = pr_number,
            version = state.version,
            review_proposal_id = feedback.review_proposal_id,
            review_dedup_key = feedback.review_dedup_key,
            reviewed_head_sha = feedback.reviewed_head_sha,
            source_ref = source_ref,
          }
          core.raise_fix_reviewing({
            dept = "observe_pr",
            repo = origin.repo,
            issue_number = origin.issue_number,
            fix = fix,
            old_head_sha = feedback.reviewed_head_sha,
            new_head_sha = current_pr.head_sha,
            new_version = reviewing_version,
            reason = "push already visible; self-healing missing reviewing marker",
            current_state = state,
          })
          return
        end
        core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "fixing", "skip-stale(head-advanced)", "PR head advanced since rejected review")
        return
      end
      local reviewing_version = core.next_fix_version(state.version)
      if not core.has_state_marker(fact_comments, origin.proposal_id, "reviewing", reviewing_version) then
        local fix_payload = core.build_replayed_fixing_payload({
          proposal_id = origin.proposal_id,
          impl_version = state.version,
        }, pr_number, feedback, source_ref)
        core.log_line("info", "observe_pr", origin.proposal_id, "SELFHEAL", {
          "state=fixing",
          "queue=devloop_fixing",
          "dedup_key=" .. tostring(fix_payload.dedup_key or ""),
        })
        core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_fixing",
        })
        core.log_raise("observe_pr", origin.proposal_id, "devloop_fixing", fix_payload)
      else
        core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "reviewing", "skip-idempotent(reviewing marker already visible)", "reviewing state marker for fix is already visible")
      end
    else
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "fixing", "skip-foreign(fix-feedback-binding)", "trusted fix feedback marker lacks review binding")
    end
    return
  end
  if state.state == "merge-ready" or state.state == "merging" then
    local fact = core.merge_ready_fact(current_pr.comments, origin.proposal_id, state.version, pr_number, current_pr.head_sha)
    if fact ~= nil then
      local merge_payload = core.build_devloop_merge_ready_payload(origin.proposal_id, fact.pr_number, state.version, {
        review_proposal_id = fact.review_proposal_id,
        review_dedup_key = fact.review_dedup_key,
        reviewed_head_sha = fact.head_sha,
      }, source_ref)
      core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "devloop_merge_ready",
      })
      core.log_raise("observe_pr", origin.proposal_id, "devloop_merge_ready", merge_payload)
    end
    return
  end
  if state.state == "blocked" and origin.issue_number ~= nil then
    local fact_comments = current_pr.comments or {}
    local decomposed = core.decomposed_fact(fact_comments, origin.proposal_id, state.version, pr_number)
    if decomposed == nil then
      return
    end
    local child_list = core.gh_exec({ cmd = core.gh_issue_list_decompose_children_cmd(origin.repo, origin.proposal_id), timeout = 30 })
    if child_list.exit_code ~= 0 then
      error("github-devloop: gh issue decompose child list failed: " .. tostring(child_list.stderr))
    end
    local complete, completed_count = core.decompose_children_complete(
      fact_comments,
      core.parse_decompose_child_issue_list(child_list.stdout),
      origin.proposal_id,
      decomposed.version,
      decomposed.pr_number,
      decomposed.count
    )
    if complete then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked", "decomposed", "skip-idempotent(decomposed children already visible)", "decompose children are complete")
      return
    end
    local payload = core.build_decompose_replay_payload(decomposed, fact_comments, source_ref)
    if payload == nil then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked", "decomposed", "skip-foreign(decompose-binding)", "trusted fix feedback for decomposed replay is not visible")
      return
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked", "decomposed", "applied(decomposed-children-missing)", "decomposed marker count exceeds derived child count " .. tostring(completed_count))
    core.log_apply("observe_pr", origin.proposal_id, "blocked", state.version, { add = {}, remove = {} }, {
      "devloop_decompose",
    })
    core.log_raise("observe_pr", origin.proposal_id, "devloop_decompose", payload)
  end
end

local function is_stalled_reviewing(current_pr, origin, pr_number, state)
  if state.state ~= "reviewing" or not core._is_git_sha(current_pr.head_sha) then
    return false
  end
  local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
  local review_version = core.safe_version_segment(state.version)
  local sr_digest = core.source_ref_digest(core.pr_source_ref(origin.repo, pr_number))
  local facts = core.review_converge_round_facts(
    current_pr.comments,
    review_proposal_id,
    origin.proposal_id,
    review_version,
    current_pr.head_sha,
    sr_digest
  )
  local round = core.max_converge_round(facts)
  return core.is_true_stall(facts, round)
end

local function maybe_apply_rereview_command(origin, pr_number, current_pr, state, source_ref)
  local command = core.operator_command_fact(current_pr.comments, "rereview")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current_pr.comments, command) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "blocked" and state.state ~= "review-meta" and state.state ~= "reviewing" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(invalid-state)", "operator rereview precondition failed")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires blocked, review-meta, or stalled reviewing state",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if state.state == "reviewing" and not is_stalled_reviewing(current_pr, origin, pr_number, state) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|stalled-reviewing", "reviewing", "refused(active-reviewing)", "operator rereview requires stalled reviewing")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires blocked, review-meta, or stalled reviewing state",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(pr-closed)", "operator rereview requires an open PR")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires an open PR",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if not core._is_git_sha(current_pr.head_sha) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(head-missing)", "operator rereview requires a current PR head")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires a current PR head",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end

  local new_version = core.operator_rereview_version(state.version, current_pr.head_sha)
  local comment_request = core.build_operator_rereview_comment_request(
    origin.repo,
    pr_number,
    origin.proposal_id,
    new_version,
    command,
    source_ref
  )
  local reviewing_payload = core.build_devloop_reviewing_payload({
    proposal_id = origin.proposal_id,
    impl_version = new_version,
  }, pr_number, source_ref, new_version)
  core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "applied(operator-rereview)", "trusted operator command requested rereview")
  core.log_apply("observe_pr", origin.proposal_id, "reviewing", new_version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
  maybe_label_hint(origin, { state = "reviewing", version = new_version }, core.issue_source_ref(origin.repo, origin.issue_number))
  return true
end

function pipeline(event)
  local pr = pr_context(event)
  local raw = event.payload or {}
  if pr == nil then
    core.log_entry("observe_pr", event, "unknown", raw.dedup_key)
    core.log_cas_decision("observe_pr", "unknown", { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "unsupported event payload")
    return
  end

  core.log_entry("observe_pr", event, "unknown", pr.dedup_key)
  core.assert_trusted_bot_configured()
  local branches = core.branch_config()
    local pr_view = core.fetch_pr_view_origin(pr.repo, pr.number, pr.updated_at)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed: " .. tostring(pr_view.stderr))
  end

  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin, has_issue_origin = origin_from_pr(pr.repo, pr.number, current_pr)
  if origin.branch == nil or origin.base_branch == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "PR branch facts missing")
    return
  end
  local ok, reason = origin_matches_pr(origin, current_pr, pr.repo, branches, false)
  if not ok then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(" .. reason .. ")", "PR origin mismatch")
    return
  end

  local source_ref = pr_source_ref(pr.repo, pr.number)
  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    local state = core.current_entity_state(current_pr.comments, origin.proposal_id)
    local merge_gate_feedback = nil
    if state.state == "reviewing" and origin.issue_number ~= nil then
      merge_gate_feedback = core.merge_gate_fix_fact(current_pr.comments, origin.proposal_id, core.next_fix_version(state.version))
    end
    if merge_gate_feedback ~= nil then
      local issue_comments = issue_comments_for_origin(origin)
      local issue_state = core.current_entity_state(issue_comments, origin.proposal_id)
      if issue_state.state == "fixing" then
        core.log_cas_decision("observe_pr", origin.proposal_id, issue_state, "fixing", "fixing", "applied(issue-fixing-replay)", "issue marker is fixing while PR marker is still reviewing")
        raise_current_state(origin, pr.number, current_pr, issue_state, source_ref, issue_comments)
        maybe_label_hint(origin, issue_state, core.issue_source_ref(origin.repo, origin.issue_number))
        return
      end
    end
    if maybe_apply_rereview_command(origin, pr.number, current_pr, state, source_ref) then
      return
    end
    if state.state ~= nil and state.state ~= "pr-open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "reviewing", state.state, "skip-idempotent(already at to_state)", state.state .. " marker visible on PR")
      raise_current_state(origin, pr.number, current_pr, state, source_ref)
      maybe_label_hint(origin, state, core.issue_source_ref(origin.repo, origin.issue_number))
      return
    end

    local transition = core.versioned_transition_status(state, { "pr-open", "unmanaged" }, "reviewing", origin.impl_version)
    if has_issue_origin and transition == "pending" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "reviewing PR marker not yet visible")
    end
    if state.state == "pr-open" and tostring(state.version or "") ~= tostring(origin.impl_version or "") then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(version-mismatch)", "PR-open marker version does not match PR origin")
      return
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "applied", "writing PR-local reviewing marker")
    local comment_request = core.build_reviewing_comment_request(origin.repo, origin.issue_number, origin, pr.number, source_ref)
    local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr.number, source_ref)
    local raised = {
      "github-proxy.github_pr_comment_request",
      "devloop_reviewing",
    }
    core.log_apply("observe_pr", origin.proposal_id, "reviewing", origin.impl_version, { add = {}, remove = {} }, raised)
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    maybe_label_hint(origin, { state = "reviewing", version = origin.impl_version }, core.issue_source_ref(origin.repo, origin.issue_number))
  end)
end

return M
