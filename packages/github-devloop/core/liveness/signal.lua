local S = {}

function S.install(M, shared)
local numeric_minutes = shared.numeric_minutes
local signal_age_from_created_at = shared.signal_age_from_created_at
local marker_attr = shared.marker_attr
local strip_liveness_timeout_suffixes = shared.strip_liveness_timeout_suffixes
local liveness_contract_signal = shared.liveness_contract_signal
local row_liveness_signal = shared.row_liveness_signal

local function newest_matching_marker_age(M, comments, family, matches, now_seconds)
  local pattern_family = tostring(family or ""):gsub("%-", "%%-")
  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. pattern_family .. ":v1.-%-%->"
  local newest_age = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments or {})) do
    local age = signal_age_from_created_at(M, M._comment_created_at(comment), now_seconds)
    if age ~= nil and (newest_age == nil or age < newest_age) then
      for marker in M._comment_body(comment):gmatch(marker_pattern) do
        if matches(marker) then
          newest_age = age
          break
        end
      end
    end
  end
  return newest_age
end

local function matching_marker_age_or_zero(M, comments, family, matches, now_seconds)
  local pattern_family = tostring(family or ""):gsub("%-", "%%-")
  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. pattern_family .. ":v1.-%-%->"
  local newest_age = nil
  local found = false
  for _, comment in ipairs(M._trusted_marker_comments(comments or {})) do
    local age = signal_age_from_created_at(M, M._comment_created_at(comment), now_seconds)
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if matches(marker) then
        found = true
        if age ~= nil and (newest_age == nil or age < newest_age) then
          newest_age = age
        end
      end
    end
  end
  if found then
    return newest_age or 0
  end
  return nil
end

local function live_signal_comments(signal, facts)
  if signal and signal.surface == "pr-comment-stream" then
    return facts and facts.current_pr and facts.current_pr.comments or nil
  end
  if signal and signal.surface == "issue-comment-stream" then
    return facts and facts.current and facts.current.comments or nil
  end
  return nil
end

local function live_signal_version(M, signal, version)
  return M.liveness_heartbeat_version(version, signal)
end

local function merge_gate_wait_identity(M, facts, state)
  local source_repo, source_pr = M.parse_pr_source_ref(facts and facts.source_ref)
  local pr_number = source_pr
  local head_sha = facts and facts.head_sha or nil
  if facts and facts.current_pr ~= nil then
    head_sha = facts.current_pr.head_sha or head_sha
  end
  if pr_number == nil and facts and facts.current_pr ~= nil then
    pr_number = facts.current_pr.number
  end
  if pr_number == nil and facts and facts.link ~= nil then
    pr_number = facts.link.pr_number
  end
  return (facts and facts.proposal_id) or (state and state.proposal_id),
    M.merge_gate_wait_version_lineage(state and state.version),
    pr_number,
    head_sha,
    source_repo
end

local function delegation_comments(facts)
  if facts and facts.current and type(facts.current.comments) == "table" then
    return facts.current.comments
  end
  if facts and facts.snapshot and type(facts.snapshot.comments) == "table" then
    return facts.snapshot.comments
  end
  return nil
end

local function fact_child_proposal_id(M, fact, parent_proposal_id, version)
  if type(fact) ~= "table" then
    return nil
  end
  if fact.proposal_id ~= nil and tostring(fact.proposal_id) ~= tostring(parent_proposal_id) then
    return nil
  end
  if fact.version ~= nil and tostring(fact.version) ~= tostring(version or "") then
    return nil
  end
  local child_proposal_id = fact.pr_proposal_id or fact.pr_proposal
  if M.parse_pr_proposal_id(child_proposal_id) == nil then
    return nil
  end
  return tostring(child_proposal_id)
end

