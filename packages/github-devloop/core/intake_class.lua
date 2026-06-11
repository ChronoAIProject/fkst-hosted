local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

local function normalized_class_reason(reason)
  local text = tostring(reason or ""):lower()
  text = text:gsub("#%d+", " ")
  text = text:gsub("%f[%w]cites?%f[^%w]", " ")
  text = text:gsub("%f[%w]prior%f[^%w]", " ")
  text = text:gsub("%f[%w]siblings?%f[^%w]", " ")
  text = text:gsub("%f[%w]rule%f[^%w]%s+%f[%w]of%f[^%w]%s+%f[%w]three%f[^%w]", " ")
  text = text:gsub("%f[%w]requires?%f[^%w]", " ")
  text = text:gsub("%f[%w]class%f[^%w]%s*%-?%s*%f[%w]level%f[^%w]", "class-level")
  text = text:gsub("[^%w%-]+", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  return text
end

function M.intake_class_identity(reason, current, issue_number)
  local seen = {}
  local siblings = {}
  for number in tostring(reason or ""):gmatch("#(%d+)") do
    local normalized = tostring(tonumber(number))
    if normalized ~= "nil"
      and normalized ~= tostring(issue_number or "")
      and seen[normalized] == nil then
      seen[normalized] = true
      table.insert(siblings, tonumber(normalized))
    end
  end
  table.sort(siblings)
  if #siblings >= 2 then
    local reason_key = normalized_class_reason(reason)
    if reason_key ~= "" then
      return "class:" .. reason_key
    end
  end
  local title_key = tostring(current and current.title or ("Issue #" .. tostring(issue_number or "unknown")))
  title_key = title_key:lower():gsub("[^%w]+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if title_key == "" then
    title_key = "issue-" .. tostring(issue_number or "unknown")
  end
  return "title:" .. title_key
end

local function class_identity_label(class_key)
  local class = tostring(class_key or ""):match("^class:(.+)$")
  if class ~= nil and class ~= "" then
    return "recurring class " .. class:gsub("%-", " ")
  end
  local title = tostring(class_key or ""):match("^title:(.+)$")
  return title or tostring(class_key or "unknown")
end

function M.intake_class_carrier_marker(class_key)
  if class_key == nil or tostring(class_key) == "" then
    error("github-devloop: invalid intake class key")
  end
  return '<!-- fkst:github-devloop:intake-class-carrier:v1 class_key="' .. tostring(class_key) .. '" -->'
end

function M.intake_class_issue_title(current, issue_number, class_key)
  local source_title = tostring(current and current.title or ("Issue #" .. tostring(issue_number or "unknown")))
  local title = "Class fix needed: " .. class_identity_label(class_key or ("title:" .. source_title))
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  return title
end

function M.find_open_intake_class_carrier(repo, issue_number, current, class_key)
  local wanted_marker = M.intake_class_carrier_marker(class_key)
  local wanted_title = M.intake_class_issue_title(current, issue_number, class_key)
  local fallback_title = M.intake_class_issue_title(current, issue_number)
  local listed = M.gh_exec({ cmd = M.gh_issue_list_intake_cmd(repo, 100), timeout = 30 })
  if listed.exit_code ~= 0 then
    error("github-devloop: gh issue intake class lookup failed: " .. tostring(listed.stderr))
  end
  for _, issue in ipairs(M.parse_issue_list_intake(listed.stdout)) do
    if tostring(issue.number) ~= tostring(issue_number)
      and (tostring(issue.body or ""):find(wanted_marker, 1, true) ~= nil
        or tostring(issue.title or "") == wanted_title
        or tostring(issue.title or "") == fallback_title) then
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

function M.build_intake_class_issue_create_request(repo, issue_number, candidate, current, reason, class_key)
  local title = M.intake_class_issue_title(current, issue_number, class_key)
  local body = "Class escalation follow-through for instance issue #" .. tostring(issue_number or "unknown")
    .. "\n\nReason:\n" .. M.neutralize_untrusted_comment_text(reason or "")
    .. "\n\nClass identity: " .. tostring(class_key or "")
    .. "\n\nRequired follow-through:\n"
    .. "- Locate or create the class-level fix intent-before-create.\n"
    .. "- Link this instance to the class issue through the parent ledger marker.\n"
    .. "- Close the instance as folded only after the class carrier exists, or keep it enabled as the class carrier if it already states the class solution.\n"
    .. "\nSource proposal: " .. tostring(candidate and candidate.proposal_id or "")
    .. "\n\n" .. M.intake_class_carrier_marker(class_key)
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
      "intake-class",
      tostring(class_key or ""),
    }),
    parent_comment_target = {
      repo = repo,
      issue_number = issue_number,
    },
    source_ref = M.normalize_source_ref(candidate and candidate.source_ref),
  }
end

end

return S
