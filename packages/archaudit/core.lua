local M = {}

local strings = require("contract.strings")
local forge_strings = require("forge.strings")
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
local audit_poll_interval_seconds = 30 * 60
-- Starting at the raw staleness deadline is too late: cron dispatch, durable
-- admission, audit codex runtime, and handoff/retry slack must all complete
-- before max staleness. Budget two poll intervals for schedule/admission jitter
-- plus 15 minutes for the current sub-10-minute audit runtime and downstream slack.
local audit_due_completion_budget_seconds = 2 * audit_poll_interval_seconds + 15 * 60
local audit_poll_interval = tostring(math.floor(audit_poll_interval_seconds / 60)) .. "m"


function M.audit_due_staleness_seconds()
  return audit_due_staleness_seconds
end

function M.audit_poll_interval_seconds()
  return audit_poll_interval_seconds
end

function M.audit_due_completion_budget_seconds()
  return audit_due_completion_budget_seconds
end

function M.audit_due_force_at_seconds(max_staleness_seconds, completion_budget_seconds)
  local staleness = max_staleness_seconds or audit_due_staleness_seconds
  local completion_budget = completion_budget_seconds or audit_due_completion_budget_seconds
  if type(staleness) ~= "number" or type(completion_budget) ~= "number"
    or staleness < 1 or completion_budget < 1 or completion_budget >= staleness then
    error("archaudit: invalid-audit-force-at-input: staleness and completion budget must be numeric and bounded")
  end
  return staleness - completion_budget
end

function M.audit_poll_interval()
  return audit_poll_interval
end

function M.producer_liveness_contracts()
  return {
    {
      producer_id = "archaudit.audit",
      trigger_source = "archaudit_tick",
      output_queues = { "github-proxy.github_issue_create_request" },
      eligibility_predicate = "overdue",
      max_staleness_seconds = audit_due_staleness_seconds,
      completion_budget_seconds = audit_due_completion_budget_seconds,
      force_at_seconds = M.audit_due_force_at_seconds(audit_due_staleness_seconds, audit_due_completion_budget_seconds),
      max_silence_seconds = audit_poll_interval_seconds,
      max_skip_budget = 0,
      progress_output = "github-proxy.github_issue_create_request",
      runtime_gate = "idle_when_not_overdue",
      adversarial_fixture = "busy_overdue",
    },
  }
end

local function append_error(errors, message)
  table.insert(errors, "producer-liveness: " .. tostring(message))
end

local function positive_minute_seconds(contract, field, errors)
  local value = tonumber(contract and contract[field])
  if value == nil or value <= 0 or math.floor(value) ~= value then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": " .. field .. " must be positive integer seconds")
    return nil
  end
  if value % 60 ~= 0 then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": " .. field .. " must be minute-aligned for restart liveness")
    return nil
  end
  return value / 60
end

local function non_negative_integer(contract, field, errors)
  local value = tonumber(contract and contract[field])
  if value == nil or value < 0 or math.floor(value) ~= value then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": " .. field .. " must be a non-negative integer")
    return nil
  end
  return value
end

local function non_empty_string(contract, field, errors)
  local value = contract and contract[field]
  if type(value) ~= "string" or value == "" then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": " .. field .. " must be a non-empty string")
    return nil
  end
  return value
end

local function optional_non_empty_string(contract, field, errors)
  local value = contract and contract[field]
  if value == nil then
    return nil
  end
  if type(value) ~= "string" or value == "" then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": " .. field .. " must be a non-empty string when declared")
    return nil
  end
  return value
end

local function output_queues(contract, errors)
  local queues = contract and contract.output_queues
  if type(queues) ~= "table" or #queues == 0 then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": output_queues must be a non-empty list")
    return nil
  end
  local copied = {}
  for index, queue in ipairs(queues) do
    if type(queue) ~= "string" or queue == "" then
      append_error(errors, tostring(contract.producer_id or "?") .. ": output_queues[" .. tostring(index) .. "] must be a non-empty string")
      return nil
    end
    table.insert(copied, queue)
  end
  return copied
end

