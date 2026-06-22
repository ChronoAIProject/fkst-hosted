local t = fkst.test

local function run_department_with_logs(path, event, opts)
  local result = t.run_department(path, event, opts)
  t.is_true(type(result) == "table")
  return result.exit_code == 0, result
end

return {
  test_driver_accepts_production_namespaced_queue = function()
    t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
      stdout = "owner/repo",
      stderr = "",
      exit_code = 0,
    })

    local ok, result = run_department_with_logs("departments/ratchet_migration_driver/main.lua", {
      queue = "github-ratchet-migration-slicer.ratchet_migration_poll",
      payload = {
        schema = "github-ratchet-migration-slicer.ratchet-migration-poll.v1",
        ratchet = "no-matching-ratchet",
      },
    })

    t.eq(ok, true)
    t.eq(#result.raises, 0)
  end,

  test_driver_skips_non_table_payloads = function()
    for _, payload in ipairs({ false, "foreign-payload", 42 }) do
      local result = t.run_department("departments/ratchet_migration_driver/main.lua", {
        queue = "ratchet_migration_poll",
        payload = payload,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
    end
  end,
}
