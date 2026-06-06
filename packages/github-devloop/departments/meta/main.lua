local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_stuck" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_ready",
  },
  stall_window = "2m",
}

function pipeline(event)
  local stuck = event.payload or {}
  if not core.is_supported_stuck(stuck) then
    core.log_entry("meta", event, "unknown", stuck.dedup_key)
    core.log_cas_decision("meta", "unknown", { state = nil, version = nil }, "stuck", "ready|blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("meta", event, stuck.proposal_id, stuck.dedup_key)
  local repo, issue_number = core.parse_proposal_id(stuck.proposal_id)
  if repo == nil then
    core.log_cas_decision("meta", stuck.proposal_id, { state = nil, version = nil }, "stuck", "ready|blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.meta_lock_key(stuck.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("meta", stuck.proposal_id, { state = nil, version = nil }, "stuck", "ready|blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_meta_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue meta view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_meta(view.stdout)
    core.log_forged_markers("meta", stuck.proposal_id, current.comments)
    local state = core.current_state(current.comments, stuck.proposal_id)
    local transition = core.versioned_transition_status(state, { "stuck" }, "ready", stuck.dedup_key)
    if state.state == nil or transition == "pending" then
      core.log_cas_decision("meta", stuck.proposal_id, state, "stuck", "ready|blocked", "retry-pending(from-state marker not yet visible)", "stuck state marker not yet visible")
      error("github-devloop: stuck state marker not yet visible for meta; retrying")
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("meta", stuck.proposal_id, state, "stuck", "ready|blocked", core.cas_outcome(state, transition, stuck.dedup_key), "current marker is no longer stuck")
      return
    end
    if core.has_meta_marker(current.comments, stuck.proposal_id, stuck.dedup_key) then
      core.log_cas_decision("meta", stuck.proposal_id, state, "stuck", "ready|blocked", "skip-idempotent(meta marker already visible)", "meta result marker for incoming version is already visible")
      return
    end
    core.log_cas_decision("meta", stuck.proposal_id, state, "stuck", "ready|blocked", "applied", "running meta codex decision")

    core.log_codex_start("meta", stuck.proposal_id, "meta")
    local result = spawn_codex_sync({
      prompt = core.build_meta_prompt(stuck.proposal_id, current),
      stall_window = M.spec.stall_window,
    })
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("meta", stuck.proposal_id, "meta", result, nil, stderr)
      error("github-devloop: meta codex failed: " .. tostring(stderr))
    end

    local parsed = core.parse_meta_action(result.stdout)
    if parsed == nil then
      core.log_codex_result("meta", stuck.proposal_id, "meta", result, nil, "parse-failed")
      return
    end
    core.log_codex_result("meta", stuck.proposal_id, "meta", result, "action=" .. tostring(parsed.action) .. " reason=" .. tostring(parsed.reason), nil)

    local to_state = parsed.action == "implement" and "ready" or "blocked"
    local comment_request = core.build_meta_comment_request(repo, issue_number, stuck, parsed.action, parsed.reason)
    local label_request = core.build_meta_label_request(repo, issue_number, stuck, parsed.action)
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    }
    if parsed.action == "implement" then
      table.insert(raised, "devloop_ready")
    end
    core.log_apply("meta", stuck.proposal_id, to_state, stuck.dedup_key, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("meta", stuck.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("meta", stuck.proposal_id, "github-proxy.github_issue_label_request", label_request)
    if parsed.action == "implement" then
      core.log_raise("meta", stuck.proposal_id, "devloop_ready", core.build_devloop_ready_payload(stuck))
    end
  end)
end

return M
