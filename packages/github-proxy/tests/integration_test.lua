local t = fkst.test
local core = require("core")

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function issue_list_json(updated_at, state)
  return string.format(
    '[{"number":42,"title":"Bridge issue","url":"https://github.example/owner/x/issues/42","updatedAt":"%s","state":"%s","labels":[{"name":"fkst-dev:enabled"},{"name":"bug"}]}]\n',
    updated_at or "2026-06-03T01:02:03Z",
    state or "OPEN"
  )
end

local function pr_list_json(updated_at, state)
  return string.format(
    '[{"number":7,"title":"Bridge PR","url":"https://github.example/owner/x/pull/7","updatedAt":"%s","state":"%s","labels":[{"name":"review"}]}]\n',
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

local function mock_bot_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = value or "fkst-test-bot" })
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

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function comment_json(body, author)
  return string.format('{"body":"%s","author":{"login":"%s"}}', json_string(body), json_string(author or "fkst-test-bot"))
end

local function mock_comment_view(comments, author)
  local rendered_comments = comments
  if type(comments) == "table" then
    local parts = {}
    for _, comment in ipairs(comments) do
      table.insert(parts, comment_json(comment.body, comment.author_login or comment.author))
    end
    rendered_comments = table.concat(parts, ",")
  else
    rendered_comments = comment_json(comments or "existing comment", author)
  end
  t.mock_command("gh issue view", {
    stdout = '{"comments":[' .. rendered_comments .. "]}\n",
  })
end

local function mock_comment_view_failure()
  t.mock_command("gh issue view", {
    stdout = "",
    stderr = "forced comment view failure",
    exit_code = 1,
  })
end

local function mock_label_view(labels)
  local parts = {}
  for _, label in ipairs(labels or {}) do
    table.insert(parts, string.format('{"name":"%s"}', label))
  end
  t.mock_command("gh issue view", {
    stdout = '{"labels":[' .. table.concat(parts, ",") .. "]}\n",
  })
end

