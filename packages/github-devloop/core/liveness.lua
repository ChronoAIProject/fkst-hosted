local S = {}
local registry = require("std.registry")
local source_refs = require("std.source_ref")

function S.install(M)
local max_timeout_attempts = 3

local function has_required_table(row, field)
  return type(row[field]) == "table" and next(row[field]) ~= nil
end

local function valid_budget(row)
  return type(row.budget) == "table"
    and tonumber(row.budget.minutes) ~= nil
    and tonumber(row.budget.minutes) > 0
    and type(row.budget.receiver_max_work_justification) == "string" and row.budget.receiver_max_work_justification ~= ""
end

local function reachable_lifecycle_states(M)
  local seen = {}
  local function add(state)
    if type(state) == "string" and state ~= "" and state ~= "unmanaged" then
      seen[state] = true
    end
  end
  for state, _ in pairs(M._label_by_state or {}) do
    add(state)
  end
  for _, state in ipairs(M._state_order or {}) do
    add(state)
  end
  for state, _ in pairs(M._state_stage_rank or {}) do
    add(state)
  end
  for state, next_states in pairs(M._state_graph or {}) do
    add(state)
    for _, next_state in ipairs(next_states or {}) do
      add(next_state)
    end
  end
  return seen
end

local function validate_restart_totality(M, rows, errors)
  local reachable = reachable_lifecycle_states(M)
  local seen = {}
  for _, row in ipairs(rows or {}) do
    local state = row and row.from_state
    if type(state) ~= "string" or state == "" then
      table.insert(errors, "restart_transition_table: row missing from_state")
    elseif reachable[state] ~= true then
      table.insert(errors, tostring(state) .. ": restart row is not a reachable lifecycle state")
    elseif seen[state] == true then
      table.insert(errors, tostring(state) .. ": duplicate restart_transition_table row")
    else
      seen[state] = true
    end
  end
  for state, _ in pairs(reachable) do
    if seen[state] ~= true then
      table.insert(errors, tostring(state) .. ": reachable lifecycle state is missing a restart_transition_table row")
    end
  end
end

local function valid_timeout(row)
  if type(row.on_timeout) ~= "table" then
    return false
  end
  if row.on_timeout.action ~= "redrive" or row.on_timeout.queue ~= row.driving_queue then
    return false
  end
  if tonumber(row.on_timeout.escalate_after_attempts) == nil
    or tonumber(row.on_timeout.escalate_after_attempts) <= 0 then
    return false
  end
  local terminal = row.on_timeout.on_escalate
  return type(terminal) == "table"
    and terminal.action == "force-terminate" and terminal.terminal_state == "blocked"
    and type(terminal.reason) == "string" and terminal.reason ~= ""
end

local liveness_contract_margin_minutes = 30

local liveness_resolver_families = {
  ["converge-round"] = {
    ["converge-round"] = true,
  },
  ["dependency-hold"] = {
    ["dependency-wait"] = true,
    ["dependency-cycle"] = true,
    ["dependency-unresolvable"] = true,
  },
  ["implement-attempt"] = {
    ["implement-attempt"] = true,
  },
  ["merge-gate-wait"] = {
    ["merge-gate-wait"] = true,
  },
  ["review-converge-round"] = {
    ["review-converge-round"] = true,
  },
}

local liveness_signal_producers = registry.load_indexed_map("core.restart.liveness_signal_producers.index", "family", nil, nil, "github-devloop")

function M.liveness_signal_producer_contract(family)
  return liveness_signal_producers[tostring(family or "")]
end

local function strip_liveness_timeout_suffixes(version)
  local text = tostring(version or "")
  local previous = nil
  while previous ~= text do
    previous = text
    text = text
      :gsub("/timeout%-reconcile/[%w%-]+/%d+$", "")
      :gsub("%-timeout%-reconcile%-[%w%-]+%-%d+$", "")
      :gsub("/timeout/[%w%-]+/%d+$", "")
      :gsub("%-timeout%-[%w%-]+%-%d+$", "")
  end
  return text
