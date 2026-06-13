local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_open_pr", "github-proxy.github_entity_changed" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_open_request",
  },
  stall_window = "2m",
}

local function open_pr_context(event)
  local payload = event.payload or {}
  if core.is_supported_open_pr(payload) then
    return {
      source = "direct",
      repo = payload.repo,
      issue_number = payload.issue_number,
      proposal_id = payload.proposal_id,
      version = payload.version,
      branch = payload.branch,
      head_sha = payload.head_sha,
      base_branch = payload.base_branch,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  if core.is_supported_issue(payload) then
    return {
      source = "poll",
      repo = payload.repo,
      issue_number = payload.number,
      proposal_id = core.proposal_id(payload.repo, payload.number),
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  return nil
end

function pipeline(event)
  local input = open_pr_context(event)
  local raw = event.payload or {}
  if input == nil then
    core.log_entry("open_pr", event, "unknown", core.payload_field(raw, "dedup_key"))
    core.log_cas_decision("open_pr", "unknown", { state = nil, version = nil }, "implementing", "pr-open", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  local proposal_id = input.proposal_id
  core.log_entry("open_pr", event, proposal_id, input.dedup_key)
  local lock_key = core.transition_lock_key(proposal_id)
  if lock_key == nil then
    core.log_cas_decision("open_pr", proposal_id, { state = nil, version = nil }, "implementing", "pr-open", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()
    local branches = core.branch_config()

    local view = core.fetch_issue_view_open_pr(input.repo, input.issue_number, raw.updated_at)
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue open-pr view failed: " .. tostring(view.stderr))
    end

    local current_issue = core.parse_issue_view_open_pr(view.stdout)
    core.log_forged_markers("open_pr", proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, proposal_id)
    if state.state == "pr-open" then
      if not core.state_label_hint_matches(current_issue.labels, "pr-open") then
        local label_request = core.build_state_label_request(
          input.repo,
          input.issue_number,
          "pr-open",
          core._dedup_key({
            "open-pr",
            "label",
            tostring(proposal_id),
            tostring(state.version or "unversioned"),
          }),
          input.source_ref
        )
        local add_labels, remove_labels = core.state_label_changes("pr-open")
        core.log_apply("open_pr", proposal_id, "pr-open", state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("open_pr", proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-idempotent(already at to_state)", "PR state marker already visible")
      return
    end
    if state.state == "reviewing" then
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
    if input.source == "direct" then
      if tostring(input.version or "") ~= tostring(state.version or "")
        or tostring(input.branch or "") ~= tostring(fact.branch or "")
        or tostring(input.head_sha or "") ~= tostring(fact.head_sha or "")
        or tostring(input.base_branch or "") ~= tostring(fact.base_branch or "") then
        core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-stale(direct-fact-mismatch)", "direct open-pr event does not match canonical implementing fact")
        return
      end
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
      local ancestry = exec_sync({ cmd = core.git_is_ancestor_cmd(fact.head_sha, head_sha), timeout = 30 })
      if ancestry.exit_code ~= 0 then
        core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", "skip-foreign(head)", "branch head is not descended from implementing fact")
        return
      end
    end

    local write_enabled = core.write_mode() == "real"
    if not write_enabled then
      core.log_line("info", "open_pr", proposal_id, "OUTBOUND", {
        "mode=dry-run",
        "queue=github-proxy.github_pr_open_request",
        "repo=" .. tostring(input.repo),
        "issue=" .. tostring(input.issue_number),
        "branch=" .. tostring(fact.branch),
        "reason=would push/create PR requires FKST_GITHUB_WRITE=1",
      })
      return
    end

    core.log_cas_decision("open_pr", proposal_id, state, "implementing", "pr-open", core.cas_outcome(state, "apply", state.version), "write gate satisfied; opening PR")
    local pr_request = core.build_pr_open_request(input.repo, input.issue_number, proposal_id, state, current_issue.title, fact.branch, head_sha, branches.integration)
    core.log_apply("open_pr", proposal_id, "pr-open", state.version, { add = {}, remove = {} }, {
      "github-proxy.github_pr_open_request",
    })
    core.log_raise("open_pr", proposal_id, "github-proxy.github_pr_open_request", pr_request)
  end)
end

pipeline = core.wrap_pipeline_failure("open_pr", pipeline)

return M
