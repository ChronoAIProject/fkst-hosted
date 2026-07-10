local ci_repair_attempts = require("core.ci_repair_attempts")
local fix_rounds = require("core.fix_rounds")
local config = require("devloop.config")
local ci_verdict = require("core.ci_verdict")
local contract_time = require("contract.time")
local transition_version = require("contract.transition_version")
local devloop_state = require("devloop.state")
local payloads_builders = require("devloop.payloads.builders")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local devloop_logging = require("devloop.logging")

local C = {}
local with_current_classification = ci_verdict.with_current_classification
local raise_admitted_round

local function invalid_time(reason)
  return { status = "policy-invalid", reason = reason }
end

local function parse_marker_time(value)
  local seconds = contract_time.iso_timestamp_epoch_seconds(value)
  if seconds == nil then
    return invalid_time("trusted marker timestamp is unparseable")
  end
  return { status = "valid", seconds = seconds }
end

local function state_entry(state)
  local lineage = parse_marker_time(transition_version.updated_at(state and state.version))
  if lineage.status == "policy-invalid" then
    return invalid_time("version lineage timestamp is unparseable")
  end
  local marker_created_at = state and state.marker_created_at
  if marker_created_at == nil or marker_created_at == "" then
    lineage.source = "version-lineage"
    return lineage
  end
  local marker = parse_marker_time(marker_created_at)
  if marker.status == "valid" then
    marker.seconds = math.max(lineage.seconds, marker.seconds)
  end
  marker.source = "state-marker"
  return marker
end

local function marker_after_state_entry(value, entry)
  if type(entry) ~= "table" or entry.status ~= "valid" then
    return invalid_time("trusted marker time requires a valid state entry")
  end
  local marker = parse_marker_time(value)
  if marker.status == "valid" then
    marker.seconds = math.max(entry.seconds, marker.seconds)
  end
  return marker
end

local function admission_context(ctx, current_pr)
  return {
    dept = ctx.dept,
    from_state = "fixing",
    proposal_id = ctx.proposal_id,
    review_proposal_id = ctx.review_proposal_id,
    review_dedup_key = ctx.review_dedup_key,
    bound_head_sha = current_pr and current_pr.head_sha or ctx.reviewed_head_sha,
    pr_number = ctx.pr_number,
    source_ref = ctx.source_ref,
    reason = ctx.reason or "own-ci-red-repair-budget-exhausted",
  }
end

local function completed_attempt(ctx, state)
  return ci_repair_attempts.fact(ctx.comments, {
    proposal_id = ctx.proposal_id,
    pr_number = ctx.pr_number,
    version = state.version,
  })
end

local function retry_window(state, attempt, now_seconds)
  local entry = state_entry(state)
  if entry.status == "policy-invalid" then
    return entry
  end
  local completion = marker_after_state_entry(attempt.comment_created_at, entry)
  if completion.status == "policy-invalid" then
    return completion
  end
  local delay_seconds = devloop_state.version_fix_round(state.version)
    * config.liveness_poll_cadence_seconds()
  return {
    status = "valid",
    completed_seconds = completion.seconds,
    delay_seconds = delay_seconds,
    due_seconds = completion.seconds + delay_seconds,
    state_entry_seconds = entry.seconds,
    state_entry_source = entry.source,
  }
end

function C.evaluate(M, state, ctx)
  local attempt = completed_attempt(ctx, state)
  if attempt == nil then
    return { kind = "redrive" }
  end

  local current_seconds = tonumber(ctx.now_seconds)
  if current_seconds == nil then
    local invalid_ctx = admission_context(ctx)
    invalid_ctx.reason = "ci-repair-retry-policy-invalid"
    return fix_rounds.terminate_own_ci_policy_invalid(state, invalid_ctx)
  end
  local window = retry_window(state, attempt, current_seconds)
  if window.status == "policy-invalid" then
    local invalid_ctx = admission_context(ctx)
    invalid_ctx.reason = "ci-repair-retry-policy-invalid"
    return fix_rounds.terminate_own_ci_policy_invalid(state, invalid_ctx)
  end
  if current_seconds < window.due_seconds then
    return {
      kind = "defer",
      attempt = attempt,
      completed_seconds = window.completed_seconds,
      delay_seconds = window.delay_seconds,
      due_seconds = window.due_seconds,
    }
  end

  local decision, mismatch, current_pr = with_current_classification(
    ctx.repo,
    ctx.pr_number,
    ctx.reviewed_head_sha,
    function(classification)
      local admission = fix_rounds.admit_own_ci_continuation(state, classification, admission_context(ctx))
      if admission.kind == "not-own-ci" then
        return {
          kind = "reviewing",
          current_pr = admission.current_pr,
          reason = "own-CI gate no longer requires repair: " .. tostring(admission.reason),
        }
      end
      if admission.kind ~= "admit" then
        return admission
      end
      admission.attempt = attempt
      local apply = ctx.apply
      if type(apply) ~= "table" then
        error("github-devloop: ci-repair-retry-apply-missing: admitted retry requires its synchronous effect")
      end
      return {
        kind = "applied",
        result = raise_admitted_round(M, apply.dept, apply.issue, state, ctx.proposal_id,
          apply.link, apply.feedback, admission, apply.tools),
      }
    end,
    {
      dept = ctx.dept,
      proposal_id = ctx.proposal_id,
      error_class = "ci-repair-reobserve-failed: PR retry admission re-observation failed",
    }
  )
  if mismatch == "head-mismatch" then
    return {
      kind = "reviewing",
      current_pr = current_pr,
      reason = "PR head advanced after the completed CI repair round",
    }
  end
  return decision
