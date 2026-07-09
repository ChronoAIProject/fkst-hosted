local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local devloop_entity = require("devloop.entity")
local error_facts = require("contract.error_facts")
local forge_validators = require("devloop.forge_validators")
local parsers_misc = require("devloop.parsers.misc")
local S = {}
local check_runs = require("forge.github.check_runs")
local contract_time = require("contract.time")
local config = require("devloop.config")

local observe_sample_marker_name = "fkst:github-devloop-integration:rollup-observe-sample:v1"
local observe_sample_replace_marker = "<!-- " .. observe_sample_marker_name
local observe_sample_marker_pattern = "<!%-%-%s*" .. observe_sample_marker_name:gsub("%-", "%%-") .. ".-%-%->"

local function bool_value(value)
  if value == true then
    return true
  end
  return type(value) == "string" and value:lower() == "true"
end

local function int_value(value)
  local parsed = tonumber(value)
  if parsed == nil then
    return 0
  end
  return math.floor(parsed)
end

local function list_from_any(value)
  if type(value) ~= "table" then
    return {}
  end
  local result = {}
  local max_index = 0
  for key, item in pairs(value) do
    if type(key) == "number" and key > max_index then
      max_index = key
    end
    table.insert(result, item)
  end
  if max_index > 0 then
    result = {}
    for index = 1, max_index do
      if value[index] ~= nil then
        table.insert(result, value[index])
      end
    end
  end
  return result
end

local function first_non_empty_list_from_keys(snapshot, keys)
  if type(snapshot) ~= "table" then
    return {}
  end
  for _, key in ipairs(keys or {}) do
    local values = list_from_any(snapshot[key])
    if #values > 0 then
      return values
    end
  end
  return {}
end

local function first_string(row, keys, default)
  if type(row) ~= "table" then
    return default or "-"
  end
  for _, key in ipairs(keys or {}) do
    local value = row[key]
    if value ~= nil then
      if type(value) == "table" then
        local nested = value.ref or value.id or value.kind
        if nested ~= nil then
          return tostring(nested)
        end
      else
        local text = tostring(value)
        if text ~= "" then
          return text
        end
      end
    end
  end
  return default or "-"
end

local function parse_time_seconds(value)
  if value == nil then
    return nil
  end
  if type(value) == "number" then
    if value > 10000000000 then
      return math.floor(value / 1000)
    end
    return math.floor(value)
  end
  return contract_time.iso_timestamp_epoch_seconds(value)
end

local function event_timestamp_seconds(event)
  if type(event) ~= "table" then
    return nil
  end
  for _, key in ipairs({ "ts", "time", "at", "observed_at", "updated_at", "created_at", "observed_at_ms", "event_ts" }) do
    local parsed = parse_time_seconds(event[key])
    if parsed ~= nil then
      return parsed
    end
  end
  return nil
end

local function event_sort_key(event)
  return event_timestamp_seconds(event) or -1
end

local function latest_entity_events(snapshot, now_seconds)
  local records = {}
  local raw_entities = first_non_empty_list_from_keys(snapshot, {
    "entities",
    "entity_timeline",
    "entity_timelines",
    "timelines",
  })
  for _, raw in ipairs(raw_entities) do
    if type(raw) == "table" then
      local events = list_from_any(raw.events or raw.timeline or raw.event_timeline)
      local latest_event = type(raw.latest_event) == "table" and raw.latest_event or nil
      if latest_event == nil and #events > 0 then
        for _, event in ipairs(events) do
          if type(event) == "table" and (latest_event == nil or event_sort_key(event) > event_sort_key(latest_event)) then
            latest_event = event
          end
        end
      end
      latest_event = latest_event or raw
      local ts = event_timestamp_seconds(latest_event)
      table.insert(records, {
        entity = first_string(raw, { "entity", "entity_id", "id", "source_ref", "ref", "key", "proposal" }),
        event = latest_event,
        dwell_seconds = ts ~= nil and ((tonumber(now_seconds) or now()) - ts) or nil,
        entity_terminal = bool_value(raw.terminal),
      })
    end
  end
  return records
end

