local base_ids = require("devloop.base_ids")
local devloop_base = require("devloop.base")
local config = require("devloop.config")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local ci_verdict = require("core.ci_verdict")
local forge_validators = require("devloop.forge_validators")
local parsers_misc = require("devloop.parsers.misc")
local payloads_builders = require("devloop.payloads.builders")
local source_refs = require("contract.source_ref")
local strings = require("contract.strings")
local OWN_CI_RED = ci_verdict.OWN_CI_RED

local C = {}

C.OWN_CI_SCHEMA = "github-devloop.own-ci-reconcile.v1"
C.MERGE_GATE_SCHEMA = "github-devloop.merge-gate-reconcile.v1"
C.FIX_LOOP_MAX_ROUNDS = "fix-loop-max-rounds"
C.CI_REPAIR_RETRY_POLICY_INVALID = "ci-repair-retry-policy-invalid"

local own_ci_reasons = {
  [C.FIX_LOOP_MAX_ROUNDS] = true,
  [C.CI_REPAIR_RETRY_POLICY_INVALID] = true,
}

local merge_gate_reasons = {
  [C.FIX_LOOP_MAX_ROUNDS] = true,
}

local function terminal_dedup_key(schema, issue_version, reason_class)
  return base_ids.dedup_key({ schema, tostring(issue_version), tostring(reason_class) })
end

local function build_terminal_intent(schema, ctx, issue_version, reason_class)
  return {
    schema = schema,
    proposal_id = ctx.proposal_id,
    review_proposal_id = ctx.review_proposal_id,
    review_dedup_key = ctx.review_dedup_key,
    issue_version = issue_version,
    reason_class = reason_class,
    bound_head_sha = ctx.bound_head_sha,
    round = devloop_state.version_fix_round(issue_version),
    pr_number = ctx.pr_number,
    dedup_key = terminal_dedup_key(schema, issue_version, reason_class),
    source_ref = base_ids.normalize_source_ref(ctx.source_ref),
  }
end

local function build_own_ci_terminal(ctx, issue_version, reason_class)
  if own_ci_reasons[reason_class] ~= true then
    error("github-devloop: own-ci-terminal-reason-invalid: unsupported reason class")
  end
  return build_terminal_intent(C.OWN_CI_SCHEMA, ctx, issue_version, reason_class)
end

local function build_merge_gate_terminal(ctx, issue_version)
  return build_terminal_intent(C.MERGE_GATE_SCHEMA, ctx, issue_version, C.FIX_LOOP_MAX_ROUNDS)
end

local function supported_terminal(payload, schema, reasons)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = base_ids.parse_proposal_id(payload.proposal_id)
  return payload.schema == schema
    and repo ~= nil
    and issue_number ~= nil
    and strings.is_path_safe_key(payload.proposal_id, devloop_base._max_key_len)
    and strings.is_path_safe_key(payload.review_proposal_id, devloop_base._max_key_len)
    and strings.is_bounded_string(payload.review_dedup_key, devloop_base._max_dedup_len)
    and strings.is_bounded_string(payload.issue_version, devloop_base._max_dedup_len)
    and reasons[payload.reason_class] == true
    and forge_validators.is_git_sha(payload.bound_head_sha)
    and tonumber(payload.round) == devloop_state.version_fix_round(payload.issue_version)
    and forge_validators.is_positive_pr_number(payload.pr_number)
    and payload.dedup_key == terminal_dedup_key(schema, payload.issue_version, payload.reason_class)
    and source_refs.has_bounded_source_ref(payload.source_ref, devloop_base._max_key_len)
end

function C.is_supported_own_ci(payload)
  return supported_terminal(payload, C.OWN_CI_SCHEMA, own_ci_reasons)
end

function C.is_supported_merge_gate(payload)
  return supported_terminal(payload, C.MERGE_GATE_SCHEMA, merge_gate_reasons)
end

local function build_decompose(intent)
  return payloads_builders.build_devloop_decompose_payload({
    proposal_id = intent.proposal_id,
    pr_number = intent.pr_number,
    issue_version = intent.issue_version,
    review_proposal_id = intent.review_proposal_id,
    review_dedup_key = intent.review_dedup_key,
    head_sha = intent.bound_head_sha,
    round = intent.round,
    source_ref = intent.source_ref,
  })
end

local function terminate(state, ctx, round, intent)
  local decompose = build_decompose(intent)
  local dept = ctx.dept or "merge"
  local from_state = ctx.from_state or state.state or "merge-ready"
  devloop_logging.log_cas_decision(dept, ctx.proposal_id, state, from_state, "blocked", "applied(fix-loop-max-rounds)", ctx.reason)
  devloop_logging.log_raise(dept, ctx.proposal_id, "devloop_fix_reconcile", intent)
  devloop_logging.log_raise(dept, ctx.proposal_id, "github-devloop-decompose.devloop_decompose", decompose)
  return {
    kind = "terminate",
    round = round,
    reconcile = intent,
    decompose = decompose,
  }
