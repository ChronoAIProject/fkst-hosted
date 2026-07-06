local catalog = require("core.catalog")
local blueprint_schema = require("core.blueprint")
local default_catalog = require("core.default_catalog")
local t = fkst.test

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function test_root()
  local token = tostring({}):gsub("[^A-Za-z0-9]", "")
  return "/tmp/fkst-workflow-catalog-test-" .. token
end

local function valid_json(id)
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "]] .. id .. [[",
    "version": "2026-07-02",
    "summary": "A bounded workflow.",
    "applies_when": "The origin issue asks for this workflow.",
    "steps": [
      {"id":"first","title":"First step","content":{"kind":"static","intent":"Do the first bounded step."}}
    ]
  }]]
end

local function valid_toml(id)
  return [=[
schema = "fkst.workflow.v1"
id = "]=] .. id .. [=["
version = "2026-07-02"
summary = "A bounded workflow."
applies_when = "The origin issue asks for this workflow."

[[steps]]
id = "first"
title = "First step"

[steps.content]
kind = "static"
intent = "Do the first bounded step."
]=]
end

local function cleanup(root)
  os.remove(root .. "/alpha.json")
  os.remove(root .. "/bad.json")
  os.remove(root .. "/bad.toml")
  os.remove(root .. "/dup-a.json")
  os.remove(root .. "/gamma.toml")
  os.remove(root .. "/ignored.txt")
  os.remove(root .. "/nested/beta.json")
  os.remove(root .. "/nested/dup-b.json")
  os.execute("rmdir " .. shell_quote(root .. "/nested") .. " >/dev/null 2>&1")
  os.execute("rmdir " .. shell_quote(root) .. " >/dev/null 2>&1")
end

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if ok ~= true and ok ~= 0 then
    error("failed to create temp catalog directory")
  end
end

local function prepare(root)
  cleanup(root)
  mkdir_p(root .. "/nested")
  file.write(root .. "/alpha.json", valid_json("alpha"))
  file.write(root .. "/nested/beta.json", valid_json("beta"))
  file.write(root .. "/dup-a.json", valid_json("dup"))
  file.write(root .. "/nested/dup-b.json", valid_json("dup"))
  file.write(root .. "/bad.json", [[{"schema":"wrong","id":"bad","version":"1","summary":"bad","applies_when":"bad","steps":[]}]] )
  file.write(root .. "/ignored.txt", valid_json("ignored"))
end

local function with_temp_root(fn)
  local root = test_root()
  local ok, err = pcall(function()
    fn(root)
  end)
  cleanup(root)
  if not ok then
    error(err, 0)
  end
end

local function error_with_code(errors, code)
  for _, item in ipairs(errors) do
    if item.error ~= nil and item.error.code == code then
      return item
    end
  end
  return nil
end

local function assert_contains(text, needle)
  t.is_true(tostring(text or ""):find(needle, 1, true) ~= nil)
end

local function assert_not_contains(text, needle)
  t.is_nil(tostring(text or ""):find(needle, 1, true))
end

