local core = require("core")
local toml = require("core.toml")
local t = fkst.test

-- The shipped example is the first thing an operator copies. A reference
-- definition that its own reader rejects, or that the validator refuses, would
-- send every adopter straight into a failed run — so it is parsed and validated
-- here rather than trusted to be right.

local function read_example()
  local path = "examples/sourcing/workflow.toml"
  local handle = io.open(path, "r")
  if handle == nil then
    error("cannot open " .. path, 0)
  end
  local text = handle:read("*a")
  handle:close()
  return text
end

return {
  test_the_example_definition_decodes_and_validates = function()
    local document, err = toml.decode(read_example())
    t.is_nil(err)
    local steps, validate_error = core.validate_definition(document)
    t.is_nil(validate_error)
    t.eq(#steps, 3)
    t.eq(steps[1].id, "scrape")
    t.eq(steps[1].kind, "run")
    t.eq(steps[2].id, "score")
    t.eq(steps[2].kind, "task")
    t.eq(steps[3].id, "publish")
    t.eq(steps[3].kind, "run")
  end,

  test_the_example_resolves_with_the_arguments_it_documents = function()
    -- The README tells an adopter to pass `role` and `min_score`. If the
    -- definition referenced a third placeholder, every copy of it would fail at
    -- the first step with "argument not supplied".
    local document = toml.decode(read_example())
    local steps = core.validate_definition(document)
    local arguments = { role = "AI Tools Application Engineer", min_score = "6" }
    for _, step in ipairs(steps) do
      local resolved, err = core.resolve_step(step, arguments)
      t.is_nil(err, ("step %s must resolve: %s"):format(step.id, tostring(err)))
      t.is_true(resolved ~= nil)
    end
  end,

  test_the_example_carries_no_credential_and_no_concrete_instance = function()
    -- The content boundary: search parameters, destination identifiers, and
    -- credential-broker service names belong in the operator's own copy. The
    -- shipped shape must carry none of them.
    local text = read_example()
    for _, forbidden in ipairs({ "ghp_", "github_pat_", "nyx_", "sk-", "xoxb-", "AKIA" }) do
      t.is_true(
        text:find(forbidden, 1, true) == nil,
        "the example must never carry a credential-shaped string: " .. forbidden
      )
    end
    -- Every operator-supplied value arrives as an argument placeholder.
    t.is_true(text:find("{{ role }}", 1, true) ~= nil)
    t.is_true(text:find("{{ min_score }}", 1, true) ~= nil)
  end,

  test_the_example_substitutes_its_search_parameter_as_one_argv_element = function()
    -- A hostile search parameter must stay one argument rather than becoming
    -- shell syntax — the property the whole argv design exists for.
    local document = toml.decode(read_example())
    local steps = core.validate_definition(document)
    local resolved = core.resolve_step(steps[1], {
      role = '" ; rm -rf / ; echo "',
      min_score = "6",
    })
    local found = false
    for _, element in ipairs(resolved.argv) do
      if element == '" ; rm -rf / ; echo "' then
        found = true
      end
    end
    t.is_true(found, "the hostile value must survive as exactly one argv element")
  end,
}