local function pr_delegation_child_proposal_id(M, facts, parent_proposal_id, delegation_version)
  local direct = facts and (facts.pr_delegation or facts["pr-delegation"]) or nil
  local child_proposal_id = fact_child_proposal_id(M, direct, parent_proposal_id, delegation_version)
  if child_proposal_id ~= nil then
    return child_proposal_id
  end
  if type(M.pr_delegation_fact) ~= "function" then
    return nil
  end
  return fact_child_proposal_id(
    M,
    M.pr_delegation_fact(delegation_comments(facts), parent_proposal_id, delegation_version),
    parent_proposal_id,
    delegation_version
  )
end

local function implement_attempt_liveness_signal(M, signal_contract, comments, proposal_id, signal_version)
  local attempt = M.latest_implement_attempt_fact(comments, proposal_id, signal_version)
  if attempt == nil then
    return {
      live = false,
      reason = "missing-implement-attempt",
      family = signal_contract.family,
      resolver = signal_contract.resolver or signal_contract.family,
    }
  end
  if type(attempt.exec_ref) ~= "string" or attempt.exec_ref == "" then
    return {
      live = false,
      reason = "missing-exec-ref",
      attempt = attempt.attempt,
      family = signal_contract.family,
      resolver = signal_contract.resolver or signal_contract.family,
    }
  end
  if M.implement_exec_ref_running(attempt.exec_ref) then
    return {
      live = true,
      reason = "codex-run-running",
      attempt = attempt.attempt,
      exec_ref = attempt.exec_ref,
      family = signal_contract.family,
      resolver = signal_contract.resolver or signal_contract.family,
    }
  end
  return {
    live = false,
    reason = "codex-run-not-running",
    attempt = attempt.attempt,
    exec_ref = attempt.exec_ref,
    family = signal_contract.family,
    resolver = signal_contract.resolver or signal_contract.family,
  }
end

