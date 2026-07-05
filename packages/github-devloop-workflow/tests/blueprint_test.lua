local blueprint = require("core.blueprint")
local t = fkst.test

local function valid_blueprint()
  return {
    schema = "fkst.workflow.v1",
    id = "workflow-one",
    version = "2026-07-02",
    summary = "A bounded two-step workflow.",
    applies_when = "The origin issue asks for this workflow.",
    selector = {
      labels_any = { "workflow" },
      title_contains_any = { "orchestrate" },
    },
    steps = {
      {
        id = "first",
        title = "Prepare the foundation",
        content = {
          kind = "static",
          intent = "Implement the first bounded step.",
        },
      },
      {
        id = "second",
        title = "Use the merged result",
        content = {
          kind = "generated",
          generator = "Read the predecessor result and produce the next bounded issue spec.",
        },
      },
    },
  }
end

local function valid_blueprint_json()
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "workflow-one",
    "version": "2026-07-02",
    "summary": "A bounded two-step workflow.",
    "applies_when": "The origin issue asks for this workflow.",
    "selector": {
      "labels_any": ["workflow"],
      "title_contains_any": ["orchestrate"]
    },
    "steps": [
      {
        "id": "first",
        "title": "Prepare the foundation",
        "content": {"kind": "static", "intent": "Implement the first bounded step."}
      },
      {
        "id": "second",
        "title": "Use the merged result",
        "content": {"kind": "generated", "generator": "Read the predecessor result and produce the next bounded issue spec."}
      }
    ]
  }]]
end

local function one_step_json(content_json)
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "workflow-one",
    "version": "2026-07-02",
    "summary": "A bounded workflow.",
    "applies_when": "The origin issue asks for this workflow.",
    "steps": [
      {"id": "first", "title": "Prepare the foundation", "content": ]] .. content_json .. [[}
    ]
  }]]
end

local function with_steps_json(steps_json)
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "workflow-one",
    "version": "2026-07-02",
    "summary": "A bounded workflow.",
    "applies_when": "The origin issue asks for this workflow.",
    "steps": ]] .. steps_json .. [[
  }]]
end

local function with_selector_json(selector_json)
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "workflow-one",
    "version": "2026-07-02",
    "summary": "A bounded workflow.",
    "applies_when": "The origin issue asks for this workflow.",
    "selector": ]] .. selector_json .. [[,
    "steps": [
      {"id":"first","title":"First","content":{"kind":"static","intent":"Do it."}}
    ]
  }]]
end

local function repeated_steps_json(count)
  local steps = {}
  for index = 1, count do
    steps[index] = '{"id":"step-' .. tostring(index) .. '","title":"Step ' .. tostring(index) .. '","content":{"kind":"static","intent":"Do the bounded work."}}'
  end
  return "[" .. table.concat(steps, ",") .. "]"
end

local function repeated_json_strings(count, value)
  local items = {}
  for index = 1, count do
    items[index] = '"' .. value .. tostring(index) .. '"'
  end
  return "[" .. table.concat(items, ",") .. "]"
end

local function parse_rejects(source, expected)
  local parsed, err = blueprint.parse_blueprint(source)
  t.is_nil(parsed)
  t.is_true(type(err) == "table")
  if expected ~= nil then
    if expected.code ~= nil then
      t.eq(err.code, expected.code)
    end
    if expected.path ~= nil then
      t.eq(err.path, expected.path)
    end
  end
end

local function validate_rejects(value, expected)
  local ok, err = blueprint.validate(value)
  t.eq(ok, false)
  t.is_true(type(err) == "table")
  if expected ~= nil then
    if expected.code ~= nil then
      t.eq(err.code, expected.code)
    end
    if expected.path ~= nil then
      t.eq(err.path, expected.path)
    end
  end
end

