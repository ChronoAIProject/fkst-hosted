local core = require("core")
local saga = require("contract.saga")

local spec = {
  consumes = { "devloop_intake_candidate" },
  produces = {
    "consensus.proposal",
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

local function intake_hand_off(candidate, decision, decision_dedup_key)
  return {
    kind = "own-intake-decision",
    proposal_id = candidate.proposal_id,
    decision = decision,
    dedup_key = decision_dedup_key or candidate.dedup_key,
    source_ref = core.normalize_source_ref(candidate.source_ref),
  }
end

local function build_direct_proposal(repo, issue_number, candidate, current, event_ts, decision_dedup_key)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = candidate.source_ref,
    content_fetch = core.context_fetch_ref_from_bundle({
      dept = "intake_judge",
      repo = repo,
      issue_number = issue_number,
      proposal_id = candidate.proposal_id,
      version = decision_dedup_key,
      tick = event_ts,
    }),
  }
  local proposal = core.build_board_proposal(issue, event_ts)
  proposal.dedup_key = decision_dedup_key or candidate.dedup_key
  proposal.effect_version = decision_dedup_key or candidate.dedup_key
  proposal.intake_hand_off = intake_hand_off(candidate, "enable", proposal.dedup_key)
  return core.validate_proposal(proposal) and proposal or nil
end

local function raise_enable_successor(dept, repo, issue_number, candidate, current, event_ts, decision_dedup_key, options)
  local opts = options or {}
  local direct_proposal = build_direct_proposal(repo, issue_number, candidate, current, event_ts, decision_dedup_key)
  if direct_proposal == nil then
    log.warn("github-devloop dept=" .. tostring(dept) .. " proposal_id=" .. tostring(candidate.proposal_id) .. " tag=SKIP reason=cannot-build-valid-direct-proposal")
    return false
  end
  local issue_ref = {
    repo = repo,
    number = issue_number,
    source_ref = candidate.source_ref,
  }
  local label_request = core.build_intake_enabled_label_request(repo, issue_number, candidate)
  local thinking_comment_request = core.build_observe_comment_request(issue_ref, direct_proposal)
  local thinking_label_request = core.build_thinking_label_request(issue_ref, direct_proposal)
  if opts.log_apply then
    local class_add, class_remove = core.intake_service_class_label_changes(candidate.service_class)
    core.log_cas_decision(dept, candidate.proposal_id, { state = nil, version = nil }, "intake-enable", "thinking", "applied(" .. tostring(opts.reason or "direct") .. ")", "raising direct intake successor event")
    core.log_apply(dept, candidate.proposal_id, "thinking", direct_proposal.effect_version, {
      add = { core._enabled_label, class_add[1], core._thinking_label },
      remove = class_remove,
    }, {
      "github-proxy.github_issue_label_request",
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
      "consensus.proposal",
    })
  end
  core.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
  core.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_comment_request", thinking_comment_request)
  core.log_raise(dept, candidate.proposal_id, "github-proxy.github_issue_label_request", thinking_label_request)
  core.log_raise(dept, candidate.proposal_id, "consensus.proposal", direct_proposal)
  return true
end

local function has_devloop_state_label(labels)
  for _, label in ipairs(labels or {}) do
    if core._state_labels[tostring(label)] then
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
  local current = core.parse_issue_view_intake_judge(view.stdout)
  current.repo, current.number = repo, issue_number
  core.log_forged_markers("intake_judge", candidate.proposal_id, current.comments)
  if current.state ~= "OPEN" then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-closed", "issue is not open")
    return nil
  end
  if core.is_intake_held(current.labels) then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-held", "fkst-dev:hold label is present")
    return nil
  end
  if not core.claim_issue_for_management("intake_judge", repo, issue_number, current, candidate.proposal_id) then
    return nil
  end

  local reintake_command = core.operator_command_fact(current.comments, "reintake")
  local has_pending_reintake = reintake_command ~= nil and not core.has_operator_command_response(current.comments, reintake_command)
  if has_pending_reintake and not core.has_intake_decision_marker(current.comments, candidate.proposal_id) then
    local refusal = core.build_operator_issue_command_refusal_request(
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
  if has_pending_reintake and (core.is_opted_in(current.labels) or has_devloop_state_label(current.labels)) then
    local refusal = core.build_operator_issue_command_refusal_request(
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
  local decision_dedup_key = core.intake_decision_dedup_key(candidate.proposal_id, current, has_pending_reintake and reintake_command or nil)
  if expected_decision_dedup_key ~= nil and tostring(decision_dedup_key or "") ~= tostring(expected_decision_dedup_key or "") then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-stale(decision-dedup-changed)", "issue intake inputs changed while codex was running")
    return nil
  end
  local intake_fact = core.intake_decision_fact(current.comments, candidate.proposal_id)
  local authoritative_state = core.current_state(current.comments, candidate.proposal_id)
  local can_replay_enable_successor = intake_fact ~= nil
    and intake_fact.decision == "enable"
    and tostring(intake_fact.dedup_key or "") == tostring(decision_dedup_key or "")
    and authoritative_state.state == nil
    and not has_pending_reintake
  if core.is_opted_in(current.labels) and not can_replay_enable_successor then
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
  if not core.is_supported_intake_candidate(candidate) then
    core.log_entry("intake_judge", event, "unknown", core.payload_field(candidate, "dedup_key"))
    core.log_cas_decision("intake_judge", "unknown", { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("intake_judge", event, candidate.proposal_id, candidate.dedup_key)
  local repo, issue_number = core.parse_issue_source_ref(candidate.source_ref)
  if repo == nil then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-foreign(source_ref)", "invalid source_ref")
    return
  end

  local lock_key = core.observe_lock_key(repo, issue_number)
  local gate = nil
  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    gate = read_current_for_candidate(repo, issue_number, candidate, event.ts)
  end)
  if gate == nil then
    return
  end

  core.log_codex_start("intake_judge", candidate.proposal_id, "intake")
  local content_fetch = core.context_fetch_from_bundle({
    dept = "intake_judge",
    repo = repo,
    issue_number = issue_number,
    proposal_id = candidate.proposal_id,
    version = gate.decision_dedup_key,
    tick = event.ts,
  })
  local result = spawn_codex_sync(core.judgment_codex_opts(
    core.build_intake_prompt(candidate.proposal_id, gate.current, content_fetch),
    core.judgment_worktree("intake", candidate.dedup_key)
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
    parsed.service_class = core.normalize_intake_service_class(nil)
    core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, "action=decline reason=parse-failed", nil)
  else
    parsed.service_class = core.normalize_intake_service_class(parsed.service_class)
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
      and core.build_operator_issue_reintake_comment_request(repo, issue_number, reintake_command, candidate, candidate.source_ref)
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
    local comment_request = core.build_intake_decision_comment_request(repo, issue_number, decision_candidate, parsed.action, parsed.reason, parsed.service_class)
    table.insert(raised, "github-proxy.github_issue_label_request")
    local class_add, class_remove = core.intake_service_class_label_changes(parsed.service_class)
    local apply_add = { class_add[1] }
    local apply_remove = class_remove
    if enables_pipeline(parsed.action) then
      table.insert(raised, "consensus.proposal")
      table.insert(raised, "github-proxy.github_issue_comment_request")
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
      local label_request = core.build_intake_tracking_label_request(repo, issue_number, candidate)
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
