local core, saga = require("core"), require("std.saga")

-- Preserve existing body line coordinates for the coverage ratchet.

local spec = {
  consumes = { "consensus.consensus_reached" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_fixing",
    "devloop_fix_reconcile",
    "devloop_decompose",
    "devloop_merge_ready",
    "devloop_review_meta",
  },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

return saga.department(spec, { done = function() return false end, act = function(event)
  local reached = event.payload or {}
  if not core.is_supported_review_result(reached) then
    core.log_entry("review_result", event, "unknown", core.payload_field(reached, "dedup_key"))
    core.log_cas_decision("review_result", "unknown", { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("review_result", event, reached.proposal_id, reached.dedup_key)
  local review_repo, proposal_pr_number, review_version, reviewed_head_sha = core.parse_pr_review_proposal_id(reached.proposal_id)
  if review_repo == nil then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop pr-review")
    return
  end
  local repo, pr_number = core.parse_pr_source_ref(reached.source_ref)
  if repo == nil or tostring(pr_number) ~= tostring(proposal_pr_number) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(source_ref)", "review source_ref does not match PR review proposal")
    return
  end

  core.assert_trusted_bot_configured()
  local branches = core.branch_config()
  local pr_view = core.gh_pr_view_origin(repo, pr_number, 30)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed for review result: " .. tostring(pr_view.stderr))
  end
  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin == nil then
    origin = core.pr_native_origin(repo, pr_number, current_pr)
  end
  if origin.repo ~= repo then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(repo)", "pr-origin repo mismatch")
    return
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(head)", "pr-origin branch mismatch")
    return
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(base)", "PR base branch mismatch")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(pr-closed)", "re-derived PR is not open")
    return
  end
  if tostring(current_pr.head_sha or "") ~= tostring(reviewed_head_sha) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(head-advanced)", "PR head advanced since reviewed diff")
    return
  end
  local reviewed_issue_version = tostring(review_version or "")
  if reviewed_issue_version == "" then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(version)", "review proposal version is missing")
    return
  end
  if reached.decision == "reject"
    and not core._is_bounded_string(reached.blocking_gap, core._max_blocking_gap_len) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "fixing", "skip-foreign(blocking-gap)", "reject review result is missing a bounded blocking_gap")
    return
  end

  local lock_key = core.review_result_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "no issue transition lock key")
    return
  end

  with_lock(lock_key, function()
    local pr_source_ref = core.pr_source_ref(origin.repo, pr_number)
    if not core.verify_pr_review_issue_claim("review_result", origin.repo, origin.issue_number, nil, origin.proposal_id) then
      return
    end
    core.log_forged_markers("review_result", origin.proposal_id, current_pr.comments)
    local state = core.current_entity_state(current_pr.comments, origin.proposal_id)
    local effective_decision = reached.decision
    local gate_owned_reject = reached.decision == "reject" and core.is_gate_owned_review_gap(reached.blocking_gap)
    local out_of_contract_reject = reached.decision == "reject" and core.is_out_of_contract_review_gap(reached.blocking_gap)
    if gate_owned_reject or out_of_contract_reject then
      effective_decision = "approve"
    end
    local issue_version = state.version
    local reflection_checkpoint = false
    if effective_decision == "reject" and core.version_fix_round(state.version) < core.max_fix_rounds() then
      issue_version = core.fix_version_from_review_version(state.version)
      reflection_checkpoint = core.version_fix_round(issue_version) == core.fix_reflection_checkpoint_round()
    end
    local to_state = effective_decision == "approve" and "merge-ready"
      or reflection_checkpoint and "review-meta"
      or "fixing"
    local current_review_version = core.safe_version_segment(state.version or "")
    local transition = core.cyclic_transition_status({
      state = state.state,
      version = current_review_version,
      stage_rank = state.stage_rank,
    }, { "reviewing" }, to_state, reviewed_issue_version)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "review decision cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "reviewing state marker not yet visible")
      error("github-devloop: reviewing marker not yet visible for review result; retrying")
    end

    if tostring(current_review_version) ~= tostring(reviewed_issue_version) then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, "skip-stale(version-mismatch)", "PR origin implementation version does not match canonical issue marker")
      return
    end

    if effective_decision == "reject" then
      local fix_round = core.version_fix_round(state.version)
      local max_rounds_hit = fix_round >= core.max_fix_rounds()
      if max_rounds_hit then
        local fix_reconcile = core.build_devloop_fix_reconcile_payload({
          proposal_id = origin.proposal_id,
          review_proposal_id = reached.proposal_id,
          review_dedup_key = reached.dedup_key,
          reviewed_head_sha = reviewed_head_sha,
          pr_number = pr_number,
          source_ref = pr_source_ref,
        }, state.version)
        local decompose = core.build_devloop_decompose_payload(fix_reconcile)
        local reason = "fix-loop-max-rounds"
        core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", "blocked", "applied(" .. reason .. ")", "review decision=reject")
        core.log_raise("review_result", origin.proposal_id, "devloop_fix_reconcile", fix_reconcile)
        core.log_raise("review_result", origin.proposal_id, "devloop_decompose", decompose)
        return
      end
    end
    core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "review decision=" .. tostring(reached.decision))
    local comment_reached = reached
    if gate_owned_reject or out_of_contract_reject then
      comment_reached = {}
      for key, value in pairs(reached) do
        comment_reached[key] = value
      end
      comment_reached.decision = "approve"
      local advisory_reason = "rejected only for gate-owned fact: "
      if out_of_contract_reject then
        advisory_reason = "rejected only for demand beyond the stated issue bounds: "
      end
      comment_reached.body = tostring(reached.body or "")
        .. "\n\nAdvisory (out-of-contract): "
        .. advisory_reason
        .. tostring(reached.blocking_gap or "")
      comment_reached.blocking_gap = nil
    end
    if reflection_checkpoint then
      comment_reached = {}
      for key, value in pairs(reached) do
        comment_reached[key] = value
      end
      comment_reached.reflection_checkpoint = true
    end
    local comment_request = core.build_review_result_comment_request(origin.repo, origin.issue_number, origin.proposal_id, issue_version, comment_reached, pr_source_ref)
    local label_request = nil
    if origin.issue_number ~= nil then
      label_request = core.build_review_result_label_request(origin.repo, origin.issue_number, origin.proposal_id, comment_reached, core.issue_source_ref(origin.repo, origin.issue_number))
    end
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_pr_comment_request",
    }
    if label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    local fix_payload = nil
    local reflection_payload = nil
    local merge_payload = nil
    if reflection_checkpoint then
      reflection_payload = core.build_devloop_fix_reflection_payload({
        proposal_id = reached.proposal_id,
        dedup_key = reached.dedup_key,
        source_ref = pr_source_ref,
      }, origin.proposal_id, issue_version, pr_number, core.version_fix_round(issue_version), pr_source_ref)
      reflection_payload.blocking_gap = reached.blocking_gap
      table.insert(raised, "devloop_review_meta")
    elseif effective_decision == "reject" then
      fix_payload = core.build_devloop_fixing_payload(origin, pr_number, {
        review_proposal_id = reached.proposal_id,
        review_dedup_key = reached.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
        fix_version = issue_version,
        framing = reached.framing,
        blocking_gap = reached.blocking_gap,
      }, pr_source_ref)
      table.insert(raised, "devloop_fixing")
    else
      merge_payload = core.build_devloop_merge_ready_payload(origin.proposal_id, pr_number, issue_version, {
        review_proposal_id = reached.proposal_id,
        review_dedup_key = reached.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
        current_head_sha = current_pr.head_sha,
      }, pr_source_ref)
      table.insert(raised, "devloop_merge_ready")
    end
    core.log_apply("review_result", origin.proposal_id, to_state, issue_version, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("review_result", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    if origin.issue_number ~= nil then
      core.log_raise("review_result", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
    if fix_payload ~= nil then
      core.log_raise("review_result", origin.proposal_id, "devloop_fixing", fix_payload)
    end
    if reflection_payload ~= nil then
      core.log_raise("review_result", origin.proposal_id, "devloop_review_meta", reflection_payload)
    end
    if merge_payload ~= nil then
      core.log_raise("review_result", origin.proposal_id, "devloop_merge_ready", merge_payload)
    end
  end)
end, wrap = core.wrap_pipeline_failure, name = "review_result" })