local reject_cases = {
  {
    name = "invalid JSON",
    source = "{not json",
    code = "invalid_json",
    path = "blueprint_json",
  },
  {
    name = "wrong schema",
    source = valid_blueprint_json():gsub("fkst.workflow.v1", "fkst.workflow.v0", 1),
    code = "invalid_schema",
    path = "schema",
  },
  {
    name = "missing schema",
    mutate = function(doc) doc.schema = nil end,
    code = "not_string",
    path = "schema",
  },
  {
    name = "non-string schema",
    mutate = function(doc) doc.schema = 42 end,
    code = "not_string",
    path = "schema",
  },
  {
    name = "missing id",
    mutate = function(doc) doc.id = nil end,
    code = "not_string",
    path = "id",
  },
  {
    name = "empty id",
    mutate = function(doc) doc.id = "" end,
    code = "empty",
    path = "id",
  },
  {
    name = "oversized id",
    mutate = function(doc) doc.id = string.rep("x", blueprint.MAX_ID_BYTES + 1) end,
    code = "too_large",
    path = "id",
  },
  {
    name = "non-string id",
    mutate = function(doc) doc.id = 42 end,
    code = "not_string",
    path = "id",
  },
  {
    name = "missing version",
    mutate = function(doc) doc.version = nil end,
    code = "not_string",
    path = "version",
  },
  {
    name = "empty version",
    mutate = function(doc) doc.version = "" end,
    code = "empty",
    path = "version",
  },
  {
    name = "oversized version",
    mutate = function(doc) doc.version = string.rep("x", blueprint.MAX_VERSION_BYTES + 1) end,
    code = "too_large",
    path = "version",
  },
  {
    name = "non-string version",
    mutate = function(doc) doc.version = 42 end,
    code = "not_string",
    path = "version",
  },
  {
    name = "missing summary",
    mutate = function(doc) doc.summary = nil end,
    code = "not_string",
    path = "summary",
  },
  {
    name = "empty summary",
    mutate = function(doc) doc.summary = "" end,
    code = "empty",
    path = "summary",
  },
  {
    name = "oversized summary",
    mutate = function(doc) doc.summary = string.rep("x", blueprint.MAX_SUMMARY_BYTES + 1) end,
    code = "too_large",
    path = "summary",
  },
  {
    name = "non-string summary",
    mutate = function(doc) doc.summary = 42 end,
    code = "not_string",
    path = "summary",
  },
  {
    name = "missing applies_when",
    mutate = function(doc) doc.applies_when = nil end,
    code = "not_string",
    path = "applies_when",
  },
  {
    name = "empty applies_when",
    mutate = function(doc) doc.applies_when = "" end,
    code = "empty",
    path = "applies_when",
  },
  {
    name = "oversized applies_when",
    mutate = function(doc) doc.applies_when = string.rep("x", blueprint.MAX_APPLIES_WHEN_BYTES + 1) end,
    code = "too_large",
    path = "applies_when",
  },
  {
    name = "non-string applies_when",
    mutate = function(doc) doc.applies_when = 42 end,
    code = "not_string",
    path = "applies_when",
  },
  {
    name = "unknown top-level field",
    mutate = function(doc) doc.extra = true end,
    code = "unknown_field",
    path = "blueprint.extra",
  },
  {
    name = "steps non-array",
    mutate = function(doc) doc.steps = "nope" end,
    code = "not_array",
    path = "steps",
  },
  {
    name = "steps non-contiguous",
    mutate = function(doc) doc.steps = { [1] = doc.steps[1], [3] = doc.steps[2] } end,
    code = "non_contiguous_array",
    path = "steps",
  },
  {
    name = "steps empty",
    source = with_steps_json("[]"),
    code = "empty_array",
    path = "steps",
  },
  {
    name = "steps over MAX",
    source = with_steps_json(repeated_steps_json(blueprint.MAX_WORKFLOW_STEPS + 1)),
    code = "too_many_items",
    path = "steps",
  },
  {
    name = "duplicate step id",
    source = with_steps_json('[{"id":"same","title":"First","content":{"kind":"static","intent":"Do first."}},{"id":"same","title":"Second","content":{"kind":"static","intent":"Do second."}}]'),
    code = "duplicate_step_id",
    path = "steps[2].id",
  },
  {
    name = "step non-table",
    mutate = function(doc) doc.steps[1] = "step" end,
    code = "not_object",
    path = "steps[1]",
  },
  {
    name = "unknown step field",
    mutate = function(doc) doc.steps[1].needs = {} end,
    code = "unknown_field",
    path = "steps[1].needs",
  },
  {
    name = "missing step id",
    mutate = function(doc) doc.steps[1].id = nil end,
    code = "not_string",
    path = "steps[1].id",
  },
  {
    name = "empty step id",
    mutate = function(doc) doc.steps[1].id = "" end,
    code = "empty",
    path = "steps[1].id",
  },
  {
    name = "oversized step id",
    mutate = function(doc) doc.steps[1].id = string.rep("x", blueprint.MAX_STEP_ID_BYTES + 1) end,
    code = "too_large",
    path = "steps[1].id",
  },
  {
    name = "non-string step id",
    mutate = function(doc) doc.steps[1].id = 42 end,
    code = "not_string",
    path = "steps[1].id",
  },
  {
    name = "missing step title",
    mutate = function(doc) doc.steps[1].title = nil end,
    code = "not_string",
    path = "steps[1].title",
  },
  {
    name = "empty step title",
    mutate = function(doc) doc.steps[1].title = "" end,
    code = "empty",
    path = "steps[1].title",
  },
  {
    name = "oversized step title",
    mutate = function(doc) doc.steps[1].title = string.rep("x", blueprint.MAX_STEP_TITLE_BYTES + 1) end,
    code = "too_large",
    path = "steps[1].title",
  },
  {
    name = "non-string step title",
    mutate = function(doc) doc.steps[1].title = 42 end,
    code = "not_string",
    path = "steps[1].title",
  },
  {
    name = "content non-object",
    source = one_step_json('"literal"'),
    code = "not_object",
    path = "steps[1].content",
  },
  {
    name = "content.kind neither static nor generated",
    source = one_step_json('{"kind":"literal","intent":"Do it."}'),
    code = "unsupported_content_kind",
    path = "steps[1].content.kind",
  },
  {
    name = "static missing intent",
    source = one_step_json('{"kind":"static"}'),
    code = "not_string",
    path = "steps[1].content.intent",
  },
  {
    name = "static carrying generator",
    source = one_step_json('{"kind":"static","intent":"Do it.","generator":"Generate it."}'),
    code = "unknown_field",
    path = "steps[1].content.generator",
  },
  {
    name = "static empty intent",
    source = one_step_json('{"kind":"static","intent":""}'),
    code = "empty",
    path = "steps[1].content.intent",
  },
  {
    name = "static oversized intent",
    source = one_step_json('{"kind":"static","intent":"' .. string.rep("x", blueprint.MAX_STATIC_INTENT_BYTES + 1) .. '"}'),
    code = "too_large",
    path = "steps[1].content.intent",
  },
  {
    name = "static non-string intent",
    source = one_step_json('{"kind":"static","intent":42}'),
    code = "not_string",
    path = "steps[1].content.intent",
  },
  {
    name = "generated missing generator",
    source = one_step_json('{"kind":"generated"}'),
    code = "not_string",
    path = "steps[1].content.generator",
  },
  {
    name = "generated carrying intent",
    source = one_step_json('{"kind":"generated","generator":"Generate it.","intent":"Do it."}'),
    code = "unknown_field",
    path = "steps[1].content.intent",
  },
  {
    name = "generated empty generator",
    source = one_step_json('{"kind":"generated","generator":""}'),
    code = "empty",
    path = "steps[1].content.generator",
  },
  {
    name = "generated oversized generator",
    source = one_step_json('{"kind":"generated","generator":"' .. string.rep("x", blueprint.MAX_GENERATOR_BYTES + 1) .. '"}'),
    code = "too_large",
    path = "steps[1].content.generator",
  },
  {
    name = "generated non-string generator",
    source = one_step_json('{"kind":"generated","generator":42}'),
    code = "not_string",
    path = "steps[1].content.generator",
  },
  {
    name = "selector unknown key",
    mutate = function(doc) doc.selector = { label = { "workflow" } } end,
    code = "unknown_field",
    path = "selector.label",
  },
  {
    name = "selector scalar",
    mutate = function(doc) doc.selector = "workflow" end,
    code = "not_object",
    path = "selector",
  },
  {
    name = "selector boolean",
    mutate = function(doc) doc.selector = true end,
    code = "not_object",
    path = "selector",
  },
  {
    name = "selector number",
    mutate = function(doc) doc.selector = 42 end,
    code = "not_object",
    path = "selector",
  },
  {
    name = "selector JSON null",
    source = with_selector_json("null"),
    code = "not_object",
    path = "selector",
  },
  {
    name = "selector labels_any not array",
    mutate = function(doc) doc.selector = { labels_any = "workflow" } end,
    code = "not_array",
    path = "selector.labels_any",
  },
  {
    name = "selector labels_any empty array",
    mutate = function(doc) doc.selector = { labels_any = {} } end,
    code = "empty_array",
    path = "selector.labels_any",
  },
  {
    name = "selector labels_any over-count",
    source = valid_blueprint_json():gsub('%["workflow"%]', repeated_json_strings(blueprint.MAX_SELECTOR_LABELS + 1, "label-"), 1),
    code = "too_many_items",
    path = "selector.labels_any",
  },
  {
    name = "selector labels_any non-string element",
    mutate = function(doc) doc.selector = { labels_any = { 42 } } end,
    code = "not_string",
    path = "selector.labels_any[1]",
  },
  {
    name = "selector labels_any empty element",
    mutate = function(doc) doc.selector = { labels_any = { "" } } end,
    code = "empty",
    path = "selector.labels_any[1]",
  },
  {
    name = "selector labels_any oversized element",
    mutate = function(doc) doc.selector = { labels_any = { string.rep("x", blueprint.MAX_SELECTOR_LABEL_BYTES + 1) } } end,
    code = "too_large",
    path = "selector.labels_any[1]",
  },
  {
    name = "selector title_contains_any not array",
    mutate = function(doc) doc.selector = { title_contains_any = "workflow" } end,
    code = "not_array",
    path = "selector.title_contains_any",
  },
  {
    name = "selector title_contains_any empty array",
    mutate = function(doc) doc.selector = { title_contains_any = {} } end,
    code = "empty_array",
    path = "selector.title_contains_any",
  },
  {
    name = "selector title_contains_any over-count",
    source = valid_blueprint_json():gsub('%["orchestrate"%]', repeated_json_strings(blueprint.MAX_SELECTOR_TITLE_TERMS + 1, "term-"), 1),
    code = "too_many_items",
    path = "selector.title_contains_any",
  },
  {
    name = "selector title_contains_any non-string element",
    mutate = function(doc) doc.selector = { title_contains_any = { 42 } } end,
    code = "not_string",
    path = "selector.title_contains_any[1]",
  },
  {
    name = "selector title_contains_any empty element",
    mutate = function(doc) doc.selector = { title_contains_any = { "" } } end,
    code = "empty",
    path = "selector.title_contains_any[1]",
  },
  {
    name = "selector title_contains_any oversized element",
    mutate = function(doc) doc.selector = { title_contains_any = { string.rep("x", blueprint.MAX_SELECTOR_TITLE_TERM_BYTES + 1) } } end,
    code = "too_large",
    path = "selector.title_contains_any[1]",
  },
  {
    name = "top-level JSON null",
    source = "null",
    code = "not_object",
    path = "blueprint",
  },
  {
    name = "top-level JSON array",
    source = "[1]",
    code = "not_object",
    path = "blueprint",
  },
  {
    name = "top-level JSON scalar",
    source = '"scalar"',
    code = "not_object",
    path = "blueprint",
  },
}

