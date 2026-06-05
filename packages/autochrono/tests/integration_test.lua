local t = fkst.test

local draft_body = "Thanks for opening this. I will review the details and follow up with the next concrete step."

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/private/tmp/fkst-packages-test/autochrono/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = runtime_root(name),
    },
  }
end

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

local function run_reply(event_payload, run_opts)
  return t.run_department("departments/reply/main.lua", {
    queue = "issue",
    payload = event_payload,
  }, run_opts)
end

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

return {
  test_reply_raises_draft_from_codex_mock = function()
    t.mock_command("codex exec", { stdout = draft_body .. "\n", exit_code = 0 })

    local result = run_reply(issue(), opts("first-reply"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "reply")
    t.eq(result.raises[1].payload.schema, "autochrono.reply.v1")
    t.eq(result.raises[1].payload.repo, "owner/repo")
    t.eq(result.raises[1].payload.issue_number, 42)
    t.eq(result.raises[1].payload.body, draft_body)
    t.eq(result.raises[1].payload.dedup_key, "autochrono:owner/repo#issue/42")
    t.eq(result.raises[1].payload.source_ref.kind, "external")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    local calls = codex_calls()
    t.eq(#calls, 1)
    t.is_true(calls[1].stdin:find("Repository: owner/repo", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("Number: 42", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("Title: Bridge issue", 1, true) ~= nil)
  end,

  test_reply_cache_hit_skips_same_issue_across_updates = function()
    local run_opts = opts("cache-hit")
    t.mock_command("codex exec", { stdout = draft_body, exit_code = 0 })

    local first = run_reply(issue(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    local second = run_reply(issue({ updated_at = "2026-06-04T05:06:07Z" }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(#codex_calls(), 1)
  end,

  test_reply_degrades_when_codex_fails = function()
    t.mock_command("codex exec", { stderr = "failed", exit_code = 7 })

    local result = run_reply(issue(), opts("codex-fails"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 1)
  end,

  test_reply_degrades_when_codex_stdout_is_empty = function()
    t.mock_command("codex exec", { stdout = " \n\t ", exit_code = 0 })

    local result = run_reply(issue(), opts("codex-empty"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 1)
  end,

  test_reply_skips_non_open_issue = function()
    local result = run_reply(issue({ state = "CLOSED" }), opts("closed-issue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,
}
