-- Unit tests for the sealed capability projector (devloop.di.select_caps) — the guard that keeps
-- make_department(caps) DI from decaying into a renamed service-locator.
local t = fkst.test
local select_caps = require("devloop.di.select_caps")
local capdefs = require("devloop.di.capdefs")

local function sample_all_caps()
  return {
    log = { info = function() return "log" end },
    state = {
      read = { get = function() return "read" end },
      cas = { update = function() return "cas" end },
      lifecycle = { open = function() return "life" end },
    },
    entity = { reader = { get = function() return "ent" end } },
    commands = { emit = function() return "emit" end },
  }
end

return {
  test_projects_only_declared_capabilities = function()
    local caps = select_caps.project(sample_all_caps(), { "log", "state.cas" }, { department = "d" })
    t.eq(caps.log.info(), "log")
    t.eq(caps.state.cas.update(), "cas")
  end,

  test_undeclared_top_level_access_errors = function()
    local caps = select_caps.project(sample_all_caps(), { "log" }, { department = "d" })
    local ok, err = pcall(function() return caps.state end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("undeclared capability", 1, true) ~= nil)
  end,

  test_undeclared_nested_access_errors = function()
    -- declared state.cas but NOT state.read: reading caps.state.read must error.
    local caps = select_caps.project(sample_all_caps(), { "state.cas" }, { department = "d" })
    local ok, err = pcall(function() return caps.state.read end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("undeclared capability", 1, true) ~= nil)
  end,

  test_mutation_is_read_only = function()
    local caps = select_caps.project(sample_all_caps(), { "log" }, { department = "d" })
    local ok, err = pcall(function() caps.log = {} end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("read%-only") ~= nil)
  end,

  test_wildcard_god_dependency_rejected = function()
    for _, bad in ipairs({ "*", "core", "all", "devloop", "services" }) do
      local ok, err = pcall(function()
        return select_caps.project(sample_all_caps(), { bad }, { department = "d" })
      end)
      t.eq(ok, false)
      t.is_true(tostring(err):find("forbidden") ~= nil)
    end
  end,

  test_missing_capability_errors = function()
    local ok, err = pcall(function()
      return select_caps.project(sample_all_caps(), { "state.writer" }, { department = "d" })
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("missing capability", 1, true) ~= nil)
  end,

  test_capdefs_taxonomy_is_role_based = function()
    t.is_true(capdefs.is_known("state.cas"))
    t.is_true(capdefs.is_known("egress.gh"))
    -- god-bundle names are NOT valid capability paths.
    t.eq(capdefs.is_known("core"), false)
    t.eq(capdefs.is_known("all"), false)
  end,
}
