local S = {}

function S.install(M)
local threshold = 3
local window_seconds = 24 * 60 * 60

local function triage_window_key(now_seconds)
  return "window-" .. tostring(math.floor((tonumber(now_seconds) or now()) / window_seconds))
end

local function normalized_fact(payload)
  if type(payload) ~= "table" then
    return nil, "payload-not-table"
  end

  local source = payload
  if type(payload.payload) == "table" then
    source = payload.payload
  end

  local queue = source.queue or payload.queue
  if not M._is_bounded_string(queue, M._max_key_len) then
    return nil, "missing-queue"
  end

  local fingerprint = source.fingerprint or payload.fingerprint
  if not M._is_bounded_string(fingerprint, M._max_key_len) then
    return nil, "missing-fingerprint"
  end

  local source_ref = source.source_ref or payload.source_ref
  if type(source_ref) ~= "table" then
    return nil, "missing-source-ref"
  end

  local normalized_source_ref = M.normalize_source_ref(source_ref)
  local repo, issue_number = M.parse_issue_source_ref(normalized_source_ref)
  local parent_target
  if repo ~= nil then
    parent_target = {
      repo = repo,
      issue_number = tostring(issue_number),
    }
  else
    local pr_repo, pr_number = M.parse_pr_source_ref(normalized_source_ref)
    if pr_repo == nil then
      return nil, "source-ref-not-issue-or-pr"
    end
    repo = pr_repo
    parent_target = {
      repo = pr_repo,
      pr_number = tostring(pr_number),
    }
  end

  local attempt = tonumber(source.attempt or payload.attempt or 1)
  if attempt == nil or attempt < 1 or attempt % 1 ~= 0 then
    return nil, "invalid-attempt"
  end

  return {
    schema = tostring(source.schema or payload.schema or ""),
    queue = tostring(queue),
    dept = tostring(source.dept or payload.dept or ""),
    error_class = M.error_fact_class({ error_class = source.error_class or payload.error_class }),
    fingerprint = tostring(fingerprint),
    source_ref = normalized_source_ref,
    source_repo = repo,
    parent_target = parent_target,
    attempt = attempt,
    terminal = (source.terminal or payload.terminal) == true,
    message = tostring(source.message or source.error or payload.error or ""),
    delivery_id = tostring(payload.delivery_id or source.delivery_id or ""),
    dead_queue = tostring(payload.queue or ""),
  }, nil
end

function M.failure_triage_dedup_key(repo, fingerprint)
  return M._dedup_key({
    "failure-triage",
    M.safe_repo(repo),
    tostring(fingerprint or "unknown"),
  })
end

local function fact_count_key(repo, fingerprint)
  return M.failure_triage_count_key(repo, fingerprint, triage_window_key())
end

function M.failure_triage_count_key(repo, fingerprint, window_key)
  return M._dedup_key({
    "failure-triage-count",
    M.safe_repo(repo),
    tostring(fingerprint or "unknown"),
    tostring(window_key or triage_window_key()),
  })
end

local function seen_key(repo, fingerprint)
  return M._dedup_key({
    "failure-triage-seen",
    M.safe_repo(repo),
    tostring(fingerprint or "unknown"),
  })
end

local function threshold_key(repo, fingerprint, window_key)
  return M._dedup_key({
    "failure-triage-threshold",
    M.safe_repo(repo),
    tostring(fingerprint or "unknown"),
    tostring(window_key or triage_window_key()),
  })
end

local function recorded_count(repo, fingerprint)
  local raw = cache_get(fact_count_key(repo, fingerprint))
  return tonumber(raw) or 0
end

local function record_count(repo, fingerprint, count)
  cache_set(fact_count_key(repo, fingerprint), tostring(count))
end

local function first_seen(repo, fingerprint)
  local key = seen_key(repo, fingerprint)
  if cache_get(key) == "1" then
    return false
  end
  cache_set(key, "1")
  return true
end

local function claim_threshold(repo, fingerprint, window_key)
  local key = threshold_key(repo, fingerprint, window_key)
  if cache_get(key) == "1" then
    return false
  end
  cache_set(key, "1")
  return true
end

local function title(fact)
  local result = "Investigate L2 failure: " .. tostring(fact.error_class) .. " in " .. tostring(fact.queue)
  if #result > M._max_title_len then
    result = M.truncate_utf8(result, M._max_title_len)
  end
  return result
end

local function body(fact, count)
  local lines = {
    "L2 failure triage filed this issue from an existing structured dead-letter fact.",
    "",
    "Contract facts:",
    "- `error_class`: `" .. tostring(fact.error_class) .. "`",
    "- `fingerprint`: `" .. tostring(fact.fingerprint) .. "`",
    "- `source_ref`: `" .. tostring(fact.source_ref.kind) .. ":" .. tostring(fact.source_ref.ref) .. "`",
    "- `attempt`: `" .. tostring(fact.attempt) .. "`",
    "- `terminal`: `" .. tostring(fact.terminal) .. "`",
    "",
    "Delivery context:",
    "- `queue`: `" .. tostring(fact.queue) .. "`",
    "- `dead_queue`: `" .. tostring(fact.dead_queue) .. "`",
    "- `dept`: `" .. tostring(fact.dept) .. "`",
    "- `delivery_id`: `" .. tostring(fact.delivery_id) .. "`",
    "- `observed_count`: `" .. tostring(count) .. "`",
    "",
    "Requested outcome:",
    "- Diagnose the structural cause behind this failure fingerprint.",
    "- Implement any fix through the normal issue -> PR -> review -> merge pipeline.",
    "- Do not mutate runtime state directly from this triage path.",
  }
  if fact.message ~= "" then
    table.insert(lines, "")
    table.insert(lines, "Failure summary:")
    table.insert(lines, M.neutralize_untrusted_comment_text(fact.message))
  end
  local result = table.concat(lines, "\n")
  if #result > M._max_body_len then
    result = M.truncate_utf8(result, M._max_body_len)
  end
  return result
end

function M.build_failure_triage_issue_create_request(fact, count)
  if type(fact) ~= "table" then
    error("github-devloop: failure triage fact is required")
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = fact.source_repo,
    title = title(fact),
    body = body(fact, count or 1),
    labels = json.decode("[]"),
    dedup_key = M.failure_triage_dedup_key(fact.source_repo, fact.fingerprint),
    parent_comment_target = fact.parent_target,
    source_ref = fact.source_ref,
  }
end

function M.failure_triage_decision(payload)
  local fact, reason = normalized_fact(payload)
  if fact == nil then
    return { action = "skip", reason = reason }
  end

  local count = recorded_count(fact.source_repo, fact.fingerprint) + 1
  record_count(fact.source_repo, fact.fingerprint, count)
  local window_key = triage_window_key()
  local is_new = first_seen(fact.source_repo, fact.fingerprint)
  local threshold_crossed = count >= threshold and claim_threshold(fact.source_repo, fact.fingerprint, window_key)
  if not is_new and not fact.terminal and not threshold_crossed then
    return {
      action = "suppress",
      reason = "below-threshold",
      fact = fact,
      count = count,
      threshold = threshold,
    }
  end

  return {
    action = "raise",
    fact = fact,
    count = count,
    threshold = threshold,
    reason = is_new and "new-fingerprint" or (fact.terminal and "terminal-fact" or "threshold-crossed"),
    request = M.build_failure_triage_issue_create_request(fact, count),
  }
end

end

return S
