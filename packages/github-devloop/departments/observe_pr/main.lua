local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_reviewing",
    "devloop_merge_ready",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

function pipeline(event)
  local pr = event.payload or {}
  if not core.is_supported_pr(pr) then
    core.log_entry("observe_pr", event, "unknown", pr.dedup_key)
    core.log_cas_decision("observe_pr", "unknown", { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "unsupported event payload")
    return
  end

  core.log_entry("observe_pr", event, "unknown", pr.dedup_key)
  core.assert_trusted_bot_configured()
  local branches = core.branch_config()
  local pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(pr.repo, pr.number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed: " .. tostring(pr_view.stderr))
  end

  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin == nil then
    if core.is_devloop_issue_branch(current_pr.head_ref_name) then
      core.log_line("info", "observe_pr", "unknown", "CAS", {
        "current_state=unknown",
        "transition=pr-open->reviewing",
        "outcome=pending(pr-origin)",
        "reason=trusted pr-origin marker absent on devloop branch",
      })
      error("github-devloop: trusted pr-origin marker not yet visible for devloop PR; retrying")
    end
    core.log_line("info", "observe_pr", "unknown", "CAS", {
      "current_state=unknown",
      "transition=pr-open->reviewing",
      "outcome=skip-foreign(pr-origin)",
      "reason=trusted pr-origin marker absent",
    })
    return
  end
  if origin.repo ~= pr.repo then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(repo)", "pr-origin repo mismatch")
    return
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(head)", "pr-origin branch mismatch")
    return
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(base)", "PR base branch mismatch")
    return
  end

  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    local issue_view = exec_sync({ cmd = core.gh_issue_view_reviewing_cmd(origin.repo, origin.issue_number), timeout = 30 })
    if issue_view.exit_code ~= 0 then
      error("github-devloop: gh issue reviewing view failed: " .. tostring(issue_view.stderr))
    end

    local current_issue = core.parse_issue_view_reviewing(issue_view.stdout)
    core.log_forged_markers("observe_pr", origin.proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, origin.proposal_id)
    if state.state == "merge-ready" or state.state == "merging" then
      local fact = core.merge_ready_fact(current_issue.comments, origin.proposal_id, state.version, pr.number)
      if fact ~= nil then
        local issue_source_ref = {
          kind = "external",
          ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
        }
        local merge_payload = core.build_devloop_merge_ready_payload(origin.proposal_id, fact.pr_number, state.version, {
          review_proposal_id = fact.review_proposal_id,
          review_dedup_key = fact.review_dedup_key,
          reviewed_head_sha = fact.head_sha,
        }, issue_source_ref)
        core.log_cas_decision("observe_pr", origin.proposal_id, state, state.state, state.state, "skip-idempotent(already at to_state)", state.state .. " marker visible")
        core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_merge_ready",
        })
        core.log_raise("observe_pr", origin.proposal_id, "devloop_merge_ready", merge_payload)
      end
      if not core.state_label_hint_matches(current_issue.labels, state.state) then
        local issue_source_ref = {
          kind = "external",
          ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
        }
        local label_request = core.build_reconcile_state_label_request(origin.repo, origin.issue_number, origin.proposal_id, state.state, state.version, issue_source_ref)
        local add_labels, remove_labels = core.state_label_changes(state.state)
        core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      return
    end
    if state.state == "reviewing" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-idempotent(already at to_state)", "reviewing marker already visible")
      local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr.number, state.version, current_pr.head_sha)
      if not core.has_any_review_result_marker(current_issue.comments, review_proposal_id, origin.proposal_id) then
        local issue_source_ref = {
          kind = "external",
          ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
        }
        local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr.number, issue_source_ref, state.version)
        core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_reviewing",
        })
        core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
      end
      if not core.state_label_hint_matches(current_issue.labels, "reviewing") then
        local issue_source_ref = {
          kind = "external",
          ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
        }
        local label_request = core.build_reconcile_state_label_request(origin.repo, origin.issue_number, origin.proposal_id, "reviewing", state.version, issue_source_ref)
        local add_labels, remove_labels = core.state_label_changes("reviewing")
        core.log_apply("observe_pr", origin.proposal_id, "reviewing", state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      return
    end
    local transition = core.versioned_transition_status(state, { "pr-open" }, "reviewing", origin.impl_version)
    if transition == "pending" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "pr-open state marker not yet visible")
      error("github-devloop: pr-open marker not yet visible for observe_pr; retrying")
    end
    if transition ~= "apply" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "PR cannot advance current issue marker")
      return
    end

    local head_view = exec_sync({ cmd = core.gh_pr_view_head_cmd(pr.repo, pr.number), timeout = 30 })
    if head_view.exit_code ~= 0 then
      error("github-devloop: gh pr head re-derive failed: " .. tostring(head_view.stderr))
    end
    local current_head = core.parse_pr_view_head_state(head_view.stdout)
    if tostring(current_head.head_ref_name or "") ~= tostring(origin.branch) then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "re-derived PR head mismatch")
      return
    end
    if tostring(current_head.base_ref_name or "") ~= tostring(origin.base_branch) then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-foreign(base)", "re-derived PR base mismatch")
      return
    end
    if tostring(current_head.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end

    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "trusted PR backpointer visible")
    local issue_source_ref = {
      kind = "external",
      ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
    }
    local comment_request = core.build_reviewing_comment_request(origin.repo, origin.issue_number, origin, pr.number, issue_source_ref)
    local label_request = core.build_reviewing_label_request(origin.repo, origin.issue_number, origin, pr.number, issue_source_ref)
    local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr.number, issue_source_ref)
    local add_labels, remove_labels = core.state_label_changes("reviewing")
    core.log_apply("observe_pr", origin.proposal_id, "reviewing", origin.impl_version, { add = add_labels, remove = remove_labels }, {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
      "devloop_reviewing",
    })
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
    core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
  end)
end

return M
