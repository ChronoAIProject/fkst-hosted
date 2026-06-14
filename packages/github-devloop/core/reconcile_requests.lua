local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

function M.build_reconcile_label_request(repo, issue_number, reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "reconcile",
      "label",
      tostring(reconcile.dedup_key),
    }),
    reconcile.source_ref
  )
end

function M.build_review_reconcile_label_request(repo, issue_number, review_reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "review-reconcile",
      "label",
      tostring(review_reconcile.dedup_key),
    }),
    review_reconcile.source_ref
  )
end

function M.build_fix_reconcile_label_request(repo, issue_number, fix_reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "fix-reconcile",
      "label",
      tostring(fix_reconcile.dedup_key),
    }),
    fix_reconcile.source_ref
  )
end

function M.build_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
  local version = M.reconcile_state_version(reconcile.base_version, reconcile.round)
  local marker = M.reconcile_marker(reconcile.proposal_id, reconcile.base_version, reconcile.round, action)
  local state_marker = M.state_marker(reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  return M.attach_issue_claim({
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("reconcile_action_prefix") .. tostring(action)
      .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
      .. "\n\n"
      .. state_marker .. "\n" .. marker
      .. "\n" .. ai_sentinel,
    dedup_key = M._dedup_key({
      "reconcile",
      "comment",
      tostring(reconcile.dedup_key),
    }),
    source_ref = M.normalize_source_ref(reconcile.source_ref),
  }, reconcile.source_ref)
end

function M.build_fix_reconcile_comment_request(repo, issue_number, fix_reconcile, action, reason)
  local version = M.fix_reconcile_state_version(fix_reconcile.issue_version)
  local marker = M.fix_reconcile_marker(fix_reconcile.proposal_id, fix_reconcile.issue_version, action)
  local state_marker = M.state_marker(fix_reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = M.parse_pr_source_ref(fix_reconcile.source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("fix_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "fix-reconcile",
    "comment",
    tostring(fix_reconcile.dedup_key),
  }), fix_reconcile.source_ref)
end

function M.build_review_reconcile_comment_request(repo, issue_number, review_reconcile, action, reason)
  local version = M.review_reconcile_state_version(review_reconcile.issue_version, review_reconcile.round)
  local marker = M.review_reconcile_marker(review_reconcile.proposal_id, review_reconcile.issue_version, review_reconcile.round, action)
  local state_marker = M.state_marker(review_reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = M.parse_pr_source_ref(review_reconcile.source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("review_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-reconcile",
    "comment",
    tostring(review_reconcile.dedup_key),
  }), review_reconcile.source_ref)
end
end

return S
