-- Turning raw engine facts into the signal tables the pure core decides from.
--
-- Every function here is pure: it takes an already-fetched fact table and returns a
-- signal. The department owns the fallible fetching, so one probe blowing up degrades
-- exactly one signal and never the tick.
--
-- WINDOW MEMORY. Several signals are deltas ("did anything COMPLETE since last
-- tick?"), which needs last tick's counters. They live in the engine's host-local
-- scratch cache (cache_get / cache_set) rather than on disk under the runtime root:
-- the cache is explicitly best-effort scratch, so losing it degrades one window to
-- "no delta observed" instead of corrupting a verdict. This is emphatically not
-- durable business state -- nothing here is required to survive a crash.
local P = {}

-- The delivery-id set is what makes "a delivery completed" observable at all: an id
-- present last window and absent now finished. It is bounded because the cache is
-- scratch, and observe already caps its own delivery list.
P.memory_key = "fkst-health/window-memory"
P.remembered_id_ceiling = 64
local id_text_ceiling = 48
local queue_counters = { "depth", "pending", "in_flight", "retrying" }

local function whole(value)
  local number = tonumber(value)
  if type(number) ~= "number" or number ~= number or number == math.huge or number == -math.huge then
    return nil
  end
  if number < 0 or math.floor(number) ~= number then
    return nil
  end
  return number
end

local function rows(facts, name)
  local list = type(facts) == "table" and facts[name] or nil
  if type(list) ~= "table" then
    return {}
  end
  return list
end

local function tally(list)
  local total = 0
  for _ in ipairs(list) do
    total = total + 1
  end
  return total
end

-- max(0, minuend - subtrahend), treating an absent subtrahend as "no delta known".
-- Every delta in this file floors at zero: a counter that went backwards (a reaped
-- history, a restarted engine) must never read as work happening in reverse.
local function nonneg_delta(minuend, subtrahend)
  if minuend == nil or subtrahend == nil then
    return 0
  end
  local delta = minuend - subtrahend
  if delta < 0 then
    return 0
  end
  return delta
end

--- Decode the previous window's counters. A missing, malformed, or expired entry
--- yields a blank memory, which reads as "no delta observed" -- never as progress.
function P.recall(cache_get)
  local memory = { deliveries = nil, dead_letters = nil, running = nil, recent = nil, open_items = nil, quiet = 0, ids = {} }
  if type(cache_get) ~= "function" then
    return memory
  end
  local ok, raw = pcall(cache_get, P.memory_key)
  if not ok or type(raw) ~= "string" or raw == "" then
    return memory
  end
  for key, value in raw:gmatch("([a-z_]+)=([^|]*)") do
    if key == "ids" then
      for id in value:gmatch("[^,]+") do
        memory.ids[id] = true
      end
    else
      memory[key] = whole(value)
    end
  end
  memory.quiet = memory.quiet or 0
  return memory
end

--- Encode this window's counters for the next tick. Best effort: a cache write that
--- fails costs one window's deltas and nothing else.
function P.remember(cache_set, snapshot)
  if type(cache_set) ~= "function" or type(snapshot) ~= "table" then
    return false
  end
  local ids, written = {}, 0
  for _, id in ipairs(type(snapshot.ids) == "table" and snapshot.ids or {}) do
    if written >= P.remembered_id_ceiling then
      break
    end
    local text = tostring(id)
    -- Stored VERBATIM, never sanitized: recall compares these against the raw ids the
    -- next observe reports, so any rewriting here would make every remembered id look
    -- absent next window and turn a quiet session into a false `working`. An id that
    -- could corrupt the record's own separators is dropped instead of mangled.
    if #text <= id_text_ceiling and text:find("[,|=]") == nil then
      written = written + 1
      table.insert(ids, text)
    end
  end
  local parts = { "v=1" }
  for _, key in ipairs({ "deliveries", "dead_letters", "running", "recent", "open_items", "quiet" }) do
    local value = whole(snapshot[key])
    if value ~= nil then
      table.insert(parts, key .. "=" .. tostring(value))
    end
  end
  table.insert(parts, "ids=" .. table.concat(ids, ","))
  return pcall(cache_set, P.memory_key, table.concat(parts, "|")) == true
end

local function queue_totals(facts)
  local totals = { depth = 0, pending = 0, in_flight = 0, retrying = 0 }
  for _, row in ipairs(rows(facts, "queues")) do
    if type(row) == "table" then
      for _, name in ipairs(queue_counters) do
        local value = whole(row[name])
        if value ~= nil then
          totals[name] = totals[name] + value
        end
      end
    end
  end
  return totals
end

local function delivery_ids(facts)
  local ids, seen = {}, {}
  for _, row in ipairs(rows(facts, "deliveries")) do
    local id = type(row) == "table" and row.delivery_id or nil
    if type(id) == "string" and id ~= "" and seen[id] == nil then
      seen[id] = true
      table.insert(ids, id)
    end
  end
  return ids, seen
end

-- A dead-lettered delivery is a fault that already exhausted its retries, and the
-- QUEUE NAME is the only thing we group by. That is a security decision as much as a
-- design one: queue names come from package source and can never carry a credential,
-- whereas any free-text error surface could. The observe snapshot's dead-letter rows
-- carry no message at all -- only a payload digest -- so nothing session-authored
-- ever reaches the narrative.
local function repeated_fault(facts)
  local counts, top, top_count = {}, nil, 0
  for _, row in ipairs(rows(facts, "dead_letters")) do
    local queue = type(row) == "table" and row.queue or nil
    if type(queue) == "string" and queue ~= "" then
      counts[queue] = (counts[queue] or 0) + 1
      if counts[queue] > top_count then
        top, top_count = queue, counts[queue]
      end
    end
  end
  return top, top_count
end

local function unreadable(why)
  return { readable = false, why = why }
end

P.unreadable = unreadable

--- Derive the `deliveries` and `faults` signals plus the memory snapshot fields from
--- one observe result. Returns three tables; both signals are unreadable when the
--- snapshot is absent or is not the schema this package understands.
function P.from_observe(facts, memory)
  if type(facts) ~= "table" then
    return unreadable("observe snapshot unavailable"), unreadable("observe snapshot unavailable"), {}
  end
  if whole(facts.schema_version) ~= 1 then
    return unreadable("unsupported observe schema"), unreadable("unsupported observe schema"), {}
  end

  memory = type(memory) == "table" and memory or {}
  local totals = queue_totals(facts)
  local ids, seen = delivery_ids(facts)
  local dead_letters = tally(rows(facts, "dead_letters"))

  -- Completed = present in the previous window's in-flight set, gone now.
  local completed = 0
  for id in pairs(type(memory.ids) == "table" and memory.ids or {}) do
    if seen[id] == nil then
      completed = completed + 1
    end
  end

  local top_fault, recurrence = repeated_fault(facts)
  local truncated = type(facts.truncated) == "table" and facts.truncated or {}

  local deliveries = {
    readable = true,
    completed_delta = completed,
    in_flight = totals.in_flight,
    retrying = totals.retrying,
    depth = totals.depth,
    dead_letters = dead_letters,
    dead_letter_delta = nonneg_delta(dead_letters, memory.dead_letters),
  }
  local faults = {
    readable = true,
    recurring = recurrence,
    -- More dead letters than the snapshot can carry is not a package's bad window;
    -- it is an engine in trouble, and the only unambiguous "framework is erroring"
    -- fact this surface exposes.
    framework_erroring = truncated.dead_letters == true,
    top_fault = top_fault,
  }
  return deliveries, faults, {
    deliveries = tally(rows(facts, "deliveries")),
    dead_letters = dead_letters,
    ids = ids,
  }
end

--- Derive the `codex` signal from an fkst.codex_runs() status table.
function P.from_codex_runs(status, memory)
  if type(status) ~= "table" or type(status.running) ~= "table" then
    return unreadable("codex run status unavailable"), {}
  end
  memory = type(memory) == "table" and memory or {}
  local running = tally(status.running)
  local recent = tally(type(status.recent) == "table" and status.recent or {})
  local finished = nonneg_delta(recent, memory.recent)
  return {
    readable = true,
    running = running,
    runs_finished = finished,
    -- A run that finished must have started, and a rise in the live count is a start
    -- we can see directly. Both are floored at zero so a reaped history cannot read
    -- as negative work.
    runs_started = finished + nonneg_delta(running, memory.running),
  }, { running = running, recent = recent }
end

--- Derive the `repository` signal from a commit count for the window.
---
--- Only commits are probed. Branch and PR enumeration would each cost a `gh` call
--- every ten minutes on every session on the platform, and a commit is the signal
--- that actually distinguishes a working session from a wedged one.
function P.from_commit_count(count)
  local commits = whole(count)
  if commits == nil then
    return unreadable("commit count unavailable")
  end
  return { readable = true, commits = commits }
end

--- Derive the `work_items` signal from a decoded `gh issue list` result.
function P.from_issue_list(issues, memory)
  if type(issues) ~= "table" then
    return unreadable("work item query unavailable"), {}
  end
  memory = type(memory) == "table" and memory or {}
  local items, open = {}, 0
  for _, issue in ipairs(issues) do
    if type(issue) == "table" then
      local number = whole(issue.number)
      local state = tostring(issue.state or "unknown"):lower()
      if number ~= nil then
        if state == "open" then
          open = open + 1
        end
        table.insert(items, { number = number, state = state, progress = "unknown" })
      end
    end
  end
  return {
    readable = true,
    open = open,
    -- Items that were open last window and are not now were closed. A rise in the
    -- open count is new work, not negative progress, so the delta floors at zero.
    closed_delta = nonneg_delta(memory.open_items or open, open),
    items = items,
  }, { open_items = open }
end

--- The consecutive-no-progress counter the core turns into stall confidence.
function P.next_quiet_windows(memory, progressed)
  if progressed then
    return 0
  end
  local previous = whole(type(memory) == "table" and memory.quiet or nil) or 0
  return previous + 1
end

return P