end

function C.raise_speculative(M, repo, issue_number, fix, current_state, current_predecessor_set, reason)
  local function raise_generation(next_version, current_ci_failure_key, current_gate_reason)
    local merge_ready = {
      proposal_id = fix.proposal_id,
      pr_number = fix.pr_number,
      version = devloop_state._strip_latest_fix_version_suffix(fix.version),
      review_proposal_id = fix.review_proposal_id,
      review_dedup_key = fix.review_dedup_key,
      reviewed_head_sha = fix.reviewed_head_sha,
      dedup_key = fix.dedup_key,
    }
    local comment_request = requests_review.build_merge_gate_fix_comment_request(M,
      repo,
      issue_number,
      merge_ready,
      next_version,
      current_gate_reason,
      fix.gate_baseline_sha,
      fix.source_ref,
      current_predecessor_set,
      {
        blocking_gap = fix.blocking_gap,
        gate_failure_excerpt = fix.gate_failure_excerpt,
        preserve_nil_gate_failure_excerpt = true,
        repair_input = fix.repair_input,
        ci_failure_key = current_ci_failure_key,
      }
    )
    local label_request = issue_number ~= nil and requests_labels.build_state_label_request(repo,
      issue_number,
      "fixing",
      fix.dedup_key .. "/label/refix/" .. tostring(devloop_state.version_fix_round(next_version)),
      entity_lib.issue_source_ref(repo, issue_number)
    ) or nil
    local add_labels, remove_labels = devloop_state.state_label_changes("fixing")
    devloop_logging.log_cas_decision("fix", fix.proposal_id, current_state, "fixing", "fixing", "applied", reason)
    local raised = { "github-proxy.github_pr_comment_request" }
    if label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    devloop_logging.log_apply("fix", fix.proposal_id, "fixing", next_version, { add = add_labels, remove = remove_labels }, raised)
    devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    if label_request ~= nil then
      devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
    return { kind = "admit", version = next_version }
  end

  if fix.repair_input ~= "ci-failure" then
    return raise_generation(
      devloop_state.next_fix_version(fix.version),
      fix.ci_failure_key,
      fix.gate_failure_excerpt or fix.blocking_gap or reason
    )
  end
  local result, mismatch, current_pr = with_current_classification(
    repo,
    fix.pr_number,
    fix.reviewed_head_sha,
    function(classification)
      local decision = fix_rounds.admit_own_ci_continuation(current_state, classification, {
        dept = "fix",
        from_state = "fixing",
        proposal_id = fix.proposal_id,
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        pr_number = fix.pr_number,
        source_ref = fix.source_ref,
        reason = "own-CI-red speculation churn exhausted the fix-round budget",
      })
      if decision.kind == "not-own-ci" then
        requests_review.raise_fix_reviewing_outcome(M, repo, issue_number, fix,
          fix.reviewed_head_sha, decision.current_pr.head_sha,
          "own-CI gate no longer requires speculative repair: " .. tostring(decision.reason))
        return { kind = "reviewing" }
      end
      if decision.kind ~= "admit" then
        return decision
      end
      return raise_generation(decision.version, decision.ci_failure_key, decision.reason)
    end,
    {
      dept = "fix",
      proposal_id = fix.proposal_id,
      error_class = "gh-pr-speculative-refix-view-failed",
    }
  )
  if mismatch == "head-mismatch" then
    requests_review.raise_fix_reviewing_outcome(M, repo, issue_number, fix,
      fix.reviewed_head_sha, current_pr.head_sha,
      "own-CI gate head changed before speculative repair")
    return { kind = "reviewing" }
  end
  return result
