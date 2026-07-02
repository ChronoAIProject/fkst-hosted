local devloop_base = require("devloop.base")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local requests_lifecycle = require("devloop.requests.lifecycle")
local parsers_issue = require("devloop.parsers.issue")
local core = require("core")
local execution_start = require("devloop.execution_start")
local operator_commands = require("devloop.operator_commands")
local saga = require("workflow.saga")
local context_bundle = require("devloop.context_bundle")
local v_execution_request = require("devloop.validators.execution_request")
local v_intake_candidate = require("devloop.validators.intake_candidate")
local m_facts = require("devloop.markers.facts")
local m_shared = require("devloop.markers.shared")
local entity_lib = require("devloop.entity")
local workflow_codex = require("workflow.codex")

local spec = {
  consumes = { "github-devloop-intake.devloop_intake_candidate" },
  produces = {
    "github-devloop.devloop_execute_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
  },
  stall_window = "2m",
}

local function intake_judge_done(_event)
  return false
end

local function decline_result(reason)
  return {
    action = "decline",
    reason = reason or "The intake decision output was malformed.",
  }
end

local function enables_pipeline(action)
  return action == "enable"
end

local function tracks_umbrella(action)
  return action == "track"
end

local function build_enable_request(candidate, decision_dedup_key)
  return execution_start.build_execution_request_payload({
    proposal_id = candidate.proposal_id,
    dedup_key = decision_dedup_key or candidate.dedup_key,
    source_ref = candidate.source_ref,
    origin = {
      package = "github-devloop-intake-default",
      route = "default",
      decision = "enable",
    },
    service_class = candidate.service_class,
  })
end

local function raise_enable_successor(dept, repo, issue_number, candidate, current, event_ts, decision_dedup_key, options)
  local opts = options or {}
  local _ = current
  local __ = event_ts
  local execution_request = build_enable_request(candidate, decision_dedup_key)
  if not v_execution_request.is_supported_execution_request(core, execution_request) then
    log.warn("github-devloop dept=" .. tostring(dept) .. " proposal_id=" .. tostring(candidate.proposal_id) .. " tag=SKIP reason=cannot-build-valid-execution-request")
    return false
  end
  local label_request = requests_labels.build_intake_enabled_label_request(core, repo, issue_number, candidate)
  if opts.log_apply then
    local class_add, class_remove = core.intake_service_class_label_changes(candidate.service_class)
    core.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "intake-enable", "execution-request", "applied(" .. tostring(opts.reason or "direct") .. ")", "raising execution request successor event")
    core.log_apply(dept, candidate.proposal_id, "enable", execution_request.dedup_key, {
      add = { core._enabled_label, class_add[1] },
      remove = class_remove,
    }, {
      "github-proxy.github_issue_label_request",
      "github-devloop.devloop_execute_request",
    })
  end
  core.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
  core.log_raise(dept, candidate.proposal_id, "github-devloop.devloop_execute_request", execution_request)
  return true
end

local function has_devloop_state_label(labels)
  for _, label in ipairs(labels or {}) do
    if core.is_state_label(label) then
      return true
    end
  end
  return false
end

local function copy_table(value)
  local result = {}
  for key, field in pairs(value or {}) do
    result[key] = field
  end
  return result
end