local function producer_liveness_row(contract, errors)
  if type(contract) ~= "table" then
    append_error(errors, "contract must be a table")
    return nil
  end
  local producer_id = non_empty_string(contract, "producer_id", errors)
  local trigger_source = non_empty_string(contract, "trigger_source", errors)
  non_empty_string(contract, "eligibility_predicate", errors)
  local progress_output = non_empty_string(contract, "progress_output", errors)
  local runtime_gate = optional_non_empty_string(contract, "runtime_gate", errors)
  local adversarial_fixture = optional_non_empty_string(contract, "adversarial_fixture", errors)
  local outputs = output_queues(contract, errors)
  local staleness_minutes = positive_minute_seconds(contract, "max_staleness_seconds", errors)
  local silence_minutes = positive_minute_seconds(contract, "max_silence_seconds", errors)
  local skip_budget = non_negative_integer(contract, "max_skip_budget", errors)
  if runtime_gate ~= nil and adversarial_fixture == nil then
    append_error(errors, tostring(contract and contract.producer_id or "?") .. ": runtime_gate must declare adversarial_fixture")
    return nil
  end
  if producer_id == nil or trigger_source == nil or progress_output == nil or outputs == nil
    or staleness_minutes == nil or silence_minutes == nil or skip_budget == nil then
    return nil
  end
  local progress_listed = false
  for _, queue in ipairs(outputs) do
    if queue == progress_output then
      progress_listed = true
      break
    end
  end
  if not progress_listed then
    append_error(errors, producer_id .. ": progress_output must be listed in output_queues")
    return nil
  end
  return {
    from_state = producer_id,
    liveness_class_id = producer_id .. ".positive-output",
    terminal = false,
    to_states = { producer_id },
    driving_queue = trigger_source,
    observe_surfaces = { liveness_scan = true },
    output_obligation = {
      kinds = outputs,
      exits = { producer_id },
    },
    budget = {
      minutes = staleness_minutes,
      receiver_max_work_justification = "Producer must emit or escalate the declared progress output within max_staleness_seconds.",
    },
    liveness_contract = {
      mode = "row-budget-bounds-receiver",
      receiver_bound_minutes = silence_minutes,
      external_wait_bound_minutes = 0,
    },
    on_timeout = {
      action = "redrive",
      queue = trigger_source,
      escalate_after_attempts = skip_budget + 1,
      on_escalate = {
        action = "force-terminate",
        terminal_state = "blocked",
        reason = "producer-output-obligation-timeout",
      },
    },
    watchdog = {
      mode = "row-budget-bounds-receiver",
      budget_ms = staleness_minutes * 60 * 1000,
    },
    actionable_epoch = {
      source = "state_entry:v1",
      generation_source = "same_as_actionable_epoch",
    },
    producer_liveness = {
      runtime_gate = runtime_gate,
      adversarial_fixture = adversarial_fixture,
    },
  }
end

function M.producer_liveness_restart_rows(contracts)
  local errors = {}
  local rows = {}
  for _, contract in ipairs(contracts or M.producer_liveness_contracts()) do
    local row = producer_liveness_row(contract, errors)
    if row ~= nil then
      table.insert(rows, row)
    end
  end
  return rows, errors
end

local function liveness_model(rows)
  local workflow_liveness_shared = require("workflow.liveness.shared")
  local workflow_liveness_contract = require("workflow.liveness.contract")
  local restart_liveness_contract = require("workflow.restart_liveness_contract")
  local model = {
    restart_package_name = "archaudit",
    restart_lifecycle_states = {},
    restart_transition_table = function()
      return rows
    end,
    restart_durable_marker_fields = function()
      return {}
    end,
    restart_responsibility_inventory_errors = function()
      return {}
    end,
  }
  local lifecycle_states = {}
  function model.is_state(state)
    return lifecycle_states[state] == true
  end
  for _, row in ipairs(rows or {}) do
    table.insert(model.restart_lifecycle_states, row.from_state)
    lifecycle_states[row.from_state] = true
    for _, next_state in ipairs(row.to_states or {}) do
      lifecycle_states[next_state] = true
    end
  end
  restart_liveness_contract.install(model, {
    workflow_ports = {
      dependency_release_marker = function()
        error("archaudit: workflow-port-unavailable: dependency_release_marker is not available for producer-liveness restart model")
      end,
      restart_transition_table = function(...)
        return model.restart_transition_table(...)
      end,
      trusted_bot_login = function()
        error("archaudit: workflow-port-unavailable: trusted_bot_login is not available for producer-liveness restart model")
      end,
    },
  })
  local shared = workflow_liveness_shared.install(model, {
    restart_package_name = model.restart_package_name,
    restart_source_root = model.restart_source_root,
    liveness_signal_producers = {},
  })
  workflow_liveness_contract.install(model, shared)
  return model
end

function M.producer_liveness_contract_errors(contracts)
  local rows, errors = M.producer_liveness_restart_rows(contracts)
  if #rows == 0 then
    return errors
  end
  local model = liveness_model(rows)
  for _, err in ipairs(model.liveness_contract_errors(rows)) do
    table.insert(errors, err)
  end
  return errors
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

local function require_audit_run_marker(trigger_reason)
  local marker = audit_run_marker(trigger_reason)
  if marker == nil then
    error("archaudit: invalid-audit-trigger: missing trigger reason")
  end
  return marker
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

local function audit_run_body(trigger_reason)
  return table.concat({
    "Architecture audit completed with zero findings.",
    "",
    "Audit trigger: " .. tostring(trigger_reason),
    "",
    require_audit_run_marker(trigger_reason),
  }, "\n")
end

local function is_audit_poll_raiser(raiser)
  if raiser == "audit_poll" then
    return true
  end
  if type(raiser) ~= "string" then
    return false
  end
  return raiser:match("^[A-Za-z0-9_%-]+%.audit_poll$") ~= nil
