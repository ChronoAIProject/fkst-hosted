local t = fkst.test
local author_policy = require("testkit.github_author_policy")

local function run_department_with_logs(path, event, opts)
  local result = t.run_department(path, event, opts)
  t.is_true(type(result) == "table")
  return result.exit_code == 0, result
end

local function mock_env_reads()
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
  for _ = 1, 2 do
    author_policy.mock_env(t, {
      env = {
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "",
        FKST_GITHUB_AUTHORIZED_LOGINS = "",
      },
    })
  end
end

return {
  test_scan_accepts_production_namespaced_queue = function()
    mock_env_reads()
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
