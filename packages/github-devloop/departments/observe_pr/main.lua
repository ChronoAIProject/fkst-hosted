local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_merge_ready",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function pr_source_ref(repo, pr_number)
  return core.pr_source_ref(repo, pr_number)
end

local function origin_from_pr(repo, pr_number, current_pr)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin ~= nil then
    return origin, true
  end
  return core.pr_native_origin(repo, pr_number, current_pr), false
end

local function origin_matches_pr(origin, current_pr, repo, branches, require_issue_backing)
  if origin.repo ~= repo then
    return false, "repo"
  end
  if require_issue_backing and origin.issue_number == nil then
    return false, "issue"
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    return false, "head"
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch) then
    return false, "base"
  end
  if origin.base_branch ~= nil
    and tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    return false, "base"
  end
  return true, "ok"
end

local function maybe_label_hint(origin, state, source_ref)
  if origin.issue_number == nil then
    return
  end
  local label_request = core.build_reconcile_state_label_request(origin.repo, origin.issue_number, origin.proposal_id, state.state, state.version, source_ref)
  local add_labels, remove_labels = core.state_label_changes(state.state)
  core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function raise_current_state(origin, pr_number, current_pr, state, source_ref)
  if state.state == "reviewing" then
    local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
    if not core.has_any_review_result_marker(current_pr.comments, review_proposal_id, origin.proposal_id) then
      local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr_number, source_ref, state.version)
      core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "devloop_reviewing",
      })
      core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    end
    return
  end
  if state.state == "fixing" then
    local reject_fact = core.review_reject_fact(current_pr.comments, origin.proposal_id, state.version)
    local meta_fix_fact = reject_fact == nil and core.review_meta_fix_fact(current_pr.comments, origin.proposal_id, state.version) or nil
    local merge_gate_fact = reject_fact == nil and meta_fix_fact == nil and core.merge_gate_fix_fact(current_pr.comments, origin.proposal_id, state.version) or nil
    local fact = reject_fact or meta_fix_fact or merge_gate_fact
    if fact ~= nil and fact.review_proposal_id ~= nil and fact.reviewed_head_sha ~= nil then
      local reviewing_version = core.next_fix_version(state.version)
      if not core.has_state_marker(current_pr.comments, origin.proposal_id, "reviewing", reviewing_version) then
        local fix_payload = core.build_devloop_fixing_payload({
          proposal_id = origin.proposal_id,
          impl_version = state.version,
        }, pr_number, {
          review_proposal_id = fact.review_proposal_id,
          review_dedup_key = fact.review_dedup_key,
          reviewed_head_sha = fact.reviewed_head_sha,
        }, source_ref)
        core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_fixing",
        })
        core.log_raise("observe_pr", origin.proposal_id, "devloop_fixing", fix_payload)
      end
    end
    return
  end
  if state.state == "merge-ready" or state.state == "merging" then
    local fact = core.merge_ready_fact(current_pr.comments, origin.proposal_id, state.version, pr_number)
    if fact ~= nil then
      local merge_payload = core.build_devloop_merge_ready_payload(origin.proposal_id, fact.pr_number, state.version, {
        review_proposal_id = fact.review_proposal_id,
        review_dedup_key = fact.review_dedup_key,
        reviewed_head_sha = fact.head_sha,
      }, source_ref)
      core.log_apply("observe_pr", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "devloop_merge_ready",
      })
      core.log_raise("observe_pr", origin.proposal_id, "devloop_merge_ready", merge_payload)
    end
  end
end

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
  local pr_view = core.fetch_pr_view_origin(pr.repo, pr.number, pr.updated_at)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed: " .. tostring(pr_view.stderr))
  end

  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin, has_issue_origin = origin_from_pr(pr.repo, pr.number, current_pr)
  if origin.branch == nil or origin.base_branch == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "PR branch facts missing")
    return
  end
  local ok, reason = origin_matches_pr(origin, current_pr, pr.repo, branches, false)
  if not ok then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(" .. reason .. ")", "PR origin mismatch")
    return
  end

  local source_ref = pr_source_ref(pr.repo, pr.number)
  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    local state = core.current_entity_state(current_pr.comments, origin.proposal_id)
    if state.state ~= nil and state.state ~= "pr-open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "reviewing", state.state, "skip-idempotent(already at to_state)", state.state .. " marker visible on PR")
      raise_current_state(origin, pr.number, current_pr, state, source_ref)
      maybe_label_hint(origin, state, core.issue_source_ref(origin.repo, origin.issue_number))
      return
    end

    local transition = core.versioned_transition_status(state, { "pr-open", "unmanaged" }, "reviewing", origin.impl_version)
    if has_issue_origin and transition == "pending" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "reviewing PR marker not yet visible")
    end
    if state.state == "pr-open" and tostring(state.version or "") ~= tostring(origin.impl_version or "") then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(version-mismatch)", "PR-open marker version does not match PR origin")
      return
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "applied", "writing PR-local reviewing marker")
    local comment_request = core.build_reviewing_comment_request(origin.repo, origin.issue_number, origin, pr.number, source_ref)
    local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr.number, source_ref)
    local raised = {
      "github-proxy.github_pr_comment_request",
      "devloop_reviewing",
    }
    core.log_apply("observe_pr", origin.proposal_id, "reviewing", origin.impl_version, { add = {}, remove = {} }, raised)
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    maybe_label_hint(origin, { state = "reviewing", version = origin.impl_version }, core.issue_source_ref(origin.repo, origin.issue_number))
  end)
end

return M