local function expected_transient_event(event)
  if type(event) ~= "table" then
    return false
  end
  return event.disposition == "expected-transient"
    or event.outcome == "retry-pending"
    or event.outcome == "skip-foreign"
    or event.outcome == "deadline-defer"
    or event.error_class == "retry-pending"
    or event.error_class == "marker-lag"
end

local function failure_facts(snapshot)
  for _, key in ipairs({ "failure_facts", "failures", "error_facts" }) do
    local facts = list_from_any(snapshot[key])
    if #facts > 0 then
      return facts
    end
  end
  return {}
end

local function fact_queue(fact)
  return first_string(fact, { "origin_queue", "queue", "event_queue" })
end

local function fact_dept(fact)
  return first_string(fact, { "origin_dept", "dept", "department", "dead_dept" }, fact_queue(fact))
end

local function fact_source_ref_kind(fact)
  local source_ref = type(fact) == "table" and fact.source_ref or nil
  if type(source_ref) == "table" then
    return tostring(source_ref.kind or "")
  end
  return ""
end

local function cron_failure_fact(fact)
  local queue = fact_queue(fact)
  return queue:find("_tick$", 1, false) ~= nil or fact_source_ref_kind(fact) == "cron"
end

local function top_level_dead_letters(snapshot)
  for _, key in ipairs({ "dlq", "dead_letters", "dead_letter" }) do
    local value = snapshot[key]
    if type(value) == "number" and value > 0 then
      return "dead-letter:count=" .. tostring(value)
    end
    if type(value) == "table" and #list_from_any(value) > 0 then
      local first = list_from_any(value)[1]
      return "dead-letter:" .. first_string(first, { "queue", "event_queue", "name" })
    end
  end
  return nil
end

local function queue_dlq(snapshot)
  for _, queue in ipairs(list_from_any(snapshot.queues or snapshot.queue_state or snapshot.queue_states)) do
    if type(queue) == "table" then
      for _, key in ipairs({ "dlq", "dead", "dead_letters", "dead_letter" }) do
        local value = queue[key]
        local count = type(value) == "table" and #list_from_any(value) or int_value(value)
        if count > 0 then
          return "queue-dlq:" .. first_string(queue, { "queue", "name", "id" }) .. ":count=" .. tostring(count)
        end
      end
    end
  end
  return nil
end

local function terminal_failure_fact(snapshot)
  local cron_counts = {}
  for _, fact in ipairs(failure_facts(snapshot)) do
    if type(fact) == "table" and cron_failure_fact(fact) then
      local key = fact_dept(fact)
      cron_counts[key] = (cron_counts[key] or 0) + 1
    end
  end
  for key, count in pairs(cron_counts) do
    if count > 1 then
      return "infra-stall:" .. tostring(key) .. ":observed_count=" .. tostring(count)
    end
  end
  for _, fact in ipairs(failure_facts(snapshot)) do
    if type(fact) == "table"
      and (bool_value(fact.terminal) or fact.disposition == "terminal")
      and not cron_failure_fact(fact) then
      return "terminal-failure:" .. fact_queue(fact)
    end
  end
  return nil
end

local function entity_anomaly(snapshot, now_seconds, stall_seconds)
  for _, row in ipairs(latest_entity_events(snapshot, now_seconds)) do
    local event = row.event
    if type(event) == "table" then
      if bool_value(event.safety_violation) or event.disposition == "safety-violation" then
        return "safety-violation:" .. row.entity
      end
      if bool_value(event.terminal) or event.disposition == "terminal" or event.tag == "DEAD_LETTER" then
        return "terminal-failure:" .. row.entity
      end
      if expected_transient_event(event) then
        goto continue
      end
      if row.entity_terminal ~= true
        and row.dwell_seconds ~= nil
        and row.dwell_seconds > (tonumber(stall_seconds) or 1800) then
        return "stalled-entity:" .. row.entity
      end
    end
    ::continue::
  end
  return nil
end

local function observed_at_ms(value)
  local number = tonumber(value)
  if number == nil then
    return nil
  end
  return math.floor(number)
end

