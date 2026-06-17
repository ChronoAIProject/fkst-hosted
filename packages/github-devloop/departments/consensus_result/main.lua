local core = require("core")
local ports_seam = require("std.ports")

local spec = {
  consumes = { "consensus.consensus_reached" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_ready",
  },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

local function result_version(reached)
  return tostring(reached.effect_version or reached.dedup_key)
end

local function with_effect_version(reached, version)
  local copy = {}
  for key, value in pairs(reached) do
    copy[key] = value
  end
  copy.dedup_key = version
  return copy
end

local function dependency_hold_effects_complete(current, reached, version)
  if type(current) ~= "table" or type(reached) ~= "table" then
    return false
  end
  return core.has_state_marker(current.comments, reached.proposal_id, "dependency_wait", version)
    and core.dependency_hold_fact(current.comments, reached.proposal_id) ~= nil
    and core.state_label_hint_matches(current.labels, "dependency_wait")
    and core.has_label(current.labels, core._blocked_on_dependency_label)
end

local function raise_result_effects(repo, issue_number, reached, current, state, gate, reason, version, to_state)
  version = version or result_version(reached)
  to_state = to_state or (gate and gate.ok and "ready" or "dependency_wait")
  local comment_request = core.build_result_comment_request(repo, issue_number, reached, to_state)
  local label_request = core.build_result_label_request(repo, issue_number, reached)
  local dependency_comment_request = nil
  local dependency_label_request = nil
  local dependency_release_comment_request = nil
  if not gate.ok then
    local marker = gate.kind == "cycle"
      and core.dependency_cycle_marker(reached.proposal_id, version)
      or (gate.kind == "unresolvable"
        and core.dependency_unresolvable_marker(reached.proposal_id, version, gate.unmet, gate.kind, gate.reason)
        or core.dependency_wait_marker(reached.proposal_id, version, gate.unmet, gate.kind, gate.reason))
    dependency_comment_request = core.build_dependency_hold_comment_request(
      repo,
      issue_number,
      reached.proposal_id,
      version,
      gate,
      marker,
      reached.source_ref
    )
    dependency_label_request = core.build_label_request(
      repo,
      issue_number,
      { core._blocked_on_dependency_label },
      {},
      core._dedup_key({ "dependency", "label", "hold", tostring(reached.proposal_id), version, tostring(gate.kind) }),
      reached.source_ref
    )
  elseif core.dependency_gate_has_notes(gate) then
    dependency_release_comment_request = core.build_dependency_release_comment_request(
      repo,
      issue_number,
      reached.proposal_id,
      tostring(reached.dedup_key),
      gate,
      reached.source_ref
    )
  end
  table.insert(label_request.remove_labels, core._blocked_on_dependency_label)

  local raised = {}
  if not core.has_result_marker(current.comments, reached.proposal_id, reached.decision, reached.dedup_key) then
    table.insert(raised, "github-proxy.github_issue_comment_request")
  end
  if not core.state_label_hint_matches(current.labels, "ready") then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  if gate.ok then
    if dependency_release_comment_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_comment_request")
    end
    table.insert(raised, "devloop_ready")
  else
    if dependency_comment_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_comment_request")
    end
    if dependency_label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
  end
  core.log_apply("consensus_result", reached.proposal_id, to_state, version, { add = { "fkst-dev:ready" }, remove = { "fkst-dev:thinking" } }, raised)

  if not core.has_result_marker(current.comments, reached.proposal_id, reached.decision, reached.dedup_key) then
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end
  if not core.state_label_hint_matches(current.labels, "ready") then
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  if not gate.ok then
    core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", "dependency_wait", "hold-dependency", gate.reason)
    if dependency_comment_request ~= nil then
      core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", dependency_comment_request)
    end
    if dependency_label_request ~= nil then
      core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_label_request", dependency_label_request)
    end
    return
  end
  if dependency_release_comment_request ~= nil then
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", dependency_release_comment_request)
  end
  core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", "ready", reason, "result effects complete or recoverable")
  local versioned_reached = with_effect_version(reached, version)
  core.log_raise("consensus_result", reached.proposal_id, "devloop_ready", core.build_devloop_ready_payload(versioned_reached))
end

local function make_department(ports)
  local function result_pipeline(event)
    local reached = event.payload or {}
    if type(reached) == "table" and reached.schema == "consensus.consensus_reached.v1"
      and reached.decision == "reject" then
      core.log_entry("consensus_result", event, tostring(reached.proposal_id or "unknown"), reached.dedup_key)
      core.log_cas_decision("consensus_result", tostring(reached.proposal_id or "unknown"), { state = nil, version = nil }, "thinking", "ready", "skip-unsupported(decision)", "issue consensus does not support reject")
      return
    end
    if not core.is_supported_result(reached) then
      core.log_entry("consensus_result", event, "unknown", core.payload_field(reached, "dedup_key"))
      core.log_cas_decision("consensus_result", "unknown", { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "unsupported event payload")
      return
    end

    core.log_entry("consensus_result", event, reached.proposal_id, reached.dedup_key)
    local version = result_version(reached)
    local repo, issue_number = core.parse_proposal_id(reached.proposal_id)
    if repo == nil then
      core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
      return
    end

    local lock_key = core.result_lock_key(reached.proposal_id)
    if lock_key == nil then
      core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "no transition lock key")
      return
    end

    with_lock(lock_key, function()
      core.assert_trusted_bot_configured()

      local current = ports.github.read_issue({
        kind = "external",
        ref = repo .. "#issue/" .. tostring(issue_number),
      }, {
        consumer = "consensus_result",
        force_fresh = true,
      })
      core.log_forged_markers("consensus_result", reached.proposal_id, current.comments)
      local state = core.current_state(current.comments, reached.proposal_id)
      local gate = core.dependency_gate(repo, issue_number, {
        proposal_id = reached.proposal_id,
        version = version,
        comments = current.comments,
      })
      local to_state = gate.ok and "ready" or "dependency_wait"
      local transition = core.versioned_transition_status(state, { "thinking" }, to_state, version)
      if transition == "idempotent" or transition == "stale" then
        if transition == "idempotent" and tostring(state.version or "") == tostring(version) then
          local complete = gate.ok
            and core.result_effects_complete(current, reached)
            or dependency_hold_effects_complete(current, reached, version)
          if complete then
            core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, "skip-idempotent(result effects complete)", "all declared result effects are derivable")
            return
          end
          raise_result_effects(
            repo,
            issue_number,
            reached,
            current,
            state,
            gate,
            "applied(result effects incomplete)",
            version,
            to_state
          )
          return
        end
        core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, version), "consensus result cannot advance current marker")
        return
      end
      if transition == "pending" then
        core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, version), "thinking state marker not yet visible")
        error("github-devloop: thinking state marker not yet visible for consensus result; retrying")
      end
      core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, version), "consensus decision=" .. tostring(reached.decision))

      raise_result_effects(repo, issue_number, reached, current, state, gate, core.cas_outcome(state, transition, version), version, to_state)
    end)
  end

  pipeline = core.wrap_pipeline_failure("consensus_result", result_pipeline)
  _G.pipeline = pipeline
  return { spec = spec, pipeline = pipeline }
end

local M = ports_seam.install(make_department)

return M
