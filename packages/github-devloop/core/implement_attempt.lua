local S = {}

function S.install(M)

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

function M.implement_version_mismatch_marker(proposal_id, expected_version, current_version, attempt)
  local n = tonumber(attempt)
  if n == nil or n < 1 or n ~= math.floor(n) then
    error("github-devloop: invalid implement version mismatch attempt")
  end
  return '<!-- fkst:github-devloop:implement-version-mismatch:v1 proposal="' .. tostring(proposal_id)
    .. '" key="' .. M.implement_version_mismatch_key(expected_version, current_version)
    .. '" attempt="' .. tostring(n)
    .. '" -->'
end

function M.latest_implement_version_mismatch_fact(comments, proposal_id, expected_version, current_version)
  if type(comments) ~= "table" then
    return nil
  end
  local expected_key = M.implement_version_mismatch_key(expected_version, current_version)
  local marker_pattern = "<!%-%- fkst:github%-devloop:implement%-version%-mismatch:v1.-%-%->"
  local latest = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_key = marker:match('key="([^"]+)"')
      local attempt = tonumber(marker:match('attempt="(%d+)"'))
      if marker_proposal == proposal_id
        and marker_key == expected_key
        and attempt ~= nil
        and attempt >= 1
        and (latest == nil or attempt > latest.attempt) then
        latest = {
          proposal_id = marker_proposal,
          key = marker_key,
          attempt = attempt,
        }
      end
    end
  end
  return latest
end

function M.implement_version_mismatch_attempt_count(comments, proposal_id, expected_version, current_version)
  local fact = M.latest_implement_version_mismatch_fact(comments, proposal_id, expected_version, current_version)
  return fact and fact.attempt or 0
end

end

return S
