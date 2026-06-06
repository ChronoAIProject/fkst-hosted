local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_review_meta" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_fixing",
    "devloop_merge_ready",
  },
  stall_window = "2m",
}

function pipeline(event)
  local review_meta = event.payload or {}
  if not core.is_supported_review_meta(review_meta) then
    core.log_entry("review_meta", event, "unknown", review_meta.dedup_key)
    core.log_cas_decision("review_meta", "unknown", { state = nil, version = nil }, "review-meta", "fixing|merge-ready|blocked", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_meta", event, review_meta.proposal_id, review_meta.dedup_key)
  local repo, issue_number = core.parse_proposal_id(review_meta.proposal_id)
  if repo == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|merge-ready|blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.transition_lock_key(review_meta.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|merge-ready|blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_review_meta_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue review-meta view failed: " .. tostring(view.stderr))
    end
    local current_issue = core.parse_issue_view_review_meta(view.stdout)
    core.log_forged_markers("review_meta", review_meta.proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, review_meta.proposal_id)
    local transition = core.cyclic_transition_status(state, { "review-meta" }, "fixing", review_meta.version)
    if transition == "pending" then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|merge-ready|blocked", "retry-pending(from-state marker not yet visible)", "review-meta state marker not yet visible")
      error("github-devloop: review-meta state marker not yet visible; retrying")
    end
    if state.state ~= "review-meta" or transition == "stale" then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|merge-ready|blocked", core.cas_outcome(state, transition, review_meta.version), "current marker is no longer review-meta")
      return
    end
    if tostring(state.version or "") ~= tostring(review_meta.version) then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|merge-ready|blocked", "skip-stale(version-mismatch)", "review-meta event version does not match canonical issue marker")
      return
    end
    if core.has_review_meta_marker(current_issue.comments, review_meta.proposal_id, review_meta.dedup_key) then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|merge-ready|blocked", "skip-idempotent(review-meta marker already visible)", "review-meta result marker for incoming version is already visible")
      return
    end

    core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|merge-ready|blocked", "applied", "running review-meta codex decision")
    core.log_codex_start("review_meta", review_meta.proposal_id, "review-meta")
    local result = spawn_codex_sync({
      prompt = core.build_review_meta_prompt(review_meta, current_issue),
      stall_window = M.spec.stall_window,
    })
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, stderr)
      error("github-devloop: review-meta codex failed: " .. tostring(stderr))
    end
    local parsed = core.parse_review_meta_action(result.stdout)
    if parsed == nil then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "parse-failed")
      error("github-devloop: review-meta codex output parse failed; retrying")
    end
    core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, "action=" .. tostring(parsed.action) .. " reason=" .. tostring(parsed.reason), nil)

    local to_state = parsed.action == "fix" and "fixing" or parsed.action == "accept" and "merge-ready" or "blocked"
    local exit_version = core.next_review_meta_action_version(review_meta.version)
    local comment_request = core.build_review_meta_comment_request(repo, issue_number, review_meta, parsed.action, parsed.reason, exit_version)
    local label_request = core.build_review_meta_label_request(repo, issue_number, review_meta, parsed.action, exit_version)
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    }
    local fix_payload = nil
    local merge_payload = nil
    if parsed.action == "fix" then
      local _, _, _, reviewed_head_sha = core.parse_pr_review_proposal_id(review_meta.review_proposal_id)
      fix_payload = core.build_devloop_fixing_payload({
        proposal_id = review_meta.proposal_id,
        impl_version = exit_version,
      }, review_meta.pr_number, {
        review_proposal_id = review_meta.review_proposal_id,
        review_dedup_key = review_meta.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
      }, review_meta.source_ref)
      table.insert(raised, "devloop_fixing")
    elseif parsed.action == "accept" then
      local _, _, _, reviewed_head_sha = core.parse_pr_review_proposal_id(review_meta.review_proposal_id)
      merge_payload = core.build_devloop_merge_ready_payload(review_meta.proposal_id, review_meta.pr_number, exit_version, {
        review_proposal_id = review_meta.review_proposal_id,
        review_dedup_key = review_meta.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
      }, review_meta.source_ref)
      table.insert(raised, "devloop_merge_ready")
    end

    core.log_apply("review_meta", review_meta.proposal_id, to_state, exit_version, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_issue_label_request", label_request)
    if fix_payload ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "devloop_fixing", fix_payload)
    end
    if merge_payload ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "devloop_merge_ready", merge_payload)
    end
  end)
end

return M
