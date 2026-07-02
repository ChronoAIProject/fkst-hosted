local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local strings = require("contract.strings")
local m_claims = require("devloop.claims")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local m_facts = require("devloop.markers.facts")
local core, saga, context_bundle = require("core"), require("workflow.saga"), require("devloop.context_bundle")
local v_review_meta = require("devloop.validators.review_meta")
local workflow_codex = require("workflow.codex")

-- Preserve existing body line coordinates for the coverage ratchet.

local spec = {
  consumes = { "devloop_review_meta" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
  },
  stall_window = "2m",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

return saga.department(spec, { done = function() return false end, act = function(event)
  local review_meta = event.payload or {}
  if not v_review_meta.is_supported_review_meta(core, review_meta) then
    core.log_entry("review_meta", event, "unknown", core.payload_field(review_meta, "dedup_key"))
    core.log_cas_decision("review_meta", "unknown", { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_meta", event, review_meta.proposal_id, review_meta.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(review_meta.proposal_id)
  if entity == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  if not m_claims.verify_pr_review_issue_claim(core, "review_meta", repo, issue_number, nil, review_meta.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(review_meta.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_meta", review_meta.proposal_id, { state = nil, version = nil }, "review-meta", "fixing|blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local view = core.gh_pr_view_origin(repo, review_meta.pr_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr review-meta view failed: " .. tostring(view.stderr))
    end
    local current_pr = parsers_pr.parse_pr_view_origin(core, view.stdout)
    local current_issue = {
      title = "PR #" .. tostring(review_meta.pr_number),
      body = "(PR-only review-meta context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_issue_view_fix(repo, issue_number, 30)
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review-meta view failed: " .. tostring(issue_view.stderr))
      end
      local parsed_issue = parsers_issue.parse_issue_view_fix(core, issue_view.stdout)
      if parsed_issue.title ~= nil and parsed_issue.title ~= "" then
        current_issue.title = parsed_issue.title
      end
    end
    core.log_forged_markers("review_meta", review_meta.proposal_id, current_pr.comments)

    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, review_meta.proposal_id)
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
    if m_facts.has_review_meta_marker(core, current_pr.comments, review_meta.proposal_id, review_meta.dedup_key) then
      core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "skip-idempotent(review-meta marker already visible)", "review-meta result marker for incoming version is already visible")
      return
    end

    core.log_cas_decision("review_meta", review_meta.proposal_id, state, "review-meta", "fixing|blocked", "applied", "running review-meta codex decision")
    local codex_started_at = now()
    core.log_codex_start("review_meta", review_meta.proposal_id, "review-meta")
    local content_fetch = context_bundle.context_fetch_from_bundle(core, {
      dept = "review_meta",
      repo = repo,
      issue_number = issue_number,
      pr_number = review_meta.pr_number,
      proposal_id = review_meta.proposal_id,
      version = review_meta.dedup_key,
      tick = event.ts,
    })
    local codex_opts = workflow_codex.judgment_codex_opts(
      core.build_review_meta_prompt(review_meta, current_issue, content_fetch),
      devloop_base.judgment_worktree_with_exec(exec_sync, "review-meta", review_meta.dedup_key)
    )
    codex_opts.role = "review-meta"
    codex_opts.proposal_id = review_meta.proposal_id
    codex_opts.dedup_key = review_meta.version
    local result = spawn_codex_sync(codex_opts)
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, stderr, {
        queue = event.queue,
        source_ref = review_meta.source_ref,
        terminal = false,
      })
      error("github-devloop: review-meta codex failed: " .. tostring(stderr))
    end
    local parsed = core.parse_review_meta_action(result.stdout)
    if parsed == nil then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "parse-failed", {
        queue = event.queue,
        source_ref = review_meta.source_ref,
        terminal = false,
      })
      parsed = {
        action = "block",
        reason = "Review-meta codex output was unparseable.",
      }
    end
    local is_reflection = review_meta.mode == "fix-reflection"
    local allowed_action = false
    if is_reflection then
      allowed_action = parsed.action == "continue" or parsed.action == "spec-gap"
    else
      allowed_action = parsed.action == "fix" or parsed.action == "block" or parsed.action == "spec-amendment"
    end
    if not allowed_action then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "invalid-action-for-mode")
      parsed = {
        action = is_reflection and "spec-gap" or "block",
        reason = "Review-meta codex output used an action outside this decision mode.",
      }
    end
    if parsed.action == "fix"
      and not strings.is_bounded_string(parsed.blocking_gap, core._max_blocking_gap_len) then
      core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, nil, "missing-blocking-gap")
      parsed = {
        action = "block",
        reason = "Review-meta fix output omitted a bounded blocking gap.",
      }
    end
    core.log_codex_result("review_meta", review_meta.proposal_id, "review-meta", result, "action=" .. tostring(parsed.action) .. " reason=" .. tostring(parsed.reason), nil)

    local to_state = (parsed.action == "fix" or parsed.action == "continue") and "fixing" or "blocked"
    local exit_version = core.next_review_meta_action_version(review_meta.version)
    local comment_request = core.build_review_meta_comment_request(repo, issue_number, review_meta, parsed.action, parsed.reason, exit_version, parsed.blocking_gap)
    local label_request = nil
    if issue_number ~= nil then
      label_request = core.build_review_meta_label_request(repo, issue_number, review_meta, parsed.action, exit_version)
    end
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_pr_comment_request",
    }
    if label_request ~= nil then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    core.log_apply("review_meta", review_meta.proposal_id, to_state, exit_version, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    if label_request ~= nil then
      core.log_raise("review_meta", review_meta.proposal_id, "github-proxy.github_issue_label_request", label_request)
    end
  end)
end, wrap = core.wrap_pipeline_failure, name = "review_meta" })
