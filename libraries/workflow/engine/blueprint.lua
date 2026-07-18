local M = {}

local fail = require("workflow.engine.errors").fail

M.SCHEMA = "fkst.workflow.v1"
M.MAX_WORKFLOW_STEPS = 16
M.MAX_ID_BYTES = 128
M.MAX_VERSION_BYTES = 64
M.MAX_SUMMARY_BYTES = 512
M.MAX_APPLIES_WHEN_BYTES = 1024
M.MAX_SELECTOR_LABELS = 16
M.MAX_SELECTOR_LABEL_BYTES = 128
M.MAX_SELECTOR_TITLE_TERMS = 16
M.MAX_SELECTOR_TITLE_TERM_BYTES = 128
M.MAX_STEP_ID_BYTES = 128
M.MAX_STEP_TITLE_BYTES = 200
M.MAX_STATIC_INTENT_BYTES = 8000
M.MAX_GENERATOR_BYTES = 8000

local TOP_LEVEL_FIELDS = {
  schema = true,
  id = true,
  version = true,
  summary = true,
  applies_when = true,
  selector = true,
  steps = true,
}

local STEP_FIELDS = {
  id = true,
  title = true,
  content = true,
}

local SELECTOR_FIELDS = {
  labels_any = {
    max_count = M.MAX_SELECTOR_LABELS,
    max_bytes = M.MAX_SELECTOR_LABEL_BYTES,
  },
  title_contains_any = {
    max_count = M.MAX_SELECTOR_TITLE_TERMS,
    max_bytes = M.MAX_SELECTOR_TITLE_TERM_BYTES,
  },
}

local CONTENT_KINDS = {
  static = {
    field = "intent",
    max = M.MAX_STATIC_INTENT_BYTES,
  },
  generated = {
    field = "generator",
    max = M.MAX_GENERATOR_BYTES,
  },
}

M.CONTENT_KINDS = CONTENT_KINDS

local function require_string(value, path, limit)
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
  return true, nil
end

local function reject_unknown_fields(tbl, allowed, path)
  for key, _ in pairs(tbl) do
    if type(key) ~= "string" then
      return false, fail(path, "array_entry", "must not contain array entries")
    end
    if not allowed[key] then
      return false, fail(path .. "." .. key, "unknown_field", "is not supported")
    end
  end
  return true, nil
end

local function array_length(tbl, path)
  if type(tbl) ~= "table" then
    local err = fail(path, "not_array", "must be an array")
    return nil, err
  end

  local count = 0
  local seen = {}
  for key, _ in pairs(tbl) do
    if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then
      local err = fail(path, "not_array", "must be an array")
      return nil, err
    end
    count = count + 1
    seen[key] = true
  end

  for index = 1, count do
    if not seen[index] then
      local err = fail(path, "non_contiguous_array", "must be a contiguous array")
      return nil, err
    end
  end

  return count, nil
end

local function validate_bounded_string_array(value, path, max_count, max_bytes)
  local count, err = array_length(value, path)
  if count == nil then
    return false, err
  end
  if count == 0 then
    return false, fail(path, "empty_array", "must not be empty")
  end
  if count > max_count then
    return false, fail(path, "too_many_items", "exceeds item limit", {
      max_count = max_count,
      actual_count = count,
    })
  end
  for index = 1, count do
    local ok, string_err = require_string(value[index], path .. "[" .. tostring(index) .. "]", max_bytes)
    if not ok then return ok, string_err end
  end
  return true, nil
end

local function validate_selector(value, path)
  if value == nil then
    return true, nil
  end
  if type(value) ~= "table" then
    return false, fail(path, "not_object", "must be an object")
  end

  for key, child in pairs(value) do
    if type(key) ~= "string" then
      return false, fail(path, "array_entry", "must not contain array entries")
    end
    local spec = SELECTOR_FIELDS[key]
    if spec == nil then
      return false, fail(path .. "." .. key, "unknown_field", "is not supported")
    end
    local ok, err = validate_bounded_string_array(child, path .. "." .. key, spec.max_count, spec.max_bytes)
    if not ok then
      return ok, err
    end
  end
  return true, nil
