local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local requests_labels = require("devloop.requests.labels")
local parsers_issue = require("devloop.parsers.issue")
local S = {}
local comment_strings = require("devloop.strings")

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

local stable_class_label_prefixes = {
  { prefix = "fingerprint:", rank = 1 },
  { prefix = "root-cause:", rank = 2 },
  { prefix = "problem:", rank = 3 },
  { prefix = "error-class:", rank = 4 },
}

local function slug_label_value(value)
  local text = tostring(value or ""):lower()
  text = text:gsub("[^%w%-]+", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if text == "" then
    return nil
  end
  return text
end

local function stable_class_label_key(label)
  local text = tostring(label or "")
  local lower = text:lower()
  for _, entry in ipairs(stable_class_label_prefixes) do
    if lower:sub(1, #entry.prefix) == entry.prefix then
      local value = slug_label_value(text:sub(#entry.prefix + 1))
      if value ~= nil then
        return entry.prefix .. value, entry.rank
      end
    end
  end
  return nil, nil
end

local function cited_sibling_set(reason, issue_number)
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
  return seen, siblings
end

local function sorted_stable_label_keys(labels)
  local by_key = {}
  local keys = {}
  for _, label in ipairs(labels or {}) do
    local key, rank = stable_class_label_key(label)
    if key ~= nil and by_key[key] == nil then
      by_key[key] = rank
      table.insert(keys, key)
    end
  end
  table.sort(keys, function(a, b)
    if by_key[a] ~= by_key[b] then
      return by_key[a] < by_key[b]
    end
    return a < b
  end)
  return keys
end

local function shared_sibling_class_key(reason, issue_number, sibling_issues)
  local cited, siblings = cited_sibling_set(reason, issue_number)
  if #siblings < 2 or type(sibling_issues) ~= "table" then
    return nil
  end
  local counts = {}
  local ranks = {}
  for _, issue in ipairs(sibling_issues) do
    if cited[tostring(issue.number)] then
      local seen_for_issue = {}
      for _, key in ipairs(sorted_stable_label_keys(issue.labels)) do
        if not seen_for_issue[key] then
          local _, rank = stable_class_label_key(key)
          counts[key] = (counts[key] or 0) + 1
          ranks[key] = rank or 999
          seen_for_issue[key] = true
        end
      end
    end
  end
  local candidates = {}
  for key, count in pairs(counts) do
    if count >= 2 then
      table.insert(candidates, key)
    end
  end
  table.sort(candidates, function(a, b)
    if ranks[a] ~= ranks[b] then
      return ranks[a] < ranks[b]
    end
    return a < b
  end)
  return candidates[1]
end

function M.intake_class_identity(reason, current, issue_number, sibling_issues)
  local shared_key = shared_sibling_class_key(reason, issue_number, sibling_issues)
  if shared_key ~= nil then
    return shared_key
  end
  local current_keys = sorted_stable_label_keys(current and current.labels)
  if current_keys[1] ~= nil then
    return current_keys[1]
  end
  return nil
end

local function class_identity_label(class_key)
  local class = tostring(class_key or ""):match("^class:(.+)$")
  if class ~= nil and class ~= "" then
    return "recurring class " .. class:gsub("%-", " ")
  end
  local stable_key = stable_class_label_key(class_key)
  local stable = stable_key and stable_key:match("^[%w%-]+:(.+)$")
  if stable ~= nil and stable ~= "" then
    return "recurring class " .. stable:gsub("%-", " ")
  end
  local title = tostring(class_key or ""):match("^title:(.+)$")
  return title or tostring(class_key or "unknown")
end

function M.fetch_recent_closed_intake_class_issues(repo)
  local listed = M.gh_issue_list_recent_closed(repo, 30, 30)
  if listed.exit_code ~= 0 then
    error("github-devloop: gh issue intake class sibling lookup failed: " .. tostring(listed.stderr))
  end
  return parsers_issue.parse_issue_list_intake(M, listed.stdout)
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
    title = base_ids.truncate_utf8(title, M._max_title_len)
  end
  return title
end

function M.find_open_intake_class_carrier(repo, issue_number, current, class_key)
  local wanted_marker = M.intake_class_carrier_marker(class_key)
  local wanted_title = M.intake_class_issue_title(current, issue_number, class_key)
  local fallback_title = M.intake_class_issue_title(current, issue_number)
  local listed = M.gh_issue_list_intake(repo, 100, 30)
  if listed.exit_code ~= 0 then
    error("github-devloop: gh issue intake class lookup failed: " .. tostring(listed.stderr))
  end
  for _, issue in ipairs(parsers_issue.parse_issue_list_intake(M, listed.stdout)) do
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
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  if safe_reason == "" then
    safe_reason = comment_strings.comment_string(M, "no_reason_provided")
  end
  if #safe_reason > M._max_meta_reason_len then
    safe_reason = base_ids.truncate_utf8(safe_reason, M._max_meta_reason_len)
  end
  local carrier_line = "Class carrier: "
  if carrier and carrier.number ~= nil then
    carrier_line = carrier_line .. "#" .. tostring(carrier.number)
  else
    carrier_line = carrier_line .. "pending intent-before-create"
  end
  return entity_lib.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop intake class follow-up: " .. tostring(outcome)
    .. "\n\n" .. carrier_line
    .. "\n\nReason:\n" .. safe_reason
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, base_ids.dedup_key({
    "intake-class",
    "followup",
    tostring(candidate.proposal_id),
    tostring(candidate.dedup_key),
    tostring(outcome),
    tostring(carrier_number),
  }), candidate.source_ref)
end

function M.build_intake_class_folded_label_request(repo, issue_number, candidate)
  return requests_labels.build_state_label_request(M,
    repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
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
    .. "\n\nReason:\n" .. devloop_base.neutralize_untrusted_comment_text(reason or "")
    .. "\n\nClass identity: " .. tostring(class_key or "")
    .. "\n\nRequired follow-through:\n"
    .. "- Locate or create the class-level fix intent-before-create.\n"
    .. "- Link this instance to the class issue through the parent ledger marker.\n"
    .. "- Close the instance as folded only after the class carrier exists, or keep it enabled as the class carrier if it already states the class solution.\n"
    .. "\nSource proposal: " .. tostring(candidate and candidate.proposal_id or "")
    .. "\n\n" .. M.intake_class_carrier_marker(class_key)
  if #body > M._max_body_len then
    body = base_ids.truncate_utf8(body, M._max_body_len)
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = base_ids.dedup_key({
      "intake-class",
      tostring(class_key or ""),
    }),
    parent_comment_target = {
      repo = repo,
      issue_number = issue_number,
    },
    source_ref = base_ids.normalize_source_ref(candidate and candidate.source_ref),
  }
end

end

return S
