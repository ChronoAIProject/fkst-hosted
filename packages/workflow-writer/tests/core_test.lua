local authoring = require("authoring")
local completion = require("completion")
local records = require("records")
local blueprint = require("workflow.engine.blueprint")
local t = fkst.test

local function sample_template_json()
  return table.concat({
    "{",
    '  "schema": "fkst.workflow.v1",',
    '  "id": "release-notes-flow",',
    '  "version": "v1",',
    '  "summary": "Draft release notes from merged PRs.",',
    '  "applies_when": "A repository requests release notes.",',
    '  "steps": [',
    '    { "id": "draft", "title": "Draft the notes",',
    '      "content": { "kind": "static", "intent": "Summarize merged PRs into release notes." } }',
    "  ]",
    "}",
  }, "\n")
end

return {
  test_builtin_blueprint_validates = function()
    local ok = blueprint.validate(records.BLUEPRINT)
    t.is_true(ok)
    t.eq(records.BLUEPRINT.id, "workflow-authoring-flow")
    t.eq(#records.BLUEPRINT.steps, 1)
    t.eq(records.BLUEPRINT.steps[1].id, records.STEP_ID)
  end,

  test_records_provider_single_record = function()
    local all = records.records()
    t.eq(#all, 1)
    t.eq(all[1].blueprint.id, "workflow-authoring-flow")
    t.is_true(blueprint.validate(all[1].blueprint))
  end,

  test_validate_drafted_template_reuses_kernel_validator = function()
    local drafted, why = authoring.validate_drafted_template(sample_template_json())
    t.is_true(drafted ~= nil, why and why.code)
    t.eq(drafted.id, "release-notes-flow")
    t.eq(drafted.steps[1].content.kind, "static")
  end,

  test_validate_drafted_template_rejects_invalid = function()
    local drafted = authoring.validate_drafted_template('{"schema":"fkst.workflow.v1","id":"x"}')
    t.eq(drafted, nil)
    local bad_schema = authoring.validate_drafted_template('{"schema":"other","id":"x","version":"v1","summary":"s","applies_when":"a","steps":[]}')
    t.eq(bad_schema, nil)
    local not_json = authoring.validate_drafted_template("not json at all")
    t.eq(not_json, nil)
  end,

  test_extract_template_json_takes_first_balanced_object = function()
    local stdout = 'chatter before\n{"schema":"fkst.workflow.v1"}\nOpened PR /pull/42\n'
    local json_text = authoring.extract_template_json(stdout)
    t.eq(json_text, '{"schema":"fkst.workflow.v1"}')
  end,

  test_extract_template_json_raises_when_absent = function()
    t.raises(function()
      authoring.extract_template_json("no object here")
    end)
  end,

  test_classify_request_defaults_to_create = function()
    local routing = authoring.classify_request({ text = "Please add a workflow that drafts weekly digests." })
    t.eq(routing.mode, "create")
    t.eq(routing.target_package, nil)
  end,

  test_classify_request_routes_refine_for_allowed_package = function()
    local text = "Please refine the review steps.\ntarget: workflow-security\nworkflow-id: security-review"
    local routing = authoring.classify_request({ text = text })
    t.eq(routing.mode, "refine")
    t.eq(routing.target_package, "workflow-security")
    t.eq(routing.target_workflow_id, "security-review")
  end,

  test_classify_request_downgrades_refine_for_unknown_package = function()
    local text = "Refine please.\ntarget: some-random-package"
    local routing = authoring.classify_request({ text = text })
    t.eq(routing.mode, "create")
    t.eq(routing.target_package, nil)
  end,

  test_id_collision_guard = function()
    local existing = { ["security-review"] = true, ["workflow-authoring-flow"] = true }
    -- create mode: any existing id collides
    t.is_true(authoring.id_collision("security-review", existing, "create", nil))
    t.is_true(not authoring.id_collision("brand-new-id", existing, "create", nil))
    -- refine mode: matching the SAME target id is an allowed in-place edit
    t.is_true(not authoring.id_collision("security-review", existing, "refine", "security-review"))
    -- refine mode: colliding with a DIFFERENT existing id is still a collision
    t.is_true(authoring.id_collision("workflow-authoring-flow", existing, "refine", "security-review"))
    -- empty id is always a collision (invalid)
    t.is_true(authoring.id_collision("", existing, "create", nil))
  end,

  test_build_prompt_selects_mode_text_and_embeds_bounds = function()
    local create_prompt = authoring.build_prompt({ text = "add a digest workflow", origin = "issue/7", repo = "owner/repo" })
    t.is_true(create_prompt:find("authoring exactly ONE new", 1, true) ~= nil)
    t.is_true(create_prompt:find("Mode: create", 1, true) ~= nil)

    local refine_prompt = authoring.build_prompt({
      text = "refine it\ntarget: workflow-security\nworkflow-id: security-review",
      origin = "issue/8",
      repo = "owner/repo",
    })
    t.is_true(refine_prompt:find("REFINING an existing", 1, true) ~= nil)
    t.is_true(refine_prompt:find("Target package to refine: workflow-security", 1, true) ~= nil)
    t.is_true(refine_prompt:find("Target workflow id: security-review", 1, true) ~= nil)
  end,

  test_completion_status_mapping = function()
    t.eq(completion.status_of_pr({ state = "merged" }), "result_ready")
    t.eq(completion.status_of_pr({ state = "open" }), "running")
    t.eq(completion.status_of_pr({ state = "transient" }), "recoverable")
    t.eq(completion.status_of_pr({ state = "invalid" }), "fatal")
    t.eq(completion.status_of_pr({ state = "weird" }), "unknown")
    t.eq(completion.status_of_pr(nil), "unknown")
  end,

  test_completion_reader_reads_child_ref_result = function()
    local reader = completion.reader({ origin = "issue/1" })
    t.eq(reader({ result = { state = "merged" } }), "result_ready")
    t.eq(reader({ result = { state = "open" } }), "running")
    t.eq(reader({}), "unknown")
    t.eq(reader("not-a-table"), "unknown")
  end,

  test_refinable_package_allowlist = function()
    t.is_true(authoring.refinable_package("workflow-security"))
    t.is_true(authoring.refinable_package("workflow-finance"))
    t.is_true(not authoring.refinable_package("random-package"))
    t.is_true(not authoring.refinable_package(""))
  end,
}
