local S = {}

function S.install(M)

function M.implement_exec_ref(proposal_id, dedup_key)
  return M._dedup_key({
    "implement-exec",
    tostring(proposal_id or ""),
    tostring(dedup_key or ""),
    "implement",
  })
end

local function codex_runs_status()
  local function one_line(value)
    if type(M._one_line) == "function" then
      return M._one_line(value)
    end
    return tostring(value or ""):gsub("[%s\r\n]+", " ")
  end
  local function fallback(reason)
    if type(M.log_line) == "function" then
      M.log_line("warn", "liveness", "github-devloop/codex-runs", "CODEX_RUNS", {
        "outcome=fallback-to-marker-budget",
        "error_class=codex-runs-unavailable",
        "reason=" .. one_line(reason),
      })
    end
    return { running = {}, recent = {}, codex_runs_fallback = true, codex_runs_error = tostring(reason or "unknown") }
  end
  if type(fkst) ~= "table" or type(fkst.codex_runs) ~= "function" then
    return fallback("fkst.codex_runs SDK primitive is unavailable")
  end
  local ok, status = pcall(fkst.codex_runs)
  if not ok then
    return fallback("fkst.codex_runs failed for implement liveness: " .. tostring(status))
  end
  if type(status) ~= "table" or type(status.running) ~= "table" then
    return fallback("fkst.codex_runs returned invalid implement liveness status")
  end
  return status
end

function M.implement_exec_ref_running(exec_ref, status)
  if type(exec_ref) ~= "string" or exec_ref == "" then
    return false
  end
  local runs = status or codex_runs_status()
  for _, run in ipairs(runs.running or {}) do
    if type(run) == "table"
      and tostring(run.status or "running") == "running"
      and tostring(run.role or "") == "implement"
      and M.implement_exec_ref(run.proposal_id, run.dedup_key) == exec_ref then
      return true
    end
  end
  return false
end

function M.implement_attempt_marker(proposal_id, dedup_key, attempt, started_at, exec_ref)
  local n = tonumber(attempt)
  if n == nil or n < 1 or n ~= math.floor(n) then
    error("github-devloop: invalid implement attempt")
  end
  local marker = '<!-- fkst:github-devloop:implement-attempt:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" attempt="' .. tostring(n)
    .. '" started_at="' .. tostring(started_at or "")
    .. '"'
  if exec_ref ~= nil and exec_ref ~= "" then
    marker = marker .. ' exec_ref="' .. tostring(exec_ref) .. '"'
  end
  return marker .. " -->"
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
      local exec_ref = marker:match('exec_ref="([^"]*)"')
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
          exec_ref = exec_ref,
        }
      end
    end
  end
  return latest
end

function M.implement_attempt_exec_live_fact(comments, proposal_id, dedup_key)
  local attempt = M.latest_implement_attempt_fact(comments, proposal_id, dedup_key)
  if attempt == nil then
    return nil, "missing-implement-attempt"
  end
  if type(attempt.exec_ref) ~= "string" or attempt.exec_ref == "" then
    return attempt, "missing-exec-ref"
  end
  if M.implement_exec_ref_running(attempt.exec_ref) then
    return attempt, "running"
  end
  return attempt, "not-running"
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