-- Keep this contract in sync with scripts/board.py. The rollup merge gate must
-- not promote a head that this repository's operator board would report dirty.
function S.verdict(snapshot, opts)
  if type(snapshot) ~= "table" then
    return { clean = false, reason = "observe-malformed" }
  end
  local options = opts or {}
  local reason = top_level_dead_letters(snapshot)
    or queue_dlq(snapshot)
    or terminal_failure_fact(snapshot)
    or entity_anomaly(snapshot, options.now_seconds, options.stall_seconds)
  if reason ~= nil then
    return { clean = false, reason = reason }
  end
  return { clean = true, reason = "clean" }
end

function S.observe_runtime_health()
  if type(fkst) ~= "table" or type(fkst.observe) ~= "function" then
    return { clean = false, reason = "observe-unavailable" }
  end
  local ok, snapshot = pcall(function()
    return fkst.observe({ include = { "queues", "errors", "events", "entities" } })
  end)
  if not ok then
    return { clean = false, reason = "observe-unavailable" }
  end
  local verdict = S.verdict(snapshot)
  if verdict.clean ~= true and verdict.reason == "observe-malformed" then
    verdict.reason = "observe-unavailable"
  end
  return verdict
end

function S.runtime_observe_gate(verdict)
  if type(verdict) ~= "table" or verdict.clean ~= true then
    local reason = type(verdict) == "table" and verdict.reason or "observe-unavailable"
    if reason == "observe-unavailable" or reason == "observe-malformed" then
      return false, "observe-unavailable", reason
    end
    return false, "runtime-unstable", reason or "observe-dirty"
  end
  return true, "runtime-clean"
end

function S.observe_sample_marker(head_sha, status, first_clean_observed_at_ms_value, sampled_at_ms_value)
  if not forge_validators.is_git_sha(head_sha) then
    error("github-devloop-integration: rollup-observe-sample-head-invalid: invalid rollup observe sample head sha")
  end
  if status ~= "clean" and status ~= "dirty" then
    error("github-devloop-integration: rollup-observe-sample-status-invalid: invalid rollup observe sample status")
  end
  local first_clean = observed_at_ms(first_clean_observed_at_ms_value)
  local sampled = observed_at_ms(sampled_at_ms_value)
  if first_clean == nil or first_clean < 0 or sampled == nil or sampled < 0 then
    error("github-devloop-integration: rollup-observe-sample-timestamp-invalid: invalid rollup observe sample timestamps")
  end
  return "<!-- " .. observe_sample_marker_name
    .. ' head_sha="' .. tostring(head_sha)
    .. '" status="' .. tostring(status)
    .. '" first_clean_observed_at_ms="' .. tostring(first_clean)
    .. '" sampled_at_ms="' .. tostring(sampled)
    .. '" -->'
end

local function parse_observe_sample_marker(marker)
  local head_sha = tostring(marker or ""):match('head_sha="([^"]+)"')
  local status = tostring(marker or ""):match('status="([^"]+)"')
  local first_clean = observed_at_ms(tostring(marker or ""):match('first_clean_observed_at_ms="([^"]+)"'))
  local sampled = observed_at_ms(tostring(marker or ""):match('sampled_at_ms="([^"]+)"'))
  if not forge_validators.is_git_sha(head_sha) then
    return nil
  end
  if status ~= "clean" and status ~= "dirty" then
    return nil
  end
  if first_clean == nil or first_clean < 0 or sampled == nil or sampled < 0 then
    return nil
  end
  return {
    head_sha = head_sha,
    status = status,
    first_clean_observed_at_ms = first_clean,
    sampled_at_ms = sampled,
    observed_at_ms = sampled,
  }
end

function S.observe_samples(comments)
  local result = {}
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(comments or {})) do
    for marker in parsers_misc._comment_body(comment):gmatch(observe_sample_marker_pattern) do
      local sample = parse_observe_sample_marker(marker)
      if sample ~= nil then
        table.insert(result, sample)
      end
    end
  end
  table.sort(result, function(a, b)
    if a.sampled_at_ms == b.sampled_at_ms then
      return tostring(a.status) < tostring(b.status)
    end
    return a.sampled_at_ms < b.sampled_at_ms
  end)
  return result
end

