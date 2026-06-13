local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_intake_candidate" },
  produces = {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
    "github-proxy.github_issue_label_request",
  },
  stall_window = "2m",
}

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

local function has_devloop_state_label(labels)
  for _, label in ipairs(labels or {}) do
    if core._state_labels[tostring(label)] then
      return true
    end
  end
  return false
end

local function judgment_worktree(role, identity)
  local runtime = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
  if runtime.exit_code ~= 0 then
    error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime.stderr))
  end
  local worktree = core.judgment_worktree_path(runtime.stdout, role, identity)
  local mkdir = exec_sync({ cmd = core.mkdir_p_cmd(worktree), timeout = 30 })
  if mkdir.exit_code ~= 0 then
    error("github-devloop: judgment scratch directory setup failed: " .. tostring(mkdir.stderr))
  end
  return worktree
end

function pipeline(event)
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

  with_lock(core.observe_lock_key(repo, issue_number), function()
    core.assert_trusted_bot_configured()

    local view = core.gh_exec({ cmd = core.gh_issue_view_intake_judge_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue intake judge view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_intake_judge(view.stdout)
    core.log_forged_markers("intake_judge", candidate.proposal_id, current.comments)
    if current.state ~= "OPEN" then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-closed", "issue is not open")
      return
    end
    if not core.claim_issue_for_management("intake_judge", repo, issue_number, current, candidate.proposal_id) then
      return
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
      return
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
      return
    end
    if has_pending_reintake then
      local expected = core.build_devloop_intake_candidate_payload(repo, issue_number, reintake_command.created_at)
      if tostring(candidate.dedup_key or "") ~= tostring(expected.dedup_key or "") then
        core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-stale-reintake-candidate", "operator reintake candidate must be keyed by command timestamp")
        return
      end
    end
    if core.is_opted_in(current.labels) then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline|escalate-to-class", "skip-enabled", "fkst-dev:enabled is already present")
      return
    end
    if core.has_intake_decision_marker(current.comments, candidate.proposal_id) and not has_pending_reintake then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|track|decline", "skip-idempotent(intake marker already visible)", "trusted intake decision marker exists")
      return
    end

    core.log_codex_start("intake_judge", candidate.proposal_id, "intake")
    local content_fetch = core.context_fetch_from_bundle({
      dept = "intake_judge",
      repo = repo,
      issue_number = issue_number,
      proposal_id = candidate.proposal_id,
      version = candidate.dedup_key,
      tick = event.ts,
    })
    local result = spawn_codex_sync(core.judgment_codex_opts(
      core.build_intake_prompt(candidate.proposal_id, current, content_fetch),
      judgment_worktree("intake", candidate.dedup_key)
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

    candidate.service_class = parsed.service_class
    local comment_request = core.build_intake_decision_comment_request(repo, issue_number, candidate, parsed.action, parsed.reason, parsed.service_class)
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
    comment_request = core.build_intake_decision_comment_request(repo, issue_number, candidate, parsed.action, parsed.reason, parsed.service_class)
    table.insert(raised, "github-proxy.github_issue_label_request")
    local class_add, class_remove = core.intake_service_class_label_changes(parsed.service_class)
    local apply_add = { class_add[1] }
    local apply_remove = class_remove
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
      local label_request = core.build_intake_enabled_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    elseif tracks_umbrella(parsed.action) then
      local label_request = core.build_intake_tracking_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    else
      local label_request = core.build_intake_service_class_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end

pipeline = core.wrap_pipeline_failure("intake_judge", pipeline)

return M