end

function M.liveness_heartbeat_version(version, contract)
  local heartbeat_version = strip_liveness_timeout_suffixes(version)
  if contract and contract.version_form == "safe_version_segment" then
    return M.safe_version_segment(heartbeat_version)
  end
  return M.strip_transition_version_suffixes(heartbeat_version)
end

local function numeric_minutes(value)
  local minutes = tonumber(value)
  if minutes == nil or minutes <= 0 then
    return nil
  end
  return minutes
end

local function non_negative_minutes(value)
  local minutes = tonumber(value)
  if minutes == nil or minutes < 0 then
    return nil
  end
  return minutes
end

local function liveness_bound_minutes(contract)
  local receiver = non_negative_minutes(contract and contract.receiver_bound_minutes)
  local external = non_negative_minutes(contract and contract.external_wait_bound_minutes)
  if receiver == nil then
    return nil
  end
  if external ~= nil and external > receiver then
    return external
  end
  return receiver
end

local function source_contains(path, needle)
  if type(path) ~= "string" or path == "" or type(needle) ~= "string" or needle == "" then
    return false
  end
  local ok, text = pcall(file.read, "packages/github-devloop/" .. path)
  return ok and tostring(text or ""):find(needle, 1, true) ~= nil
end

local function validate_liveness_signal_producer(M, state, signal, family, resolver, errors)
  local producer_key = signal.producer
  if type(producer_key) ~= "string" or producer_key == "" then
    table.insert(errors, state .. ": live-defer signal must declare a producer binding")
    return
  end
  local binding = liveness_signal_producers[producer_key]
  if binding == nil then
    table.insert(errors, state .. ": live-defer signal producer binding does not exist: " .. tostring(producer_key))
    return
  end
  if producer_key ~= family then
    table.insert(errors, state .. ": live-defer producer binding family mismatch: " .. tostring(producer_key))
  end
  if binding.resolver ~= resolver then
    table.insert(errors, state .. ": live-defer producer binding resolver mismatch: " .. tostring(producer_key))
  end
  if binding.surface ~= signal.surface then
    table.insert(errors, state .. ": live-defer producer binding surface mismatch: " .. tostring(producer_key))
  end
  if binding.version_form ~= signal.version_form then
    table.insert(errors, state .. ": live-defer producer binding version_form mismatch: " .. tostring(producer_key))
  end
  if liveness_resolver_families[resolver] == nil or liveness_resolver_families[resolver][family] ~= true then
    table.insert(errors, state .. ": live-defer resolver does not read marker family: " .. tostring(resolver) .. "/" .. tostring(family))
  end
  if M.restart_durable_marker_fields()[family] == nil then
    return
  end
  local marker_source = binding.marker_source or "core/requests.lua"
  local request_source = binding.request_source or "core/requests.lua"
  if not (source_contains(binding.producer, binding.marker_builder) or source_contains(marker_source, binding.marker_builder))
    or not source_contains(request_source, binding.request_builder)
    or not source_contains(binding.producer, binding.request_builder) then
    table.insert(errors, state .. ": live-defer producer binding is not reachable from declared producer: " .. tostring(producer_key))
  end
  if not source_contains(binding.producer, binding.queue) then
    table.insert(errors, state .. ": live-defer producer binding does not emit declared queue: " .. tostring(producer_key))
  end
end

local function validate_liveness_signal_shape(M, state, signal, label, errors)
  local family = signal.family
  local resolver = signal.resolver or family
  if type(family) ~= "string" or family == "" then
    table.insert(errors, state .. ": " .. label .. " must declare an existing marker family")
  elseif M.restart_durable_marker_fields()[family] == nil then
    table.insert(errors, state .. ": " .. label .. " marker family does not exist: " .. tostring(family))
  end
  if liveness_resolver_families[resolver] == nil then
    table.insert(errors, state .. ": " .. label .. " has no resolver: " .. tostring(resolver))
  end
  if numeric_minutes(signal.max_age_minutes) == nil then
    table.insert(errors, state .. ": " .. label .. " must declare finite max_age_minutes")
  end
  if signal.surface ~= "issue-comment-stream" and signal.surface ~= "pr-comment-stream" then
    table.insert(errors, state .. ": " .. label .. " must declare surface")
  end
  if signal.version_form ~= "raw" and signal.version_form ~= "safe_version_segment" then
    table.insert(errors, state .. ": " .. label .. " must declare version_form")
  end
  validate_liveness_signal_producer(M, state, signal, family, resolver, errors)