local tests = {
  test_valid_blueprint_parses_and_validates = function()
    local parsed, err = blueprint.parse_blueprint(valid_blueprint_json())
    t.is_nil(err)
    t.eq(parsed.schema, "fkst.workflow.v1")
    t.eq(parsed.id, "workflow-one")
    t.eq(#parsed.steps, 2)
    t.eq(parsed.selector.labels_any[1], "workflow")
    t.eq(parsed.selector.title_contains_any[1], "orchestrate")

    local ok, validate_err = blueprint.validate(parsed)
    t.eq(ok, true)
    t.is_nil(validate_err)
  end,

  test_valid_selector_with_both_fields = function()
    local doc = valid_blueprint()
    doc.selector = {
      labels_any = { "workflow", "fkst" },
      title_contains_any = { "orchestrate", "workflow" },
    }
    local ok, err = blueprint.validate(doc)
    t.eq(ok, true)
    t.is_nil(err)
  end,

  test_rejects_duplicate_step_ids = function()
    parse_rejects(with_steps_json('[{"id":"same","title":"First","content":{"kind":"static","intent":"Do first."}},{"id":"same","title":"Second","content":{"kind":"static","intent":"Do second."}}]'), {
      code = "duplicate_step_id",
      path = "steps[2].id",
    })
  end,
}

for _, case in ipairs(reject_cases) do
  tests["test_rejects_" .. case.name:gsub("[^A-Za-z0-9]+", "_"):gsub("_$", "")] = function()
    if case.source ~= nil then
      parse_rejects(case.source, {
        code = case.code,
        path = case.path,
      })
    else
      local doc = valid_blueprint()
      case.mutate(doc)
      validate_rejects(doc, {
        code = case.code,
        path = case.path,
      })
    end
  end
end

return tests