local function latest_observe_sample(comments)
  local samples = S.observe_samples(comments)
  return samples[#samples]
end

function S.observe_sample_comment_request(repo, pr_number, head_sha, verdict, observed_seconds, comments, source_ref)
  local status = type(verdict) == "table" and verdict.clean == true and "clean" or "dirty"
  local sampled_ms = math.floor((tonumber(observed_seconds) or now()) * 1000)
  local previous = latest_observe_sample(comments)
  local first_clean_ms = sampled_ms
  if status == "clean"
    and previous ~= nil
    and previous.status == "clean"
    and tostring(previous.head_sha) == tostring(head_sha)
    and tonumber(previous.first_clean_observed_at_ms) ~= nil
    and previous.first_clean_observed_at_ms <= sampled_ms then
    first_clean_ms = previous.first_clean_observed_at_ms
  end
  local marker = S.observe_sample_marker(head_sha, status, first_clean_ms, sampled_ms)
  local reason = type(verdict) == "table" and tostring(verdict.reason or status) or status
  local body = "github-devloop-integration rollup observe sample"
    .. "\n\n"
    .. "status=" .. status
    .. "\n"
    .. "reason=" .. error_facts.one_line(reason)
    .. "\n"
    .. "head_sha=" .. tostring(head_sha)
    .. "\n"
    .. "first_clean_observed_at_ms=" .. tostring(first_clean_ms)
    .. "\n"
    .. "sampled_at_ms=" .. tostring(sampled_ms)
    .. "\n\n"
    .. marker
  return devloop_entity.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, body, base_ids.dedup_key({
    "rollup-observe-sample",
    tostring(repo or ""),
    tostring(pr_number or ""),
  }), source_ref or devloop_entity.pr_source_ref(repo, pr_number), {
    replace_marker = observe_sample_replace_marker,
  })
end

function S.observe_soak_verdict(comments, head_sha, required_seconds, now_seconds)
  if not forge_validators.is_git_sha(head_sha) then
    return false, "runtime-soak-head-missing"
  end
  local now_ms = math.floor((tonumber(now_seconds) or now()) * 1000)
  local required_ms = math.floor((tonumber(required_seconds) or 0) * 1000)
  local sample = latest_observe_sample(comments)
  if sample == nil then
    return false, "runtime-soak-no-clean-sample"
  end
  if tostring(sample.head_sha) ~= tostring(head_sha) then
    return false, "runtime-soak-head-changed:marker_head=" .. tostring(sample.head_sha)
  end
  if sample.status ~= "clean" then
    return false, "runtime-soak-reset:dirty_sampled_at_ms=" .. tostring(sample.sampled_at_ms)
  end
  local age_ms = now_ms - sample.first_clean_observed_at_ms
  if age_ms < required_ms then
    return false,
      "runtime-soak-pending:age_seconds=" .. tostring(math.floor(age_ms / 1000))
        .. ":required_seconds=" .. tostring(math.floor(required_ms / 1000))
  end
  return true, "runtime-soak-ok"
end

function S.observe_sample_marker_name()
  return observe_sample_marker_name
end

function S.observe_sample_replace_marker()
  return observe_sample_replace_marker
end

function S.install(M)
local strings = require("contract.strings")
local devloop_logging = require("devloop.logging")
local detector = "rollup-health"
local default_red_window_minutes = 30

local function format_timestamp(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(seconds) or now())
end

local function age_minutes(timestamp, now_seconds)
  local seconds = contract_time.iso_timestamp_epoch_seconds(timestamp)
  if seconds == nil then
    return nil
  end
  local age = (tonumber(now_seconds) or now()) - seconds
  if age < 0 then
    return nil
  end
  return math.floor(age / 60)
end

local function failed_check_timestamp(entry)
  if type(entry) ~= "table" then
    return nil
  end
  return entry.completedAt or entry.completed_at or entry.updatedAt or entry.updated_at or entry.createdAt or entry.created_at
end

local function rollup_red_started_at(pr)
  local entries = type(pr) == "table" and pr.status_check_rollup or nil
  if type(entries) ~= "table" then
    return nil
  end
  local started_at = nil
  for _, entry in ipairs(entries) do
    local single_pr = { status_check_rollup = { entry } }
    local green, reason = check_runs.pr_rollup_green(single_pr)
    if not green and reason == "rollup-red" then
      local timestamp = failed_check_timestamp(entry)
        local seconds = contract_time.iso_timestamp_epoch_seconds(timestamp)
      if seconds ~= nil then
        local current_started_seconds = contract_time.iso_timestamp_epoch_seconds(started_at)
        if current_started_seconds == nil or seconds < current_started_seconds then
          started_at = timestamp
        end
      end
    end
  end
  return started_at
