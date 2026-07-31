-- fkst-health evidence core: pure, total, deterministic.
--
-- WHY this file is pure and why the rules -- not the codex -- own the verdict:
-- fkst-health ships in the default manifest, so it rides every session on the
-- platform and a defect here is a fleet-wide defect. Keeping the decision in plain
-- Lua over plain tables makes every verdict auditable, reproducible under unit test,
-- and reachable even when the narrating codex fails.
--
-- WHY it is named health.lua and not core.lua: the G-DEVLOOP-SERVICE-LOCATOR
-- ratchet counts `require("core")` and `core.<member>` reads across every package's
-- department files and is shrink-only, so a new department requiring "core" fails CI.
--
-- Totality contract: H.decide MUST return a valid verdict for EVERY input -- nil,
-- a scalar, an empty table, malformed nested values, partially unreadable probes --
-- and MUST NOT error. Callers rely on that to keep a heartbeat alive when the pod
-- around them is misbehaving.
local H = {}

-- The v1 status taxonomy, byte-identical to the control plane's HealthStatus enum
-- (fkst-hosted backend/src/session_health/report.rs). The control plane relays the
-- string verbatim and maps anything it does not recognise to `unknown`, so emitting
-- outside this set silently degrades every report.
local status_set = {
  working = true,
  idle = true,
  blocked = true,
  stalled = true,
  failing = true,
  unknown = true,
}

H.statuses = { "working", "idle", "blocked", "stalled", "failing", "unknown" }

-- The raiser's cron interval in seconds, rendered into every report as
-- expected_interval_secs so the control plane's staleness watchdog reads the
-- producer's OWN declared cadence instead of hardcoding a package's tick. Change
-- this and raisers/health_poll.lua together or the watchdog misjudges staleness.
H.expected_interval_seconds = 600

-- A fault must recur at least this often inside one window before a session that is
-- still producing output is called `blocked` rather than `working`. One repeat is
-- ordinary retry behaviour; three is a pattern.
H.fault_recurrence_threshold = 3

-- A single no-progress window is weak evidence: from the outside a long codex turn
-- looks exactly like a wedge. Confidence only rises once a second consecutive window
-- agrees. The control plane renders confidence but never acts on it.
H.stall_confidence_windows = 2

-- Bounds mirrored from the v1 contract so a well-formed report is never dropped
-- downstream. Named without a max_*_len / max_*_bytes shape on purpose: those names
-- are what the G-CONTENT-TRUNCATION ratchet inventories.
H.evidence_entry_ceiling = 32
H.work_item_ceiling = 64
H.headline_character_ceiling = 200

-- Signals that can speak to forward movement. `faults` is deliberately absent: a
-- readable log that shows no errors says nothing about whether work advanced.
local progress_bearing = { "deliveries", "codex", "repository", "work_items" }

local function as_table(value)
  if type(value) == "table" then
    return value
  end
  return {}
end

-- A non-negative integer, or nil when the value is absent, malformed, negative,
-- fractional, NaN, or infinite. Every count in a verdict flows through here so a
-- malformed probe degrades one field instead of erroring the tick.
local function counter(row, name)
  local value = tonumber(as_table(row)[name])
  if type(value) ~= "number" then
    return nil
  end
  if value ~= value or value == math.huge or value == -math.huge then
    return nil
  end
  if value < 0 or math.floor(value) ~= value then
    return nil
  end
  return value
end

local function positive(row, name)
  local value = counter(row, name)
  return value ~= nil and value > 0
end

local function flag(row, name)
  return as_table(row)[name] == true
end

local function bounded_text(value, ceiling)
  local text = tostring(value == nil and "" or value)
  if #text <= ceiling then
    return text
  end
  -- truncate_utf8 is an engine SDK primitive; falling back keeps this module usable
  -- (and unit-testable) with no engine globals present at all.
  if type(truncate_utf8) == "function" then
    local ok, cut = pcall(truncate_utf8, text, ceiling)
    if ok and type(cut) == "string" then
      return cut
    end
  end
  return text:sub(1, ceiling)
end

-- Collapse whitespace so an interpolated probe string can never inject a newline
-- into the TOML front matter's headline.
local function one_line(value, ceiling)
  local text = tostring(value == nil and "" or value):gsub("%s+", " ")
  return bounded_text((text:gsub("^%s+", ""):gsub("%s+$", "")), ceiling)
end

function H.is_status(value)
  return type(value) == "string" and status_set[value] == true
end

-- A signal counts as read only when the probe explicitly said so. An absent or
-- malformed signal is unreadable, never silently treated as "nothing happened".
local function readable(observations, name)
  local signal = as_table(observations)[name]
  if type(signal) ~= "table" or signal.readable ~= true then
    return nil
  end
  return signal
