local testing = require("testkit.testing")
local t = fkst.test

-- Selection, at the DEPARTMENT boundary rather than the pure helper.
--
-- The production failure this guards was invisible to a helper-level test: the
-- helper was fed hand-written string assignees, while `issue_list_intake` hands
-- the department `gh`'s real JSON, whose assignees are OBJECTS. Every dispatched
-- run was declined and the schedule died at its watchdog. These tests feed the
-- department the shape `gh` actually returns.

local RUN_ISSUE_BODY = table.concat({
  '<!-- fkst-cron-dispatch:v1 schedule="5917" workflow="cron-acceptance" '
    .. 'slot="2026-08-07T04:46:06Z" manual="false" -->',
  "",
  "### Arguments",
  "",
  "```toml",
  'topic = "AI Tools"',
  "```",
}, "\n")

local function department_with(issues)
  local module = require("departments.run_execute.main")
  return module.make_department({
    github = {
      issue_list_intake = function()
        return issues
      end,
    },
    read_env = function(name)
      if name == "FKST_GITHUB_REPO" then
        return "ChronoAIProject/fkst-hosted"
      elseif name == "FKST_SESSION_CREATOR" then
        return "chronoai-shining"
      elseif name == "FKST_GITHUB_BOT_LOGIN" then
        return "chronoai-fkst[bot]"
      end
      return nil
    end,
    -- Refuse the definition read: the run FAILS, which is enough to prove the
    -- issue was SELECTED — the only thing these tests are about.
    exec = function()
      return { exit_code = 1, stdout = "", stderr = "no such file" }
    end,
    codex = function()
      return { status = "completed" }
    end,
  })
end

local function tick()
  return { queue = "scheduled_run_tick", payload = {} }
end

return {
  test_a_run_issue_from_the_real_listing_is_selected = function()
    local department = department_with({
      {
        number = 5919,
        body = RUN_ISSUE_BODY,
        assignees = { { id = "U_kgDODuiITQ", login = "chronoai-shining", name = "Shining" } },
      },
    })
    local out = testing.run_fake(department, tick())
    t.eq(#out.raises, 1)
    t.eq(out.raises[1].payload.run_issue, 5919)
    t.eq(out.raises[1].payload.schedule_issue, 5917)
  end,

  test_another_creators_run_issue_is_left_alone = function()
    local department = department_with({
      {
        number = 5919,
        body = RUN_ISSUE_BODY,
        assignees = { { login = "someone-else" } },
      },
    })
    local out = testing.run_fake(department, tick())
    t.eq(#out.raises, 0)
  end,

  test_an_ordinary_work_issue_is_a_clean_no_op = function()
    local department = department_with({
      { number = 42, body = "please fix the button", assignees = { { login = "chronoai-shining" } } },
    })
    local out = testing.run_fake(department, tick())
    t.eq(#out.raises, 0)
  end,
}
