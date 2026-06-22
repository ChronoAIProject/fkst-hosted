local t = fkst.test

local function run_department_with_logs(path, event, opts)
  local result = t.run_department(path, event, opts)
  t.is_true(type(result) == "table")
  return result.exit_code == 0, result
end

return {
  test_scan_accepts_production_namespaced_queue = function()
    t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
      stdout = "owner/repo",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api --paginate --slurp", {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })

    local ok, result = run_department_with_logs("departments/external_pr_intake/main.lua", {
      queue = "github-external-pr-intake.external_pr_scan",
      payload = {
        schema = "github-external-pr-intake.v1",
      },
    })

    t.eq(ok, true)
    t.eq(#result.raises, 0)
  end,

  test_candidate_non_table_payload_fails_closed = function()
    local ok, result = run_department_with_logs("departments/external_pr_intake/main.lua", {
      queue = "external_pr_candidate",
      payload = "foreign-payload",
    })

    t.eq(ok, false)
    t.eq(#result.raises, 0)
  end,
}
