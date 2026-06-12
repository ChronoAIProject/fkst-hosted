local S = {}

function S.install(M)
function M.validate_proposal(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(proposal.proposal_id)
  if repo == nil or issue_number == nil then
    local review_repo, pr_number = M.parse_pr_review_proposal_id(proposal.proposal_id)
    if review_repo == nil or pr_number == nil then
      return false
    end
    if not M._is_path_safe_key(proposal.proposal_id, M._max_key_len) or not M._is_path_safe_key(proposal.dedup_key, M._max_dedup_len) then
      return false
    end
  else
    if not M.is_safe_proposal_ref(proposal.proposal_id, proposal.dedup_key) then
      return false
    end
  end
  if not M._is_bounded_string(proposal.title, M._max_title_len) then
    return false
  end
  if not M._is_bounded_string(proposal.body, M._max_body_len) then
    return false
  end
  if proposal.content_fetch ~= nil and not M._is_bounded_string(proposal.content_fetch, 4000) then
    return false
  end
  return M._has_bounded_source_ref(proposal.source_ref)
end
function M.is_supported_issue(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "issue"
    and payload.repo ~= nil
    and payload.number ~= nil
    and payload.title ~= nil
    and payload.updated_at ~= nil
    and M.issue_ref_round_trips(payload.repo, payload.number)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_pr(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "pr"
    and payload.repo ~= nil
    and M.is_safe_pr_number(payload.number)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_pr_opened(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-proxy.pr-opened.v1"
    or not M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M.is_safe_pr_number(payload.pr_number)
    or not M._is_bounded_string(payload.impl_version, M._max_dedup_len)
    or not M._is_git_ref_safe(payload.branch)
    or not M._is_git_sha(payload.head_sha)
    or not M._is_git_ref_safe(payload.base_branch)
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end
  local source_repo, source_pr = M.parse_pr_source_ref(payload.source_ref)
  local issue_repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return source_repo ~= nil
    and source_pr ~= nil
    and issue_repo ~= nil
    and issue_number ~= nil
    and tostring(source_repo) == tostring(payload.repo)
    and tostring(source_pr) == tostring(payload.pr_number)
    and tostring(issue_repo) == tostring(payload.repo)
    and tostring(issue_number) == tostring(payload.issue_number)
end

function M.is_supported_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and payload.decision == "approve"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and M._is_bounded_string(payload.body, M._max_body_len)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_review_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and (payload.decision == "approve" or payload.decision == "reject")
    and M.is_safe_pr_review_result_ref(payload.proposal_id, payload.dedup_key)
    and M._is_bounded_string(payload.body, M._max_body_len)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and (payload.blocking_gap == nil or M._is_bounded_string(payload.blocking_gap, M._max_blocking_gap_len))
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_unresolved(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_converge.v1"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and payload.body == nil
    and payload.angle_results == nil
    and payload.decision == nil
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_pr_review_unresolved(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_converge.v1"
    and M.is_safe_pr_review_result_ref(payload.proposal_id, payload.dedup_key)
    and payload.body == nil
    and payload.angle_results == nil
    and payload.decision == nil
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.ready.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and (payload.impl_retry_attempt == nil
      or (tonumber(payload.impl_retry_attempt) ~= nil
        and tonumber(payload.impl_retry_attempt) >= 1
        and tonumber(payload.impl_retry_attempt) == math.floor(tonumber(payload.impl_retry_attempt))
        and tonumber(payload.impl_retry_attempt) <= M._max_impl_retry_attempts))
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_reviewing(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.reviewing.v1"
    and M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_open_pr(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.open-pr.v1"
    or not M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M._is_bounded_string(payload.version, M._max_dedup_len)
    or not M._is_git_ref_safe(payload.branch)
    or not M._is_git_sha(payload.head_sha)
    or not M._is_git_ref_safe(payload.base_branch)
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end
  local repo, issue_number = M.parse_issue_source_ref(payload.source_ref)
  return repo ~= nil
    and issue_number ~= nil
    and tostring(repo) == tostring(payload.repo)
    and tostring(issue_number) == tostring(payload.issue_number)
    and tostring(payload.proposal_id) == M.proposal_id(repo, issue_number)
end

function M.is_supported_fixing(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.fixing.v1"
    or not M.is_safe_pr_number(payload.pr_number)
    or not M._is_bounded_string(payload.version, M._max_dedup_len)
    or not M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    or not M._is_git_sha(payload.reviewed_head_sha)
    or (payload.gate_baseline_sha ~= nil and not M._is_git_sha(payload.gate_baseline_sha))
    or (payload.gate_failure_excerpt ~= nil and not M._is_bounded_string(payload.gate_failure_excerpt, M._max_rollup_failure_summary_len))
    or (payload.framing ~= nil and not M._is_bounded_string(payload.framing, M._max_framing_len))
    or (payload.blocking_gap ~= nil and not M._is_bounded_string(payload.blocking_gap, M._max_blocking_gap_len))
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end

  if not M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key) then
    return false
  end
  if tostring(payload.dedup_key):sub(1, #"fixing/replay/") ~= "fixing/replay/" then
    return true
  end

  local replay_dedup = M._dedup_key({
    "fixing",
    "replay",
    tostring(payload.proposal_id),
    tostring(payload.version),
    tostring(payload.pr_number),
    tostring(payload.review_dedup_key),
    tostring(payload.gate_baseline_sha or "nobase"),
    tostring(payload.reviewed_head_sha),
  })
  return tostring(payload.dedup_key) == replay_dedup
end

function M.is_supported_review_meta(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.review-meta.v1"
    or not M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key) then
    return false
  end
  local has_valid_identity = payload.mode == "fix-reflection"
    and M.parse_entity_proposal_id(payload.proposal_id) ~= nil
    and M._is_path_safe_key(payload.dedup_key, M._max_dedup_len)
  if payload.mode ~= "fix-reflection" then
    has_valid_identity = M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
  end
  return has_valid_identity
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M.is_safe_pr_number(payload.pr_number)
    and tonumber(payload.n) ~= nil
    and (payload.mode == nil or payload.mode == "fix-reflection")
    and (payload.fix_round == nil or tonumber(payload.fix_round) ~= nil)
    and (payload.blocking_gap == nil or M._is_bounded_string(payload.blocking_gap, M._max_blocking_gap_len))
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_merge_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.merge-ready.v1"
    and M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    and M._is_git_sha(payload.reviewed_head_sha)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_intake_candidate(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.intake-candidate.v1"
    or not M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end
  local repo, issue_number = M.parse_issue_source_ref(payload.source_ref)
  return repo ~= nil
    and issue_number ~= nil
    and tostring(repo) == tostring(payload.repo)
    and tostring(issue_number) == tostring(payload.issue_number)
    and tostring(payload.proposal_id) == M.proposal_id(repo, issue_number)
end
end

return S
