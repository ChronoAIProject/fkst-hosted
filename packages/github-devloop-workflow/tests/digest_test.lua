local digest = require("core.digest")
local blueprint = require("core.blueprint")
local t = fkst.test

local canonical_json = [[{
  "schema": "fkst.workflow.v1",
  "id": "workflow-one",
  "version": "2026-07-02",
  "summary": "A bounded workflow.",
  "applies_when": "The origin issue asks for this workflow.",
  "selector": {
    "labels_any": ["workflow"],
    "title_contains_any": ["orchestrate"]
  },
  "steps": [
    {
      "id": "first",
      "title": "Prepare the foundation",
      "content": {
        "kind": "static",
        "intent": "Implement the first bounded step."
      }
    },
    {
      "id": "second",
      "title": "Use the merged result",
      "content": {
        "kind": "generated",
        "generator": "Read the predecessor result and produce the next bounded issue spec."
      }
    }
  ]
}]]

local reordered_json = [[{
  "steps": [
    {"content": {"intent": "Implement the first bounded step.", "kind": "static"}, "title": "Prepare the foundation", "id": "first"},
    {"title": "Use the merged result", "id": "second", "content": {"generator": "Read the predecessor result and produce the next bounded issue spec.", "kind": "generated"}}
  ],
  "selector": {"title_contains_any": ["orchestrate"], "labels_any": ["workflow"]},
  "applies_when": "The origin issue asks for this workflow.",
  "summary": "A bounded workflow.",
  "version": "2026-07-02",
  "id": "workflow-one",
  "schema": "fkst.workflow.v1"
}]]

local different_json = [[{
  "schema": "fkst.workflow.v1",
  "id": "workflow-one",
  "version": "2026-07-02",
  "summary": "A bounded workflow.",
  "applies_when": "The origin issue asks for this workflow.",
  "steps": [
    {
      "id": "first",
      "title": "Prepare the foundation",
      "content": {
        "kind": "static",
        "intent": "Implement a different first bounded step."
      }
    },
    {
      "id": "second",
      "title": "Use the merged result",
      "content": {
        "kind": "generated",
        "generator": "Read the predecessor result and produce the next bounded issue spec."
      }
    }
  ]
}]]

local function parse(source)
  local parsed, err = blueprint.parse_blueprint(source)
  if parsed == nil then
    error(err and err.code or "parse failed")
  end
  return parsed
end

local function digest_or_error(doc)
  local value, err = digest.blueprint_digest(doc)
  if value == nil then
    error(err and err.code or "digest failed")
  end
  return value
end

local tests = {
  test_same_semantic_blueprint_has_same_digest_across_json_ordering = function()
    local first = digest_or_error(parse(canonical_json))
    local second = digest_or_error(parse(reordered_json))
    t.eq(first, second)
  end,

  test_different_blueprint_has_different_digest = function()
    local first = digest_or_error(parse(canonical_json))
    local other = digest_or_error(parse(different_json))
    t.is_true(first ~= other)
  end,

  test_digest_is_bounded_string = function()
    local value = digest_or_error(parse(canonical_json))
    t.is_true(type(value) == "string")
    t.is_true(value ~= "")
    t.is_true(#value <= digest.MAX_DIGEST_BYTES)
    t.is_true(value:match("^d%-%d+$") ~= nil)
  end,

  test_invalid_blueprint_returns_structured_error = function()
    local value, err = digest.blueprint_digest({ schema = "wrong" })
    t.is_nil(value)
    t.is_true(type(err) == "table")
    t.eq(err.code, "invalid_schema")
  end,
}

return tests
