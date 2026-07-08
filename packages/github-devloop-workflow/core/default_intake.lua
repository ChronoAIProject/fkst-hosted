local context_bundle = require("devloop.context_bundle")
local devloop_base = require("devloop.base")
local devloop_commands = require("devloop.commands")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_lib = require("devloop.entity")
local execution_start = require("devloop.execution_start")
local m_claims = require("devloop.claims")
local m_facts = require("devloop.markers.facts")
local m_shared = require("devloop.markers.shared")
local operator_commands = require("devloop.operator_commands")
local parsers_issue = require("devloop.parsers.issue")
local requests_labels = require("devloop.requests.labels")
local requests_lifecycle = require("devloop.requests.lifecycle")
local v_execution_request = require("devloop.validators.execution_request")
local v_intake_candidate = require("devloop.validators.intake_candidate")
local workflow_codex = require("workflow.codex")

local M = {}

local function malformed_decision(reason)
  return {
    action = "decline",
    reason = reason or "The intake decision output was malformed.",
  }
end

local function is_enable(action)
  return action == "enable"
end

local function is_tracking(action)
  return action == "track"
end

local function execution_request_for(candidate, decision_dedup_key)
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

local function copy_fields(value)
  local result = {}
  for key, field in pairs(value or {}) do
    result[key] = field
  end
  return result
end

local function issue_has_active_devloop_state(labels, blocked_reached)
  for _, label in ipairs(labels or {}) do
    if devloop_state.is_state_label(label)
      and not (blocked_reached and tostring(label) == tostring(devloop_base._blocked_label)) then
      return true
    end
  end
  return false
end

local function raise_enable_successor(package_core, dept, repo, issue_number, candidate, current, event_ts, decision_dedup_key, options)
  local opts = options or {}
  local _ = current
  local __ = event_ts
  local execution_request = execution_request_for(candidate, decision_dedup_key)
  if not v_execution_request.is_supported_execution_request(execution_request) then
    log.warn("github-devloop dept=" .. tostring(dept) .. " proposal_id=" .. tostring(candidate.proposal_id) .. " tag=SKIP reason=cannot-build-valid-execution-request")
    return false
  end
  local label_request = requests_labels.build_intake_enabled_label_request(package_core, repo, issue_number, candidate)
  if opts.log_apply then
    local class_add, class_remove = package_core.intake_service_class_label_changes(candidate.service_class)
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "intake-enable", "execution-request", "applied(" .. tostring(opts.reason or "direct") .. ")", "raising execution request successor event")
    devloop_logging.log_apply(dept, candidate.proposal_id, "enable", execution_request.dedup_key, {
      add = { package_core._enabled_label, class_add[1] },
      remove = class_remove,
    }, {
      "github-proxy.github_issue_label_request",
      "github-devloop.devloop_execute_request",
    })
  end
  devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
  devloop_logging.log_raise(dept, candidate.proposal_id, "github-devloop.devloop_execute_request", execution_request)
  return true
end

