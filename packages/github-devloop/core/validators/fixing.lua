return function(M)
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
end