local function live_signal_age(M, row, state, facts, now_seconds)
  local signal = row_liveness_signal(row)
  local resolver = signal and (signal.resolver or signal.family) or nil
  local comments = live_signal_comments(signal, facts)
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local signal_version = live_signal_version(M, signal, state and state.version)
  if resolver == "dependency-hold" then
    local hold = M.dependency_hold_fact(comments, proposal_id)
    if hold ~= nil and tostring(hold.version or "") == tostring(signal_version or "") then
      return signal_age_from_created_at(M, hold.comment_created_at, now_seconds) or 0
    end
    return matching_marker_age_or_zero(M, comments, "dependency-wait", function(marker)
      return marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(signal_version or "")
    end, now_seconds) or matching_marker_age_or_zero(M, comments, "dependency-cycle", function(marker)
      return marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(signal_version or "")
    end, now_seconds) or matching_marker_age_or_zero(M, comments, "dependency-unresolvable", function(marker)
      return marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(signal_version or "")
    end, now_seconds)
  end
  if resolver == "implement-attempt" then
    return nil
  end
  if resolver == "converge-round" then
    local source_ref = facts and facts.source_ref
    local sr_digest = M.source_ref_digest(source_ref)
    local base_version = M.version_loop_round(signal_version) > 0 and M.converge_base_version(signal_version) or signal_version
    return newest_matching_marker_age(M, comments, "converge-round", function(marker)
      return marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(base_version)
        and marker_attr(marker, "source_ref") == tostring(sr_digest)
    end, now_seconds)
  end
  if resolver == "review-converge-round" then
    local head_sha = facts and facts.head_sha
    local review_proposal_id = facts and facts.review_proposal_id
    local source_repo, source_pr = M.parse_pr_source_ref(facts and facts.source_ref)
    if source_repo ~= nil
      and source_pr ~= nil
      and M._is_git_sha(head_sha) then
      review_proposal_id = M.pr_review_proposal_id(source_repo, source_pr, strip_liveness_timeout_suffixes(state and state.version), head_sha)
    end
    local sr_digest = M.source_ref_digest(facts and facts.source_ref)
    return matching_marker_age_or_zero(M, comments, "review-converge-round", function(marker)
      return marker_attr(marker, "proposal") == tostring(review_proposal_id)
        and marker_attr(marker, "issue_proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(signal_version)
        and marker_attr(marker, "head_sha") == tostring(head_sha)
        and marker_attr(marker, "source_ref") == tostring(sr_digest)
    end, now_seconds)
  end
  if resolver == "merge-gate-wait" then
    local wait_proposal_id, wait_version, pr_number, head_sha = merge_gate_wait_identity(M, facts, state)
    if wait_proposal_id == nil or pr_number == nil or not M._is_git_sha(head_sha) then
      return nil
    end
    return newest_matching_marker_age(M, comments, "merge-gate-wait", function(marker)
      return marker_attr(marker, "proposal") == tostring(wait_proposal_id)
        and marker_attr(marker, "version") == tostring(wait_version)
        and marker_attr(marker, "pr") == tostring(pr_number)
        and marker_attr(marker, "head_sha") == tostring(head_sha)
    end, now_seconds)
  end
  if resolver == "child-state" then
    local child_proposal_id = pr_delegation_child_proposal_id(M, facts, proposal_id, signal_version)
    if child_proposal_id == nil then
      return nil
    end
    local terminal_states = {}
    for _, terminal_state in ipairs(row and row.defer and row.defer.terminal_states or {}) do
      terminal_states[tostring(terminal_state)] = true
    end
    local latest = nil
    local pattern_family = "state"
    local marker_pattern = "<!%-%- fkst:github%-devloop:" .. pattern_family .. ":v1.-%-%->"
    for _, comment in ipairs(M._trusted_marker_comments(comments or {})) do
      local age = signal_age_from_created_at(M, M._comment_created_at(comment), now_seconds)
      for marker in M._comment_body(comment):gmatch(marker_pattern) do
        if marker_attr(marker, "proposal") == tostring(child_proposal_id) then
          local child_state = marker_attr(marker, "state")
          if terminal_states[child_state] ~= true
            and (latest == nil or (age ~= nil and (latest.age == nil or age < latest.age))) then
            latest = {
              age = age or 0,
            }
          end
        end
      end
    end
    return latest and latest.age or nil
  end
  return nil
end

function M.restart_row_liveness_signal(row, state, facts, now_seconds)
  local contract = row and row.liveness_contract
  if type(contract) ~= "table" then
    return { live = false, reason = "missing-contract" }
  end
  local signal_contract = liveness_contract_signal(contract)
  if type(signal_contract) ~= "table" then
    return { live = false, reason = "no-liveness-signal" }
  end
  local resolver = signal_contract.resolver or signal_contract.family
  if resolver == "implement-attempt" then
    local comments = live_signal_comments(signal_contract, facts)
    local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
    local signal_version = live_signal_version(M, signal_contract, state and state.version)
    return implement_attempt_liveness_signal(M, signal_contract, comments, proposal_id, signal_version)
  end
  local max_age = numeric_minutes(signal_contract.max_age_minutes)
  if max_age == nil then
    return { live = false, reason = "invalid-liveness-signal" }
  end
  local age = live_signal_age(M, row, state, facts, now_seconds)
  if age ~= nil and age < max_age then
    return {
      live = true,
      age_minutes = age,
      max_age_minutes = max_age,
      family = signal_contract.family,
      resolver = signal_contract.resolver or signal_contract.family,
    }
  end
  return {
    live = false,
    age_minutes = age,
    max_age_minutes = max_age,
    family = signal_contract.family,
    resolver = signal_contract.resolver or signal_contract.family,
  }
end

function M.restart_row_receiver_liveness(row, state, facts, now_seconds)
  if M.restart_row_has_registered_actionable_epoch(row)
    and row
    and row.watchdog
    and row.watchdog.mode == "live-defer" then
    local eval = M.actionable_epoch_resolve(row, state, facts, now_seconds)
    if type(facts) == "table" then
      facts.actionable_epoch_eval = eval
    end
    if eval.status == "deferred" then
      return {
        action = "defer",
        reason = "actionable-epoch-deferred",
        signal = {
          family = row.defer and row.defer.live_marker,
          resolver = row.actionable_epoch and row.actionable_epoch.source,
        },
      }
    end
    return {
      action = "stuck",
      reason = eval.status == "contract_invalid" and "actionable-epoch-contract-invalid" or "actionable-epoch-actionable",
      actionable_epoch = eval,
    }
  end
  local contract = row and row.liveness_contract
  if type(contract) ~= "table" then
    return { action = "stuck", reason = "missing-contract" }
  end
  if contract.mode == "live-defer" then
    local signal = M.restart_row_liveness_signal(row, state, facts, now_seconds)
    if signal.live then
      return {
        action = "defer",
        reason = "live-signal",
        signal = signal,
      }
    end
    return {
      action = "stuck",
      reason = "signal-stale-or-missing",
      signal = signal,
    }
  end
  if contract.mode == "row-budget-bounds-receiver" then
    local absolute_due, state_age = M.liveness_timeout_due(row, state, now_seconds)
    if absolute_due then
      return {
        action = "stuck",
        reason = "row-budget-absolute-cap",
        age_minutes = state_age,
        receiver_bound_minutes = contract.receiver_bound_minutes,
        external_wait_bound_minutes = contract.external_wait_bound_minutes,
      }
    end
    if type(contract.progress_signal) == "table" then
      local signal = M.restart_row_liveness_signal(row, state, facts, now_seconds)
      if signal.live then
        return {
          action = "defer",
          reason = "live-signal",
          signal = signal,
          receiver_bound_minutes = contract.receiver_bound_minutes,
          external_wait_bound_minutes = contract.external_wait_bound_minutes,
        }
      end
      return {
        action = "stuck",
        reason = "signal-stale-or-missing",
        signal = signal,
        receiver_bound_minutes = contract.receiver_bound_minutes,
        external_wait_bound_minutes = contract.external_wait_bound_minutes,
      }
    end
    return {
      action = "stuck",
      reason = "row-budget-bounds-receiver",
      receiver_bound_minutes = contract.receiver_bound_minutes,
      external_wait_bound_minutes = contract.external_wait_bound_minutes,
    }
  end
  return { action = "stuck", reason = "unsupported-contract" }
end
function M.restart_row_liveness_deferred(row, state, facts, now_seconds)
  return M.restart_row_receiver_liveness(row, state, facts, now_seconds).action == "defer"
end

function M.restart_row_observable_on(row, surface)
  return type(row) == "table"
    and row.terminal == false
    and type(row.observe_surfaces) == "table"
    and row.observe_surfaces[tostring(surface or "")] == true
end

function M.restart_observe_replay_due(row, surface, state, facts, now_seconds)
  if not M.restart_row_observable_on(row, surface) then
    return false
  end
  if surface == "issue" and row.from_state == "thinking" then
    return true
  end
  if surface == "liveness_scan" then
    return not M.restart_row_liveness_deferred(row, state, facts, now_seconds)
  end
  return false
end

function M.restart_observe_timeout_due(row, surface, state, facts, now_seconds)
  if type(row) ~= "table" or row.terminal == true then
    return false
  end
  if M.restart_row_liveness_deferred(row, state, facts, now_seconds) then
    return false
  end
  local due = M.liveness_timeout_due_with_facts(row, state, facts, now_seconds) == true
  if not due then
    local scan = surface == "liveness_scan" or surface == "issue_liveness_scan"
    return scan and M.liveness_timeout_decision_with_facts(row, state, facts, now_seconds).action == "redrive"
  end
  if type(row.timeout_surfaces) == "table" and row.timeout_surfaces[tostring(surface or "")] == true then
    return true
  end
  return M.liveness_timeout_decision_with_facts(row, state, facts, now_seconds).action == "escalate"
end

end

return S