end

local function validate_content(content, path)
  if type(content) ~= "table" then
    return false, fail(path, "not_object", "must be an object")
  end

  local variant = CONTENT_KINDS[content.kind]
  if variant == nil then
    return false, fail(path .. ".kind", "unsupported_content_kind", "must be static or generated")
  end

  local allowed = {
    kind = true,
    [variant.field] = true,
  }
  for key, _ in pairs(content) do
    if type(key) ~= "string" then
      return false, fail(path, "array_entry", "must not contain array entries")
    end
    if not allowed[key] then
      return false, fail(path .. "." .. key, "unknown_field", "is not supported")
    end
  end

  return require_string(content[variant.field], path .. "." .. variant.field, variant.max)
end

local function validate_step(step, index, seen_ids)
  local path = "steps[" .. tostring(index) .. "]"
  if type(step) ~= "table" then
    return false, fail(path, "not_object", "must be an object")
  end
  local ok, why = reject_unknown_fields(step, STEP_FIELDS, path)
  if not ok then return ok, why end
  ok, why = require_string(step.id, path .. ".id", M.MAX_STEP_ID_BYTES)
  if not ok then return ok, why end
  if seen_ids[step.id] then
    return false, fail(path .. ".id", "duplicate_step_id", "duplicates an earlier step id")
  end
  seen_ids[step.id] = true
  ok, why = require_string(step.title, path .. ".title", M.MAX_STEP_TITLE_BYTES)
  if not ok then return ok, why end
  return validate_content(step.content, path .. ".content")
end

function M.validate(tbl)
  if type(tbl) ~= "table" then
    return false, fail("blueprint", "not_object", "must be an object")
  end
  if #tbl > 0 then
    return false, fail("blueprint", "not_object", "must be an object")
  end
  local ok, why = reject_unknown_fields(tbl, TOP_LEVEL_FIELDS, "blueprint")
  if not ok then return ok, why end
  if type(tbl.schema) ~= "string" then
    return false, fail("schema", "not_string", "must be a string")
  end
  if tbl.schema ~= M.SCHEMA then
    return false, fail("schema", "invalid_schema", "must be " .. M.SCHEMA)
  end
  ok, why = require_string(tbl.id, "id", M.MAX_ID_BYTES)
  if not ok then return ok, why end
  ok, why = require_string(tbl.version, "version", M.MAX_VERSION_BYTES)
  if not ok then return ok, why end
  ok, why = require_string(tbl.summary, "summary", M.MAX_SUMMARY_BYTES)
  if not ok then return ok, why end
  ok, why = require_string(tbl.applies_when, "applies_when", M.MAX_APPLIES_WHEN_BYTES)
  if not ok then return ok, why end
  ok, why = validate_selector(tbl.selector, "selector")
  if not ok then return ok, why end

  local step_count, step_error = array_length(tbl.steps, "steps")
  if step_count == nil then
    return false, step_error
  end
  if step_count == 0 then
    return false, fail("steps", "empty_array", "must not be empty")
  end
  if step_count > M.MAX_WORKFLOW_STEPS then
    return false, fail("steps", "too_many_items", "exceeds MAX_WORKFLOW_STEPS", {
      max_count = M.MAX_WORKFLOW_STEPS,
      actual_count = step_count,
    })
  end

  local seen_ids = {}
  for index = 1, step_count do
    ok, why = validate_step(tbl.steps[index], index, seen_ids)
    if not ok then return ok, why end
  end

  return true, nil
end

function M.parse_blueprint(json_string)
  if type(json_string) ~= "string" then
    local err = fail("blueprint_json", "not_string", "must be a string")
    return nil, err
  end
  local ok, decoded = pcall(json.decode, json_string)
  if not ok then
    local err = fail("blueprint_json", "invalid_json", "invalid JSON", {
      error = tostring(decoded),
    })
    return nil, err
  end
  local valid, why = M.validate(decoded)
  if not valid then
    return nil, why
  end
  return decoded, nil
end

function M.install(target)
  target.blueprint = M
end

return M