end

-- The budget is derived only from the stable `/fix/N` version lineage, never from a
-- drifting external key (e.g. a merge-queue predecessor set), so no such key can reset it.
local function admit_decision(state)
  local round = devloop_state.version_fix_round(state.version)
  if round >= config.max_fix_rounds() then
    return { kind = "terminate", round = round }
  end
  local version = devloop_state.next_fix_version(state.version)
  return {
    kind = "admit",
    round = devloop_state.version_fix_round(version),
    version = version,
  }
end

-- This is the only operation allowed to admit an own-CI-red fixing continuation.
function C.admit_own_ci_continuation(state, classification, ctx)
  if type(state) ~= "table" or type(classification) ~= "table" or type(ctx) ~= "table" then
    error("github-devloop: own-ci-admission-invalid: state, classification, and context are required")
  end
  local current_pr = classification.current_pr
  local bound_head_sha = tostring(classification.head_sha or "")
  if bound_head_sha == "" then
    error("github-devloop: own-ci-admission-invalid: current PR head is required")
  end
  if type(current_pr) ~= "table" or tostring(current_pr.head_sha or "") ~= bound_head_sha then
    error("github-devloop: own-ci-admission-invalid: classified PR head is inconsistent")
  end
  local pr_state = tostring(current_pr.state or ""):upper()
  if pr_state == "MERGED" then
    return { kind = "pr-merged", current_pr = current_pr }
  end
  if pr_state ~= "OPEN" then
    return { kind = "pr-closed", current_pr = current_pr }
  end
  if (ctx.head_branch ~= nil and tostring(current_pr.head_ref_name or "") ~= tostring(ctx.head_branch))
    or (ctx.base_branch ~= nil and tostring(current_pr.base_ref_name or "") ~= tostring(ctx.base_branch)) then
    return { kind = "identity-mismatch", current_pr = current_pr }
  end
  if classification.kind ~= OWN_CI_RED then
    return {
      kind = "not-own-ci",
      reason = classification.reason,
      current_pr = current_pr,
      bound_head_sha = bound_head_sha,
    }
  end
  local decision = admit_decision(state)
  if decision.kind == "terminate" then
    local terminal_ctx = {}
    for key, value in pairs(ctx) do terminal_ctx[key] = value end
    terminal_ctx.bound_head_sha = bound_head_sha
    terminal_ctx.reason = ctx.reason or classification.reason
    local intent = build_own_ci_terminal(terminal_ctx, state.version, C.FIX_LOOP_MAX_ROUNDS)
    return terminate(state, terminal_ctx, decision.round, intent)
  end
  decision.current_pr = current_pr
  decision.reason = classification.reason
  decision.ci_failure_key = classification.ci_failure_key
  decision.bound_head_sha = bound_head_sha
  return decision
end

function C.terminate_own_ci_policy_invalid(state, ctx)
  if type(state) ~= "table" or type(ctx) ~= "table" then
    error("github-devloop: own-ci-policy-terminal-invalid: state and context are required")
  end
  local round = devloop_state.version_fix_round(state.version)
  local intent = build_own_ci_terminal(ctx, state.version, C.CI_REPAIR_RETRY_POLICY_INVALID)
  return terminate(state, ctx, round, intent)
end

-- Non-own-CI merge-gate continuations retain their existing capped behavior. They do not
-- carry the own-CI classification and therefore cannot call the authority above.
function C.admit_or_terminate(state, ctx)
  if type(state) ~= "table" or type(ctx) ~= "table" then
    error("github-devloop: fix-round-admission-invalid: state and context are required")
  end
  local decision = admit_decision(state)
  if decision.kind == "terminate" then
    local intent = build_merge_gate_terminal(ctx, state.version)
    return terminate(state, ctx, decision.round, intent)
  end
  return decision
end

function C.admit_merge_failure(merge_ready, current_state, current_pr, source_ref, reason, classification)
  local ctx = {
    dept = "merge",
    from_state = "merge-ready",
    proposal_id = merge_ready.proposal_id,
    review_proposal_id = merge_ready.review_proposal_id,
    review_dedup_key = merge_ready.review_dedup_key,
    pr_number = merge_ready.pr_number,
    source_ref = source_ref,
    reason = reason,
  }
  local own_ci_red = parsers_misc.is_ci_red_reason(reason)
  if own_ci_red and classification == nil then
    error("github-devloop: own-ci-admission-classification-required: own-CI repair requires a current classification")
  end
  if not own_ci_red and classification ~= nil then
    error("github-devloop: own-ci-admission-classification-misapplied: own-CI classification cannot authorize a non-own-CI repair")
  end
  if own_ci_red then
    return C.admit_own_ci_continuation(current_state, classification, ctx)
  end
  ctx.bound_head_sha = current_pr.head_sha
  return C.admit_or_terminate(current_state, ctx)
end

return C
