local t = fkst.test

return {
  test_audit_department_does_not_touch_global_pipeline = function()
    local source = file.read("packages/archaudit/departments/audit/main.lua")

    t.is_true(source:find("_G.pipeline", 1, true) == nil)
  end,
}