function M.read_current_for_candidate(package_core, dept, repo, issue_number, candidate, event_ts, expected_decision_dedup_key)
  local view = devloop_commands.gh_issue_view_intake_judge(repo, issue_number, 30)
  if view.exit_code ~= 0 then
    error("github-devloop: gh-issue-view-failed: gh issue intake judge view failed: " .. tostring(view.stderr))
  end
  local current = parsers_issue.parse_issue_view_intake_judge(package_core, view.stdout)
  current.repo, current.number = repo, issue_number
  devloop_logging.log_forged_markers(dept, candidate.proposal_id, current.comments)
  if current.state ~= "OPEN" then
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-closed", "issue is not open")
    return nil
  end
  if devloop_base.is_intake_held(current.labels) then
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-held", "fkst-dev:hold label is present")
    return nil
  end
  if not m_claims.claim_issue_for_management(package_core, dept, repo, issue_number, current, candidate.proposal_id) then
    return nil
  end

  local reintake_command = operator_commands.operator_command_fact(current.comments, "reintake")
  local has_pending_reintake = reintake_command ~= nil and not operator_commands.has_operator_command_response(current.comments, reintake_command)
  if has_pending_reintake and not m_facts.has_intake_decision_marker(current.comments, candidate.proposal_id) then
    local refusal = operator_commands.build_operator_issue_command_refusal_request(repo,
      issue_number,
      reintake_command,
      "reintake requires an existing intake decision",
      candidate.source_ref
    )
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "refused(reintake-no-intake-decision)", "operator reintake requires an existing intake decision")
    devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return nil
  end
  local blocked_reached = devloop_state.reached(current.comments, candidate.proposal_id, "blocked", {
    domain = "github-devloop-issue",
  })
  if has_pending_reintake and ((devloop_base.is_opted_in(current.labels) and not blocked_reached) or issue_has_active_devloop_state(current.labels, blocked_reached)) then
    local refusal = operator_commands.build_operator_issue_command_refusal_request(repo,
      issue_number,
      reintake_command,
      "reintake requires terminal blocked or no active devloop state; use rereview, reready, or reimplement for recoverable active states",
      candidate.source_ref
    )
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "refused(reintake-active-state)", "operator reintake requires terminal blocked or no active devloop state")
    devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return nil
  end
  if has_pending_reintake then
    local expected = tostring(reintake_command.created_at or "")
    if tostring(candidate.reintake_command_created_at or "") ~= expected then
      devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-stale-reintake-candidate", "operator reintake candidate must be keyed by command timestamp")
      return nil
    end
  end

  local decision_dedup_key = devloop_base.intake_decision_dedup_key(candidate.proposal_id, current, has_pending_reintake and reintake_command or nil)
  if expected_decision_dedup_key ~= nil and tostring(decision_dedup_key or "") ~= tostring(expected_decision_dedup_key or "") then
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-stale(decision-dedup-changed)", "issue intake inputs changed while codex was running")
    return nil
  end
  local intake_fact = m_facts.intake_decision_fact(current.comments, candidate.proposal_id)
  local reached_thinking = devloop_state.reached(current.comments, candidate.proposal_id, "thinking", {
    domain = "github-devloop",
  })
  local can_replay_enable_successor = intake_fact ~= nil
    and intake_fact.decision == "enable"
    and tostring(intake_fact.dedup_key or "") == tostring(decision_dedup_key or "")
    and not reached_thinking
    and not has_pending_reintake
  if devloop_base.is_opted_in(current.labels) and not has_pending_reintake and not can_replay_enable_successor then
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-enabled", "fkst-dev:enabled is already present")
    return nil
  end
  if intake_fact ~= nil and not has_pending_reintake then
    if can_replay_enable_successor then
      local replay_candidate = copy_fields(candidate)
      replay_candidate.service_class = intake_fact.service_class
      raise_enable_successor(package_core, dept, repo, issue_number, replay_candidate, current, event_ts, intake_fact.dedup_key, {
        log_apply = true,
        reason = "visible-intake-fact",
      })
      return nil
    end
    if tostring(intake_fact.dedup_key or "") == tostring(decision_dedup_key or "") then
      devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-idempotent(intake marker already visible)", "trusted intake decision marker exists")
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

