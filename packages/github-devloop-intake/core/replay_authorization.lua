local base_ids = require("devloop.base_ids")
local devloop_base = require("devloop.base")
local m_claims = require("devloop.claims")

local S = {}

local target_queue = "github-devloop-intake.devloop_intake_candidate"
local target_dept = "github-devloop-intake-default.intake_judge"

local function source_ref_value(source_ref)
  if type(source_ref) ~= "table" then
    return nil
  end
  return source_ref.ref or source_ref.reference
end

local function source_ref_equal(left, right)
  return type(left) == "table"
    and type(right) == "table"
    and left.kind == right.kind
    and source_ref_value(left) == source_ref_value(right)
end

local function has_truncated_delivery_facts(snapshot)
  local truncated = snapshot and snapshot.truncated
  if type(truncated) ~= "table" then
    return true
  end
  return truncated.deliveries ~= false or truncated.dead_letters ~= false
end

local function validate_observe_snapshot(snapshot)
  if type(snapshot) ~= "table" then
    return nil, "observe-unavailable"
  end
  if type(snapshot.deliveries) ~= "table" or type(snapshot.dead_letters) ~= "table" then
    return nil, "observe-missing-delivery-facts"
  end
  if has_truncated_delivery_facts(snapshot) then
    return nil, "observe-truncated"
  end
  return snapshot, nil
end

local function read_observe_snapshot()
  if type(fkst) ~= "table" or type(fkst.observe) ~= "function" then
    return nil, "observe-unavailable"
  end
  local ok, snapshot = pcall(function()
    return fkst.observe({ limit = 10000 })
  end)
  if not ok then
    return nil, "observe-unavailable:" .. tostring(snapshot)
  end
  return validate_observe_snapshot(snapshot)
end

local function matches_lineage(row, source_ref)
  return type(row) == "table"
    and row.queue == target_queue
    and row.dept == target_dept
    and source_ref_equal(row.source, source_ref)
end

local function matching_live_delivery(snapshot, source_ref)
  for _, row in ipairs(snapshot.deliveries or {}) do
    if matches_lineage(row, source_ref) then
      local status = tostring(row.status or "")
      if status == "pending" or status == "in-flight" or status == "retrying" then
        return row
      end
      return row
    end
  end
  return nil
end

local function is_terminal_tombstone(row, source_ref)
  return matches_lineage(row, source_ref)
    and type(row.delivery_id) == "string"
    and row.delivery_id ~= ""
    and tonumber(row.attempts) ~= nil
    and tonumber(row.attempts) >= 1
    and row.permanent == true
    and row.replayable == false
end

local function latest_terminal_tombstone(snapshot, source_ref)
  local selected = nil
  for _, row in ipairs(snapshot.dead_letters or {}) do
    if is_terminal_tombstone(row, source_ref) then
      if selected == nil or tonumber(row.dead_at_ms or 0) >= tonumber(selected.dead_at_ms or 0) then
        selected = row
      end
    end
  end
  return selected
end

function S.terminal_precondition(source_ref)
  local normalized = base_ids.normalize_source_ref(source_ref)
  local snapshot, observe_reason = read_observe_snapshot()
  if snapshot == nil then
    return nil, observe_reason, nil
  end
  if matching_live_delivery(snapshot, normalized) ~= nil then
    return nil, "live-delivery-present", snapshot
  end
  local terminal = latest_terminal_tombstone(snapshot, normalized)
  if terminal == nil then
    return nil, "terminal-dlq-absent", snapshot
  end
  return terminal, nil, snapshot
end

function S.successor_key(proposal_id, terminal)
  return base_ids.dedup_key({
    "intake-replay",
    tostring(proposal_id),
    tostring(terminal.delivery_id),
    tostring(terminal.attempts),
  })
end

function S.once_key(successor_key)
  return "github-devloop-intake/intake-replay/" .. tostring(successor_key)
end

function S.authorize(current, proposal_id, source_ref, opts)
  local options = opts or {}
  if type(current) ~= "table" or current.state ~= "OPEN" then
    return nil, "not-open"
  end
  if options.has_trusted_progress == true then
    return nil, "trusted-progress-visible"
  end
  if m_claims.claim_mode_active() ~= "assignee" then
    return nil, "claim-mode-not-assignee"
  end
  local owner = m_claims.claim_owner()
  if m_claims.issue_claim_state(current.assignees, owner, current.labels) ~= "self" then
    return nil, "not-self-only-assignee"
  end
  local repo, issue_number = devloop_base.parse_issue_source_ref(source_ref)
  if repo == nil or issue_number == nil then
    return nil, "source-ref-unmatchable"
  end

  local normalized = base_ids.normalize_source_ref(source_ref)
  local snapshot, observe_reason
  if options.observe_snapshot ~= nil then
    snapshot, observe_reason = validate_observe_snapshot(options.observe_snapshot)
  else
    snapshot, observe_reason = read_observe_snapshot()
  end
  if snapshot == nil then
    return nil, observe_reason
  end
  if matching_live_delivery(snapshot, normalized) ~= nil then
    return nil, "live-delivery-present"
  end
  local terminal = options.terminal
  if terminal ~= nil and not is_terminal_tombstone(terminal, normalized) then
    return nil, "terminal-dlq-absent"
  end
  terminal = terminal or latest_terminal_tombstone(snapshot, normalized)
  if terminal == nil then
    return nil, "terminal-dlq-absent"
  end

  local successor_key = S.successor_key(proposal_id, terminal)
  return {
    repo = repo,
    issue_number = issue_number,
    terminal = terminal,
    successor_key = successor_key,
    once_key = S.once_key(successor_key),
  }, nil
end

function S.install(M)
  M.intake_replay_authorize = function(...) return S.authorize(...) end
  M.intake_replay_terminal_precondition = function(...) return S.terminal_precondition(...) end
  M.intake_replay_successor_key = S.successor_key
  M.intake_replay_once_key = S.once_key
end

return S
