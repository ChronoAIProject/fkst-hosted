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

local function epoch_seconds(value)
  local n = tonumber(value)
  if n == nil then
    return nil
  end
  if n > 100000000000000 then
    n = n / 1000000
  elseif n > 100000000000 then
    n = n / 1000
  end
  return math.floor(n)
end

local function format_started_at(started_at)
  local n = epoch_seconds(started_at)
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
  local started = epoch_seconds(started_at)
  local finished = epoch_seconds(finished_at) or now()
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

function M.work_card_run_id(parts)
  if type(parts) ~= "table" then
    error("github-devloop: invalid work card run id")
  end
  return M._dedup_key(parts)
end

function M.work_card_marker(proposal_id, run_id)
  if run_id == nil or tostring(run_id) == "" then
    error("github-devloop: invalid work card run id")
  end
  return '<!-- fkst:github-devloop:work-card:v1 proposal="' .. tostring(proposal_id)
    .. '" run_id="' .. tostring(run_id)
    .. '" -->'
end

function M.implement_attempt_marker(proposal_id, dedup_key, attempt, started_at)
  local n = tonumber(attempt)
  if n == nil or n < 1 or n ~= math.floor(n) then
    error("github-devloop: invalid implement attempt")
  end
  return '<!-- fkst:github-devloop:implement-attempt:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" attempt="' .. tostring(n)
    .. '" started_at="' .. tostring(started_at or "")
    .. '" -->'
end

function M.latest_implement_attempt_fact(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:implement%-attempt:v1.-%-%->"
  local latest = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local attempt = tonumber(marker:match('attempt="(%d+)"'))
      local started_at = marker:match('started_at="([^"]*)"')
      if marker_proposal == proposal_id
        and marker_dedup == tostring(dedup_key)
        and attempt ~= nil
        and attempt >= 1
        and (latest == nil or attempt > latest.attempt) then
        latest = {
          proposal_id = marker_proposal,
          dedup_key = marker_dedup,
          attempt = attempt,
          started_at = started_at,
        }
      end
    end
  end
  return latest
end

function M.implement_attempt_count(comments, proposal_id, dedup_key)
  local fact = M.latest_implement_attempt_fact(comments, proposal_id, dedup_key)
  return fact and fact.attempt or 0
end

function M.build_work_card_comment_request(target, card)
  if type(target) ~= "table" or card == nil or not is_supported_role(card.role) then
    error("github-devloop: invalid work card request")
  end
  if card.run_id == nil or tostring(card.run_id) == "" then
    error("github-devloop: missing work card run id")
  end
  local run_id = tostring(card.run_id)
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
  table.insert(lines, M.work_card_marker(card.proposal_id, run_id))

  local source_ref = card.source_ref
  if source_ref == nil then
    source_ref = target.kind == "pr"
      and M.pr_source_ref(target.repo, target.number)
      or M.issue_source_ref(target.repo, target.number)
  end
  return M.build_entity_comment_request(target, table.concat(lines, "\n"), M._dedup_key({
    "work-card",
    tostring(card.proposal_id),
    run_id,
  }), source_ref, {
    replace_marker = M.work_card_marker(card.proposal_id, run_id),
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
