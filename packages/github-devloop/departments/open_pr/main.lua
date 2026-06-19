local core, saga = require("core"), require("std.saga")

local M = {}

local spec = {
  consumes = { "devloop_open_pr", "github-proxy.github_entity_changed" },
  produces = {
    "github-proxy.github_pr_open_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  },
  stall_window = "2m",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function open_pr_context(event)
  local payload = event.payload or {}
  if core.event_queue_matches(event, "devloop_open_pr") and core.is_supported_open_pr(payload) then
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
  if core.event_queue_matches(event, "github-proxy.github_entity_changed") and core.is_supported_issue(payload) then
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

local function impl_failed_ready(input, fact, state)
  return {
    proposal_id = input.proposal_id,
    dedup_key = state.version,
    source_ref = input.source_ref or fact.source_ref,
  }
end

local function raise_impl_failed(repo, issue_number, ready, reason, detail)
  local comment_request = core.build_impl_failure_comment_request(repo, issue_number, ready, reason, detail)
  local label_request = core.build_impl_failed_label_request(repo, issue_number, ready, reason)
  local add_labels, remove_labels = core.state_label_changes("impl-failed")
  core.log_apply("open_pr", ready.proposal_id, "impl-failed", ready.dedup_key, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("open_pr", ready.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("open_pr", ready.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

return saga.department(spec, { done = function() return false end, act = function(event)
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

    local view = core.fetch_issue_view_open_pr(input.repo, input.issue_number, raw.updated_at, {
      force_fresh = true,
    })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue open-pr view failed: " .. tostring(view.stderr))
    end

    local current_issue = core.parse_issue_view_open_pr(view.stdout)
    if not core.verify_pr_review_issue_claim("open_pr", input.repo, input.issue_number, current_issue, proposal_id) then
      return
    end
    core.log_forged_markers("open_pr", proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, proposal_id)
    if state.state == "pr-open" then
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

    local branch_ref = core.git_show_ref(".", fact.branch, 30)
    if branch_ref.exit_code ~= 0 then
      error("github-devloop: implementing branch missing: " .. tostring(branch_ref.stderr))
    end
    local branch_head = core.git_rev_parse_branch(".", fact.branch, 30)
    if branch_head.exit_code ~= 0 then
      error("github-devloop: implementing branch head missing: " .. tostring(branch_head.stderr))
    end
    local head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
    if not core.is_safe_head_sha(head_sha) then
      error("github-devloop: unsafe implementing branch head")
    end
    if head_sha ~= fact.head_sha then
      local ancestry = core.git_is_ancestor(fact.head_sha, head_sha, 30)
      if ancestry.exit_code ~= 0 then
        core.log_cas_decision("open_pr", proposal_id, state, "implementing", "impl-failed", "applied(non-descendant-head)", "branch head is not descended from implementing fact")
        raise_impl_failed(
          input.repo,
          input.issue_number,
          impl_failed_ready(input, fact, state),
          "non-descendant-head",
          "The implementation branch head is not descended from the trusted implementing fact head, so open_pr cannot safely create a PR from it."
        )
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
end, wrap = core.wrap_pipeline_failure, name = "open_pr" })
