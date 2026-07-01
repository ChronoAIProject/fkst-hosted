local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local error_facts = require("contract.error_facts")
local strings = require("contract.strings")
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
  if not strings.is_bounded_string(queue, M._max_key_len) then
    return nil, "missing-queue"
  end

  local fingerprint = source.fingerprint or payload.fingerprint
  if not strings.is_bounded_string(fingerprint, M._max_key_len) then
    return nil, "missing-fingerprint"
  end

  local source_ref = source.source_ref or payload.source_ref
  if type(source_ref) ~= "table" then
    return nil, "missing-source-ref"
  end

  local attempt = tonumber(source.attempt or payload.attempt or 1)
  if attempt == nil or attempt < 1 or attempt % 1 ~= 0 then
    return nil, "invalid-attempt"
  end

  local normalized_source_ref
  if source_ref.kind == "cron" and tostring(source_ref.ref or "") == "" then
    normalized_source_ref = { kind = "cron", ref = "" }
  else
    normalized_source_ref = base_ids.normalize_source_ref(source_ref)
  end
  local repo, issue_number = devloop_base.parse_issue_source_ref(normalized_source_ref)
  local parent_target
  if repo ~= nil then
    parent_target = {
      repo = repo,
      issue_number = tostring(issue_number),
    }
  else
    local pr_repo, pr_number = devloop_base.parse_pr_source_ref(normalized_source_ref)
    if pr_repo == nil then
      return {
        schema = tostring(source.schema or payload.schema or ""),
        queue = tostring(queue),
        dept = tostring(source.dept or payload.dept or ""),
        error_class = M.error_fact_class({ error_class = source.error_class or payload.error_class }),
        fingerprint = tostring(fingerprint),
        source_ref = normalized_source_ref,
        attempt = attempt,
        terminal = (source.terminal or payload.terminal) == true,
        message = tostring(source.message or source.error or payload.error or ""),
        delivery_id = tostring(payload.delivery_id or source.delivery_id or ""),
        dead_queue = tostring(payload.queue or ""),
        no_issue_parent = true,
      }, nil
    end
    repo = pr_repo
    parent_target = {
      repo = pr_repo,
      pr_number = tostring(pr_number),
    }
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
  return base_ids.dedup_key({
    "failure-triage",
    base_ids.safe_repo(repo),
    tostring(fingerprint or "unknown"),
  })
end

local function fact_count_key(repo, fingerprint)
  return M.failure_triage_count_key(repo, fingerprint, triage_window_key())
end

function M.failure_triage_count_key(repo, fingerprint, window_key)
  return base_ids.dedup_key({
    "failure-triage-count",
    base_ids.safe_repo(repo),
    tostring(fingerprint or "unknown"),
    tostring(window_key or triage_window_key()),
  })
end

local function seen_key(repo, fingerprint)
  return base_ids.dedup_key({
    "failure-triage-seen",
    base_ids.safe_repo(repo),
    tostring(fingerprint or "unknown"),
  })
end

local function threshold_key(repo, fingerprint, window_key)
  return base_ids.dedup_key({
    "failure-triage-threshold",
    base_ids.safe_repo(repo),
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

local function display_text(value, limit)
  local text = devloop_base.neutralize_untrusted_comment_text(error_facts.one_line(value))
  text = text:gsub("`", "'"):gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    text = "unknown"
  end
  if limit ~= nil and #text > limit then
    text = base_ids.truncate_utf8(text, limit)
  end
  return text
end

local function display_source_ref(source_ref)
  local value = tostring(source_ref and source_ref.kind or "") .. ":" .. tostring(source_ref and source_ref.ref or "")
  return display_text(value, M._max_key_len * 2 + 1)
end

local function title(fact)
  local result = "Investigate L2 failure: " .. display_text(fact.error_class, M._max_key_len)
    .. " in " .. display_text(fact.queue, M._max_key_len)
  if #result > M._max_title_len then
    result = base_ids.truncate_utf8(result, M._max_title_len)
  end
  return result
end

local function body(fact, count)
  local lines = {
    "L2 failure triage filed this issue from an existing structured dead-letter fact.",
    "",
    "Contract facts:",
    "- `error_class`: `" .. display_text(fact.error_class, M._max_key_len) .. "`",
    "- `fingerprint`: `" .. display_text(fact.fingerprint, M._max_key_len) .. "`",
    "- `source_ref`: `" .. display_source_ref(fact.source_ref) .. "`",
    "- `attempt`: `" .. display_text(fact.attempt, M._max_key_len) .. "`",
    "- `terminal`: `" .. display_text(fact.terminal, M._max_key_len) .. "`",
    "",
    "Delivery context:",
    "- `queue`: `" .. display_text(fact.queue, M._max_key_len) .. "`",
    "- `dead_queue`: `" .. display_text(fact.dead_queue, M._max_key_len) .. "`",
    "- `dept`: `" .. display_text(fact.dept, M._max_key_len) .. "`",
    "- `delivery_id`: `" .. display_text(fact.delivery_id, M._max_key_len) .. "`",
    "- `observed_count`: `" .. display_text(count, M._max_key_len) .. "`",
    "",
    "Requested outcome:",
    "- Diagnose the structural cause behind this failure fingerprint.",
    "- Implement any fix through the normal issue -> PR -> review -> merge pipeline.",
    "- Do not mutate runtime state directly from this triage path.",
  }
  if fact.message ~= "" then
    table.insert(lines, "")
    table.insert(lines, "Failure summary:")
    table.insert(lines, devloop_base.neutralize_untrusted_comment_text(fact.message))
  end
  local result = table.concat(lines, "\n")
  if #result > M._max_body_len then
    result = base_ids.truncate_utf8(result, M._max_body_len)
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
  if fact.no_issue_parent == true then
    return { action = "skip", reason = "no-issue-parent", fact = fact }
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
