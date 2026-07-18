-- workflow.engine.marker
--
-- Blueprint / materialization / terminal / lineage HTML-comment markers, with the
-- marker NAMESPACE parameterized out of the kernel.
--
-- The namespace token (e.g. "fkst:workflow-engine", "fkst:workflow-security") is
-- injected per adapter via `M.for_namespace(ns)`, which returns a table exposing the
-- build_*/parse_* functions bound to `ns`. Two reasons this MUST be injected:
--   1. a hardcoded product token would trip the workflow library conformance pack; and
--   2. co-resident adapters sharing one issue must not read/overwrite each other's markers.
--
-- Namespace-free helpers (`latest_materialization_by_slot`, the state tables, the byte
-- limits) stay directly on M and are shared by every bound table.
local strings = require("contract.strings")
local fail = require("workflow.engine.errors").fail

local M = {}

M.DEFAULT_NAMESPACE = "fkst:workflow-engine"

M.MAX_ORIGIN_PROPOSAL_ID_BYTES = 200
M.MAX_WORKFLOW_ID_BYTES = 128
M.MAX_PLAN_DIGEST_BYTES = 64
M.MAX_SLOT_ID_BYTES = 128
M.MAX_MATERIALIZATION_DIGEST_BYTES = M.MAX_PLAN_DIGEST_BYTES
M.MAX_CHILD_DEDUP_KEY_BYTES = 512
M.MAX_CHILD_ISSUE_BYTES = 30
M.MAX_TERMINAL_REASON_CODE_BYTES = 128

M.MATERIALIZATION_STATES = {
  pending = true,
  generated = true,
  created = true,
}

M.MATERIALIZATION_STATE_RANK = {
  pending = 1,
  generated = 2,
  created = 3,
}

M.TERMINAL_STATES = {
  done = true,
  blocked = true,
  error = true,
}

local function attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

-- Escape Lua-pattern magic characters so an arbitrary namespace token (which may contain
-- '-' and other magic) can be spliced into a gmatch pattern literally.
local function escape_pattern(text)
  return (tostring(text):gsub("([%^%$%(%)%%%.%[%]%*%+%-%?])", "%%%1"))
end

local function marker_pattern(ns, kind)
  return "<!%-%- " .. escape_pattern(ns) .. ":" .. kind .. ":v1.-%-%->"
end

local function validate_attr(value, path, limit)
  if type(value) ~= "string" then
    return false, fail(path, "not_string", "must be a string")
  end
  if value == "" then
    return false, fail(path, "empty", "must not be empty")
  end
  if #value > limit then
    return false, fail(path, "too_large", "exceeds byte limit", {
      max_bytes = limit,
      actual_bytes = #value,
    })
  end
  if value:find("%c") ~= nil or value:find('"', 1, true) ~= nil or value:find("[<>]") ~= nil then
    return false, fail(path, "invalid_marker_attr", "must be safe for a marker attribute")
  end
  if not strings.is_path_safe_key(value, limit) then
    return false, fail(path, "invalid_key", "must be a safe bounded key")
  end
  return true, nil
end

local function validate_origin(value, path)
  return validate_attr(value, path, M.MAX_ORIGIN_PROPOSAL_ID_BYTES)
end

local function validate_workflow(value, path)
  return validate_attr(value, path, M.MAX_WORKFLOW_ID_BYTES)
end

local function validate_digest(value, path)
  return validate_attr(value, path, M.MAX_PLAN_DIGEST_BYTES)
end

local function validate_slot(value, path)
  return validate_attr(value, path, M.MAX_SLOT_ID_BYTES)
end

local function validate_materialization_digest(value, path)
  return validate_attr(value, path, M.MAX_MATERIALIZATION_DIGEST_BYTES)
end

local function validate_child_dedup(value, path)
  return validate_attr(value, path, M.MAX_CHILD_DEDUP_KEY_BYTES)
end

local function validate_child_issue(value, path)
  if value == nil or value == "" then
    return true, nil, ""
  end
  if type(value) ~= "string" then
    return false, fail(path, "not_string", "must be a string")
  end
  if #value > M.MAX_CHILD_ISSUE_BYTES then
    return false, fail(path, "too_large", "exceeds byte limit", {
      max_bytes = M.MAX_CHILD_ISSUE_BYTES,
      actual_bytes = #value,
    })
  end
  if value:find("%c") ~= nil or value:find('"', 1, true) ~= nil or value:find("[<>]") ~= nil then
    return false, fail(path, "invalid_marker_attr", "must be safe for a marker attribute")
  end
  if value:match("^%d+$") == nil then
    return false, fail(path, "invalid_issue_number", "must be a numeric issue string")
  end
  return true, nil, value
