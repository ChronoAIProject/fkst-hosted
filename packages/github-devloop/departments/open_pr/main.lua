local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "github-proxy.github_pr_open_request",
  },
  stall_window = "2m",
}

function pipeline(event)
  local issue = event.payload or {}
  if not core.is_supported_issue(issue) then
    core.log_entry("open_pr", event, "unknown", issue.dedup_key)
    core.log_cas_decision("open_pr", "unknown", { state = nil, version = nil }, "implementing", "pr-open", "skip-foreign(issue)", "unsupported event payload")
    return
  end

  local proposal_id = core.proposal_id(issue.repo, issue.number)
  core.log_entry("open_pr", event, proposal_id, issue.dedup_key)
  local lock_key = core.transition_lock_key(proposal_id)
  if lock_key == nil then
    core.log_cas_decision("open_pr", proposal_id, { state = nil, version = nil }, "implementing", "pr-open", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()

    local view = exec_sync({ cmd = core.gh_issue_view_open_pr_cmd(issue.repo, issue.number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue open-pr view failed: " .. tostring(view.stderr))
    end

    local current_issue = core.parse_issue_view_open_pr(view.stdout)
    core.log_forged_markers("open_pr", proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, proposal_id)
    if state.state == "pr-open" or state.state == "reviewing" then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-idempotent(already at to_state)", "PR state marker already visible")
      return
    end

    local transition = core.transition_status(state, { "implementing" }, "pr-open")
    if transition == "pending" then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-idempotent(not-at-implementing)", "not implementing yet; wide fanout event is not for open_pr")
      return
    end
    if transition == "stale" then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", core.cas_outcome(state, transition, state.version), "implementing state cannot advance to PR")
      return
    end

    local fact = core.implementing_fact(current_issue.comments, proposal_id, state.version)
    if fact == nil then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "retry-pending(implementing fact marker not visible)", "branch fact marker missing")
      error("github-devloop: implementing branch fact not visible for open_pr; retrying")
    end
    if tostring(fact.base_branch or "") ~= tostring(branches.integration) then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-foreign(base)", "implementing fact base branch mismatch")
      return
    end

    local branch_ref = exec_sync({ cmd = core.git_show_ref_cmd(".", fact.branch), timeout = 30 })
    if branch_ref.exit_code ~= 0 then
      error("github-devloop: implementing branch missing: " .. tostring(branch_ref.stderr))
    end
    local branch_head = exec_sync({ cmd = core.git_rev_parse_branch_cmd(".", fact.branch), timeout = 30 })
    if branch_head.exit_code ~= 0 then
      error("github-devloop: implementing branch head missing: " .. tostring(branch_head.stderr))
    end
    local head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
    if not core.is_safe_head_sha(head_sha) then
      error("github-devloop: unsafe implementing branch head")
    end
    if head_sha ~= fact.head_sha then
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-foreign(head)", "branch head moved past implementing fact")
      return
    end

    local write_enabled = core.write_mode() == "real"
    if not write_enabled then
      core.log_line("info", "open_pr", proposal_id, "OUTBOUND", {
        "mode=dry-run",
        "queue=github-proxy.github_pr_open_request",
        "repo=" .. tostring(issue.repo),
        "issue=" .. tostring(issue.number),
        "branch=" .. tostring(fact.branch),
        "reason=would push/create PR requires FKST_GITHUB_WRITE=1",
      })
      return
    end

    core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", core.cas_outcome(state, "apply", state.version), "write gate satisfied; opening PR")
    local pr_request = core.build_pr_open_request(issue.repo, issue.number, proposal_id, state, current_issue.title, fact.branch, fact.head_sha, branches.integration)
    core.log_apply("open_pr", proposal_id, "pr-open", state.version, { add = {}, remove = {} }, {
      "github-proxy.github_pr_open_request",
    })
    core.log_raise("open_pr", proposal_id, "github-proxy.github_pr_open_request", pr_request)
  end)
end

return M