end

local function validate_liveness_contract(M, row, errors)
  local state = tostring(row.from_state or "?")
  local contract = row.liveness_contract
  if type(contract) ~= "table" then
    table.insert(errors, state .. ": non-terminal row must declare exactly one liveness_contract")
    return
  end
  local mode = contract.mode
  if mode ~= "row-budget-bounds-receiver" and mode ~= "live-defer" then
    table.insert(errors, state .. ": liveness_contract must declare exactly one supported mode")
    return
  end
  if mode == "row-budget-bounds-receiver" then
    local bound = liveness_bound_minutes(contract)
    if bound == nil then
      table.insert(errors, state .. ": row-budget-bounds-receiver must declare receiver_bound_minutes")
      return
    end
    local budget_minutes = tonumber(row.budget and row.budget.minutes)
    if budget_minutes == nil or budget_minutes < bound + liveness_contract_margin_minutes then
      table.insert(errors, state .. ": budget.minutes must be at least max(declared receiver/external bounds) + margin")
    end
    if contract.progress_signal ~= nil then
      validate_liveness_signal_shape(M, state, contract.progress_signal, "row-budget progress_signal", errors)
      local signal_max_age = numeric_minutes(contract.progress_signal.max_age_minutes)
      if budget_minutes ~= nil and signal_max_age ~= nil and signal_max_age >= budget_minutes then
        table.insert(errors, state .. ": row-budget progress_signal max_age_minutes must be less than budget.minutes")
      end
    end
    return
  end

  local signal = contract.signal
  if type(signal) ~= "table" then
    table.insert(errors, state .. ": live-defer must declare a signal")
    return
  end
  validate_liveness_signal_shape(M, state, signal, "live-defer signal", errors)
end