end

function M.normalize_audit_tick_event(event)
  if type(event) ~= "table" then
    return nil, "missing-event"
  end
  local queue = tostring(event.queue or "")
  if queue ~= "archaudit.archaudit_tick" and queue ~= "archaudit_tick" then
    return nil, "wrong-queue"
  end
  local payload = event.payload
  if type(payload) ~= "table" then
    return nil, "missing-payload"
  end
  if not is_audit_poll_raiser(payload.raiser) then
    return nil, "wrong-raiser"
  end
  local slot = payload.slot or payload.cron_slot or payload.detected_at or event.ts
  if slot == nil or tostring(slot) == "" then
    return nil, "missing-slot"
  end
  local slot_text = tostring(slot)
  if not strings.is_bounded_string(slot_text, 120) then
    return nil, "malformed-slot"
  end
  return {
    reason = "stale",
    slot = slot_text,
    source_ref = {
      kind = "cron",
      ref = "audit_poll/slot/" .. strings.sanitize_key(slot_text, 120),
    },
  }, nil
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
  if forge_strings.split_repo(repo) == nil then
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

function M.validate_finding_shape(finding)
  return type(finding) == "table"
    and bounded(finding.file, file_limit)
    and type(finding.line) == "number"
    and finding.line >= 1
    and math.floor(finding.line) == finding.line
    and bounded(finding.rule, rule_limit)
    and bounded(finding.why, why_limit)
    and bounded(finding.suggested_fix, fix_limit)
end

function M.finding_line_exists(finding, text)
  if not M.validate_finding_shape(finding) then
    return false
  end
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

function M.validate_finding(finding)
  return M.validate_finding_shape(finding)
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

function M.audit_run_dedup_key(repo, now_seconds, max_staleness_seconds)
  if type(now_seconds) ~= "number" or type(max_staleness_seconds) ~= "number" or max_staleness_seconds < 1 then
    error("archaudit: invalid-audit-run-dedup-input: timestamps and staleness budget must be numeric")
  end
  local bucket = math.floor(now_seconds / max_staleness_seconds)
  local seed = tostring(repo) .. "|" .. tostring(bucket)
  local readable = table.concat({
    "archaudit-run",
    strings.sanitize_key(repo, 120),
    tostring(bucket),
    strings.decimal_checksum(seed),
  }, "/")
  return readable:sub(1, github_proxy_limits.dedup_key)
end

function M.audit_run_dedup_bucket(now_seconds, max_staleness_seconds)
  if type(now_seconds) ~= "number" or type(max_staleness_seconds) ~= "number" or max_staleness_seconds < 1 then
    error("archaudit: invalid-audit-run-dedup-input: timestamps and staleness budget must be numeric")
  end
  return math.floor(now_seconds / max_staleness_seconds)
end

function M.audit_run_current_window_seen(latest_seconds, now_seconds, max_staleness_seconds)
  if latest_seconds == nil then
    return false
  end
  if type(latest_seconds) ~= "number" or latest_seconds > now_seconds then
    return true
  end
  return M.audit_run_dedup_bucket(latest_seconds, max_staleness_seconds) == M.audit_run_dedup_bucket(now_seconds, max_staleness_seconds)
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

function M.build_audit_run_issue_create_request(repo, trigger_reason, label_available, now_seconds, max_staleness_seconds)
  assert_request_field(M.validate_repo(repo), "repo")
  local dedup_key = M.audit_run_dedup_key(repo, now_seconds, max_staleness_seconds)
  local title = "Archaudit: audit completed with zero findings"
  local body = audit_run_body(trigger_reason)
  local source_ref_ref = strings.sanitize_key(repo, 120) .. "#archaudit-run/" .. strings.decimal_checksum(dedup_key)
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
  return "fkst:archaudit:audit-run:v1"
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
  if tostring(issue.body or ""):find("fkst:archaudit:audit-run:v1", 1, true) == nil then
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

function M.audit_due_verdict(issues, trusted_login, now_seconds, max_staleness_seconds, completion_budget_seconds)
  if type(now_seconds) ~= "number" or type(max_staleness_seconds) ~= "number" or max_staleness_seconds < 1 then
    error("archaudit: invalid-audit-staleness-input: timestamps and staleness budget must be numeric")
  end
  local force_at_seconds = M.audit_due_force_at_seconds(max_staleness_seconds, completion_budget_seconds)
  local latest = M.latest_audit_issue_seconds(issues, trusted_login)
  if latest == nil then
    return true, "no durable audit issue marker", nil
  end
  if latest > now_seconds then
    return false, "latest audit issue marker is in the future", latest
  end
  local age_seconds = now_seconds - latest
  if age_seconds >= max_staleness_seconds then
    return true, "audit max staleness elapsed", latest
  end
  if age_seconds >= force_at_seconds then
    return true, "audit completion budget threshold elapsed", latest
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
