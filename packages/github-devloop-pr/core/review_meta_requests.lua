local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local conv_reconcile = require("devloop.convergence.reconcile")
local S = {}
local comment_strings = require("devloop.strings")

function S.install(M)
local strings = require("contract.strings")
local m_builders = require("devloop.markers.builders")
local ai_sentinel = "⟦AI:FKST⟧"

local function normalized_reflection_action(review_meta, action)
  if review_meta.mode == "fix-reflection" and action == "spec-gap" then
    return "spec-amendment"
  end
  return action
end

local function review_meta_to_state(action)
  if action == "fix" or action == "continue" then
    return "fixing"
  end
  return "blocked"
end

local function review_meta_action_text(review_meta, action)
  if review_meta.mode == "fix-reflection" then
    return tostring(action)
  end
  if action == "spec-amendment" then
    return "blocked-pending-spec"
  end
  return tostring(action)
end

local function review_meta_result_marker(review_meta, action, reason, state_version, blocking_gap)
  if review_meta.mode ~= "fix-reflection" then
    return m_builders.review_meta_marker(M, review_meta.proposal_id, review_meta.dedup_key, action, state_version, blocking_gap, reason)
  end
  local marker = m_builders.fix_reflection_marker(M, 
    review_meta.proposal_id,
    review_meta.dedup_key,
    action,
    state_version,
    review_meta.fix_round or review_meta.n or M.version_fix_round(review_meta.version)
  )
  if action == "continue" then
    marker = marker .. "\n" .. m_builders.review_meta_marker(M, 
      review_meta.proposal_id,
      review_meta.dedup_key,
      "fix",
      state_version,
      review_meta.blocking_gap,
      reason
    )
  end
  return marker
end

function M.build_fix_review_meta_label_request(repo, issue_number, fix, reason)
  return requests_labels.build_state_label_request(M,
    repo,
    issue_number,
    "review-meta",
    base_ids.dedup_key({
      "fix",
      "label",
      "review-meta",
      tostring(reason or "no-fix"),
      tostring(fix.review_dedup_key),
    }),
    fix.source_ref
  )
end

function M.build_fix_review_meta_comment_request(repo, issue_number, fix, reason, detail)
  local safe_reason = strings.sanitize_key(reason or "no-fix", M._max_key_len):gsub("/", "-")
  local text = tostring(detail or "")
  if #text > M._max_impl_output_len then
    text = base_ids.truncate_utf8(text, M._max_impl_output_len)
  end
  if text == "" then
    text = comment_strings.comment_string(M, "no_fix_output")
  end
  text = devloop_base.neutralize_untrusted_comment_text(text)
  local state_marker = M.state_marker(fix.proposal_id, "review-meta", fix.version)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, comment_strings.comment_string(M, "fix_escalated_to_review_meta_prefix") .. safe_reason
    .. "\n\n" .. text
    .. "\n\n" .. state_marker
    .. "\n" .. m_builders.review_meta_marker(M, fix.proposal_id, fix.review_dedup_key), base_ids.dedup_key({
    "fix",
    "comment",
    "review-meta",
    safe_reason,
    tostring(fix.dedup_key),
  }), fix.source_ref)
end

function M.build_review_meta_label_request(repo, issue_number, review_meta, action, version)
  local normalized = normalized_reflection_action(review_meta, action)
  return requests_labels.build_state_label_request(M,
    repo,
    issue_number,
    review_meta_to_state(normalized),
    base_ids.dedup_key({
      "review-meta",
      "label",
      tostring(action),
      tostring(review_meta.dedup_key),
      tostring(version or review_meta.version),
    }),
    review_meta.source_ref
  )
end

function M.build_review_meta_comment_request(repo, issue_number, review_meta, action, reason, version, blocking_gap)
  local normalized = normalized_reflection_action(review_meta, action)
  local to_state = review_meta_to_state(normalized)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  local state_version = version or review_meta.version
  local marker = review_meta_result_marker(review_meta, action, reason, state_version, blocking_gap)
  local prefix = review_meta.mode == "fix-reflection" and comment_strings.comment_string(M, "fix_reflection_prefix") or comment_strings.comment_string(M, "review_meta_action_prefix")
  local request = entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = review_meta.pr_number,
  }, prefix .. review_meta_action_text(review_meta, normalized == "spec-amendment" and action or normalized)
    .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
    .. "\n\n" .. M.state_marker(review_meta.proposal_id, to_state, state_version)
    .. "\n" .. marker, base_ids.dedup_key({
    review_meta.mode == "fix-reflection" and "fix-reflection" or "review-meta",
    "comment",
    tostring(review_meta.dedup_key),
    tostring(state_version),
  }), review_meta.source_ref)
  if action == "fix" or action == "continue" then
    local _, _, _, reviewed_head_sha = devloop_base.parse_pr_review_proposal_id(review_meta.review_proposal_id)
    return requests_review.attach_fixing_handoff(M, request, review_meta.proposal_id, review_meta.pr_number, state_version, {
      review_proposal_id = review_meta.review_proposal_id,
      review_dedup_key = review_meta.dedup_key,
      reviewed_head_sha = reviewed_head_sha,
      blocking_gap = blocking_gap or review_meta.blocking_gap,
    }, review_meta.source_ref)
  end
  return request
end

function M.build_review_reconcile_label_request(repo, issue_number, review_reconcile)
  return requests_labels.build_state_label_request(M,
    repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
      "review-reconcile",
      "label",
      tostring(review_reconcile.dedup_key),
    }),
    review_reconcile.source_ref
  )
end

function M.build_fix_reconcile_label_request(repo, issue_number, fix_reconcile)
  return requests_labels.build_state_label_request(M,
    repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
      "fix-reconcile",
      "label",
      tostring(fix_reconcile.dedup_key),
    }),
    fix_reconcile.source_ref
  )
end

function M.build_fix_reconcile_comment_request(repo, _issue_number, fix_reconcile, action, reason)
  local version = conv_reconcile.fix_reconcile_state_version(M, fix_reconcile.issue_version)
  local marker = conv_reconcile.fix_reconcile_marker(M, fix_reconcile.proposal_id, fix_reconcile.issue_version, action)
  local state_marker = M.state_marker(fix_reconcile.proposal_id, "blocked", version)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = devloop_base.parse_pr_source_ref(fix_reconcile.source_ref)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, comment_strings.comment_string(M, "fix_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, base_ids.dedup_key({
    "fix-reconcile",
    "comment",
    tostring(fix_reconcile.dedup_key),
  }), fix_reconcile.source_ref)
end

function M.build_review_reconcile_comment_request(repo, _issue_number, review_reconcile, action, reason, state_version)
  local version = state_version or conv_reconcile.review_reconcile_state_version(M, review_reconcile.issue_version, review_reconcile.round)
  local marker = conv_reconcile.review_reconcile_marker(M, review_reconcile.proposal_id, review_reconcile.issue_version, review_reconcile.round, action)
  local state_marker = M.state_marker(review_reconcile.proposal_id, "blocked", version)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = devloop_base.parse_pr_source_ref(review_reconcile.source_ref)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, comment_strings.comment_string(M, "review_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, base_ids.dedup_key({
    "review-reconcile",
    "comment",
    tostring(review_reconcile.dedup_key),
  }), review_reconcile.source_ref)
end
end

return S
