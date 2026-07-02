-- Unit tests for the DI providers/composition helper (devloop.di.providers).
local t = fkst.test
local providers = require("devloop.di.providers")

return {
  test_build_all_exposes_role_scoped_caps = function()
    local all = providers.build_all({ egress = { github = { view = 1 }, git = { fetch = 1 } } })
    t.is_true(type(all.log) == "table")
    t.is_true(type(all.state.read.current_state) == "function")
    t.is_true(type(all.state.cas.cas_outcome) == "function")
    t.is_true(all.egress.gh.view == 1)
    t.is_true(all.egress.git.fetch == 1)
  end,

  test_build_department_projects_only_declared_caps = function()
    local all = providers.build_all({})
    local captured
    local dept_mod = {
      spec = { name = "d", caps = { requires = { "log", "state.read" } } },
      cap_deps = { "log", "state.read" },
      make_department = function(caps)
        captured = caps
        return { spec = { name = "d" }, pipeline = function() end }
      end,
    }
    local dept = providers.build_department(dept_mod, all, { department = "d" })
    t.is_true(type(dept.pipeline) == "function")
    -- declared caps reachable
    t.is_true(type(captured.log) == "table")
    t.is_true(type(captured.state.read) == "table")
    -- undeclared cap sealed off
    local ok = pcall(function() return captured.state.cas end)
    t.eq(ok, false)
    local ok2 = pcall(function() return captured.egress end)
    t.eq(ok2, false)
  end,

  test_build_department_rejects_missing_make_department = function()
    local ok = pcall(function()
      return providers.build_department({ spec = {} }, providers.build_all({}), {})
    end)
    t.eq(ok, false)
  end,
}
