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

function M.persistence_class()
  return "composed_judgment_pipeline"
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

local function audit_run_marker(trigger_reason)
  if trigger_reason == nil or trigger_reason == "" then
    return nil
  end
  if trigger_reason ~= "idle" and trigger_reason ~= "stale" then
    error("archaudit: invalid-audit-trigger: " .. tostring(trigger_reason))
  end
  return '<!-- fkst:archaudit:audit-run:v1 reason="' .. tostring(trigger_reason) .. '" -->'
end

local function body_text(finding, dedup_key, trigger_reason)
  local lines = {
    "Architecture doctrine violation:",
    "",
    "File: " .. tostring(finding.file) .. ":" .. tostring(finding.line),
    "Rule: " .. tostring(finding.rule),
  }
  if trigger_reason ~= nil and trigger_reason ~= "" then
    table.insert(lines, "Audit trigger: " .. tostring(trigger_reason))
  end
  table.insert(lines, "")
  table.insert(lines, "Why:")
  table.insert(lines, tostring(finding.why))
  table.insert(lines, "")
  table.insert(lines, "Suggested fix:")
  table.insert(lines, tostring(finding.suggested_fix))
  table.insert(lines, "")
  table.insert(lines, "<!-- archaudit-dedup: " .. tostring(dedup_key) .. " -->")
  local marker = audit_run_marker(trigger_reason)
  if marker ~= nil then
    table.insert(lines, marker)
  end
  return table.concat(lines, "\n")
end

function M.audit_tick_payload(slot)
  return {
    schema = "archaudit.tick.v1",
    slot = tostring(slot or ""),
    source_ref = {
      kind = "cron",
      ref = "archaudit/audit_poll/" .. tostring(slot or ""),
    },
  }
end

function M.validate_audit_tick_payload(payload)
  if type(payload) ~= "table" or payload.schema ~= "archaudit.tick.v1" then
    return false
  end
  if not strings.is_bounded_string(payload.slot, 80) then
    return false
  end
  if type(payload.source_ref) ~= "table" or payload.source_ref.kind ~= "cron" then
    return false
  end
  return payload.source_ref.ref == "archaudit/audit_poll/" .. tostring(payload.slot)
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

function M.build_issue_create_request(repo, finding, label_available, trigger_reason)
  assert_request_field(M.validate_repo(repo), "repo")
  local dedup_key = M.dedup_key(repo, finding)
  local title = "Archaudit: " .. tostring(finding.file) .. ":" .. tostring(finding.line) .. " " .. one_line(finding.rule)
  local body = body_text(finding, dedup_key, trigger_reason)
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

function M.audit_issue_search_query()
  return "archaudit-dedup:"
end

local function issue_author_login(issue)
  if type(issue) ~= "table" then
    return nil
  end
  if type(issue.author) == "table" and issue.author.login ~= nil then
    return tostring(issue.author.login)
  end
  if type(issue.user) == "table" and issue.user.login ~= nil then
    return tostring(issue.user.login)
  end
  if issue.author_login ~= nil then
    return tostring(issue.author_login)
  end
  return nil
end

function M.parse_audit_issue_search(stdout)
  local ok, decoded = pcall(decode_json, stdout or "[]")
  if not ok or type(decoded) ~= "table" then
    error("archaudit: audit-search-malformed-json: GitHub audit issue search")
  end
  local issues = {}
  for _, issue in ipairs(decoded) do
    if type(issue) == "table" then
      table.insert(issues, {
        number = issue.number,
        title = issue.title,
        state = issue.state,
        body = tostring(issue.body or ""),
        created_at = issue.createdAt or issue.created_at,
        updated_at = issue.updatedAt or issue.updated_at,
        author_login = issue_author_login(issue),
        url = issue.url,
      })
    end
  end
  return issues
end

local function trusted_audit_issue(issue, trusted_login)
  if type(issue) ~= "table" then
    return false
  end
  if tostring(issue.body or ""):find("archaudit-dedup:", 1, true) == nil then
    return false
  end
  if trusted_login == nil or trusted_login == "" then
    return false
  end
  return tostring(issue.author_login or "") == tostring(trusted_login)
end

function M.latest_audit_issue_seconds(issues, trusted_login)
  local latest = nil
  for _, issue in ipairs(issues or {}) do
    if trusted_audit_issue(issue, trusted_login) then
      local seconds = M.iso_timestamp_epoch_seconds(issue.created_at) or M.iso_timestamp_epoch_seconds(issue.updated_at)
      if seconds ~= nil and (latest == nil or seconds > latest) then
        latest = seconds
      end
    end
  end
  return latest
end

function M.audit_due_verdict(issues, trusted_login, now_seconds, max_staleness_seconds)
  if type(now_seconds) ~= "number" or type(max_staleness_seconds) ~= "number" or max_staleness_seconds < 1 then
    error("archaudit: invalid-audit-staleness-input: timestamps and staleness budget must be numeric")
  end
  local latest = M.latest_audit_issue_seconds(issues, trusted_login)
  if latest == nil then
    return true, "no durable audit issue marker", nil
  end
  if latest > now_seconds then
    return false, "latest audit issue marker is in the future", latest
  end
  if now_seconds - latest >= max_staleness_seconds then
    return true, "audit max staleness elapsed", latest
  end
  return false, "recent audit issue marker", latest
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