end

local function liveness_comments(facts)
  if facts and facts.current_pr and type(facts.current_pr.comments) == "table" then
    return facts.current_pr.comments
  end
  if facts and facts.current and type(facts.current.comments) == "table" then
    return facts.current.comments
  end
  if facts and facts.snapshot and type(facts.snapshot.comments) == "table" then
    return facts.snapshot.comments
  end
  return {}
end

function C.resolve_liveness_hold(row, state, facts, now_seconds)
  local defer = row and row.defer or nil
  if type(defer) ~= "table"
    or defer.hold_fact ~= "ci-repair-attempt:v1"
    or defer.hold_resolver ~= "ci-repair-backoff" then
    return { status = "contract_invalid", reason = "unsupported durable hold declaration" }
  end
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local pr_number = facts and facts.link and facts.link.pr_number or nil
  if pr_number == nil then
    pr_number = select(2, require("devloop.base").parse_pr_source_ref(facts and facts.source_ref))
  end
  local attempt = completed_attempt({
    comments = liveness_comments(facts),
    proposal_id = proposal_id,
    pr_number = pr_number,
  }, state)
  if attempt == nil then
    return { status = "absent" }
  end
  local current_seconds = tonumber(now_seconds)
  if current_seconds == nil then
    return { status = "contract_invalid", reason = "durable hold requires an explicit clock" }
  end
  local window = retry_window(state, attempt, current_seconds)
  if window.status == "policy-invalid" then
    return {
      status = "contract_invalid",
      reason = "ci-repair-retry-policy-invalid: " .. tostring(window.reason),
    }
  end
  local status = current_seconds < window.due_seconds and "held" or "released"
  local fact_id = table.concat({
    "ci-repair-attempt:v1",
    tostring(proposal_id or ""),
    tostring(pr_number or ""),
    tostring(state and state.version or ""),
  }, ":")
  return {
    status = status,
    fact_family = "ci-repair-attempt:v1",
    fact_id = fact_id,
    due_ms = window.due_seconds * 1000,
    completed_seconds = window.completed_seconds,
    state_entry_source = window.state_entry_source,
    attempt = attempt,
  }
end

raise_admitted_round = function(M, dept, issue, state, proposal_id, link, feedback, decision, tools)
  local current_pr = decision.current_pr
  local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
  local next_payload = payloads_builders.build_devloop_fixing_payload({
    proposal_id = proposal_id,
    impl_version = decision.version,
  }, link.pr_number, {
    review_proposal_id = feedback.review_proposal_id,
    review_dedup_key = feedback.review_dedup_key,
    reviewed_head_sha = current_pr.head_sha,
    blocking_gap = feedback.blocking_gap,
    gate_baseline_sha = current_pr.base_ref_oid,
    predecessor_set = feedback.predecessor_set,
    ci_failure_key = decision.ci_failure_key,
    gate_failure_excerpt = decision.reason,
  }, source_ref)
  local request = requests_review.build_merge_gate_fix_comment_request(M,
    issue.repo,
    issue.number,
    {
      proposal_id = proposal_id,
      pr_number = link.pr_number,
      version = state.version,
      review_proposal_id = feedback.review_proposal_id,
      review_dedup_key = feedback.review_dedup_key,
      reviewed_head_sha = current_pr.head_sha,
    },
    decision.version,
    decision.reason,
    current_pr.base_ref_oid,
    source_ref,
    feedback.predecessor_set,
    {
      blocking_gap = feedback.blocking_gap,
      gate_failure_excerpt = decision.reason,
      ci_failure_key = decision.ci_failure_key,
      current_head_sha = current_pr.head_sha,
    }
  )
  request.handoff.dedup_key = next_payload.dedup_key
  local effects = {
    { queue = "github-proxy.github_pr_comment_request", payload = request },
  }
  if issue.number ~= nil then
    table.insert(effects, {
      queue = "github-proxy.github_issue_label_request",
      payload = requests_labels.build_state_label_request(issue.repo, issue.number, "fixing", base_ids.dedup_key({
        "ci-repair",
        "label",
        "fixing",
        tostring(proposal_id),
        tostring(link.pr_number),
        tostring(decision.version),
      }), entity_lib.issue_source_ref(issue.repo, issue.number)),
    })
  end
  devloop_logging.log_cas_decision(dept, proposal_id, state, "fixing", "fixing", "applied(ci-repair-next-round)", decision.reason)
  return tools.raise_effects(dept, proposal_id, "fixing", decision.version, { add = {}, remove = {} }, effects)
end

return C
