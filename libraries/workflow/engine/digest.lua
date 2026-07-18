local strings = require("contract.strings")
local blueprint = require("workflow.engine.blueprint")
local fail = require("workflow.engine.errors").fail

local M = {}

M.MAX_DIGEST_BYTES = 64

local function append_field(parts, name, value)
  table.insert(parts, name)
  table.insert(parts, tostring(#value))
  table.insert(parts, value)
end

local function content_value(step)
  local kind = step.content.kind
  local variant = blueprint.CONTENT_KINDS[kind]
  if variant == nil then
    return nil
  end
  return step.content[variant.field]
end

function M.canonical_blueprint_string(doc)
  local ok, err = blueprint.validate(doc)
  if not ok then
    return nil, err
  end

  local parts = {}
  append_field(parts, "schema", doc.schema)
  append_field(parts, "id", doc.id)
  append_field(parts, "version", doc.version)
  for _, step in ipairs(doc.steps) do
    append_field(parts, "step.id", step.id)
    append_field(parts, "step.title", step.title)
    append_field(parts, "step.content.kind", step.content.kind)
    append_field(parts, "step.content.value", content_value(step))
  end
  return table.concat(parts, "\n"), nil
end

function M.blueprint_digest(doc)
  local canonical, err = M.canonical_blueprint_string(doc)
  if canonical == nil then
    return nil, err
  end
  local digest = "d-" .. strings.decimal_checksum(canonical)
  if #digest > M.MAX_DIGEST_BYTES then
    return nil, fail("digest", "too_large", "exceeds byte limit", {
      max_bytes = M.MAX_DIGEST_BYTES,
      actual_bytes = #digest,
    })
  end
  return digest, nil
end

function M.install(target)
  target.digest = M
end

return M
