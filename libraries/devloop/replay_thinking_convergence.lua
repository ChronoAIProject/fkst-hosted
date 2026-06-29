local S = {}
local convergence_shared = require("devloop.convergence.shared")
local transition_version = require("contract.transition_version")

function S.install(M)
  local function visible_true_stall(issue, state, facts)
    local current = facts and facts.current or issue
    local proposal_id = facts and facts.proposal_id
    local source_ref = (facts and facts.source_ref) or (issue and issue.source_ref)
    if current == nil or type(state) ~= "table" or state.state ~= "thinking" or proposal_id == nil then
      return nil
    end
    local base_version = transition_version.strip_suffixes(state.version)
    local sr_digest = convergence_shared.source_ref_digest(source_ref)
    local converge_facts = M.converge_round_facts(current.comments, proposal_id, base_version, sr_digest)
    local round = M.max_converge_round(converge_facts)
    if not M.is_true_stall(converge_facts, round) then
      return nil
    end
    return {
      proposal_id = proposal_id,
      base_version = base_version,
      round = round,
      dedup_key = "reconcile:" .. tostring(base_version) .. "/loop/" .. tostring(round),
      source_ref = M.normalize_source_ref(source_ref),
    }
  end

  function M.replay_thinking_true_stall_blocked(dept, issue, state, facts, log_skip, raise_effects)
    local proposal_id = facts and facts.proposal_id
    local current = facts and facts.current or issue
    local reconcile = visible_true_stall(issue, state, facts)
    if reconcile == nil then
      return nil
    end
    if M.has_reconcile_marker(current.comments, proposal_id, reconcile.base_version, reconcile.round) then
      return log_skip(dept, proposal_id, state, "thinking", "blocked", "skip-idempotent(reconcile marker already visible)", "reconcile result marker for visible true-stall round is already visible")
    end
    local version = M.reconcile_terminal_state_version(state.version, reconcile.round)
    local transition = M.versioned_transition_status(state, { "thinking" }, "blocked", version)
    if transition == "idempotent" or transition == "stale" then
      return log_skip(dept, proposal_id, state, "thinking", "blocked", M.cas_outcome(state, transition, version), "current marker cannot be reconciled from thinking")
    end
    local action = "drop"
    local reason = "no-actionable-framing-after-" .. tostring(reconcile.round) .. "-rounds"
    local comment_request = M.build_reconcile_comment_request(issue.repo, issue.number, reconcile, action, reason, version)
    local label_request = M.build_reconcile_label_request(issue.repo, issue.number, reconcile)
    local add_labels, remove_labels = M.state_label_changes("blocked")
    M.log_cas_decision(dept, proposal_id, state, "thinking", "blocked", M.cas_outcome(state, transition, version), reason)
    return raise_effects(dept, proposal_id, "blocked", version, { add = add_labels, remove = remove_labels }, {
      { queue = "github-proxy.github_issue_comment_request", payload = comment_request },
      { queue = "github-proxy.github_issue_label_request", payload = label_request },
    })
  end
end

return S
