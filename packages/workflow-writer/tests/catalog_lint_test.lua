-- Catalog-lint / guard test: the on-disk blueprints/authoring-flow.json mirror must
-- validate under the SAME kernel validator (workflow.engine.blueprint) that gates every
-- catalog record and every delivered authored template on load. This is the reused
-- catalog-lint the delivered-file PR's CI runs -- it is NOT re-implemented here.
local blueprint = require("workflow.engine.blueprint")
local records = require("records")
local t = fkst.test

local ONDISK_PATH = "packages/workflow-writer/blueprints/authoring-flow.json"

return {
  test_ondisk_template_parses_and_validates = function()
    local source = file.read(ONDISK_PATH)
    t.is_true(type(source) == "string" and source ~= "", "missing on-disk template")
    local parsed, why = blueprint.parse_blueprint(source)
    t.is_true(parsed ~= nil, why and why.code)
    t.eq(parsed.id, "workflow-authoring-flow")
    t.eq(#parsed.steps, 1)
    t.eq(parsed.steps[1].content.kind, "generated")
  end,

  test_ondisk_template_matches_builtin_identity = function()
    local source = file.read(ONDISK_PATH)
    local parsed = assert(blueprint.parse_blueprint(source))
    t.eq(parsed.id, records.BLUEPRINT.id)
    t.eq(parsed.version, records.BLUEPRINT.version)
    t.eq(parsed.steps[1].id, records.BLUEPRINT.steps[1].id)
  end,

  test_builtin_is_generated_single_step = function()
    -- The authoring step MUST be generated (a real codex runner is injected); a static
    -- degraded mode would silently disable authoring.
    t.eq(#records.BLUEPRINT.steps, 1)
    t.eq(records.BLUEPRINT.steps[1].content.kind, "generated")
    t.is_true(blueprint.validate(records.BLUEPRINT))
  end,
}