local function apply_intake_decision(package_core, dept, repo, issue_number, event, candidate, gate, parsed)
  with_lock(gate.lock_key, function()
    local current_gate = M.read_current_for_candidate(package_core, dept, repo, issue_number, candidate, event.ts, gate.decision_dedup_key)
    if current_gate == nil then
      return
    end
    local current = current_gate.current
    local decision_dedup_key = current_gate.decision_dedup_key
    local reintake_command = current_gate.reintake_command
    local has_pending_reintake = current_gate.has_pending_reintake

    candidate.service_class = parsed.service_class
    local decision_candidate = copy_fields(candidate)
    decision_candidate.dedup_key = decision_dedup_key
    local command_comment_request = has_pending_reintake
      and operator_commands.build_operator_issue_reintake_comment_request(repo, issue_number, reintake_command, candidate, candidate.source_ref)
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
      local sibling_issues = package_core.fetch_recent_closed_intake_class_issues(repo)
      class_key = package_core.intake_class_identity(parsed.reason, current, issue_number, sibling_issues)
      if class_key == nil then
        parsed.action = "enable"
        parsed.reason = tostring(parsed.reason or "") .. "\n\nNo stable recurring-class identity was found; enabling as an ordinary issue instead of creating a title-derived class carrier."
      else
        class_carrier = package_core.find_open_intake_class_carrier(repo, issue_number, current, class_key)
        table.insert(raised, "github-proxy.github_issue_comment_request")
        table.insert(raised, "github-proxy.github_issue_label_request")
        if class_carrier == nil then
          table.insert(raised, "github-proxy.github_issue_create_request")
        end
      end
    end
    candidate.service_class = parsed.service_class
    local comment_request = requests_lifecycle.build_intake_decision_comment_request(package_core, repo, issue_number, decision_candidate, parsed.action, parsed.reason, parsed.service_class)
    table.insert(raised, "github-proxy.github_issue_label_request")
    local class_add, class_remove = package_core.intake_service_class_label_changes(parsed.service_class)
    local apply_add = { class_add[1] }
    local apply_remove = class_remove
    if is_enable(parsed.action) then
      table.insert(raised, "github-devloop.devloop_execute_request")
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    if is_enable(parsed.action) then
      table.insert(apply_add, 1, devloop_base._enabled_label)
    elseif is_tracking(parsed.action) then
      table.insert(apply_add, 1, devloop_base._tracking_label)
    end
    devloop_logging.log_apply(dept, candidate.proposal_id, parsed.action, candidate.dedup_key, {
      add = apply_add,
      remove = apply_remove,
    }, raised)
    if command_comment_request ~= nil then
      devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", command_comment_request)
    end
    devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    if parsed.action == "escalate-to-class" then
      local followup_comment = package_core.build_intake_class_followup_comment_request(
        repo,
        issue_number,
        candidate,
        class_carrier,
        "folded",
        parsed.reason
      )
      local folded_label = package_core.build_intake_class_folded_label_request(repo, issue_number, candidate)
      devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", followup_comment)
      devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", folded_label)
      if class_carrier == nil then
        local create_request = package_core.build_intake_class_issue_create_request(repo, issue_number, candidate, current, parsed.reason, class_key)
        devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_create_request", create_request)
      end
    end
    if is_enable(parsed.action) then
      raise_enable_successor(package_core, dept, repo, issue_number, candidate, current, event.ts, decision_dedup_key)
    elseif is_tracking(parsed.action) then
      local label_request = requests_labels.build_intake_tracking_label_request(package_core, repo, issue_number, candidate)
      devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    else
      local label_request = package_core.build_intake_service_class_label_request(repo, issue_number, candidate)
      devloop_logging.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end

function M.act(package_core, event, opts)
  opts = opts or {}
  local dept = opts.dept or "intake_judge"
  local candidate = event.payload or {}
  if not v_intake_candidate.is_supported_intake_candidate(candidate) then
    devloop_logging.log_entry(dept, event, "unknown", devloop_logging.payload_field(candidate, "dedup_key"))
    devloop_logging.log_cas_decision(dept, "unknown", { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  devloop_logging.log_entry(dept, event, candidate.proposal_id, candidate.dedup_key)
  local repo, issue_number = devloop_base.parse_issue_source_ref(candidate.source_ref)
  if repo == nil then
    devloop_logging.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(source_ref)", "invalid source_ref")
    return
  end

  local lock_key = entity_lib.observe_lock_key(repo, issue_number)
  local gate = nil
  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()
    gate = M.read_current_for_candidate(package_core, dept, repo, issue_number, candidate, event.ts)
  end)
  if gate == nil then
    return
  end
  gate.lock_key = lock_key

  local ctx = {
    repo = repo,
    issue_number = issue_number,
    candidate = candidate,
    current = gate.current,
    decision_dedup_key = gate.decision_dedup_key,
    reintake_command = gate.reintake_command,
    has_pending_reintake = gate.has_pending_reintake,
    lock_key = lock_key,
    event_ts = event.ts,
  }
  if type(opts.before_codex) == "function" and opts.before_codex(ctx) then
    return
  end

  devloop_logging.log_codex_start(dept, candidate.proposal_id, "intake")
  local content_fetch = context_bundle.context_fetch_from_bundle(package_core, {
    dept = dept,
    repo = repo,
    issue_number = issue_number,
    proposal_id = candidate.proposal_id,
    version = gate.decision_dedup_key,
    tick = event.ts,
  })
  local result = spawn_codex_sync(workflow_codex.judgment_codex_opts(
    package_core.build_intake_prompt(candidate.proposal_id, gate.current, content_fetch),
    devloop_base.judgment_worktree_with_exec(exec_sync, "intake", candidate.dedup_key)
  ))
  if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
    local stderr = type(result) == "table" and result.stderr or "nil result"
    devloop_logging.log_codex_result(dept, candidate.proposal_id, "intake", result, nil, stderr, {
      queue = event.queue,
      source_ref = candidate.source_ref,
      terminal = false,
    })
    error("github-devloop: intake-codex-failed: intake codex failed: " .. tostring(stderr))
  end

  local parsed = package_core.parse_intake_action(result.stdout)
  if parsed == nil then
    parsed = malformed_decision()
    parsed.service_class = m_shared.normalize_intake_service_class(nil)
    devloop_logging.log_codex_result(dept, candidate.proposal_id, "intake", result, "action=decline reason=parse-failed", nil)
  else
    parsed.service_class = m_shared.normalize_intake_service_class(parsed.service_class)
    devloop_logging.log_codex_result(dept, candidate.proposal_id, "intake", result, "action=" .. tostring(parsed.action) .. " class=" .. tostring(parsed.service_class) .. " reason=" .. tostring(parsed.reason), nil)
  end

  apply_intake_decision(package_core, dept, repo, issue_number, event, candidate, gate, parsed)
end

return M
