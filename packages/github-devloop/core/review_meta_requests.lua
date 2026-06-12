local S = {}

function S.install(M)
local max_evidence_len = 600

local function build_comment_evidence_digest(comments)
  local text = table.concat(M.comment_bodies(comments), "\n\n")
  text = text:gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return M.comment_string("comment_evidence_empty")
  end
  if #text > max_evidence_len then
    text = M.truncate_utf8(text, max_evidence_len)
  end
  return text
end

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
    return M.review_meta_marker(review_meta.proposal_id, review_meta.dedup_key, action, state_version, blocking_gap, reason)
  end
  local marker = M.fix_reflection_marker(
    review_meta.proposal_id,
    review_meta.dedup_key,
    action,
    state_version,
    review_meta.fix_round or review_meta.n or M.version_fix_round(review_meta.version)
  )
  if action == "continue" then
    marker = marker .. "\n" .. M.review_meta_marker(
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
  return M.build_state_label_request(
    repo,
    issue_number,
    "review-meta",
    M._dedup_key({
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
  local safe_reason = M.sanitize_key(reason or "no-fix"):gsub("/", "-")
  local text = tostring(detail or "")
  if #text > M._max_impl_output_len then
    text = M.truncate_utf8(text, M._max_impl_output_len)
  end
  if text == "" then
    text = M.comment_string("no_fix_output")
  end
  text = M.neutralize_untrusted_comment_text(text)
  local state_marker = M.state_marker(fix.proposal_id, "review-meta", fix.version)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, M.comment_string("fix_escalated_to_review_meta_prefix") .. safe_reason
    .. "\n\n" .. text
    .. "\n\n" .. state_marker
    .. "\n" .. M.review_meta_marker(fix.proposal_id, fix.review_dedup_key), M._dedup_key({
    "fix",
    "comment",
    "review-meta",
    safe_reason,
    tostring(fix.dedup_key),
  }), fix.source_ref)
end

function M.build_review_meta_label_request(repo, issue_number, review_meta, action, version)
  local normalized = normalized_reflection_action(review_meta, action)
  return M.build_state_label_request(
    repo,
    issue_number,
    review_meta_to_state(normalized),
    M._dedup_key({
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
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local state_version = version or review_meta.version
  local marker = review_meta_result_marker(review_meta, action, reason, state_version, blocking_gap)
  local prefix = review_meta.mode == "fix-reflection" and M.comment_string("fix_reflection_prefix") or M.comment_string("review_meta_action_prefix")
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = review_meta.pr_number,
  }, prefix .. review_meta_action_text(review_meta, normalized == "spec-amendment" and action or normalized)
    .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
    .. "\n\n" .. M.state_marker(review_meta.proposal_id, to_state, state_version)
    .. "\n" .. marker, M._dedup_key({
    review_meta.mode == "fix-reflection" and "fix-reflection" or "review-meta",
    "comment",
    tostring(review_meta.dedup_key),
    tostring(state_version),
  }), review_meta.source_ref)
end

function M.build_spec_amendment_issue_create_request(repo, issue_number, review_meta, title_brief, reason, comments)
  local title = "Spec amendment needed: " .. tostring(title_brief or ("Issue #" .. tostring(issue_number or "unknown")))
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  local body = "Spec flaw statement:\n" .. M.neutralize_untrusted_comment_text(reason or "")
    .. "\n\nEvidence digest:\n" .. M.neutralize_untrusted_comment_text(build_comment_evidence_digest(comments))
    .. "\n\nParent issue: #" .. tostring(issue_number or "unknown")
    .. "\nParent PR: #" .. tostring(review_meta.pr_number)
    .. "\nReview proposal: " .. tostring(review_meta.review_proposal_id)
    .. "\nReview dedup: " .. tostring(review_meta.dedup_key)
    .. "\n\nThis issue requests a spec revision only. Do not edit the human-authored parent issue text."
  if #body > M._max_body_len then
    body = M.truncate_utf8(body, M._max_body_len)
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = M._dedup_key({
      "spec-amendment",
      tostring(review_meta.proposal_id),
      tostring(review_meta.dedup_key),
    }),
    parent_comment_target = {
      repo = repo,
      pr_number = review_meta.pr_number,
    },
    source_ref = M.normalize_source_ref(review_meta.source_ref),
  }
end
end

return S
