local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_review_meta" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_create_request",
    "devloop_fixing",
  },
  stall_window = "2m",
}

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
  local review_meta = event.payload or {}
  if not core.is_supported_review_meta(review_meta) then
    core.log_entry("review_meta", event, "unknown", review_meta.dedup_key)
    core.log_cas_decision("review_meta", "unknown", { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_meta", event, review_meta.proposal_id, review_meta.dedup_key)
  local entity = core.parse_entity_proposal_id(review_meta.proposal_id)
  if entity == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number

  local lock_key = core.transition_lock_key(review_meta.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = core.gh_exec({ cmd = core.gh_pr_view_origin_cmd(repo, review_meta.pr_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr review-meta view failed: " .. tostring(view.stderr))
    end
    local current_pr = core.parse_pr_view_origin(view.stdout)
    local current_issue = {
      title = "PR #" .. tostring(review_meta.pr_number),
      body = "(PR-only review-meta context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_exec({ cmd = core.gh_issue_view_fix_cmd(repo, issue_number), timeout = 30 })
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review-meta view failed: " .. tostring(issue_view.stderr))
      end
      local parsed_issue = core.parse_issue_view_fix(issue_view.stdout)
      if parsed_issue.title ~= nil and parsed_issue.title ~= "" then
        current_issue.title = parsed_issue.title
      end
    end
    core.log_forged_markers("review_meta", review_meta.proposal_id, current_pr.comments)
    local state = core.current_entity_state(current_pr.comments, review_meta.proposal_id)
    local transition = core.cyclic_transition_status(state, { "review-meta" }, "fixing", review_meta.version)
    if transition == "pending" then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "retry-pending(from-state marker not yet visible)", "review-meta state marker not yet visible")
      error("github-devloop: review-meta state marker not yet visible; retrying")
    end
    if state.state ~= "review-meta" or transition == "stale" then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", core.cas_outcome(state, transition, review_meta.version), "current marker is no longer review-meta")
      return
    end
    if tostring(state.version or "") ~= tostring(review_meta.version) then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "skip-stale(version-mismatch)", "review-meta event version does not match canonical issue marker")
      return
    end
    if core.has_review_meta_marker(current_pr.comments, review_meta.proposal_id, review_meta.dedup_key) then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "skip-idempotent(review-meta marker already visible)", "review-meta result marker for incoming version is already visible")
      return
    end

    core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "applied", "running review-meta codex decision")
    core.log_codex_start("review_meta", review_meta.proposal_id, "review-meta")
    local content_fetch = core.context_fetch_from_bundle({
      dept = "review_meta",
      repo = repo,
      issue_number = issue_number,
      pr_number = review_meta.pr_number,
      proposal_id = review_meta.proposal_id,
      version = review_meta.dedup_key,
      tick = event.ts,
    })
    local result = spawn_codex_sync(core.judgment_codex_opts(
      core.build_review_meta_prompt(review_meta, current_issue, content_fetch),
      judgment_worktree("review-meta", review_meta.dedup_key)
    ))
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, stderr)
      error("github-devloop: review-meta codex failed: " .. tostring(stderr))
    end
    local parsed = core.parse_review_meta_action(result.stdout)
    if parsed == nil then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "parse-failed")
      parsed = {
        action = "block",
        reason = "Review-meta codex output was unparseable.",
      }
    end
    if parsed.action == "fix"
      and not core._is_bounded_string(parsed.blocking_gap, core._max_blocking_gap_len) then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "missing-blocking-gap")
      parsed = {
        action = "block",
        reason = "Review-meta fix output omitted a bounded blocking gap.",
      }
    end
    core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, "action=" .. tostring(parsed.action) .. " reason=" .. tostring(parsed.reason), nil)

    local to_state = parsed.action == "fix" and "fixing" or "blocked"
    local exit_version = core.next_review_meta_action_version(review_meta.version)
    local comment_request = core.build_review_meta_comment_request(repo, issue_number, review_meta, parsed.action, parsed.reason, exit_version, parsed.blocking_gap)
    local label_request = nil
    if issue_number ~= nil then
      label_request = core.build_review_meta_label_request(repo, issue_number, review_meta, parsed.action, exit_version)
    end
    local spec_issue_request = nil
    if parsed.action == "spec-amendment" then
      spec_issue_request = core.build_spec_amendment_issue_create_request(
        repo,
        issue_number,
        review_meta,
        current_issue.title,
        parsed.reason,
        current_pr.comments
      )
    end
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_pr_comment_request",
    }
    if label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    if spec_issue_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_create_request")
    end
    local fix_payload = nil
    if parsed.action == "fix" then
      local _, _, _, reviewed_head_sha = core.parse_pr_review_proposal_id(review_meta.review_proposal_id)
      fix_payload = core.build_devloop_fixing_payload({
        proposal_id = review_meta.proposal_id,
        impl_version = exit_version,
      }, review_meta.pr_number, {
        review_proposal_id = review_meta.review_proposal_id,
        review_dedup_key = review_meta.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
        blocking_gap = parsed.blocking_gap,
      }, review_meta.source_ref)
      table.insert(raised, "devloop_fixing")
    end

    core.log_apply("review_meta", review_meta.proposal_id, to_state, exit_version, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    if label_request ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
    if spec_issue_request ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_issue_create_request", spec_issue_request)
    end
    if fix_payload ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "devloop_fixing", fix_payload)
    end
  end)
end

return M