end

local function validate_member(value, path, allowed, code)
  if type(value) ~= "string" then
    return false, fail(path, "not_string", "must be a string")
  end
  if allowed[value] ~= true then
    return false, fail(path, code, "is not an allowed value")
  end
  return true, nil
end

local function validate_materialization_state(value, path)
  return validate_member(value, path, M.MATERIALIZATION_STATES, "invalid_materialization_state")
end

local function validate_terminal_state(value, path)
  return validate_member(value, path, M.TERMINAL_STATES, "invalid_terminal_state")
end

local function validate_reason_code(value, path)
  return validate_attr(value, path, M.MAX_TERMINAL_REASON_CODE_BYTES)
end

-- ---------------------------------------------------------------------------
-- Namespace-free fact extractors (attribute readers). These inspect an already
-- matched marker string, so they never need the namespace token.
-- ---------------------------------------------------------------------------

local function fact_from_marker(marker, origin_proposal_id)
  local origin = attr(marker, "origin")
  local workflow = attr(marker, "workflow")
  local digest = attr(marker, "digest")
  local ok = validate_origin(origin, "origin")
  if not ok then return nil end
  ok = validate_workflow(workflow, "workflow")
  if not ok then return nil end
  ok = validate_digest(digest, "digest")
  if not ok then return nil end
  if origin ~= tostring(origin_proposal_id) then
    return nil
  end
  return {
    origin = origin,
    workflow = workflow,
    digest = digest,
  }
end

local function materialization_fact_from_marker(marker, origin_proposal_id, slot_id)
  local origin = attr(marker, "origin")
  local blueprint_digest = attr(marker, "blueprint_digest")
  local slot = attr(marker, "slot")
  local predecessor_ref_digest = attr(marker, "predecessor_ref_digest")
  local gen_contract_digest = attr(marker, "gen_contract_digest")
  local gen_spec_digest = attr(marker, "gen_spec_digest")
  local child_dedup = attr(marker, "child_dedup")
  local child_issue = attr(marker, "child_issue")
  local state = attr(marker, "state")

  local ok = validate_origin(origin, "origin")
  if not ok then return nil end
  ok = validate_materialization_digest(blueprint_digest, "blueprint_digest")
  if not ok then return nil end
  ok = validate_slot(slot, "slot")
  if not ok then return nil end
  ok = validate_materialization_digest(predecessor_ref_digest, "predecessor_ref_digest")
  if not ok then return nil end
  ok = validate_materialization_digest(gen_contract_digest, "gen_contract_digest")
  if not ok then return nil end
  ok = validate_materialization_digest(gen_spec_digest, "gen_spec_digest")
  if not ok then return nil end
  ok = validate_child_dedup(child_dedup, "child_dedup")
  if not ok then return nil end
  ok = validate_child_issue(child_issue, "child_issue")
  if not ok then return nil end
  ok = validate_materialization_state(state, "state")
  if not ok then return nil end
  if origin ~= tostring(origin_proposal_id) then
    return nil
  end
  if slot_id ~= nil and slot ~= tostring(slot_id) then
    return nil
  end

  return {
    origin = origin,
    blueprint_digest = blueprint_digest,
    slot = slot,
    predecessor_ref_digest = predecessor_ref_digest,
    gen_contract_digest = gen_contract_digest,
    gen_spec_digest = gen_spec_digest,
    child_dedup = child_dedup,
    child_issue = child_issue ~= "" and child_issue or nil,
    state = state,
  }
end

local function terminal_fact_from_marker(marker, origin_proposal_id)
  local origin = attr(marker, "origin")
  local state = attr(marker, "state")
  local reason_code = attr(marker, "reason_code")
  local ok = validate_origin(origin, "origin")
  if not ok then return nil end
  ok = validate_terminal_state(state, "state")
  if not ok then return nil end
  ok = validate_reason_code(reason_code, "reason_code")
  if not ok then return nil end
  if origin ~= tostring(origin_proposal_id) then
    return nil
  end
  return {
    origin = origin,
    state = state,
    reason_code = reason_code,
  }
end

local function lineage_fact_from_marker(marker)
  local origin = attr(marker, "origin")
  local blueprint_digest = attr(marker, "blueprint_digest")
  local slot = attr(marker, "slot")
  local ok = validate_origin(origin, "origin")
  if not ok then return nil end
  ok = validate_digest(blueprint_digest, "blueprint_digest")
  if not ok then return nil end
  ok = validate_slot(slot, "slot")
  if not ok then return nil end
  return {
    origin = origin,
    blueprint_digest = blueprint_digest,
    slot = slot,
  }
