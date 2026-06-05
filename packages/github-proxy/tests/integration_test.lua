local t = fkst.test

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function issue_list_json(updated_at, state)
  return string.format(
    '[{"number":42,"title":"Bridge issue","url":"https://github.example/owner/x/issues/42","updatedAt":"%s","state":"%s"}]\n',
    updated_at or "2026-06-03T01:02:03Z",
    state or "OPEN"
  )
end

local function pr_list_json(updated_at, state)
  return string.format(
    '[{"number":7,"title":"Bridge PR","url":"https://github.example/owner/x/pull/7","updatedAt":"%s","state":"%s"}]\n',
    updated_at or "2026-06-03T02:03:04Z",
    state or "OPEN"
  )
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/github-proxy/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function base_env(name, extra)
  local env = {
    FKST_GITHUB_REPO = "owner/x",
    FKST_RUNTIME_ROOT = runtime_root(name),
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return env
end

local function opts(name, extra_env)
  return {
    env = base_env(name, extra_env),
  }
end

local function mock_repo_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = value or "owner/x" })
end

local function mock_write_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = value or "" })
end

local function mock_issue_list(stdout, exit_code, stderr)
  t.mock_command("gh issue list", {
    stdout = stdout or issue_list_json(),
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_pr_list(stdout, exit_code, stderr)
  t.mock_command("gh pr list", {
    stdout = stdout or pr_list_json(),
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_poll(issue_stdout, pr_stdout)
  mock_repo_env()
  mock_issue_list(issue_stdout)
  mock_pr_list(pr_stdout)
end

local function mock_comment_view(comments)
  t.mock_command("gh issue view", {
    stdout = string.format('{"comments":[{"body":"%s"}]}\n', comments or "existing comment"),
  })
end

local function mock_comment_write()
  t.mock_command("gh issue comment", { stdout = "", exit_code = 0 })
end

local function calls_matching(needle)
  local matches = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      table.insert(matches, call)
    end
  end
  return matches
end

local function count_calls(needle)
  return #calls_matching(needle)
end

return {
  test_inbound_poll_raises_issue_and_pr_then_cache_hit = function()
    local event = { queue = "github_poll_tick", payload = {} }
    local run_opts = opts("inbound-cache-hit")

    mock_poll()
    local first = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(first.exit_code, 0)
    t.eq(first.raises[1].queue, "github_entity_changed")
    t.eq(first.raises[1].payload.type, "issue")
    t.eq(first.raises[1].payload.repo, "owner/x")
    t.eq(first.raises[1].payload.number, 42)
    t.eq(first.raises[1].payload.title, "Bridge issue")
    t.eq(first.raises[1].payload.updated_at, "2026-06-03T01:02:03Z")
    t.eq(first.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-03T01:02:03Z")
    t.eq(first.raises[1].payload.source_ref.kind, "external")
    t.eq(first.raises[1].payload.source_ref.ref, "owner/x#issue/42")
    t.eq(first.raises[2].queue, "github_entity_changed")
    t.eq(first.raises[2].payload.type, "pr")
    t.eq(first.raises[2].payload.repo, "owner/x")
    t.eq(first.raises[2].payload.number, 7)
    t.eq(first.raises[2].payload.title, "Bridge PR")
    t.eq(first.raises[2].payload.url, "https://github.example/owner/x/pull/7")
    t.eq(first.raises[2].payload.state, "OPEN")
    t.eq(first.raises[2].payload.updated_at, "2026-06-03T02:03:04Z")
    t.eq(first.raises[2].payload.dedup_key, "owner/x#pr#7@2026-06-03T02:03:04Z")
    t.eq(first.raises[2].payload.source_ref.kind, "external")
    t.eq(first.raises[2].payload.source_ref.ref, "owner/x#pr/7")
    t.is_nil(first.raises[3])

    mock_poll()
    local second = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(count_calls("gh issue list"), 2)
    t.eq(count_calls("gh pr list"), 2)
  end,

  test_inbound_poll_re_raises_when_updated_at_changes = function()
    local event = { queue = "github_poll_tick", payload = {} }
    local run_opts = opts("inbound-updated-at-change")

    mock_poll()
    local first = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)

    mock_poll(
      issue_list_json("2026-06-04T05:06:07Z"),
      pr_list_json("2026-06-04T06:07:08Z")
    )
    local changed = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(changed.exit_code, 0)
    t.eq(#changed.raises, 2)
    t.eq(changed.raises[1].payload.type, "issue")
    t.eq(changed.raises[1].payload.updated_at, "2026-06-04T05:06:07Z")
    t.eq(changed.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-04T05:06:07Z")
    t.eq(changed.raises[2].payload.type, "pr")
    t.eq(changed.raises[2].payload.updated_at, "2026-06-04T06:07:08Z")
    t.eq(changed.raises[2].payload.dedup_key, "owner/x#pr#7@2026-06-04T06:07:08Z")
  end,

  test_inbound_poll_re_raises_closed_lifecycle_state_when_updated_at_changes = function()
    local event = { queue = "github_poll_tick", payload = {} }
    local run_opts = opts("inbound-closed-change")

    mock_poll()
    local first = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)
    t.eq(first.raises[1].payload.type, "issue")
    t.eq(first.raises[1].payload.state, "OPEN")

    mock_poll(
      issue_list_json("2026-06-04T09:10:11Z", "CLOSED"),
      pr_list_json()
    )
    local closed = t.run_department("departments/github_poll/main.lua", event, run_opts)
    t.eq(closed.exit_code, 0)
    t.eq(#closed.raises, 1)
    t.eq(closed.raises[1].queue, "github_entity_changed")
    t.eq(closed.raises[1].payload.type, "issue")
    t.eq(closed.raises[1].payload.number, 42)
    t.eq(closed.raises[1].payload.updated_at, "2026-06-04T09:10:11Z")
    t.eq(closed.raises[1].payload.state, "CLOSED")
    t.eq(closed.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-04T09:10:11Z")
  end,

  test_inbound_poll_continues_when_issue_list_fails = function()
    mock_repo_env()
    mock_issue_list("", 2, "forced issue list failure")
    mock_pr_list()

    local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, opts("issue-list-fails"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "pr")
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh pr list"), 1)
  end,

  test_inbound_poll_continues_when_pr_list_fails = function()
    mock_repo_env()
    mock_issue_list()
    mock_pr_list("", 2, "forced pr list failure")

    local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, opts("pr-list-fails"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "issue")
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh pr list"), 1)
  end,

  test_inbound_poll_no_raise_without_repo_env = function()
    mock_repo_env("")

    local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, {
      env = {
        FKST_GITHUB_REPO = "",
        FKST_RUNTIME_ROOT = runtime_root("missing-repo"),
      },
    })

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh pr list"), 0)
  end,

  test_outbound_dry_run_write_and_marker_idempotency = function()
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        issue_number = 42,
        body = "fkst reply",
        dedup_key = "reply-42",
      },
    }

    mock_repo_env()
    mock_write_env("")
    local dry = t.run_department("departments/github_comment/main.lua", event, opts("comment-dry-run"))
    t.eq(dry.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 0)

    mock_repo_env()
    mock_write_env("1")
    mock_comment_view("existing comment")
    mock_comment_write()
    local write = t.run_department("departments/github_comment/main.lua", event, opts("comment-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(write.exit_code, 0)

    -- The mocked `gh issue comment` does not run, so assert on the body file the
    -- department actually wrote: it must carry the reply text and the HTML
    -- marker that makes the next poll's comment idempotent.
    local written = file.read("/tmp/fkst-github-proxy-comment-reply-42.md")
    t.is_true(written:find("fkst reply", 1, true) ~= nil)
    t.is_true(written:find("<!-- fkst:github-proxy:comment:reply-42 -->", 1, true) ~= nil)

    mock_repo_env()
    mock_write_env("1")
    mock_comment_view("existing comment <!-- fkst:github-proxy:comment:reply-42 -->")
    local again = t.run_department("departments/github_comment/main.lua", event, opts("comment-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(again.exit_code, 0)

    local comment_calls = calls_matching("gh issue comment")
    t.eq(#comment_calls, 1)
    t.is_true(comment_calls[1].rendered:find("gh issue comment", 1, true) ~= nil)
    t.eq(comment_calls[1].rendered:find("github.com", 1, true), nil)
    t.eq(count_calls("gh issue view"), 2)
  end,
}