end

local function any_readable(observations, names)
  for _, name in ipairs(names) do
    if readable(observations, name) ~= nil then
      return true
    end
  end
  return false
end

-- Forward movement: something completed, landed, or closed inside the window.
local function progress_facts(observations)
  local facts = {}
  local deliveries = readable(observations, "deliveries")
  if deliveries ~= nil and positive(deliveries, "completed_delta") then
    table.insert(facts, "deliveries completed=" .. tostring(counter(deliveries, "completed_delta")))
  end
  local codex = readable(observations, "codex")
  if codex ~= nil and positive(codex, "runs_finished") then
    table.insert(facts, "codex runs finished=" .. tostring(counter(codex, "runs_finished")))
  end
  local repository = readable(observations, "repository")
  if repository ~= nil then
    if positive(repository, "commits") then
      table.insert(facts, "commits=" .. tostring(counter(repository, "commits")))
    end
    if positive(repository, "new_branches") then
      table.insert(facts, "new branches=" .. tostring(counter(repository, "new_branches")))
    end
    if positive(repository, "new_pull_requests") then
      table.insert(facts, "new PRs=" .. tostring(counter(repository, "new_pull_requests")))
    end
  end
  local work_items = readable(observations, "work_items")
  if work_items ~= nil and positive(work_items, "closed_delta") then
    table.insert(facts, "work items closed=" .. tostring(counter(work_items, "closed_delta")))
  end
  return facts
end

-- Output without forward movement: the session is doing things but nothing has
-- landed. This is what separates `blocked` (busy, wedged on one fault) from
-- `stalled` (nothing happening at all).
local function producing_output(observations)
  local codex = readable(observations, "codex")
  if codex ~= nil and (positive(codex, "runs_started") or positive(codex, "running")) then
    return true
  end
  local deliveries = readable(observations, "deliveries")
  if deliveries ~= nil and (positive(deliveries, "in_flight") or positive(deliveries, "retrying")) then
    return true
  end
  return false
end

local function failing_reason(observations)
  local faults = readable(observations, "faults")
  if faults ~= nil and flag(faults, "framework_erroring") then
    return one_line(faults.top_fault or "framework reported an error", 120)
  end
  local deliveries = readable(observations, "deliveries")
  if deliveries ~= nil and positive(deliveries, "dead_letter_delta") then
    return "dead letters grew by " .. tostring(counter(deliveries, "dead_letter_delta"))
  end
  return nil
end

local function recurring_fault(observations)
  local faults = readable(observations, "faults")
  if faults == nil then
    return nil
  end
  local count = counter(faults, "recurring")
  if count == nil or count < H.fault_recurrence_threshold then
    return nil
  end
  return count, one_line(faults.top_fault or "an unnamed fault", 120)
end

local function open_work_items(observations)
  local work_items = readable(observations, "work_items")
  if work_items == nil then
    return nil
  end
  return counter(work_items, "open")
end

-- The ordered decision rules. First match wins; every branch returns a taxonomy
-- string. `unknown` is the fallthrough, NOT a reaction to a single failed probe:
-- unreadable deliveries plus visible commits is `working`, never `unknown`.
local function decide_status(observations)
  local open = open_work_items(observations)
  if open ~= nil and open == 0 then
    return "idle", nil
  end

  local failure = failing_reason(observations)
  if failure ~= nil then
    return "failing", failure
  end

  local facts = progress_facts(observations)
  if #facts > 0 then
    return "working", table.concat(facts, ", ")
  end

  local count, fault = recurring_fault(observations)
  if count ~= nil and producing_output(observations) then
    return "blocked", fault .. " recurred " .. tostring(count) .. " times"
  end

  -- Only claim a stall when some signal that CAN show movement was actually read.
  -- Concluding "no progress" from zero progress-bearing evidence would raise a false
  -- alarm on every session whose probes happened to fail.
  if any_readable(observations, progress_bearing) then
    return "stalled", nil
  end

  return "unknown", nil
end

local function consecutive_windows(observations)
  return counter(as_table(observations).window, "consecutive_no_progress") or 0
end

local function confidence_for(status, observations)
  if status == "unknown" then
    return "low"
  end
  if status == "stalled" then
    if consecutive_windows(observations) >= H.stall_confidence_windows then
      return "high"
    end
    return "low"
  end
  local total = 0
  for _, name in ipairs(progress_bearing) do
    if readable(observations, name) ~= nil then
      total = total + 1
    end
  end
  if readable(observations, "faults") ~= nil then
    total = total + 1
  end
  if total >= #progress_bearing + 1 then
    return "high"
  end
  return "medium"
end

--- The dominant fault's detail table, when the faults probe read one.
local function fault_detail(observations)
  local faults = as_table(as_table(observations).faults)
  local detail = faults.detail
  return type(detail) == "table" and detail or nil
