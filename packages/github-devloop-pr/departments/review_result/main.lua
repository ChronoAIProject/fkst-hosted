local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local strings = require("contract.strings")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local parsers_pr = require("devloop.parsers.pr")
local m_facts = require("devloop.markers.facts")
local convergence_shared, github_risk = require("devloop.convergence.shared"), require("devloop.github_risk")
local core, saga = require("core"), require("workflow.saga")
local transition_version = require("contract.transition_version")
local config = require("devloop.config")

local payloads_builders = require("devloop.payloads.builders")
local payloads_predicates = require("devloop.payloads.predicates")
local conv_reconcile = require("devloop.convergence.reconcile")
local v_review_result = require("devloop.validators.review_result")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local devloop_commands = require("devloop.commands")
-- Preserve existing body line coordinates for the coverage ratchet.

local spec = {
  consumes = { "consensus.consensus_reached" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_fix_reconcile",
    "github-devloop-decompose.devloop_decompose",
  },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function copy_reached_with_review_dedup(reached, review_dedup_key)
  local copy = {}
  for key, value in pairs(reached or {}) do
    copy[key] = value
  end
  copy.dedup_key = review_dedup_key
  return copy
end

return saga.department(spec, { done = function() return false end, act = function(event)
  local reached = type(event.payload) == "table" and event.payload or {}
  if reached.schema ~= "consensus.consensus_reached.v1"
    or type(reached.proposal_id) ~= "string"
    or reached.proposal_id:match("^github%-devloop/pr%-review/") == nil then
    devloop_logging.log_entry("review_result", event, "unknown", devloop_logging.payload_field(reached, "dedup_key"))
    devloop_logging.log_cas_decision("review_result", "unknown", { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  local review_repo, proposal_pr_number, review_version, reviewed_head_sha = devloop_base.parse_pr_review_proposal_id(reached.proposal_id)
  if review_repo == nil then
    error("github-devloop: review-result-invalid: owned review proposal_id is malformed")
  end
  if not v_review_result.is_supported_review_result(reached) then
    error("github-devloop: review-result-invalid: owned review result violates the consumer contract")
  end

  devloop_logging.log_entry("review_result", event, reached.proposal_id, reached.dedup_key)
  local repo, pr_number = devloop_base.parse_pr_source_ref(reached.source_ref)
  if devloop_base.safe_pr_review_repo_segment(repo) ~= review_repo
    or tostring(pr_number) ~= tostring(proposal_pr_number) then
    error("github-devloop: review-result-invalid: owned review source_ref does not match proposal_id")
  end
  local canonical_review_dedup = devloop_base.canonical_pr_review_consensus_dedup_for_proposal(
    reached.dedup_key,
    reached.proposal_id
  )
  if canonical_review_dedup == nil then
    error("github-devloop: review-result-invalid: owned review dedup does not match proposal_id")
  end
  if reached.decision == "reject"
    and not strings.is_bounded_string(reached.blocking_gap, devloop_base._max_blocking_gap_len) then
    error("github-devloop: review-result-invalid: owned reject is missing a bounded blocking_gap")
  end

  devloop_base.assert_trusted_bot_configured()
  local branches = config.branch_config()
  local pr_view = devloop_commands.gh_pr_view_origin(repo, pr_number, 30)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh-pr-review-result-view-failed: gh pr origin view failed for review result: " .. tostring(pr_view.stderr))
  end
  local current_pr = parsers_pr.parse_pr_view_origin(pr_view.stdout)
  local origin = m_facts.pr_origin_fact(current_pr.comments)
  if origin == nil then
    origin = entity_lib.pr_native_origin(repo, pr_number, current_pr)
  end
  if origin.repo ~= repo then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(repo)", "pr-origin repo mismatch")
    return
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(head)", "pr-origin branch mismatch")
    return
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(base)", "PR base branch mismatch")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(pr-closed)", "re-derived PR is not open")
    return
  end
  if tostring(current_pr.head_sha or "") ~= tostring(reviewed_head_sha) then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(head-advanced)", "PR head advanced since reviewed diff")
    return
  end
  local reviewed_issue_version = tostring(review_version or "")
  if reviewed_issue_version == "" then
    devloop_logging.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(version)", "review proposal version is missing")
    return
  end
  local lock_key = entity_lib.review_result_lock_key(origin.proposal_id)
  if lock_key == nil then
    error("github-devloop: review-result-invalid: owned review has no issue transition lock key")
  end

  with_lock(lock_key, function()
    local pr_source_ref = entity_lib.pr_source_ref(origin.repo, pr_number)
    if not m_claims.verify_pr_review_issue_claim("review_result", origin.repo, origin.issue_number, nil, origin.proposal_id) then
      return
    end
    devloop_logging.log_forged_markers("review_result", origin.proposal_id, current_pr.comments)
    local state = require("devloop.entity").current_entity_state(current_pr.comments, origin.proposal_id)
    local effective_decision = reached.decision
    local comment_reached = copy_reached_with_review_dedup(reached, canonical_review_dedup)
    local gate_owned_reject = reached.decision == "reject" and payloads_predicates.is_gate_owned_review_gap(reached.blocking_gap)
    local out_of_contract_reject = reached.decision == "reject" and payloads_predicates.is_out_of_contract_review_gap(reached.blocking_gap)
    if gate_owned_reject or out_of_contract_reject then
      effective_decision = "approve"
    end

    local high_risk_paths = {}
    local paths_digest = nil
    local angle_digest = nil
    local high_risk_angle_not_approved = false
    if effective_decision == "approve" then
      local name_result = devloop_commands.gh_pr_diff_name_only(repo, pr_number, 30)
      local risk = github_risk.github_diff_name_risk(name_result)
      high_risk_paths = risk.high_risk_paths or {}
      if risk.known == false then
        devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", "merge-ready", "retry-pending(high-risk-review-evidence:" .. tostring(risk.reason or "unknown") .. ")", "review diff risk is undecidable")
        error("github-devloop: review-diff-risk-undecidable: review diff risk is undecidable; retrying")
      elseif risk.high_risk == true then
        local high_risk_approved = false
        if type(reached.angle_results) == "table" then
          for _, item in ipairs(reached.angle_results) do
            if type(item) == "table"
              and item.angle == "high-risk"
              and item.verdict == "approve" then
              high_risk_approved = true
            end
          end
        end
        if not high_risk_approved then
          effective_decision = "reject"
          high_risk_angle_not_approved = true
          comment_reached = copy_reached_with_review_dedup(reached, canonical_review_dedup)
          comment_reached.decision = "reject"
          comment_reached.blocking_gap = "high-risk-angle-not-approved"
          comment_reached.body = "High-risk PR approval did not include an approving high-risk angle."
        end
        if effective_decision == "approve" then
          paths_digest = github_risk.github_paths_digest(risk.paths)
          angle_digest = convergence_shared.converge_angles_digest(reached.angle_results)
        end
      end
    end

    local issue_version = state.version
    local reflection_checkpoint = false
    if effective_decision == "reject" and devloop_state.version_fix_round(state.version) < config.max_fix_rounds() then
      issue_version = devloop_state.fix_version_from_review_version(state.version)
      reflection_checkpoint = devloop_state.version_fix_round(issue_version) == devloop_base.fix_reflection_checkpoint_round()
    end
    local to_state = effective_decision == "approve" and "merge-ready"
      or reflection_checkpoint and "review-meta"
      or "fixing"
    local current_review_version = transition_version.safe_version_segment(state.version or "")
    local transition = devloop_state.cyclic_transition_status({
      state = state.state,
      version = current_review_version,
      stage_rank = state.stage_rank,
    }, { "reviewing" }, to_state, reviewed_issue_version)
    if transition == "idempotent" or transition == "stale" then
      devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, devloop_state.cas_outcome(state, transition, canonical_review_dedup), "review decision cannot advance current marker")
      return
    end
    if transition == "pending" then
      devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, devloop_state.cas_outcome(state, transition, canonical_review_dedup), "reviewing state marker not yet visible")
      error("github-devloop: review-result-marker-missing: reviewing marker not yet visible for review result; retrying")
    end

    if tostring(current_review_version) ~= tostring(reviewed_issue_version) then
      devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, "skip-stale(version-mismatch)", "PR origin implementation version does not match canonical issue marker")
      return
    end

    if effective_decision == "reject" then
      local fix_round = devloop_state.version_fix_round(state.version)
      local max_rounds_hit = fix_round >= config.max_fix_rounds()
      if max_rounds_hit then
        local fix_reconcile = conv_reconcile.build_devloop_fix_reconcile_payload({
          proposal_id = origin.proposal_id,
          review_proposal_id = reached.proposal_id,
          review_dedup_key = canonical_review_dedup,
          reviewed_head_sha = reviewed_head_sha,
          pr_number = pr_number,
          source_ref = pr_source_ref,
        }, state.version)
        local decompose = payloads_builders.build_devloop_decompose_payload(fix_reconcile)
        local reason = "fix-loop-max-rounds"
        devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", "blocked", "applied(" .. reason .. ")", "review decision=reject")
        devloop_logging.log_raise("review_result", origin.proposal_id, "devloop_fix_reconcile", fix_reconcile)
        devloop_logging.log_raise("review_result", origin.proposal_id, "github-devloop-decompose.devloop_decompose", decompose)
        return
      end
    end
    if high_risk_angle_not_approved then
      devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, devloop_state.cas_outcome(state, transition, canonical_review_dedup) .. "(high-risk-angle-not-approved)", "high-risk PR approval lacks high-risk angle approval")
    else
      devloop_logging.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, devloop_state.cas_outcome(state, transition, canonical_review_dedup), "review decision=" .. tostring(reached.decision))
    end
    if (gate_owned_reject or out_of_contract_reject) and not high_risk_angle_not_approved then
      comment_reached = copy_reached_with_review_dedup(reached, canonical_review_dedup)
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
      local base_reached = comment_reached
      comment_reached = {}
      for key, value in pairs(base_reached) do
        comment_reached[key] = value
      end
      comment_reached.reflection_checkpoint = true
    end
    if effective_decision == "approve" then
      comment_reached.current_head_sha = current_pr.head_sha
    end
    local comment_request = requests_review.build_review_result_comment_request(core, origin.repo, origin.issue_number, origin.proposal_id, issue_version, comment_reached, pr_source_ref)
    local evidence_request = nil
    if effective_decision == "approve" and #high_risk_paths > 0 then
      evidence_request = requests_review.build_high_risk_review_evidence_comment_request(origin.repo, origin.proposal_id, issue_version, comment_reached, pr_number, reviewed_head_sha, paths_digest, angle_digest, pr_source_ref)
    end
    local label_request = nil
    if origin.issue_number ~= nil then
      label_request = requests_labels.build_review_result_label_request(origin.repo, origin.issue_number, origin.proposal_id, comment_reached, entity_lib.issue_source_ref(origin.repo, origin.issue_number))
    end
    local add_labels, remove_labels = devloop_state.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_pr_comment_request",
    }
    if evidence_request ~= nil then
      table.insert(raised, "github-proxy.github_pr_comment_request")
    end
    if label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    devloop_logging.log_apply("review_result", origin.proposal_id, to_state, issue_version, { add = add_labels, remove = remove_labels }, raised)
    devloop_logging.log_raise("review_result", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    if evidence_request ~= nil then
      devloop_logging.log_raise("review_result", origin.proposal_id, "github-proxy.github_pr_comment_request", evidence_request)
    end
    if origin.issue_number ~= nil then
      devloop_logging.log_raise("review_result", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end, wrap = devloop_logging.wrap_pipeline_failure, name = "review_result" })