function M.liveness_contract_errors(rows)
  local errors = {}
  local table_rows = rows or M.restart_transition_table()
  validate_restart_totality(M, table_rows, errors)
  for _, row in ipairs(table_rows) do
    if type(row.from_state) ~= "string" or row.from_state == "" then
      table.insert(errors, "row: missing from_state")
    end
    if type(row.terminal) ~= "boolean" then
      table.insert(errors, tostring(row.from_state or "?") .. ": terminal must be boolean")
    end
    if row.terminal == true then
      if row.output_obligation ~= nil then
        table.insert(errors, tostring(row.from_state or "?") .. ": terminal row must not declare output_obligation")
      end
    else
      if not has_required_table(row, "output_obligation") then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare output_obligation")
      end
      if not valid_budget(row) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare a positive budget with receiver_max_work_justification")
      end
      if not valid_timeout(row) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare redrive on_timeout for its driving queue plus force-terminate on_escalate to blocked")
      end
      if type(row.observe_surfaces) ~= "table" or next(row.observe_surfaces) == nil then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare observe_surfaces")
      else
        for surface, enabled in pairs(row.observe_surfaces) do
          if surface ~= "issue" and surface ~= "pr" and surface ~= "liveness_scan" then
            table.insert(errors, tostring(row.from_state or "?") .. ": unsupported observe surface " .. tostring(surface))
          end
          if enabled ~= true then
            table.insert(errors, tostring(row.from_state or "?") .. ": observe surface must be true: " .. tostring(surface))
          end
        end
      end
      if row.pr_recovery ~= nil then
        if type(row.pr_recovery) ~= "table" then
          table.insert(errors, tostring(row.from_state or "?") .. ": pr_recovery must be a table")
        else
          for name, recovery in pairs(row.pr_recovery) do
            if name ~= "not_mergeable" then
              table.insert(errors, tostring(row.from_state or "?") .. ": unsupported pr_recovery " .. tostring(name))
            elseif type(recovery) ~= "table"
              or recovery.to_state ~= "fixing"
              or recovery.queue ~= "devloop_fixing" then
              table.insert(errors, tostring(row.from_state or "?") .. ": not_mergeable pr_recovery must target fixing via devloop_fixing")
            end
          end
        end
      end
      if row.timeout_surfaces ~= nil then
        if type(row.timeout_surfaces) ~= "table" then
          table.insert(errors, tostring(row.from_state or "?") .. ": timeout_surfaces must be a table")
        else
          for surface, enabled in pairs(row.timeout_surfaces) do
            if surface ~= "issue" and surface ~= "issue_liveness_scan" and surface ~= "pr" and surface ~= "liveness_scan" then
              table.insert(errors, tostring(row.from_state or "?") .. ": unsupported timeout surface " .. tostring(surface))
            end
            if enabled ~= true then
              table.insert(errors, tostring(row.from_state or "?") .. ": timeout surface must be true: " .. tostring(surface))
            end
          end
        end
      end
      validate_liveness_contract(M, row, errors)
      if (type(row.to_states) ~= "table" or #row.to_states == 0)
        and (type(row.reentry_commands) ~= "table" or #row.reentry_commands == 0) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare at least one next state")
      end
    end
    for _, next_state in ipairs(row.to_states or {}) do
      if M._label_by_state[next_state] == nil then
        table.insert(errors, tostring(row.from_state or "?") .. ": unknown next state " .. tostring(next_state))
      end
    end
  end
  if #errors == 0 then
    for _, inventory_errors in ipairs({ M.restart_liveness_inventory_errors(table_rows), M.restart_responsibility_inventory_errors(table_rows) }) do
      for _, err in ipairs(inventory_errors) do table.insert(errors, err) end
    end
  end
  return errors
end

local function signal_age_from_created_at(M, created_at, now_seconds)
  local created_seconds = M.iso_timestamp_epoch_seconds(created_at)
  local current_seconds = tonumber(now_seconds)
  if created_seconds ~= nil and current_seconds ~= nil and current_seconds >= created_seconds then
    return math.floor((current_seconds - created_seconds) / 60)
  end
  return nil
end

local function marker_attr(marker, name)
  return tostring(marker or ""):match(name .. '="([^"]*)"')
end

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

local function liveness_contract_signal(contract)
  if type(contract) ~= "table" then
    return nil
  end
  if contract.mode == "live-defer" then
    return contract.signal
  end
  if contract.mode == "row-budget-bounds-receiver" then
    return contract.progress_signal
  end
  return nil
end

local function row_liveness_signal(row)
  return liveness_contract_signal(row and row.liveness_contract)
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
    local attempt = M.latest_implement_attempt_fact(comments, proposal_id, signal_version)
    if attempt ~= nil then
      local started = tonumber(attempt.started_at)
      local current_seconds = tonumber(now_seconds)
      if started ~= nil and current_seconds ~= nil and current_seconds >= started then
        return math.floor((current_seconds - started) / 60)
      end
    end
    return newest_matching_marker_age(M, comments, "implement-attempt", function(marker)
      return marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "dedup") == tostring(signal_version or "")
    end, now_seconds)
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

function M.liveness_terminal_states(rows)
  local terminals = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    if row.terminal == true then
      table.insert(terminals, row.from_state)
    end
  end
  return terminals
end

function M.issue_marker_liveness_sweep_states(rows)
  local states = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    if row.terminal == false then
      states[row.from_state] = true
    end
  end
  return states
end

function M.issue_marker_liveness_sweep_contract_errors(rows, sweep_states)
  local errors = {}
  local declared_states = sweep_states or M.issue_marker_liveness_sweep_states(rows)
  for _, row in ipairs(rows or M.restart_transition_table()) do
    if row.terminal == false and declared_states[row.from_state] ~= true then
      table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal issue-marker state is not reachable by liveness sweep")
    end
    if row.terminal == true and declared_states[row.from_state] == true then
      table.insert(errors, tostring(row.from_state or "?") .. ": terminal issue-marker state must not be re-driven by liveness sweep")
    end
  end
  return errors
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

function M.liveness_budget_minutes(state_name)
  local row = M.restart_transition_row(state_name)
  return row and row.budget and tonumber(row.budget.minutes) or nil
end

function M.liveness_state_age_minutes(state, now_seconds)
  if type(state) ~= "table" then
    return nil
  end
  if state.marker_created_at ~= nil and state.marker_created_at ~= "" then
    local created_seconds = M.iso_timestamp_epoch_seconds(state.marker_created_at)
    local current_seconds = tonumber(now_seconds)
    if created_seconds ~= nil and current_seconds ~= nil and current_seconds >= created_seconds then
      return math.floor((current_seconds - created_seconds) / 60)
    end
  end
  return M.stall_suspect_age_minutes(state.version, now_seconds)
end

function M.liveness_timeout_attempt(row, state, facts)
  local eval = facts and facts.actionable_epoch_eval
  if M.restart_row_has_registered_actionable_epoch(row) then
    return M.actionable_epoch_timeout_attempt(row, state, facts)
  end
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local comments = facts and facts.current and facts.current.comments or nil
  local from_state = row and row.from_state
  local version = state and state.version
  local durable_round = M.timeout_attempt_round(comments, proposal_id, version, from_state)
  local version_round = M.version_timeout_round(version, from_state)
  return math.max(durable_round or 0, version_round or 0)
end

function M.next_liveness_timeout_version(row, state, facts)
  local from = tostring(row.from_state)
  local escaped = from:gsub("%-", "%%-")
  local base = tostring(state and state.version or "")
  -- Replace, not stack, the trailing timeout segment for this state so the version
  -- stays bounded as attempts climb: V -> V/timeout/<state>/1 -> V/timeout/<state>/2.
  -- The attempt count itself is read from the full (pre-strip) version, so it keeps
  -- advancing across sweeps even though the suffix never accumulates.
  local previous = nil
  while previous ~= base do
    previous = base
    base = base:gsub("/timeout/" .. escaped .. "/%d+$", "")
  end
  return base .. "/timeout/" .. from .. "/" .. tostring(M.liveness_timeout_attempt(row, state, facts) + 1)
end

function M.liveness_timeout_due(row, state, now_seconds)
  if row == nil or row.terminal == true then
    return false, nil
  end
  local budget = row.budget and tonumber(row.budget.minutes) or nil
  local age = M.liveness_state_age_minutes(state, now_seconds)
  if budget == nil or age == nil or age < budget then
    return false, age
  end
  return true, age
end

local function live_signal_max_age(row)
  return numeric_minutes(row_liveness_signal(row) and row_liveness_signal(row).max_age_minutes)
end

function M.liveness_timeout_due_with_facts(row, state, facts, now_seconds)
  if row == nil or row.terminal == true then
    return false, nil
  end
  if M.restart_row_has_registered_actionable_epoch(row) then
    return M.actionable_epoch_timeout_due(row, state, facts, now_seconds)
  end
  local contract = row.liveness_contract
  if type(contract) == "table" and contract.mode == "row-budget-bounds-receiver" then
    return M.liveness_timeout_due(row, state, now_seconds)
  end
  local signal_max_age = live_signal_max_age(row)
  if signal_max_age ~= nil then
    local signal = M.restart_row_liveness_signal(row, state, facts, now_seconds)
    if signal.age_minutes ~= nil then
      if signal.age_minutes < signal_max_age then
        return false, signal.age_minutes
      end
      return true, signal.age_minutes
    end
  end
  return M.liveness_timeout_due(row, state, now_seconds)
end

local function timeout_escalation(row, state, age, facts)
  local attempt = M.liveness_timeout_attempt(row, state, facts)
  local limit = tonumber(row.on_timeout and row.on_timeout.escalate_after_attempts) or max_timeout_attempts
  local next_version = M.next_liveness_timeout_version(row, state, facts)
  if attempt >= limit then
    return {
      action = "escalate",
      attempt = attempt,
      age_minutes = age,
    }
  end
  if attempt + 1 >= limit then
    return {
      action = "escalate",
      attempt = attempt + 1,
      age_minutes = age,
    }
  end
  return {
    action = "redrive",
    attempt = attempt + 1,
    age_minutes = age,
    version = next_version,
  }
end

local function build_timeout_reconcile(row, entity, state, facts, decision)
  local source_ref = (facts and facts.source_ref) or (entity and entity.source_ref) or (state and state.source_ref)
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  if source_refs.has_bounded_source_ref(source_ref, M._max_key_len)
    and M._is_path_safe_key(proposal_id, M._max_key_len)
    and M._is_bounded_string(state and state.version, M._max_dedup_len) then
    return "devloop_timeout_reconcile", M.build_devloop_timeout_reconcile_payload(row, state, proposal_id, source_ref, decision.attempt)
  end
  return nil, nil
end

function M.build_liveness_timeout_reconcile_payload(row, entity, state, facts, decision)
  return build_timeout_reconcile(row, entity, state, facts, decision)
end

function M.liveness_timeout_decision(row, state, now_seconds)
  local due, age = M.liveness_timeout_due(row, state, now_seconds)
  if not due then
    return {
      action = "wait",
      age_minutes = age,
    }
  end
  return timeout_escalation(row, state, age)
end

function M.liveness_timeout_decision_with_facts(row, state, facts, now_seconds)
  local due, age = M.liveness_timeout_due_with_facts(row, state, facts, now_seconds)
  local limit = tonumber(row and row.on_timeout and row.on_timeout.escalate_after_attempts) or max_timeout_attempts
  local heartbeat = M.actionable_epoch_heartbeat_decision(row, state, facts, due, age, limit)
  if heartbeat ~= nil then return heartbeat end
  if not due then
    return { action = "wait", age_minutes = age }
  end
  return timeout_escalation(row, state, age, facts)
end

local function timeout_attempt_target(entity, facts)
  local source_ref = (facts and facts.source_ref) or (entity and entity.source_ref)
  local kind = "issue"
  local repo = entity and entity.repo
  local number = entity and entity.number
  local _, pr_number = M.parse_pr_source_ref(source_ref)
  if pr_number ~= nil then
    local parsed_repo = select(1, M.parse_proposal_id(facts and facts.proposal_id))
    kind = "pr"
    repo = parsed_repo or repo
    number = pr_number
  end
  if kind == "issue" then
    local parsed_repo, issue_number = M.parse_proposal_id(facts and facts.proposal_id)
    repo = parsed_repo or repo
    number = issue_number or number
  end
  if repo == nil or number == nil then
    return nil
  end
  return {
    kind = kind,
    repo = repo,
    number = number,
  }
end

local function emit_timeout_attempt_marker(dept, entity, state, row, facts, proposal_id, attempt)
  local target = timeout_attempt_target(entity, facts)
  local source_ref = (facts and facts.source_ref) or (entity and entity.source_ref) or (state and state.source_ref)
  if target ~= nil then
    local eval = facts and facts.actionable_epoch_eval
    if M.restart_row_has_registered_actionable_epoch(row)
      and type(eval) == "table"
      and eval.status == "actionable"
      and eval.generation_key ~= nil then
      M.log_raise(dept, proposal_id, target.kind == "pr" and "github-proxy.github_pr_comment_request" or "github-proxy.github_issue_comment_request", M.build_timeout_attempt_v2_comment_request(target, proposal_id, state, row, source_ref, attempt, eval.generation_key))
    else
      M.log_raise(dept, proposal_id, target.kind == "pr" and "github-proxy.github_pr_comment_request" or "github-proxy.github_issue_comment_request", M.build_timeout_attempt_comment_request(target, proposal_id, state, row, source_ref, attempt))
    end
  end
end

local function emit_decompose_exhausted_marker(dept, entity, state, facts, proposal_id, attempt)
  local target = timeout_attempt_target(entity, facts)
  local source_ref = (facts and facts.source_ref) or (entity and entity.source_ref) or (state and state.source_ref)
  if target ~= nil then
    local request = M.build_decompose_exhausted_comment_request(target, proposal_id, state, source_ref, attempt)
    M.log_apply(dept, proposal_id, nil, nil, { add = {}, remove = {} }, {
      target.kind == "pr" and "github-proxy.github_pr_comment_request" or "github-proxy.github_issue_comment_request",
    })
    M.log_raise(dept, proposal_id, target.kind == "pr" and "github-proxy.github_pr_comment_request" or "github-proxy.github_issue_comment_request", request)
    return true
  end
  return false
end

function M.maybe_timeout_redrive_from_table(dept, entity, state, table_row, facts)
  local row = table_row or M.restart_transition_row(state and state.state)
  if row == nil or row.terminal == true then
    return false
  end
  local comments = facts and facts.current and facts.current.comments or nil
  local proposal_id = facts and facts.proposal_id or state and state.proposal_id
  local matches, mismatch = M.timeout_lineage_matches_current(state, facts and facts.fresh_current_state)
  if not matches then
    M.log_cas_decision(dept, proposal_id, facts and facts.fresh_current_state or state, row.from_state, row.driving_queue, "stale_timeout_noop(" .. tostring(mismatch) .. ")", "timeout watchdog lineage no longer matches freshly derived current state")
    return true
  end
  if row.from_state == "blocked" and M.has_decompose_exhausted_marker(comments, proposal_id, state and state.version) then
    M.log_cas_decision(dept, proposal_id, state, "blocked", row.driving_queue, "skip-idempotent(decompose-exhausted)", "blocked decompose output obligation already reached terminal stop")
    return true
  end
  local receiver_liveness = M.restart_row_receiver_liveness(row, state, facts, (facts and facts.now_seconds) or now())
  if receiver_liveness.action == "defer" then
    local signal = receiver_liveness.signal or {}
    M.log_cas_decision(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-timeout-count(live-signal:" .. tostring(signal.family or "unknown") .. ")", "receiver liveness contract signal is still fresh")
    return true
  end
  local decision = M.liveness_timeout_decision_with_facts(row, state, facts, (facts and facts.now_seconds) or now())
  if decision.action == "wait" then
    return false
  end
  M.log_cas_decision(dept, proposal_id, state, row.from_state, row.driving_queue, "timeout-" .. decision.action, "state output obligation exceeded budget")
  if decision.action == "escalate" then
    if row.from_state == "blocked" then
      return emit_decompose_exhausted_marker(dept, entity, state, facts, proposal_id, decision.attempt)
    end
    local queue, payload = build_timeout_reconcile(row, entity, state, facts, decision)
    if queue ~= nil then
      M.log_apply(dept, proposal_id, nil, nil, { add = {}, remove = {} }, { queue })
      M.log_raise(dept, proposal_id, queue, payload)
      return true
    end
    return false
  end
  local replay = M.replay_from_table_classified(dept, entity, {
    state = state.state,
    version = state.version,
    proposal_id = state.proposal_id,
    stage_rank = state.stage_rank,
    marker_created_at = state.marker_created_at,
  }, row, facts)
  if replay.kind == "stuck" then
    M.log_cas_decision(dept, proposal_id, state, row.from_state, row.driving_queue, "timeout-stuck(" .. tostring(replay.outcome or "replay-declined") .. ")", "state output obligation is unmet and replay did not emit a consumable redrive")
  end
  if replay.kind == "issued" or replay.kind == "stuck" then
    emit_timeout_attempt_marker(dept, entity, state, row, facts, proposal_id, decision.attempt)
    return true
  end
  return false
end

end

return S
