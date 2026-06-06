local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_ready" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  stall_window = "10m",
}

local function raise_stuck(repo, issue_number, ready, reason, detail)
  local comment_request = core.build_impl_failure_comment_request(repo, issue_number, ready, reason, detail)
  local label_request = core.build_impl_failed_label_request(repo, issue_number, ready, reason)
  local add_labels, remove_labels = core.state_label_changes("impl-failed")
  core.log_apply("implement", ready.proposal_id, "impl-failed", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

function pipeline(event)
  local ready = event.payload or {}
  if not core.is_supported_ready(ready) then
    core.log_entry("implement", event, "unknown", ready.dedup_key)
    core.log_cas_decision("implement", "unknown", { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("implement", event, ready.proposal_id, ready.dedup_key)
  local repo, issue_number = core.parse_proposal_id(ready.proposal_id)
  if repo == nil then
    core.log_cas_decision("implement", ready.proposal_id, { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.implement_lock_key(ready.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("implement", ready.proposal_id, { state = nil, version = nil }, "ready", "implementing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_implement_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue implement view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_implement(view.stdout)
    core.log_forged_markers("implement", ready.proposal_id, current.comments)
    local state = core.current_state(current.comments, ready.proposal_id)
    if state.state == "implementing" or state.state == "impl-failed" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", "skip-idempotent(already at to_state)", "implementation fact marker already visible")
      return
    end
    local transition = core.versioned_transition_status(state, { "ready" }, "implementing", ready.dedup_key)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready event cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready state marker not yet visible")
      error("github-devloop: ready state marker not yet visible for implement; retrying")
    end
    core.log_cas_decision("implement", ready.proposal_id, state, "ready", "implementing", core.cas_outcome(state, transition, ready.dedup_key), "ready marker visible; attempting implementation")

    local issue_slug = core.safe_issue_slug(repo, issue_number)
    core.log_line("info", "implement", ready.proposal_id, "IMPLEMENT", {
      "issue_slug=" .. tostring(issue_slug),
      "reason=implementation fact marker absent for this version",
    })
    local worktree = setup_worktree("devloop-" .. issue_slug)
    core.log_codex_start("implement", ready.proposal_id, "implement")
    local result = spawn_codex_sync({
      prompt = core.build_implement_prompt(ready.proposal_id, current),
      worktree = worktree,
      stall_window = M.spec.stall_window,
    })

    if type(result) ~= "table" or result.exit_code ~= 0 then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, stderr)
      raise_stuck(repo, issue_number, ready, "codex-failed", stderr)
      return
    end
    core.log_codex_result("implement", ready.proposal_id, "implement", result, "result=completed", nil)

    local status = exec_sync({ cmd = core.git_status_cmd(worktree), timeout = 30 })
    if status.exit_code ~= 0 then
      error("github-devloop: git status failed: " .. tostring(status.stderr))
    end

    if tostring(status.stdout or "") == "" then
      local detail = tostring(result.stdout or "")
      if detail == "" then
        detail = tostring(result.stderr or "")
      end
      core.log_codex_result("implement", ready.proposal_id, "implement", result, nil, "no-changes")
      raise_stuck(repo, issue_number, ready, "no-changes", detail)
      return
    end

    local comment_request = core.build_implementing_comment_request(repo, issue_number, ready, worktree)
    local label_request = core.build_implementing_label_request(repo, issue_number, ready)
    local add_labels, remove_labels = core.state_label_changes("implementing")
    core.log_apply("implement", ready.proposal_id, "implementing", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    })
    core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("implement", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end)
end

return M
