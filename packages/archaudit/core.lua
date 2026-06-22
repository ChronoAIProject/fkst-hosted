local M = {}

local strings = require("contract.strings")
local error_facts = require("contract.error_facts")

local file_limit = 240
local rule_limit = 80
local why_limit = 1000
local fix_limit = 1000
local github_proxy_limits = {
  repo = 200,
  title = 240,
  body = 12000,
  dedup_key = 512,
  source_ref_kind = 80,
  source_ref_ref = 200,
}
local observe_schema_version = 1
local audit_due_staleness_seconds = 24 * 60 * 60

function M.persistence_class()
  return "composed_judgment_pipeline"
end

function M.audit_due_staleness_seconds()
  return audit_due_staleness_seconds
end

function M.producer_liveness_contracts()
  return {
    {
      producer_id = "archaudit.audit",
      trigger_source = "audit_due",
      output_queues = { "github-proxy.github_issue_create_request" },
      eligibility_predicate = "overdue",
      max_staleness_seconds = audit_due_staleness_seconds,
      max_silence_seconds = audit_due_staleness_seconds,
      max_skip_budget = 0,
      progress_output = "github-proxy.github_issue_create_request",
    },
  }
end

local function bounded(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function marker_safe(value)
  return tostring(value):find('[<>"\r\n]') == nil
end

local function assert_request_field(ok, field)
  if not ok then
    error("archaudit: invalid-issue-create-field: " .. tostring(field), 0)
  end
end

local function one_line(value)
  return tostring(value or ""):gsub("%s+", " ")
end

local function body_text(finding, dedup_key)
  return table.concat({
    "Architecture doctrine violation:",
    "",
    "File: " .. tostring(finding.file) .. ":" .. tostring(finding.line),
    "Rule: " .. tostring(finding.rule),
    "",
    "Why:",
    tostring(finding.why),
    "",
    "Suggested fix:",
    tostring(finding.suggested_fix),
    "",
    "<!-- archaudit-dedup: " .. tostring(dedup_key) .. " -->",
  }, "\n")
end

local function required_list(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("archaudit: observe-malformed-facts: malformed " .. name)
  end
  local count = 0
  local max_index = 0
  for key, _item in pairs(value) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      error("archaudit: observe-malformed-facts: malformed " .. name)
    end
    count = count + 1
    if key > max_index then
      max_index = key
    end
  end
  if max_index ~= count then
    error("archaudit: observe-malformed-facts: malformed " .. name)
  end
  return value
end

local function required_int(row, name)
  local value = row[name]
  if type(value) ~= "number" or value < 0 or math.floor(value) ~= value then
    error("archaudit: observe-malformed-metric: " .. tostring(name) .. " must be a non-negative integer")
  end
  return value
end

local function required_table(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("archaudit: observe-malformed-facts: malformed " .. name)
  end
  return value
end

local function required_bool(row, name)
  local value = row[name]
  if type(value) ~= "boolean" then
    error("archaudit: observe-malformed-facts: " .. tostring(name) .. " must be a boolean")
  end
  return value
end

local function decode_json(text)
  return json.decode(text)
end

function M.validate_repo(repo)
  if not strings.is_bounded_string(repo, github_proxy_limits.repo) then
    return false
  end
  if strings.split_repo(repo) == nil then
    return false
  end
  return tostring(repo):find("^[%w._-]+/[%w._-]+$") ~= nil
end

function M.validate_observe_facts(facts)
  if type(facts) ~= "table" then
    error("archaudit: observe-malformed-top-level: facts must be a table")
  end
  if facts.schema_version ~= observe_schema_version then
    error("archaudit: observe-unknown-schema-version: expected schema_version=1")
  end
  if type(facts.generated_at_ms) ~= "number" or facts.generated_at_ms < 0 or math.floor(facts.generated_at_ms) ~= facts.generated_at_ms then
    error("archaudit: observe-malformed-facts: generated_at_ms must be a non-negative integer")
  end
  required_table(facts, "source")
  local limits = required_table(facts, "limits")
  required_int(limits, "max_deliveries")
  required_int(limits, "max_dead_letters")
  local truncated = required_table(facts, "truncated")
  required_bool(truncated, "deliveries")
  required_bool(truncated, "dead_letters")
  required_list(facts, "queues")
  required_list(facts, "deliveries")
  required_list(facts, "dead_letters")
  for _, row in ipairs(facts.queues) do
    if type(row) ~= "table" then
      error("archaudit: observe-malformed-queue-row: queue row must be a table")
    end
    if type(row.queue) ~= "string" or row.queue == "" then
      error("archaudit: observe-malformed-queue-name: queue name must be non-empty")
    end
    required_int(row, "depth")
    required_int(row, "pending")
    required_int(row, "in_flight")
    required_int(row, "retrying")
  end
  return facts
end

function M.observe_now_seconds(facts)
  M.validate_observe_facts(facts)
  return math.floor(facts.generated_at_ms / 1000)
end

function M.observe(exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("archaudit: missing-exec: observe requires exec_sync")
  end
  local result = run({ cmd = 'fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("archaudit: observe-unreadable: " .. tostring(result and result.stderr or "no result"))
  end
  local ok, decoded = pcall(decode_json, result.stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("archaudit: observe-malformed-json: observe returned malformed JSON")
  end
  return M.validate_observe_facts(decoded)
end

function M.is_idle_observe(facts)
  M.validate_observe_facts(facts)
  if facts.truncated.deliveries then
    return false, "current observe truncated deliveries"
  end
  if facts.truncated.dead_letters then
    return false, "current observe truncated dead_letters"
  end
  for _, row in ipairs(facts.queues) do
    for _, field in ipairs({ "pending", "in_flight", "retrying", "depth" }) do
      if row[field] > 0 then
        return false, "current observe busy queue=" .. tostring(row.queue) .. " " .. field .. "=" .. tostring(row[field])
      end
    end
  end
  if #facts.deliveries > 0 then
    return false, "current observe deliveries=" .. tostring(#facts.deliveries)
  end
  if #facts.dead_letters > 0 then
    return false, "current observe dead_letters=" .. tostring(#facts.dead_letters)
  end
  return true, nil
end

function M.build_prompt(repo, max_findings)
  return table.concat({
    "You are an architecture audit judge for repo " .. tostring(repo) .. ".",
    "Read repository files and CLAUDE.md yourself from the local checkout.",
    "Do not edit files. Do not run gh. Do not run git.",
    "Find only concrete architecture-doctrine violations: god-class, god-state, coupling, SRP, Demeter, DIP, or similar local drift.",
    "Every finding must cite an exact file and line and propose a small local refactor.",
    "Do not report vague smells, umbrellas, grouped unrelated problems, invented rules, or special-case big items.",
    "Return strict JSON only: an array of at most " .. tostring(max_findings) .. " objects.",
    'Object schema: {"file":"packages/example/core.lua","line":42,"rule":"SRP","why":"...","suggested_fix":"..."}',
  }, "\n")
end

function M.parse_findings_json(stdout)
  local raw = strings.trim(stdout or "")
  if raw:sub(1, 1) ~= "[" or raw:sub(-1) ~= "]" then
    error("archaudit: malformed-json: codex output is not a JSON array")
  end
  local ok, decoded = pcall(decode_json, stdout or "")
  if not ok then
    error("archaudit: malformed-json: codex output is malformed JSON")
  end
  if type(decoded) ~= "table" then
    error("archaudit: non-array-json: codex output is not a JSON array")
  end
  local count = 0
  for key, _value in pairs(decoded) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      error("archaudit: non-array-json: codex output is not a JSON array")
    end
    if key > count then
      count = key
    end
  end
  if count ~= #decoded then
    error("archaudit: malformed-json: codex output is not a dense JSON array")
  end
  local findings = {}
  for index, item in ipairs(decoded) do
    if type(item) ~= "table"
      or not bounded(item.file, file_limit)
      or type(item.line) ~= "number"
      or item.line < 1
      or math.floor(item.line) ~= item.line
      or not bounded(item.rule, rule_limit)
      or not bounded(item.why, why_limit)
      or not bounded(item.suggested_fix, fix_limit) then
      error("archaudit: invalid-finding-shape: index=" .. tostring(index))
    end
    table.insert(findings, {
      file = item.file,
      line = item.line,
      rule = item.rule,
      why = item.why,
      suggested_fix = item.suggested_fix,
    })
  end
  return findings
end

function M.validate_finding(finding)
  if type(finding) ~= "table" or not bounded(finding.file, file_limit) or type(finding.line) ~= "number" then
    return false
  end
  local text = file.read(finding.file)
  if type(text) ~= "string" or text == "" then
    return false
  end
  local count = 0
  for _line in (text .. "\n"):gmatch("([^\n]*)\n") do
    count = count + 1
    if count == finding.line then
      return true
    end
  end
  return false
end

function M.dedup_key(repo, finding)
  local seed = table.concat({
    tostring(repo),
    tostring(finding.file),
    tostring(finding.line),
    tostring(finding.rule),
  }, "|")
  local readable = table.concat({
    "archaudit",
    strings.sanitize_key(repo, 120),
    strings.sanitize_key(finding.file, 160),
    tostring(finding.line),
    strings.sanitize_key(finding.rule, 80),
    strings.decimal_checksum(seed),
  }, "/")
  return readable:sub(1, github_proxy_limits.dedup_key)
end

function M.build_issue_create_request(repo, finding, label_available)
  assert_request_field(M.validate_repo(repo), "repo")
  local dedup_key = M.dedup_key(repo, finding)
  local title = "Archaudit: " .. tostring(finding.file) .. ":" .. tostring(finding.line) .. " " .. one_line(finding.rule)
  local body = body_text(finding, dedup_key)
  local source_ref_ref = tostring(repo) .. "#" .. tostring(finding.file) .. ":" .. tostring(finding.line) .. "#archaudit-create-intent"
  assert_request_field(strings.is_bounded_string(title, github_proxy_limits.title), "title")
  assert_request_field(strings.is_bounded_string(body, github_proxy_limits.body), "body")
  assert_request_field(strings.is_bounded_string(dedup_key, github_proxy_limits.dedup_key) and marker_safe(dedup_key), "dedup_key")
  assert_request_field(strings.is_bounded_string("repo-site", github_proxy_limits.source_ref_kind), "source_ref.kind")
  assert_request_field(strings.is_bounded_string(source_ref_ref, github_proxy_limits.source_ref_ref), "source_ref.ref")
  local labels = {}
  if label_available then
    labels = { "archaudit" }
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = title,
    body = body,
    labels = labels,
    dedup_key = dedup_key,
    source_ref = {
      kind = "repo-site",
      ref = source_ref_ref,
    },
  }
end

local function days_from_civil(year, month, day)
  if month <= 2 then
    year = year - 1
    month = month + 12
  end
  local era = math.floor(year / 400)
  local yoe = year - era * 400
  local doy = math.floor((153 * (month - 3) + 2) / 5) + day - 1
  local doe = yoe * 365 + math.floor(yoe / 4) - math.floor(yoe / 100) + doy
  return era * 146097 + doe - 719468
end

function M.iso_timestamp_epoch_seconds(timestamp)
  local parts = { tostring(timestamp or ""):match("^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)Z$") }
  if #parts ~= 6 then
    return nil
  end
  for index, part in ipairs(parts) do
    parts[index] = tonumber(part)
  end
  local year, month, day, hour, minute, second = parts[1], parts[2], parts[3], parts[4], parts[5], parts[6]
  if month < 1 or month > 12 or day < 1 or day > 31 or hour > 23 or minute > 59 or second > 59 then
    return nil
  end
  return days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second
end

function M.idle_hint_freshness(detected_seconds, expires_seconds, now_seconds, budget_seconds)
  if type(detected_seconds) ~= "number" or type(now_seconds) ~= "number" or type(budget_seconds) ~= "number" then
    error("archaudit: malformed-idle-hint: timestamp inputs must be numeric")
  end
  if now_seconds - detected_seconds > budget_seconds then
    return "stale"
  end
  if expires_seconds ~= nil then
    if type(expires_seconds) ~= "number" then
      error("archaudit: malformed-idle-hint: expires_at must be numeric")
    end
    if expires_seconds <= now_seconds then
      return "expired"
    end
  end
  return "fresh"
end

function M.failure_fact(dept, tag, error_class, event, message, terminal)
  local fields = error_facts.error_fact_fields(error_class, type(event) == "table" and event.queue or nil, dept, message, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(message))
  return "archaudit dept=" .. tostring(dept) .. " tag=" .. tostring(tag) .. " " .. table.concat(fields, " ")
end

function M.skip_fact(dept, event, why, terminal)
  local fields = error_facts.error_fact_fields("terminal-skip", type(event) == "table" and event.queue or nil, dept, why, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  return "archaudit dept=" .. tostring(dept) .. " tag=SKIP " .. table.concat(fields, " ")
end

return M