local function assert_step_ids(bp, expected)
  t.eq(#bp.steps, #expected)
  for index, id in ipairs(expected) do
    t.eq(bp.steps[index].id, id)
    t.eq(bp.steps[index].content.kind, "generated")
    t.is_true(#bp.steps[index].content.generator <= blueprint_schema.MAX_GENERATOR_BYTES)
    assert_contains(bp.steps[index].content.generator, "source_ref")
    assert_contains(bp.steps[index].content.generator, "TDD/tests live inside this child PR")
    assert_contains(bp.steps[index].content.generator, "devloop consensus, CI, PR review, or merge gates")
    assert_contains(bp.steps[index].content.generator, "devloop provides those")
    assert_contains(bp.steps[index].content.generator, "no-changes is fatal")
    assert_not_contains(bp.steps[index].content.generator, "feasibility_probe")
    assert_not_contains(bp.steps[index].content.generator, "EligibilityManifest")
  end
end

local function assert_conservative_applies_when(bp)
  assert_contains(bp.applies_when, "Conservative router")
  assert_contains(bp.applies_when, "origin issue text")
  assert_contains(bp.applies_when, "unambiguously matches this flow and exactly one flow")
  assert_contains(bp.applies_when, "choose none/plain devloop")
end

local tests = {
  test_load_catalog_keeps_valid_rejects_invalid_and_records_duplicates = function()
    with_temp_root(function(root)
      prepare(root)

      local loaded = catalog.load_catalog(root)

      t.eq(loaded.valid.alpha.blueprint.id, "alpha")
      t.is_true(loaded.valid.alpha.path:sub(-10) == "alpha.json")
      t.eq(loaded.valid.beta.blueprint.id, "beta")
      t.is_true(loaded.valid.beta.path:sub(-16) == "nested/beta.json")
      t.is_nil(loaded.valid.dup)
      t.is_nil(loaded.valid.ignored)
      t.eq(#loaded.duplicates, 1)
      t.eq(loaded.duplicates[1].id, "dup")
      t.eq(#loaded.duplicates[1].paths, 2)
      t.is_true(error_with_code(loaded.errors, "invalid_schema") ~= nil)
      local duplicate = error_with_code(loaded.errors, "duplicate_id")
      t.is_true(duplicate ~= nil)
      t.eq(duplicate.error.meta.id, "dup")
      t.eq(#duplicate.error.meta.peers, 2)
    end)
  end,

  test_load_catalog_accepts_toml_files_through_shared_validator = function()
    with_temp_root(function(root)
      mkdir_p(root)
      file.write(root .. "/gamma.toml", valid_toml("gamma"))

      local loaded = catalog.load_catalog(root)

      t.eq(#loaded.errors, 0)
      t.eq(#loaded.duplicates, 0)
      t.eq(loaded.valid.gamma.blueprint.id, "gamma")
      t.is_true(loaded.valid.gamma.path:sub(-10) == "gamma.toml")
      t.eq(loaded.valid.gamma.blueprint.steps[1].content.kind, "static")
      t.eq(loaded.valid.gamma.blueprint.steps[1].content.intent, "Do the first bounded step.")
    end)
  end,

  test_invalid_toml_catalog_file_fails_closed = function()
    with_temp_root(function(root)
      mkdir_p(root)
      file.write(root .. "/bad.toml", "schema = [")

      local loaded = catalog.load_catalog(root)

      t.eq(#loaded.errors, 1)
      t.eq(loaded.errors[1].path, root .. "/bad.toml")
      t.eq(loaded.errors[1].error.code, "invalid_toml")
      assert_contains(loaded.errors[1].error.message, "TOML")
    end)
  end,

  test_validate_records_is_shared_by_builtin_catalog = function()
    local records = default_catalog.records()
    local loaded = catalog.validate_records(records)

    t.eq(#loaded.errors, 0)
    t.eq(#loaded.duplicates, 0)
    t.eq(default_catalog.count, 3)
    t.eq(loaded.valid["software-feature-flow"].path, "builtin:software-feature-flow")
    t.eq(loaded.valid["software-refactor-flow"].path, "builtin:software-refactor-flow")
    t.eq(loaded.valid["software-contract-migration-flow"].path, "builtin:software-contract-migration-flow")
    t.is_nil(loaded.valid["software-dev-flow"])
  end,

  test_builtin_mature_software_flows_have_governing_generated_steps = function()
    local loaded = catalog.validate_records(default_catalog.records())
    local feature = loaded.valid["software-feature-flow"].blueprint
    local refactor = loaded.valid["software-refactor-flow"].blueprint
    local migration = loaded.valid["software-contract-migration-flow"].blueprint

    assert_conservative_applies_when(feature)
    assert_conservative_applies_when(refactor)
    assert_conservative_applies_when(migration)

    assert_step_ids(feature, { "walking-skeleton", "production-slice" })
    assert_contains(feature.steps[1].content.generator, "Cockburn walking skeleton")
    assert_contains(feature.steps[1].content.generator, "thinnest executable end-to-end path")
    assert_contains(feature.steps[1].content.generator, "smoke or acceptance test")
    assert_contains(feature.steps[2].content.generator, "MERGED walking-skeleton result")
    assert_contains(feature.steps[2].content.generator, "edge cases, negative cases, and tests")

    assert_step_ids(refactor, { "characterization-tests", "behavior-preserving-restructure" })
    assert_contains(refactor.steps[1].content.generator, "Feathers-style characterization tests")
    assert_contains(refactor.steps[1].content.generator, "tests only")
    assert_contains(refactor.steps[1].content.generator, "CURRENT externally observable behavior")
    assert_contains(refactor.steps[2].content.generator, "MERGED characterization-tests result")
    assert_contains(refactor.steps[2].content.generator, "Fowler-style internal restructuring")
    assert_contains(refactor.steps[2].content.generator, "Preserve externally observable behavior")

    assert_step_ids(migration, { "expand", "migrate", "contract" })
    assert_contains(migration.steps[1].content.generator, "Fowler Parallel Change expand")
    assert_contains(migration.steps[1].content.generator, "backward-compatible adapter")
    assert_contains(migration.steps[1].content.generator, "old contract must still work")
    assert_contains(migration.steps[2].content.generator, "MERGED expand result")
    assert_contains(migration.steps[2].content.generator, "move in-repo producers and consumers")
    assert_contains(migration.steps[3].content.generator, "MERGED migrate result")
    assert_contains(migration.steps[3].content.generator, "remove the old contract form")
    assert_contains(migration.steps[3].content.generator, "temporary bridge")
  end,

  test_validate_records_rejects_duplicate_ids_across_sources = function()
    local records = default_catalog.records()
    table.insert(records, {
      path = "external/software-feature-flow.json",
      blueprint = records[1].blueprint,
    })

    local loaded = catalog.validate_records(records)

    t.is_nil(loaded.valid["software-feature-flow"])
    t.eq(#loaded.duplicates, 1)
    t.eq(loaded.duplicates[1].id, "software-feature-flow")
    t.eq(loaded.duplicates[1].paths[1], "builtin:software-feature-flow")
    t.eq(loaded.duplicates[1].paths[2], "external/software-feature-flow.json")
    t.eq(error_with_code(loaded.errors, "duplicate_id").error.meta.id, "software-feature-flow")
  end,

  test_rejects_invalid_root_dir = function()
    local loaded = catalog.load_catalog("")
    t.eq(#loaded.errors, 1)
    t.eq(loaded.errors[1].error.code, "invalid_root_dir")
  end,

  test_records_file_list_failure = function()
    local previous_list = file.list
    file.list = function(_path)
      error("forced list failure")
    end
    local ok, loaded = pcall(function()
      return catalog.load_catalog("/tmp/fkst-workflow-catalog-test-list-failure")
    end)
    file.list = previous_list
    if not ok then
      error(loaded, 0)
    end
    t.eq(#loaded.errors, 1)
    t.eq(loaded.errors[1].error.code, "file_list_failed")
  end,

  test_records_file_read_failure = function()
    with_temp_root(function(root)
      mkdir_p(root)
      file.write(root .. "/gone.json", valid_json("gone"))
      local previous_read = file.read
      file.read = function(path)
        if path:sub(-9) == "gone.json" then
          error("forced read failure")
        end
        return previous_read(path)
      end
      local ok, loaded = pcall(function()
        return catalog.load_catalog(root)
      end)
      file.read = previous_read
      if not ok then
        error(loaded, 0)
      end
      t.eq(#loaded.errors, 1)
      t.eq(loaded.errors[1].error.code, "file_read_failed")
    end)
  end,

  test_rejects_over_max_catalog_files = function()
    with_temp_root(function(root)
      mkdir_p(root)
      for index = 1, catalog.MAX_CATALOG_FILES + 1 do
        file.write(root .. "/wf-" .. tostring(index) .. ".json", valid_json("wf-" .. tostring(index)))
      end
      local loaded = catalog.load_catalog(root)
      t.eq(#loaded.errors, 1)
      t.eq(loaded.errors[1].error.code, "too_many_files")
    end)
  end,
}

return tests
