local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

function M.intake_class_issue_title(current, issue_number)
  local source_title = tostring(current and current.title or ("Issue #" .. tostring(issue_number or "unknown")))
  local title = "Class fix needed: " .. source_title
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  return title
end

function M.find_open_intake_class_carrier(repo, issue_number, current)
  local wanted_title = M.intake_class_issue_title(current, issue_number)
  local listed = M.gh_exec({ cmd = M.gh_issue_list_intake_cmd(repo, 100), timeout = 30 })
  if listed.exit_code ~= 0 then
    error("github-devloop: gh issue intake class lookup failed: " .. tostring(listed.stderr))
  end
  for _, issue in ipairs(M.parse_issue_list_intake(listed.stdout)) do
    if tostring(issue.number) ~= tostring(issue_number)
      and tostring(issue.title or "") == wanted_title then
      return issue
    end
  end
  return nil
end

function M.intake_class_followup_marker(proposal_id, carrier_number, outcome, dedup_key)
  if outcome ~= "folded" and outcome ~= "carrier" then
    error("github-devloop: invalid intake class follow-up outcome")
  end
  if carrier_number == nil or tostring(carrier_number) == "" then
    error("github-devloop: invalid intake class follow-up carrier")
  end
  return '<!-- fkst:github-devloop:intake-class-followup:v1 proposal="' .. tostring(proposal_id)
    .. '" carrier="' .. tostring(carrier_number)
    .. '" outcome="' .. tostring(outcome)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.build_intake_class_followup_comment_request(repo, issue_number, candidate, carrier, outcome, reason)
  local carrier_number = carrier and carrier.number or "pending-create"
  local marker = M.intake_class_followup_marker(candidate.proposal_id, carrier_number, outcome, candidate.dedup_key)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  if safe_reason == "" then
    safe_reason = M.comment_string("no_reason_provided")
  end
  if #safe_reason > M._max_meta_reason_len then
    safe_reason = M.truncate_utf8(safe_reason, M._max_meta_reason_len)
  end
  local carrier_line = "Class carrier: "
  if carrier and carrier.number ~= nil then
    carrier_line = carrier_line .. "#" .. tostring(carrier.number)
  else
    carrier_line = carrier_line .. "pending intent-before-create"
  end
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop intake class follow-up: " .. tostring(outcome)
    .. "\n\n" .. carrier_line
    .. "\n\nReason:\n" .. safe_reason
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "intake-class",
    "followup",
    tostring(candidate.proposal_id),
    tostring(candidate.dedup_key),
    tostring(outcome),
    tostring(carrier_number),
  }), candidate.source_ref)
end

function M.build_intake_class_folded_label_request(repo, issue_number, candidate)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "intake-class",
      "label",
      "folded",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    candidate.source_ref
  )
end

end

return S