local function mock_pr_open_guard(labels, comments)
  local rendered_labels = {}
  for _, label in ipairs(labels or { "fkst-dev:implementing", "fkst-dev:pr-authorized" }) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    if type(comment) == "table" then
      table.insert(rendered_comments, comment_json(comment.body, comment.author_login or comment.author))
    else
      table.insert(rendered_comments, comment_json(comment, "fkst-test-bot"))
    end
  end
  t.mock_command("--json labels,comments", {
    stdout = '{"labels":[' .. table.concat(rendered_labels, ",") .. '],"comments":[' .. table.concat(rendered_comments, ",") .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_head(head_sha)
  t.mock_command("git show-ref --verify refs/heads", {
    stdout = tostring(head_sha or "abc123") .. " refs/heads/devloop-owner-x-42-01HY\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_non_branch_ref_head(head_sha)
  t.mock_command("git show-ref --verify refs/heads", {
    stdout = tostring(head_sha or "abc123") .. " refs/tags/devloop-owner-x-42-01HY\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_comment_write()
  t.mock_command("gh issue comment", { stdout = "", exit_code = 0 })
end

local function mock_label_write()
  t.mock_command("gh issue edit", { stdout = "", exit_code = 0 })
end

local function mock_pr_head_list(stdout)
  t.mock_command("gh pr list", {
    stdout = stdout or "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_head_state(head_sha, state, head_repo, is_cross_repository)
  local cross = "false"
  if is_cross_repository == true then
    cross = "true"
  end
  t.mock_command("--json headRefOid", {
    stdout = string.format(
      '{"headRefOid":"%s","state":"%s","headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s}\n',
      head_sha or "abc123",
      state or "OPEN",
      head_repo or "owner/x",
      cross
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_git_push()
  t.mock_command("git push -u origin", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_create(number)
  t.mock_command("gh pr create", {
    stdout = string.format("https://github.example/owner/x/pull/%d\n", number or 7),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_create_stdout(stdout)
  t.mock_command("gh pr create", {
    stdout = stdout or "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_comment_view(comments, author)
  local rendered_comments = comments
  if type(comments) == "table" then
    local parts = {}
    for _, comment in ipairs(comments) do
      table.insert(parts, comment_json(comment.body, comment.author_login or comment.author))
    end
    rendered_comments = table.concat(parts, ",")
  else
    rendered_comments = comment_json(comments or "existing pr comment", author)
  end
  t.mock_command("gh pr view", {
    stdout = '{"comments":[' .. rendered_comments .. "]}\n",
  })
end

local function mock_pr_comment_write()
  t.mock_command("gh pr comment", { stdout = "", exit_code = 0 })
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

local function long_dedup(suffix, total_len)
  local prefix = "github-devloop/issue/owner/x/42/result/"
  return prefix .. string.rep("v", total_len - #prefix - #suffix) .. suffix
end

local function pr_open_event()
  return {
    queue = "github_pr_open_request",
    payload = {
      schema = "github-proxy.pr-open.v1",
      repo = "owner/x",
      issue_number = 42,
      proposal_id = "github-devloop/issue/owner/x/42",
      impl_version = "v1",
      expected_state = "implementing",
      expected_version = "v1",
      branch = "devloop-owner-x-42-01HY",
      head_sha = "abc123",
      title = "Implement decision recorder",
      body = 'github-devloop implementation PR for issue #42\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/owner/x/42" issue="42" branch="devloop-owner-x-42-01HY" impl_version="v1" -->',
      issue_comment_body_template = 'github-devloop PR opened: #{{pr_number}}\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="pr-open" version="v1" stage_rank="650" -->\n<!-- fkst:github-devloop:pr-link:v1 proposal="github-devloop/issue/owner/x/42" pr="{{pr_number}}" branch="devloop-owner-x-42-01HY" impl_version="v1" -->',
      issue_label_add = { "fkst-dev:pr-open" },
      issue_label_remove = { "fkst-dev:implementing", "fkst-dev:pr-authorized" },
      dedup_key = "open-pr/github-devloop/issue/owner/x/42/v1/devloop-owner-x-42-01HY",
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    },
  }
end

local function pr_open_guard_comments(extra)
  local comments = {
    '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="implementing" version="v1" stage_rank="600" -->',
    '<!-- fkst:github-devloop:implementing:v1 proposal="github-devloop/issue/owner/x/42" dedup="v1" branch="devloop-owner-x-42-01HY" head_sha="abc123" -->',
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return comments
end

local function pr_open_visible_comments(extra)
  local comments = {
    'github-devloop PR opened: #9\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="pr-open" version="v1" stage_rank="650" -->\n<!-- fkst:github-devloop:pr-link:v1 proposal="github-devloop/issue/owner/x/42" pr="9" branch="devloop-owner-x-42-01HY" impl_version="v1" -->\n' .. core.comment_marker("open-pr/github-devloop/issue/owner/x/42/v1/devloop-owner-x-42-01HY"),
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return pr_open_guard_comments(comments)
end

local function reviewing_marker()
  return '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->'
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
    t.eq(first.raises[1].payload.labels[1], "fkst-dev:enabled")
    t.eq(first.raises[1].payload.labels[2], "bug")
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
    t.eq(first.raises[2].payload.labels[1], "review")
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
        repo = "owner/x",
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
    mock_bot_env()
    mock_comment_view("existing comment")
    mock_comment_write()
    local write = t.run_department("departments/github_comment/main.lua", event, opts("comment-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(write.exit_code, 0)

    -- The mocked `gh issue comment` does not run, so assert on the body file the
    -- department actually wrote: it must carry the reply text and the HTML
    -- marker that makes the next poll's comment idempotent.
    local written = file.read("/tmp/fkst-github-proxy-comment-owner_x-issue-42.md")
    t.is_true(written:find("fkst reply", 1, true) ~= nil)
    t.is_true(written:find("<!-- fkst:github-proxy:comment:reply-42 -->", 1, true) ~= nil)

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
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

  test_same_version_meta_comment_marker_dedups_opposite_action = function()
    local dedup = "meta/comment/github-devloop/issue/owner/x/42/stuck/3/consensus-github-devloop/issue/owner/x/42/v1"
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = 'github-devloop meta action: implement\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="ready" version="v1" -->',
        dedup_key = dedup,
      },
    }

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment")
    mock_comment_write()
    local first = t.run_department("departments/github_comment/main.lua", event, opts("comment-meta-first", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(first.exit_code, 0)

    event.payload.body = 'github-devloop meta action: block\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="blocked" version="v1" -->'
    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment " .. core.comment_marker(dedup))
    local second = t.run_department("departments/github_comment/main.lua", event, opts("comment-meta-second", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(second.exit_code, 0)

    t.eq(count_calls("gh issue comment"), 1)
    local written = file.read("/tmp/fkst-github-proxy-comment-owner_x-issue-42.md")
    t.is_true(written:find("github-devloop meta action: implement", 1, true) ~= nil)
    t.eq(written:find("github-devloop meta action: block", 1, true), nil)
    t.is_true(written:find(core.comment_marker(dedup), 1, true) ~= nil)
  end,

  test_forged_proxy_comment_marker_does_not_suppress_bot_state_marker_comment = function()
    local dedup = "meta/comment/github-devloop/issue/owner/x/42/stuck/3/consensus-github-devloop/issue/owner/x/42/v1"
    local state_marker = '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="blocked" version="v1" -->'
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "github-devloop meta action: block\n\n" .. state_marker,
        dedup_key = dedup,
      },
    }

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view({
      {
        body = "forged user marker " .. core.comment_marker(dedup),
        author_login = "ordinary-user",
      },
    })
    mock_comment_write()
    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-forged-marker", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)

    local written = file.read("/tmp/fkst-github-proxy-comment-owner_x-issue-42.md")
    t.is_true(written:find(state_marker, 1, true) ~= nil)
    t.is_true(written:find(core.comment_marker(dedup), 1, true) ~= nil)
  end,

  test_neutralized_forged_proxy_comment_marker_does_not_suppress_later_real_comment = function()
    local dedup = "meta/comment/github-devloop/issue/owner/x/42/stuck/3/consensus-github-devloop/issue/owner/x/42/v2"
    local state_marker = '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="blocked" version="v2" -->'
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "github-devloop meta action: block\n\n" .. state_marker,
        dedup_key = dedup,
      },
    }

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view({
      {
        body = "quoted untrusted marker &lt;!-- fkst:github-proxy:comment:" .. dedup .. " -->",
        author_login = "fkst-test-bot",
      },
    })
    mock_comment_write()
    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-neutralized-forged-marker", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)

    local written = file.read("/tmp/fkst-github-proxy-comment-owner_x-issue-42.md")
    t.is_true(written:find(state_marker, 1, true) ~= nil)
    t.is_true(written:find(core.comment_marker(dedup), 1, true) ~= nil)
  end,

  test_long_comment_dedup_uses_bounded_runtime_key_and_full_marker = function()
    local dedup_v1 = long_dedup("-v1", 430)
    local dedup_v2 = long_dedup("-v2", 430)
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "long fkst reply",
        dedup_key = dedup_v1,
      },
    }

    t.is_true(dedup_v1 ~= dedup_v2)
    t.is_true(#dedup_v1 > 400)
    t.is_true(core.comment_marker(dedup_v1) ~= core.comment_marker(dedup_v2))

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment")
    mock_comment_write()
    local first = t.run_department("departments/github_comment/main.lua", event, opts("comment-long-v1", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(first.exit_code, 0)

    local path = "/tmp/fkst-github-proxy-comment-owner_x-issue-42.md"
    local written_v1 = file.read(path)
    t.is_true(written_v1:find(core.comment_marker(dedup_v1), 1, true) ~= nil)

    event.payload.dedup_key = dedup_v2
    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment " .. core.comment_marker(dedup_v1))
    mock_comment_write()
    local second = t.run_department("departments/github_comment/main.lua", event, opts("comment-long-v2", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(second.exit_code, 0)

    local written_v2 = file.read(path)
    t.is_true(written_v2:find(core.comment_marker(dedup_v2), 1, true) ~= nil)
    t.eq(count_calls("gh issue comment"), 2)
  end,

  test_near_max_comment_dedup_boundary_writes = function()
    local dedup = long_dedup("-max", 512)
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "max dedup reply",
        dedup_key = dedup,
      },
    }

    t.eq(#dedup, 512)
    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment")
    mock_comment_write()
    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-long-max", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)

    local written = file.read("/tmp/fkst-github-proxy-comment-owner_x-issue-42.md")
    t.is_true(written:find(core.comment_marker(dedup), 1, true) ~= nil)
    t.eq(count_calls("gh issue comment"), 1)
  end,

  test_comment_request_uses_payload_repo = function()
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/payload",
        issue_number = 42,
        body = "payload repo reply",
        dedup_key = "payload-repo-reply",
      },
    }

    mock_repo_env("owner/env")
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment")
    mock_comment_write()
    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-payload-repo", {
      FKST_GITHUB_REPO = "owner/env",
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)

    local view_calls = calls_matching("gh issue view")
    t.eq(#view_calls, 1)
    t.is_true(view_calls[1].rendered:find("--repo 'owner/payload'", 1, true) ~= nil)
    local comment_calls = calls_matching("gh issue comment")
    t.eq(#comment_calls, 1)
    t.is_true(comment_calls[1].rendered:find("--repo 'owner/payload'", 1, true) ~= nil)
  end,

  test_comment_real_write_failure_errors_for_retry = function()
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "fkst reply",
        dedup_key = "reply-failure",
      },
    }

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view("existing comment")
    t.mock_command("gh issue comment", {
      stdout = "",
      stderr = "forced comment failure",
      exit_code = 1,
    })

    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-write-fails", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("gh issue comment"), 1)
  end,

  test_comment_real_write_view_failure_errors_for_retry = function()
    local event = {
      queue = "github_issue_comment_request",
      payload = {
        repo = "owner/x",
        issue_number = 42,
        body = "fkst reply",
        dedup_key = "reply-view-failure",
      },
    }

    mock_repo_env()
    mock_write_env("1")
    mock_bot_env()
    mock_comment_view_failure()

    local result = t.run_department("departments/github_comment/main.lua", event, opts("comment-view-fails", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("gh issue view"), 1)
    t.eq(count_calls("gh issue comment"), 0)
  end,

  test_label_request_dry_run_write_and_rewrite = function()
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-dev:ready" },
        remove_labels = { "fkst-dev:thinking" },
        dedup_key = "github-devloop/issue/owner/x/42/result",
        source_ref = {
          kind = "external",
          ref = "owner/x#issue/42",
        },
      },
    }

    mock_write_env("")
    local dry = t.run_department("departments/github_issue_label/main.lua", event, opts("label-dry-run"))
    t.eq(dry.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 0)

    local write_opts = opts("label-write", {
      FKST_GITHUB_WRITE = "1",
    })
    mock_write_env("1")
    mock_label_write()
    local write = t.run_department("departments/github_issue_label/main.lua", event, write_opts)
    t.eq(write.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 1)
    local edit_calls = calls_matching("gh issue edit")
    t.is_true(edit_calls[1].rendered:find("--add-label 'fkst-dev:ready'", 1, true) ~= nil)
    t.is_true(edit_calls[1].rendered:find("--remove-label 'fkst-dev:thinking'", 1, true) ~= nil)

    mock_write_env("1")
    mock_label_write()
    local again = t.run_department("departments/github_issue_label/main.lua", event, write_opts)
    t.eq(again.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 2)
  end,

  test_long_label_dedup_uses_bounded_lock_key = function()
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-dev:ready" },
        remove_labels = {},
        dedup_key = long_dedup("-label", 430),
        source_ref = {
          kind = "external",
          ref = "owner/x#issue/42",
        },
      },
    }

    t.is_true(#event.payload.dedup_key > 400)
    mock_write_env("1")
    mock_label_write()
    local result = t.run_department("departments/github_issue_label/main.lua", event, opts("label-long-dedup", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 1)
  end,

  test_label_request_writes_without_state_precondition = function()
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-dev:ready" },
        remove_labels = { "fkst-dev:thinking" },
        dedup_key = "github-devloop/issue/owner/x/42/ready-hint",
        source_ref = {
          kind = "external",
          ref = "owner/x#issue/42",
        },
      },
    }

    local write_opts = opts("label-no-precondition", {
      FKST_GITHUB_WRITE = "1",
    })

    mock_write_env("1")
    mock_label_write()
    local current = t.run_department("departments/github_issue_label/main.lua", event, write_opts)
    t.eq(current.exit_code, 0)
    t.eq(count_calls("gh issue view"), 0)
    t.eq(count_calls("gh issue edit"), 1)
    local current_edit = calls_matching("gh issue edit")[1]
    t.is_true(current_edit.rendered:find("--add-label 'fkst-dev:ready'", 1, true) ~= nil)
    t.is_true(current_edit.rendered:find("--remove-label 'fkst-dev:thinking'", 1, true) ~= nil)
  end,

  test_label_request_applies_exclusive_hint_without_state_precondition = function()
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-dev:blocked" },
        remove_labels = { "fkst-dev:stuck", "fkst-dev:thinking", "fkst-dev:ready" },
        dedup_key = "github-devloop/issue/owner/x/42/blocked-hint",
        source_ref = {
          kind = "external",
          ref = "owner/x#issue/42",
        },
      },
    }

    mock_write_env("1")
    mock_label_write()
    local result = t.run_department("departments/github_issue_label/main.lua", event, opts("label-blocked-hint", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue view"), 0)
    t.eq(count_calls("gh issue edit"), 1)
    local edit = calls_matching("gh issue edit")[1]
    t.is_true(edit.rendered:find("--add-label 'fkst-dev:blocked'", 1, true) ~= nil)
    t.is_true(edit.rendered:find("--remove-label 'fkst-dev:ready'", 1, true) ~= nil)
	  end,

  test_pr_open_request_dry_run_does_not_push_or_create = function()
    mock_write_env("")
    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-dry-run"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
  end,

  test_pr_open_request_pushes_creates_comments_and_labels = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh issue edit"), 1)
    t.eq(count_calls("--json labels,comments"), 2)
    local create = calls_matching("gh pr create")[1]
    t.eq(create.rendered:find("--json", 1, true), nil)

    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #7", 1, true) ~= nil)
    t.is_true(issue_written:find('state="pr-open"', 1, true) ~= nil)
    t.is_true(issue_written:find('pr="7"', 1, true) ~= nil)

    local pr_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-pr-comment.md")
    t.is_true(pr_written:find("fkst:github-devloop:pr-origin:v1", 1, true) ~= nil)
  end,

  test_pr_open_request_read_after_write_lag_skips_tail_label_update = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_guard_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-read-after-write-lag", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("--json labels,comments"), 2)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_skips_tail_label_update_when_issue_advanced_to_reviewing = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({ reviewing_marker() }))

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-tail-reviewing", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("--json labels,comments"), 2)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_applies_tail_label_update_when_issue_is_still_pr_open = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-tail-pr-open", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 1)
    local edit = calls_matching("gh issue edit")[1]
    t.is_true(edit.rendered:find("--add-label 'fkst-dev:pr-open'", 1, true) ~= nil)
    t.is_true(edit.rendered:find("--remove-label 'fkst-dev:implementing'", 1, true) ~= nil)
    t.is_true(edit.rendered:find("--remove-label 'fkst-dev:pr-authorized'", 1, true) ~= nil)
  end,

  test_pr_open_request_skips_when_authorization_revoked_at_write_time = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_guard_comments())

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-revoked", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
  end,

  test_pr_open_request_skips_when_branch_moved_past_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("def456")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-branch-moved", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_skips_when_same_named_tag_matches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_non_branch_ref_head("abc123")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-same-named-tag", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_resolves_created_pr_with_head_list_when_create_stdout_is_unparseable = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create_stdout("created pull request\n")
    mock_pr_head_list('[{"number":11,"url":"https://github.example/owner/x/pull/11","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-list-after-create", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr list"), 2)
    t.eq(count_calls("gh pr create"), 1)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #11", 1, true) ~= nil)
  end,

  test_pr_open_request_fails_closed_when_created_pr_head_mismatches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("def456", "OPEN")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-created-head-mismatch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_fails_closed_when_created_pr_is_not_open = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "CLOSED")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-created-closed", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_reuses_existing_pr_without_duplicate_create = function()
    local event = pr_open_event()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-existing", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 1)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #9", 1, true) ~= nil)
  end,

  test_pr_open_request_fails_closed_when_existing_pr_head_mismatches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("def456", "OPEN")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-existing-head-mismatch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_fails_closed_when_existing_pr_is_cross_repo = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", "fork/x", true)

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-existing-fork", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_does_not_reuse_closed_same_head_pr = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"CLOSED"}]\n')
    mock_git_push()
    mock_pr_create(10)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-closed-head-not-reused", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #10", 1, true) ~= nil)
  end,

  test_pr_open_retry_after_issue_marker_self_heals_missing_pr_backpointer_and_label = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN")
    mock_pr_comment_view("existing pr comment without origin")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-self-heal", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh issue edit"), 1)
    t.eq(count_calls("--json labels,comments"), 2)

    local pr_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-pr-comment.md")
    t.is_true(pr_written:find("fkst:github-devloop:pr-origin:v1", 1, true) ~= nil)
    local edit = calls_matching("gh issue edit")[1]
    t.is_true(edit.rendered:find("--add-label 'fkst-dev:pr-open'", 1, true) ~= nil)
    t.is_true(edit.rendered:find("--remove-label 'fkst-dev:pr-authorized'", 1, true) ~= nil)
  end,

  test_pr_open_guard_uses_canonical_rank_so_meta_escalated_implementing_can_open_pr = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:stuck", "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_guard_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="stuck" version="v1" stage_rank="300" -->',
    }))
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:stuck", "fkst-dev:implementing", "fkst-dev:pr-authorized" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-canonical-rank", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
  end,

  test_pr_open_retry_after_reviewing_does_not_revert_issue_label = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->',
    }))
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN")
    mock_pr_comment_view({ {
      body = 'github-devloop implementation PR for issue #42\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/owner/x/42" issue="42" branch="devloop-owner-x-42-01HY" impl_version="v1" -->',
      author_login = "fkst-test-bot",
    } })
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->',
    }))

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-reviewing-no-label-regress", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
    t.eq(count_calls("--json labels,comments"), 2)
  end,
}
