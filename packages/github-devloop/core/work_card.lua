local S = {}

function S.install(M)
local max_work_card_detail_len = 240

local function is_supported_role(role)
  return role == "implement" or role == "fix" or role == "review" or role == "review-meta"
end

local function compact_text(value, limit)
  local text = tostring(value or ""):gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  if #text > limit then
    text = M.truncate_utf8(text, limit)
  end
  return M.neutralize_untrusted_comment_text(text)
end

local function short_sha(value)
  local text = tostring(value or "")
  if M._is_git_sha(text) then
    return text:sub(1, 12)
  end
  return nil
end

local function format_started_at(started_at)
  local n = tonumber(started_at)
  if n ~= nil then
    return os.date("!%Y-%m-%dT%H:%M:%SZ", n)
  end
  local text = tostring(started_at or "")
  if text ~= "" then
    return text
  end
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now())
end

local function format_duration(started_at, finished_at)
  local started = tonumber(started_at)
  local finished = tonumber(finished_at) or now()
  if started == nil or finished < started then
    return nil
  end
  local seconds = math.floor(finished - started)
  if seconds < 60 then
    return tostring(seconds) .. "s"
  end
  local minutes = math.floor(seconds / 60)
  local rem = seconds % 60
  if minutes < 60 then
    return tostring(minutes) .. "m " .. tostring(rem) .. "s"
  end
  local hours = math.floor(minutes / 60)
  return tostring(hours) .. "h " .. tostring(minutes % 60) .. "m"
end

local function role_label(role)
  local key = "work_card_role_" .. tostring(role):gsub("%-", "_")
  return M.comment_string(key)
end

function M.work_card_marker(proposal_id)
  return '<!-- fkst:github-devloop:work-card:v1 proposal="' .. tostring(proposal_id) .. '" -->'
end

function M.build_work_card_comment_request(target, card)
  if type(target) ~= "table" or card == nil or not is_supported_role(card.role) then
    error("github-devloop: invalid work card request")
  end
  local started_at = format_started_at(card.started_at)
  local lines = {}
  local round = tonumber(card.round)
  local header = M.comment_string("work_card_running_prefix") .. role_label(card.role)
  if round ~= nil and round > 0 then
    header = header .. " " .. M.comment_string("work_card_round_open") .. tostring(round) .. M.comment_string("work_card_round_close")
  end
  table.insert(lines, header)
  table.insert(lines, M.comment_string("work_card_started_label") .. started_at)
  local baseline = short_sha(card.gate_baseline_sha or card.base_sha)
  if baseline ~= nil then
    table.insert(lines, M.comment_string("work_card_baseline_label") .. baseline)
  end
  local previous = compact_text(card.last_stage, max_work_card_detail_len)
  if previous ~= nil then
    table.insert(lines, M.comment_string("work_card_previous_label") .. previous)
  end
  local outcome = compact_text(card.outcome, max_work_card_detail_len)
  if outcome ~= nil then
    table.insert(lines, M.comment_string("work_card_outcome_label") .. outcome)
  end
  local duration = format_duration(card.started_at, card.finished_at)
  if duration ~= nil and outcome ~= nil then
    table.insert(lines, M.comment_string("work_card_duration_label") .. duration)
  end
  table.insert(lines, "")
  table.insert(lines, M.work_card_marker(card.proposal_id))

  local source_ref = card.source_ref
  if source_ref == nil then
    source_ref = target.kind == "pr"
      and M.pr_source_ref(target.repo, target.number)
      or M.issue_source_ref(target.repo, target.number)
  end
  local version = card.version or card.dedup_key or started_at
  return M.build_entity_comment_request(target, table.concat(lines, "\n"), M._dedup_key({
    "work-card",
    tostring(card.proposal_id),
    tostring(card.role),
    tostring(version),
    tostring(outcome or "running"),
  }), source_ref, {
    replace_marker = M.work_card_marker(card.proposal_id),
  })
end

function M.log_work_card(dept, proposal_id, queue, request)
  if M.write_mode() ~= "real" then
    return
  end
  M.log_raise(dept, proposal_id, queue, request)
end

end

return S