end

-- Namespace-free: pick the highest-ranked materialization fact per slot.
function M.latest_materialization_by_slot(facts)
  local by_slot = {}
  for _, fact in ipairs(facts or {}) do
    if type(fact) == "table" and type(fact.slot) == "string" and type(fact.state) == "string" then
      local rank = M.MATERIALIZATION_STATE_RANK[fact.state]
      if rank ~= nil then
        local current = by_slot[fact.slot]
        local current_rank = current ~= nil and M.MATERIALIZATION_STATE_RANK[current.state] or nil
        -- State is monotonic pending -> generated -> created; highest rank wins, and
        -- repeated same-state records use latest stream order for replay freshness.
        if current == nil or current_rank == nil or rank >= current_rank then
          by_slot[fact.slot] = fact
        end
      end
    end
  end
  return by_slot
end

-- ---------------------------------------------------------------------------
-- Namespace-bound builders/parsers. `bound(ns)` closes the four emitters and four
-- gmatch patterns over the injected namespace token and returns them as a table.
-- ---------------------------------------------------------------------------

local function bound(ns)
  local B = { namespace = ns }

  local blueprint_pattern = marker_pattern(ns, "blueprint")
  local materialization_pattern = marker_pattern(ns, "materialization")
  local terminal_pattern = marker_pattern(ns, "terminal")
  local lineage_pattern = marker_pattern(ns, "lineage")

  function B.build_blueprint_marker(origin_proposal_id, workflow_id, plan_digest)
    local ok, err = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then return nil, err end
    ok, err = validate_workflow(workflow_id, "workflow_id")
    if not ok then return nil, err end
    ok, err = validate_digest(plan_digest, "plan_digest")
    if not ok then return nil, err end

    return "<!-- " .. ns .. ":blueprint:v1 origin=\"" .. origin_proposal_id
      .. "\" workflow=\"" .. workflow_id
      .. "\" digest=\"" .. plan_digest
      .. "\" -->",
      nil
  end

  function B.parse_blueprint_marker(comment_body, origin_proposal_id)
    if type(comment_body) ~= "string" then
      return nil
    end
    local ok = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then
      return nil
    end

    -- Caller owns bot-author trust filtering; this parser only inspects one body string.
    local latest_marker = nil
    for marker in comment_body:gmatch(blueprint_pattern) do
      if attr(marker, "origin") == tostring(origin_proposal_id) then
        latest_marker = marker
      end
    end
    if latest_marker == nil then
      return nil
    end
    return fact_from_marker(latest_marker, origin_proposal_id)
  end

  function B.build_materialization_marker(
    origin_proposal_id,
    blueprint_digest,
    slot_id,
    predecessor_ref_digest,
    generator_contract_digest,
    generated_spec_digest,
    child_dedup_key,
    child_issue,
    state
  )
    local ok, err = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then return nil, err end
    ok, err = validate_materialization_digest(blueprint_digest, "blueprint_digest")
    if not ok then return nil, err end
    ok, err = validate_slot(slot_id, "slot_id")
    if not ok then return nil, err end
    ok, err = validate_materialization_digest(predecessor_ref_digest, "predecessor_ref_digest")
    if not ok then return nil, err end
    ok, err = validate_materialization_digest(generator_contract_digest, "generator_contract_digest")
    if not ok then return nil, err end
    ok, err = validate_materialization_digest(generated_spec_digest, "generated_spec_digest")
    if not ok then return nil, err end
    ok, err = validate_child_dedup(child_dedup_key, "child_dedup_key")
    if not ok then return nil, err end
    local child_issue_attr
    ok, err, child_issue_attr = validate_child_issue(child_issue, "child_issue")
    if not ok then return nil, err end
    ok, err = validate_materialization_state(state, "state")
    if not ok then return nil, err end

    return "<!-- " .. ns .. ":materialization:v1 origin=\"" .. origin_proposal_id
      .. "\" blueprint_digest=\"" .. blueprint_digest
      .. "\" slot=\"" .. slot_id
      .. "\" predecessor_ref_digest=\"" .. predecessor_ref_digest
      .. "\" gen_contract_digest=\"" .. generator_contract_digest
      .. "\" gen_spec_digest=\"" .. generated_spec_digest
      .. "\" child_dedup=\"" .. child_dedup_key
      .. "\" child_issue=\"" .. child_issue_attr
      .. "\" state=\"" .. state
      .. "\" -->",
      nil
  end

  function B.parse_materialization_marker(comment_body, origin_proposal_id, slot_id)
    if type(comment_body) ~= "string" then
      return nil
    end
    local ok = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then
      return nil
    end
    ok = validate_slot(slot_id, "slot_id")
    if not ok then
      return nil
    end

    -- Caller owns bot-author trust filtering; this parser only inspects one body string.
    local latest_marker = nil
    for marker in comment_body:gmatch(materialization_pattern) do
      if attr(marker, "origin") == tostring(origin_proposal_id) and attr(marker, "slot") == tostring(slot_id) then
        latest_marker = marker
      end
    end
    if latest_marker == nil then
      return nil
    end
    return materialization_fact_from_marker(latest_marker, origin_proposal_id, slot_id)
  end

  function B.parse_materialization_markers(comment_body, origin_proposal_id)
    local facts = {}
    if type(comment_body) ~= "string" then
      return facts
    end
    local ok = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then
      return facts
    end
    for marker in comment_body:gmatch(materialization_pattern) do
      local fact = materialization_fact_from_marker(marker, origin_proposal_id, nil)
      if fact ~= nil then
        table.insert(facts, fact)
      end
    end
    return facts
  end

  function B.build_terminal_marker(origin_proposal_id, terminal_state, reason_code)
    local ok, err = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then return nil, err end
    ok, err = validate_terminal_state(terminal_state, "terminal_state")
    if not ok then return nil, err end
    ok, err = validate_reason_code(reason_code, "reason_code")
    if not ok then return nil, err end

    return "<!-- " .. ns .. ":terminal:v1 origin=\"" .. origin_proposal_id
      .. "\" state=\"" .. terminal_state
      .. "\" reason_code=\"" .. reason_code
      .. "\" -->",
      nil
  end

  function B.parse_terminal_marker(comment_body, origin_proposal_id)
    if type(comment_body) ~= "string" then
      return nil
    end
    local ok = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then
      return nil
    end

    -- Caller owns bot-author trust filtering; this parser only inspects one body string.
    local latest_marker = nil
    for marker in comment_body:gmatch(terminal_pattern) do
      if attr(marker, "origin") == tostring(origin_proposal_id) then
        latest_marker = marker
      end
    end
    if latest_marker == nil then
      return nil
    end
    return terminal_fact_from_marker(latest_marker, origin_proposal_id)
  end

  function B.build_lineage_header(origin_proposal_id, blueprint_digest, slot_id)
    local ok, err = validate_origin(origin_proposal_id, "origin_proposal_id")
    if not ok then return nil, err end
    ok, err = validate_digest(blueprint_digest, "blueprint_digest")
    if not ok then return nil, err end
    ok, err = validate_slot(slot_id, "slot_id")
    if not ok then return nil, err end

    return "<!-- " .. ns .. ":lineage:v1 origin=\"" .. origin_proposal_id
      .. "\" blueprint_digest=\"" .. blueprint_digest
      .. "\" slot=\"" .. slot_id
      .. "\" -->",
      nil
  end

  function B.parse_lineage_header(text)
    if type(text) ~= "string" then
      return nil
    end
    for marker in text:gmatch(lineage_pattern) do
      return lineage_fact_from_marker(marker)
    end
    return nil
  end

  -- Namespace-free convenience, mirrored onto the bound table.
  B.latest_materialization_by_slot = M.latest_materialization_by_slot

  return B
end

-- Return a build/parse table bound to `ns` (e.g. "fkst:workflow-security").
function M.for_namespace(ns)
  if type(ns) ~= "string" or ns == "" then
    error("workflow-engine: invalid-marker-namespace: namespace token must be a non-empty string", 0)
  end
  return bound(ns)
end

-- Default binding: the module-level build_*/parse_* functions use DEFAULT_NAMESPACE so
-- existing callers keep working. Adapters MUST call M.for_namespace(<their-token>).
local DEFAULT = bound(M.DEFAULT_NAMESPACE)
M.build_blueprint_marker = DEFAULT.build_blueprint_marker
M.parse_blueprint_marker = DEFAULT.parse_blueprint_marker
M.build_materialization_marker = DEFAULT.build_materialization_marker
M.parse_materialization_marker = DEFAULT.parse_materialization_marker
M.parse_materialization_markers = DEFAULT.parse_materialization_markers
M.build_terminal_marker = DEFAULT.build_terminal_marker
M.parse_terminal_marker = DEFAULT.parse_terminal_marker
M.build_lineage_header = DEFAULT.build_lineage_header
M.parse_lineage_header = DEFAULT.parse_lineage_header

function M.install(target)
  target.marker = M
end

return M