local function read_current_for_candidate(repo, issue_number, candidate, event_ts, expected_decision_dedup_key)
  local view = core.gh_issue_view_intake_judge(repo, issue_number, 30)
  if view.exit_code ~= 0 then
    error("github-devloop: gh issue intake judge view failed: " .. tostring(view.stderr))
  end
  local current = parsers_issue.parse_issue_view_intake_judge(core, view.stdout)
  current.repo, current.number = repo, issue_number
  core.log_forged_markers("intake_judge", candidate.proposal_id, current.comments)
  if current.state ~= "OPEN" then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-closed", "issue is not open")
    return nil
  end
  if devloop_base.is_intake_held(current.labels) then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-held", "fkst-dev:hold label is present")
    return nil
  end
  if not m_claims.claim_issue_for_management(core, "intake_judge", repo, issue_number, current, candidate.proposal_id) then
    return nil
  end

  local reintake_command = operator_commands.operator_command_fact(core, current.comments, "reintake")
  local has_pending_reintake = reintake_command ~= nil and not operator_commands.has_operator_command_response(core, current.comments, reintake_command)
  if has_pending_reintake and not m_facts.has_intake_decision_marker(core, current.comments, candidate.proposal_id) then
    local refusal = operator_commands.build_operator_issue_command_refusal_request(
      core,
      repo,
      issue_number,
      reintake_command,
      "reintake requires an existing intake decision",
      candidate.source_ref
    )
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "refused(reintake-no-intake-decision)", "operator reintake requires an existing intake decision")
    core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return nil
  end
  if has_pending_reintake and (devloop_base.is_opted_in(current.labels) or has_devloop_state_label(current.labels)) then
    local refusal = operator_commands.build_operator_issue_command_refusal_request(
      core,
      repo,
      issue_number,
      reintake_command,
      "reintake requires no active devloop state",
      candidate.source_ref
    )
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "refused(reintake-active-state)", "operator reintake requires no active devloop state")
    core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return nil
  end
  if has_pending_reintake then
    local expected = tostring(reintake_command.created_at or "")
    if tostring(candidate.reintake_command_created_at or "") ~= expected then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-stale-reintake-candidate", "operator reintake candidate must be keyed by command timestamp")
      return nil
    end
  end
  local decision_dedup_key = devloop_base.intake_decision_dedup_key(candidate.proposal_id, current, has_pending_reintake and reintake_command or nil)
  if expected_decision_dedup_key ~= nil and tostring(decision_dedup_key or "") ~= tostring(expected_decision_dedup_key or "") then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-stale(decision-dedup-changed)", "issue intake inputs changed while codex was running")
    return nil
  end
  local intake_fact = m_facts.intake_decision_fact(core, current.comments, candidate.proposal_id)
  local authoritative_state = core.current_state(current.comments, candidate.proposal_id)
  local can_replay_enable_successor = intake_fact ~= nil
    and intake_fact.decision == "enable"
    and tostring(intake_fact.dedup_key or "") == tostring(decision_dedup_key or "")
    and authoritative_state.state == nil
    and not has_pending_reintake
  if devloop_base.is_opted_in(current.labels) and not can_replay_enable_successor then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-enabled", "fkst-dev:enabled is already present")
    return nil
  end
  if intake_fact ~= nil and not has_pending_reintake then
    if can_replay_enable_successor then
      local replay_candidate = copy_table(candidate)
      replay_candidate.service_class = intake_fact.service_class
      raise_enable_successor("intake_judge", repo, issue_number, replay_candidate, current, event_ts, intake_fact.dedup_key, {
        log_apply = true,
        reason = "visible-intake-fact",
      })
      return nil
    end
    if tostring(intake_fact.dedup_key or "") == tostring(decision_dedup_key or "") then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-idempotent(intake marker already visible)", "trusted intake decision marker exists")
      return nil
    end
  end

  return {
    current = current,
    decision_dedup_key = decision_dedup_key,
    reintake_command = reintake_command,
    has_pending_reintake = has_pending_reintake,
  }
end

