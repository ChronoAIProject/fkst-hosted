local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_intake_candidate" },
  produces = {
    "github-proxy.github_issue_comment_request",
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

function pipeline(event)
  local candidate = event.payload or {}
  if not core.is_supported_intake_candidate(candidate) then
    core.log_entry("intake_judge", event, "unknown", candidate.dedup_key)
    core.log_cas_decision("intake_judge", "unknown", { state = nil, version = nil }, "candidate", "enable|decline", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("intake_judge", event, candidate.proposal_id, candidate.dedup_key)
  local repo, issue_number = core.parse_issue_source_ref(candidate.source_ref)
  if repo == nil then
    core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|decline", "skip-foreign(source_ref)", "invalid source_ref")
    return
  end

  with_lock(core.observe_lock_key(repo, issue_number), function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_intake_judge_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue intake judge view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_intake_judge(view.stdout)
    core.log_forged_markers("intake_judge", candidate.proposal_id, current.comments)
    if current.state ~= "OPEN" then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|decline", "skip-closed", "issue is not open")
      return
    end
    if core.is_opted_in(current.labels) then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|decline", "skip-enabled", "fkst-dev:enabled is already present")
      return
    end
    if core.has_intake_decision_marker(current.comments, candidate.proposal_id) then
      core.log_cas_decision("intake_judge", candidate.proposal_id, { state = nil, version = nil }, "candidate", "enable|decline", "skip-idempotent(intake marker already visible)", "trusted intake decision marker exists")
      return
    end

    core.log_codex_start("intake_judge", candidate.proposal_id, "intake")
    local result = spawn_codex_sync({
      prompt = core.build_intake_prompt(candidate.proposal_id, current),
    })
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, nil, stderr)
      error("github-devloop: intake codex failed: " .. tostring(stderr))
    end

    local parsed = core.parse_intake_action(result.stdout)
    if parsed == nil then
      parsed = decline_result()
      core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, "action=decline reason=parse-failed", nil)
    else
      core.log_codex_result("intake_judge", candidate.proposal_id, "intake", result, "action=" .. tostring(parsed.action) .. " reason=" .. tostring(parsed.reason), nil)
    end

    local comment_request = core.build_intake_decision_comment_request(repo, issue_number, candidate, parsed.action, parsed.reason)
    core.log_apply("intake_judge", candidate.proposal_id, parsed.action, candidate.dedup_key, {
      add = parsed.action == "enable" and { core._enabled_label } or {},
      remove = {},
    }, parsed.action == "enable" and {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    } or {
      "github-proxy.github_issue_comment_request",
    })
    core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    if parsed.action == "enable" then
      local label_request = core.build_intake_enabled_label_request(repo, issue_number, candidate)
      core.log_raise("intake_judge", candidate.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end

return M
