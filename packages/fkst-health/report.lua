-- Pure rendering of the v1 health report: its filename and its TOML front matter.
--
-- The authoritative consumer is the control plane's parser at
-- fkst-hosted backend/src/session_health/{report,naming}.rs. Everything here exists
-- to emit exactly what that parser accepts, so the two are kept deliberately close:
--
--   * front matter is TOML fenced by `+++`, NOT YAML and NOT `---`;
--   * TOML requires every scalar key to precede any table, so [[evidence]] and
--     [[work_items]] are rendered LAST -- get that wrong and the document is not
--     valid TOML at all;
--   * the filename stamp is UTC and colon-free, because the same string becomes an
--     object key, a tar entry path, and a URL path segment;
--   * an absent work-label namespace omits the segment AND its joining hyphen --
--     no placeholder, no leading hyphen -- because the parser splits the prefix by
--     anchoring on the trailing 36-character UUID.
--
-- This module is pure: it renders strings and touches nothing. The department owns
-- every fallible step.
local strings = require("contract.strings")

local R = {}

-- Bumped only alongside a change the control-plane parser must be taught about.
R.schema_version = 1
R.producer = "fkst-health@0.1.0"

R.filename_marker = "-health-agent-status-report-"
R.filename_suffix = ".md"

-- Bounds mirrored from the v1 contract. They are re-applied here, at the
-- serialization boundary, even though the core already bounds its verdict: this file
-- is the trust boundary to the control plane, and the collector silently SKIPS a
-- report larger than its ceiling -- a dropped heartbeat reads as a stalled engine.
R.report_byte_ceiling = 256 * 1024
R.headline_character_ceiling = 200
R.evidence_entry_ceiling = 32
R.evidence_key_ceiling = 64
R.evidence_value_ceiling = 256
R.work_item_ceiling = 64
R.work_item_field_ceiling = 64
R.confidence_ceiling = 32

-- Reserved for the front matter plus the fallback body, so a pathological narrative
-- can never push a report past the collector's ceiling.
local body_budget_floor = 4096

local function seconds(value)
  local number = tonumber(value)
  if type(number) ~= "number" or number ~= number or number == math.huge or number == -math.huge then
    return 0
  end
  return math.floor(number)
end

local function clip(value, ceiling)
  local text = tostring(value == nil and "" or value)
  if #text <= ceiling then
    return text
  end
  if type(truncate_utf8) == "function" then
    local ok, cut = pcall(truncate_utf8, text, ceiling)
    if ok and type(cut) == "string" then
      return cut
    end
  end
  return text:sub(1, ceiling)
end

local function flatten(value, ceiling)
  local text = tostring(value == nil and "" or value):gsub("%s+", " ")
  return clip((text:gsub("^%s+", ""):gsub("%s+$", "")), ceiling)
end

local function present(value)
  local text = strings.trim(value)
  if text == "" then
    return nil
  end
  return text
end

-- Filename segments must survive becoming a path component and an object key. The
-- parser rejects a name carrying a separator or a control character outright, so a
-- malformed environment value is sanitized here rather than silently producing a
-- report nothing downstream will index.
local function segment(value)
  local text = present(value)
  if text == nil then
    return nil
  end
  -- Dot runs collapse BEFORE the generic sanitizer, and any leading separator is
  -- stripped after it. The parser rejects a name that begins with `.` or carries a
  -- `..` component, so a hostile or merely malformed environment value must not be
  -- able to produce a filename nothing downstream will index.
  local safe = strings.runtime_safe_segment((text:gsub("%.%.+", "_")))
  safe = (safe:gsub("^[%._%-]+", ""))
  if safe == "" then
    return nil
  end
  return safe
end

--- "YYYYMMDD-HHMMSS", UTC, colon-free.
function R.stamp(epoch_seconds)
  return os.date("!%Y%m%d-%H%M%S", seconds(epoch_seconds))
end