local function act_intake_judge(event)
  local candidate = event.payload or {}
  if not v_intake_candidate.is_supported_intake_candidate(core, candidate) then
    core.log_entry("intake_judge", event, "unknown", core.payload_field(candidate, "dedup_key"))
    core.log_cas_decision("intake_judge", "unknown", { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("intake_judge", event, candidate.proposal_id, candidate.dedup_key)
  local repo, issue_number = devloop_base.parse_issue_source_ref(candidate.source_ref)
  if repo == nil then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(source_ref)", "invalid source_ref")
    return
  end

  local lock_key = entity_lib.observe_lock_key(repo, issue_number)
  local gate = nil
  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()
    gate = read_current_for_candidate(repo, issue_number, candidate, event.ts)
  end)
  if gate == nil then
    return
  end

  core.log_codex_start("intake_judge", candidate.proposal_id, "intake")
  local content_fetch = context_bundle.context_fetch_from_bundle(core, {
    dept = "intake_judge",
    repo = repo,
    issue_number = issue_number,
    proposal_id = candidate.proposal_id,
    version = gate.decision_dedup_key,
    tick = event.ts,
  })
  local result = spawn_codex_sync(workflow_codex.judgment_codex_opts(
    core.build_intake_prompt(candidate.proposal_id, gate.current, content_fetch),
    devloop_base.judgment_worktree_with_exec(exec_sync, "intake", candidate.dedup_key)
  ))
  if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
    local stderr = type(result) == "table" and result.stderr or "nil result"
    core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, nil, stderr, {
      queue = event.queue,
      source_ref = candidate.source_ref,
      terminal = false,
    })
    error("github-devloop: intake codex failed: " .. tostring(stderr))
  end

  local parsed = core.parse_intake_action(result.stdout)
  if parsed == nil then
    parsed = decline_result()
    parsed.service_class = m_shared.normalize_intake_service_class(nil)
    core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, "action=decline reason=parse-failed", nil)
  else
    parsed.service_class = m_shared.normalize_intake_service_class(parsed.service_class)
    core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, "action=" .. tostring(parsed.action) .. " class=" .. tostring(parsed.service_class) .. " reason=" .. tostring(parsed.reason), nil)
  end

  with_lock(lock_key, function()
    local current_gate = read_current_for_candidate(repo, issue_number, candidate, event.ts, gate.decision_dedup_key)
    if current_gate == nil then
      return
    end
    local current = current_gate.current
    local decision_dedup_key = current_gate.decision_dedup_key
    local reintake_command = current_gate.reintake_command
    local has_pending_reintake = current_gate.has_pending_reintake

    candidate.service_class = parsed.service_class
    local decision_candidate = copy_table(candidate)
    decision_candidate.dedup_key = decision_dedup_key
    local command_comment_request = has_pending_reintake
      and operator_commands.build_operator_issue_reintake_comment_request(core, repo, issue_number, reintake_command, candidate, candidate.source_ref)
      or nil
    local raised = {
      "github-proxy.github_issue_comment_request",
    }
    if command_comment_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_comment_request")
    end
    local class_carrier = nil
    local class_key = nil
    if parsed.action == "escalate-to-class" then
      local sibling_issues = core.fetch_recent_closed_intake_class_issues(repo)
      class_key = core.intake_class_identity(parsed.reason, current, issue_number, sibling_issues)
      if class_key == nil then
        parsed.action = "enable"
        parsed.reason = tostring(parsed.reason or "") .. "\n\nNo stable recurring-class identity was found; enabling as an ordinary issue instead of creating a title-derived class carrier."
      else
        class_carrier = core.find_open_intake_class_carrier(repo, issue_number, current, class_key)
        table.insert(raised, "github-proxy.github_issue_comment_request")
        table.insert(raised, "github-proxy.github_issue_label_request")
        if class_carrier == nil then
          table.insert(raised, "github-proxy.github_issue_create_request")
        end
      end
    end
    candidate.service_class = parsed.service_class
    local comment_request = requests_lifecycle.build_intake_decision_comment_request(core, repo, issue_number, decision_candidate, parsed.action, parsed.reason, parsed.service_class)
    table.insert(raised, "github-proxy.github_issue_label_request")
    local class_add, class_remove = core.intake_service_class_label_changes(parsed.service_class)
    local apply_add = { class_add[1] }
    local apply_remove = class_remove
    if enables_pipeline(parsed.action) then
      table.insert(raised, "github-devloop.devloop_execute_request")
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    if enables_pipeline(parsed.action) then
      table.insert(apply_add, 1, core._enabled_label)
    elseif tracks_umbrella(parsed.action) then
      table.insert(apply_add, 1, core._tracking_label)
    end
    core.log_apply("intake_judge", candidate.proposal_id, parsed.action, candidate.dedup_key, {
      add = apply_add,
      remove = apply_remove,
    }, raised)
    if command_comment_request ~= nil then
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", command_comment_request)
    end
    core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    if parsed.action == "escalate-to-class" then
      local followup_comment = core.build_intake_class_followup_comment_request(
        repo,
        issue_number,
        candidate,
        class_carrier,
        "folded",
        parsed.reason
      )
      local folded_label = core.build_intake_class_folded_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", followup_comment)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", folded_label)
      if class_carrier == nil then
        local create_request = core.build_intake_class_issue_create_request(repo, issue_number, candidate, current, parsed.reason, class_key)
        core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_create_request", create_request)
      end
    end
    if enables_pipeline(parsed.action) then
      raise_enable_successor("intake_judge", repo, issue_number, candidate, current, event.ts, decision_dedup_key)
    elseif tracks_umbrella(parsed.action) then
      local label_request = requests_labels.build_intake_tracking_label_request(core, repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    else
      local label_request = core.build_intake_service_class_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end

return saga.department(spec, {
  done = intake_judge_done,
  act = act_intake_judge,
  wrap = core.wrap_pipeline_failure,
  name = "intake_judge",
})
