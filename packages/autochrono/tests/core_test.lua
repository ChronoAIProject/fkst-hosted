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
    local after_update = core.reply_dedup_key(issue().repo, issue({ updated_at = "2026-06-04T05:06:07Z" }).issue_number)
    t.eq(first, "autochrono:owner/repo#issue/42")
    t.eq(first, second)
    t.eq(first, after_update)
    t.eq(first:find("2026", 1, true), nil)
  end,

  test_replied_cache_key_is_readable_path = function()
    t.eq(core.replied_cache_key("owner/repo", 42), "autochrono/replied/owner/repo/issue/42")
  end,

  test_is_eligible_accepts_open_autochrono_issue = function()
    t.eq(core.is_eligible(issue()), true)
    t.eq(core.is_eligible(issue({ state = "CLOSED" })), false)
    t.eq(core.is_eligible(issue({ schema = "other.issue.v1" })), false)
    t.eq(core.is_eligible({
      schema = "autochrono.issue.v1",
      issue_number = 42,
      state = "OPEN",
    }), false)
    t.eq(core.is_eligible({
      schema = "autochrono.issue.v1",
      repo = "owner/repo",
      state = "OPEN",
    }), false)
    t.eq(core.is_eligible({}), false)
    t.eq(core.is_eligible(nil), false)
  end,

  test_build_prompt_contains_issue_context = function()
    local prompt = core.build_prompt(issue())
    t.is_true(prompt:find("Repository: owner/repo", 1, true) ~= nil)
    t.is_true(prompt:find("Number: 42", 1, true) ~= nil)
    t.is_true(prompt:find("Title: Bridge issue", 1, true) ~= nil)
    t.is_true(prompt:find("Do not claim work has been completed.", 1, true) ~= nil)
  end,

  test_clean_draft_trims_stdout_and_rejects_empty_body = function()
    t.eq(core.clean_draft("  Draft body. \n"), "Draft body.")
    t.is_nil(core.clean_draft(" \n\t "))
  end,

  test_build_reply_request_preserves_payload_fields = function()
    local source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    }
    local payload = core.build_reply_request(issue({ source_ref = source_ref }), "Draft body.")

    t.eq(payload.schema, "autochrono.reply.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.issue_number, 42)
    t.eq(payload.body, "Draft body.")
    t.eq(payload.dedup_key, "autochrono:owner/repo#issue/42")
    t.is_true(payload.source_ref == source_ref)
    t.eq(payload.source_ref.kind, "external")
    t.eq(payload.source_ref.ref, "owner/repo#issue/42")
  end,

  test_build_reply_request_requires_source_ref = function()
    local without_source_ref = issue()
    without_source_ref.source_ref = nil
    t.raises(function()
      core.build_reply_request(without_source_ref, "Draft body.")
    end)
  end,
}