--- RFC3339 in UTC, the only timestamp form the control-plane parser accepts.
function R.rfc3339(epoch_seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", seconds(epoch_seconds))
end

--- `<namespace>-<session_id>-health-agent-status-report-<YYYYMMDD>-<HHMMSS>.md`,
--- with the namespace segment and its joining hyphen omitted when unset.
function R.filename(namespace, session_id, epoch_seconds)
  local session = segment(session_id) or "unknown-session"
  local tail = R.filename_marker .. R.stamp(epoch_seconds) .. R.filename_suffix
  local namespaced = segment(namespace)
  if namespaced == nil then
    return session .. tail
  end
  return namespaced .. "-" .. session .. tail
end

-- A JSON string literal is also a valid TOML basic string: TOML accepts exactly the
-- escapes contract.strings.json_string emits (\b \t \n \f \r \" \\ \uXXXX). Reusing
-- it keeps one escaping implementation in the repository instead of a second copy.
local function quoted(key, value)
  return key .. " = " .. strings.json_string(value) .. "\n"
end

local function integer(key, value)
  return key .. " = " .. tostring(seconds(value)) .. "\n"
end

local function scalars(document, interval_seconds)
  local out = {}
  -- Order matters twice over: the schema marker first so a reader can reject an
  -- unknown version cheaply, and every scalar before the arrays-of-tables below.
  table.insert(out, "fkst_health_report = " .. tostring(R.schema_version) .. "\n")
  table.insert(out, quoted("session_id", flatten(document.session_id, R.evidence_key_ceiling)))
  local namespace = present(document.namespace)
  if namespace ~= nil then
    table.insert(out, quoted("namespace", flatten(namespace, R.evidence_key_ceiling)))
  end
  table.insert(out, quoted("producer", R.producer))
  table.insert(out, quoted("generated_at", R.rfc3339(document.generated_at)))
  if document.window_start ~= nil then
    table.insert(out, quoted("window_start", R.rfc3339(document.window_start)))
  end
  table.insert(out, integer("expected_interval_secs", interval_seconds))
  table.insert(out, quoted("status", flatten(document.status, R.confidence_ceiling)))
  table.insert(out, quoted("headline", flatten(document.headline, R.headline_character_ceiling)))
  local confidence = present(document.confidence)
  if confidence ~= nil then
    table.insert(out, quoted("confidence", flatten(confidence, R.confidence_ceiling)))
  end
  return out
end

local function evidence_tables(out, entries)
  local written = 0
  for _, entry in ipairs(type(entries) == "table" and entries or {}) do
    if written >= R.evidence_entry_ceiling then
      break
    end
    local key = type(entry) == "table" and flatten(entry.key, R.evidence_key_ceiling) or ""
    if key ~= "" then
      written = written + 1
      table.insert(out, "\n[[evidence]]\n")
      table.insert(out, quoted("key", key))
      table.insert(out, quoted("value", flatten(entry.value, R.evidence_value_ceiling)))
    end
  end
end

local function work_item_tables(out, items)
  local written = 0
  for _, item in ipairs(type(items) == "table" and items or {}) do
    if written >= R.work_item_ceiling then
      break
    end
    local number = type(item) == "table" and tonumber(item.number) or nil
    if number ~= nil and number == number and math.floor(number) == number then
      written = written + 1
      table.insert(out, "\n[[work_items]]\n")
      table.insert(out, "number = " .. string.format("%d", number) .. "\n")
      table.insert(out, quoted("state", flatten(item.state, R.work_item_field_ceiling)))
      table.insert(out, quoted("progress", flatten(item.progress, R.work_item_field_ceiling)))
    end
  end
end

--- Render one complete report document.
---
--- document = { session_id, namespace?, generated_at, window_start?, status,
---              headline, confidence?, evidence = {{key,value}...},
---              work_items = {{number,state,progress}...}, body }
---
--- Total, like the core: every field is coerced, so a malformed verdict still yields
--- a parseable report rather than no heartbeat at all.
function R.render(document, interval_seconds)
  document = type(document) == "table" and document or {}
  local out = scalars(document, interval_seconds)
  evidence_tables(out, document.evidence)
  work_item_tables(out, document.work_items)
  local front_matter = "+++\n" .. table.concat(out) .. "+++\n"
  local budget = R.report_byte_ceiling - #front_matter - 1
  if budget < body_budget_floor then
    budget = body_budget_floor
  end
  local body = clip(tostring(document.body == nil and "" or document.body), budget)
  if body ~= "" and body:sub(-1) ~= "\n" then
    body = body .. "\n"
  end
  return front_matter .. body
end

return R