end

local function snapshot_path(repo, pr_number, head_sha)
  local safe_repo = base_ids.safe_repo(repo):gsub("/", "-"):gsub("%-+", "-")
  local safe_head = strings.sanitize_key(tostring(head_sha or "unknown"), false):gsub("[/%s]+", "-")
  safe_head = safe_head:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if safe_head == "" then
    safe_head = "unknown"
  end
  if #safe_head > 80 then
    safe_head = safe_head:sub(1, 80):gsub("%-+$", "")
  end
  return "/tmp/fkst-github-devloop-rollup-health-" .. safe_repo .. "-pr-" .. tostring(pr_number) .. "-" .. safe_head .. ".json"
end

local function write_snapshot(repo, evidence)
  local path = snapshot_path(repo, evidence.pr_number, evidence.head_sha)
  file.write(path, "{"
    .. '"detector":' .. strings.json_string(detector)
    .. ',"repo":' .. strings.json_string(repo)
    .. ',"pr_number":' .. tostring(tonumber(evidence.pr_number) or 0)
    .. ',"upstream_branch":' .. strings.json_string(evidence.upstream_branch)
    .. ',"integration_branch":' .. strings.json_string(evidence.integration_branch)
    .. ',"head_sha":' .. strings.json_string(evidence.head_sha)
    .. ',"updated_at":' .. strings.json_string(evidence.updated_at)
    .. ',"red_started_at":' .. strings.json_string(evidence.red_started_at)
    .. ',"age_minutes":' .. tostring(tonumber(evidence.age_minutes) or 0)
    .. ',"threshold_minutes":' .. tostring(tonumber(evidence.threshold_minutes) or 0)
    .. ',"failing_check":' .. strings.json_string(evidence.failing_check)
    .. ',"generated_at":' .. strings.json_string(format_timestamp(evidence.now_seconds))
    .. "}\n")
  return path
end

local function failure_identity(failing_check)
  local identity = tostring(failing_check or "rollup-red")
  identity = identity:gsub(";.*$", "")
  identity = identity:gsub(":.*$", "")
  identity = devloop_base.neutralize_untrusted_comment_text(devloop_base._neutralize_fkst_markers(identity))
  identity = error_facts.one_line(identity):gsub("^%s+", ""):gsub("%s+$", "")
  if identity == "" then
    identity = "rollup-red"
  end
  if #identity > 80 then
    identity = identity:sub(1, 80):gsub("%s+$", "")
  end
  return identity
end

function M.rollup_red_window_minutes(exec)
  local raw = devloop_base.read_env("FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES", exec)
  if raw == nil or strings.trim(raw) == "" then
    return default_red_window_minutes
  end
  local value = tonumber(strings.trim(raw))
  if value == nil or value ~= math.floor(value) or value < 1 or value > 1440 then
    error("github-devloop: config-invalid: invalid FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES")
  end
  return value
end

function M.rollup_health_dedup_key(repo, failing_check)
  return base_ids.dedup_key({
    detector,
    tostring(repo or ""),
    failure_identity(failing_check),
  })
end

local function alert_title(evidence)
  return "Rollup health: integration->dev is red on " .. failure_identity(evidence.failing_check)
end

local function alert_body(evidence, snapshot)
  local lines = {
    "Rollup health watchdog fired from deterministic CI status signals.",
    "",
    "Detector: `" .. detector .. "`",
    "Rollup PR: #" .. tostring(evidence.pr_number),
    "Branches: `" .. tostring(evidence.integration_branch) .. "` -> `" .. tostring(evidence.upstream_branch) .. "`",
    "Head: `" .. tostring(evidence.head_sha) .. "`",
    "Failing check: `" .. tostring(evidence.failing_check) .. "`",
    "Red age: " .. tostring(evidence.age_minutes) .. " minutes",
    "Threshold: " .. tostring(evidence.threshold_minutes) .. " minutes",
    "Evidence snapshot: `" .. tostring(snapshot) .. "`",
    "",
    "Requested outcome:",
    "- Diagnose why the rollup PR is red and blocking integration delivery.",
    "- File any fix through the normal intake, consensus, implementation, and review pipeline.",
    "- This watchdog must not repair, merge, relabel, or mutate runtime state directly.",
  }
  local body = table.concat(lines, "\n")
  if #body > M._max_body_len then
    body = base_ids.truncate_utf8(body, M._max_body_len)
  end
  return body