end

H.fault_detail = fault_detail

local function headline_for(status, detail, observations)
  local open = open_work_items(observations)
  local items = open == nil and "an unknown number of" or tostring(open)
  if status == "working" then
    return "Session is making progress: " .. tostring(detail) .. "."
  end
  if status == "idle" then
    return "No open work items; the pod is inside its reap grace window."
  end
  if status == "failing" then
    -- Name the thing that is broken. "dead letters grew by 2" is a number; a reader
    -- needs the work item and the department to act on it.
    local fault = fault_detail(observations)
    if fault ~= nil then
      local where = fault.work_item ~= nil and ("work item #" .. tostring(fault.work_item))
        or ("queue " .. tostring(fault.queue))
      return where
        .. " keeps failing and has exhausted its retries ("
        .. tostring(fault.count)
        .. " dead letter(s)"
        .. (fault.dept ~= nil and ("; " .. fault.dept) or "")
        .. ")."
    end
    return "The framework is erroring: " .. tostring(detail) .. "."
  end
  if status == "blocked" then
    return "Producing output but wedged: " .. tostring(detail) .. "."
  end
  if status == "stalled" then
    local windows = consecutive_windows(observations)
    return "No progress in the last "
      .. tostring(math.floor(H.expected_interval_seconds / 60))
      .. "m with "
      .. items
      .. " work item(s) open (consecutive quiet windows: "
      .. tostring(windows)
      .. ")."
  end
  return "No health signal could be read this window."
end

local function push(list, key, value)
  if #list >= H.evidence_entry_ceiling then
    return
  end
  table.insert(list, { key = key, value = one_line(value, 240) })
end

local function signal_evidence(list, observations, name, fields)
  local signal = readable(observations, name)
  if signal == nil then
    local raw = as_table(observations)[name]
    local why = type(raw) == "table" and raw.why or nil
    push(list, name .. "_readable", "false")
    if why ~= nil then
      push(list, name .. "_unreadable_why", why)
    end
    return
  end
  push(list, name .. "_readable", "true")
  for _, field in ipairs(fields) do
    local value = counter(signal, field)
    if value ~= nil then
      push(list, name .. "_" .. field, value)
    elseif type(signal[field]) == "boolean" then
      push(list, name .. "_" .. field, tostring(signal[field]))
    end
  end
end

local function build_evidence(observations, status)
  local list = {}
  push(list, "status_rule", status)
  push(list, "consecutive_no_progress_windows", consecutive_windows(observations))
  signal_evidence(list, observations, "deliveries", {
    "completed_delta", "in_flight", "retrying", "depth", "dead_letters", "dead_letter_delta",
  })
  signal_evidence(list, observations, "codex", { "runs_started", "runs_finished", "running" })
  signal_evidence(list, observations, "repository", {
    "commits", "new_branches", "new_pull_requests",
  })
  signal_evidence(list, observations, "work_items", { "open", "closed_delta" })
  signal_evidence(list, observations, "faults", { "recurring", "framework_erroring" })
  local faults = readable(observations, "faults")
  if faults ~= nil and faults.top_fault ~= nil then
    push(list, "faults_top", faults.top_fault)
  end
  return list
end

-- Work items are relayed for display only; every field is coerced and bounded here
-- because the control plane trusts a producer's numbers but not its lengths.
local function build_work_items(observations)
  local signal = readable(observations, "work_items")
  local out = {}
  if signal == nil then
    return out
  end
  for _, item in ipairs(type(signal.items) == "table" and signal.items or {}) do
    if #out >= H.work_item_ceiling then
      break
    end
    local number = counter(item, "number")
    if number ~= nil then
      table.insert(out, {
        number = number,
        state = one_line(as_table(item).state or "unknown", 64),
        progress = one_line(as_table(item).progress or "unknown", 64),
      })
    end
  end
  return out
end

-- H.decide(observations) -> verdict
--
-- observations is a table of independently-probed signals, each shaped
--   { readable = <boolean>, why = <string when unreadable>, <counters...> }
-- plus an optional `window = { consecutive_no_progress = <integer> }`.
--
-- The returned verdict is always well formed:
--   { status, confidence, headline, evidence = {{key,value}...},
--     work_items = {{number,state,progress}...}, progressed = <boolean> }
function H.decide(observations)
  local status, detail = decide_status(observations)
  if not H.is_status(status) then
    status = "unknown"
  end
  return {
    status = status,
    confidence = confidence_for(status, observations),
    headline = one_line(headline_for(status, detail, observations), H.headline_character_ceiling),
    evidence = build_evidence(observations, status),
    work_items = build_work_items(observations),
    progressed = status == "working",
  }
end

return H
