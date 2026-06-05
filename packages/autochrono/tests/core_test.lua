local core = require("core")
local t = fkst.test

local function issue(extra)
  local value = {
    schema = "autochrono.issue.v1",
    repo = "owner/repo",
    issue_number = 42,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_reply_dedup_key_is_stable_across_updates = function()
    local first = core.reply_dedup_key("owner/repo", 42)
    local second = core.reply_dedup_key("owner/repo", 42)
    t.eq(first, "autochrono:owner/repo#issue/42")
    t.eq(first, second)
    t.eq(first:find("2026", 1, true), nil)
  end,

  test_replied_cache_key_is_readable_path = function()
    t.eq(core.replied_cache_key("owner/repo", 42), "autochrono/replied/owner/repo/issue/42")
  end,

  test_is_eligible_accepts_open_autochrono_issue = function()
    t.eq(core.is_eligible(issue()), true)
    t.eq(core.is_eligible(issue({ state = "CLOSED" })), false)
    t.eq(core.is_eligible(issue({ schema = "other.issue.v1" })), false)
    t.eq(core.is_eligible({}), false)
  end,

  test_build_prompt_contains_issue_context = function()
    local prompt = core.build_prompt(issue())
    t.is_true(prompt:find("Repository: owner/repo", 1, true) ~= nil)
    t.is_true(prompt:find("Number: 42", 1, true) ~= nil)
    t.is_true(prompt:find("Title: Bridge issue", 1, true) ~= nil)
    t.is_true(prompt:find("Do not claim work has been completed.", 1, true) ~= nil)
  end,

  test_draft_reply_uses_injected_spawner_and_cleans_stdout = function()
    local seen_prompt = nil
    local body = core.draft_reply(issue(), function(opts)
      seen_prompt = opts.prompt
      t.eq(opts.stall_window, "2m")
      return {
        stdout = "  Thanks for opening this. I will review the details and follow up with the next concrete step.  \n",
        stderr = "",
        exit_code = 0,
      }
    end)

    t.is_true(seen_prompt:find("Bridge issue", 1, true) ~= nil)
    t.eq(body, "Thanks for opening this. I will review the details and follow up with the next concrete step.")
  end,

  test_draft_reply_degrades_on_failure_or_empty_stdout = function()
    t.is_nil(core.draft_reply(issue(), function(_opts)
      return { stdout = "draft", stderr = "failed", exit_code = 1 }
    end))
    t.is_nil(core.draft_reply(issue(), function(_opts)
      return { stdout = "   \n", stderr = "", exit_code = 0 }
    end))
  end,
}