end

function M.build_rollup_health_issue_create_request(repo, evidence, snapshot)
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = alert_title(evidence),
    body = alert_body(evidence, snapshot),
    -- When rollup auto-fix is on, the watchdog issue is born as an enabled,
    -- expedite work-item so the loop claims + fixes the red rollup ahead of new
    -- issues. Default off ⇒ no labels (passive watchdog, today's behavior).
    labels = evidence.rollup_autofix
      and json.decode('["fkst-dev:enabled","fkst-class:expedite"]')
      or json.decode("[]"),
    dedup_key = M.rollup_health_dedup_key(repo, evidence.failing_check),
    parent_comment_target = {
      repo = repo,
      issue_number = tostring(evidence.pr_number),
    },
    source_ref = {
      kind = "external",
      ref = tostring(repo or "") .. "#" .. detector .. "/pr/" .. tostring(evidence.pr_number),
    },
  }
end

function M.observe_rollup_health(repo, upstream, integration, pr, now_seconds, threshold_minutes)
  local current_seconds = tonumber(now_seconds) or now()
  local threshold = tonumber(threshold_minutes) or M.rollup_red_window_minutes()
  local green, reason = check_runs.pr_rollup_green(pr)
  if green then
    log.info("github-devloop dept=rollup_scan tag=ROLLUP_HEALTH action=no-op reason=rollup-green")
    return { action = "no-op", reason = "rollup-green" }
  end
  if reason ~= "rollup-red" then
    log.info("github-devloop dept=rollup_scan tag=ROLLUP_HEALTH action=no-op reason=" .. tostring(reason))
    return { action = "no-op", reason = reason }
  end

  local red_started_at = rollup_red_started_at(pr)
  local age = age_minutes(red_started_at, current_seconds)
  if age == nil then
    log.info("github-devloop dept=rollup_scan tag=ROLLUP_HEALTH action=no-op reason=age-unknown")
    return { action = "no-op", reason = "age-unknown" }
  end
  if age < threshold then
    log.info("github-devloop dept=rollup_scan tag=ROLLUP_HEALTH action=suppress"
      .. " reason=red-window"
      .. " age_minutes=" .. tostring(age)
      .. " threshold_minutes=" .. tostring(threshold))
    return { action = "suppress", reason = "red-window", age_minutes = age }
  end

  local failing_check = parsers_misc.pr_rollup_failure_summary(pr)
  if failing_check == "" then
    failing_check = "rollup-red"
  end
  local evidence = {
    now_seconds = current_seconds,
    repo = repo,
    pr_number = pr and pr.number,
    upstream_branch = upstream,
    integration_branch = integration,
    head_sha = pr and pr.head_sha,
    updated_at = pr and pr.updated_at,
    red_started_at = red_started_at,
    age_minutes = age,
    threshold_minutes = threshold,
    failing_check = failing_check,
    rollup_autofix = config.rollup_autofix_enabled(),
  }
  local snapshot = write_snapshot(repo, evidence)
  local request = M.build_rollup_health_issue_create_request(repo, evidence, snapshot)
  devloop_logging.log_raise("rollup_scan", detector .. "/" .. tostring(pr and pr.number or "unknown"), "github-proxy.github_issue_create_request", request)
  log.info("github-devloop dept=rollup_scan tag=ROLLUP_HEALTH"
    .. " action=raise"
    .. " pr=" .. tostring(pr and pr.number or "")
    .. " head_sha=" .. tostring(pr and pr.head_sha or "")
    .. " age_minutes=" .. tostring(age)
    .. " threshold_minutes=" .. tostring(threshold)
    .. " failing_check=" .. error_facts.one_line(failing_check)
    .. " snapshot_path=" .. tostring(snapshot)
    .. " dedup_key=" .. tostring(request.dedup_key))
  return {
    action = "raise",
    request = request,
    snapshot_path = snapshot,
  }
end
end

return S
