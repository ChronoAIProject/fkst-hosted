local h = require("tests.proxy_integration_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls

local function event(extra)
  local payload = {
    schema = "github-proxy.issue-create.v1",
    repo = "owner/x",
    title = "Split blocked PR into smaller work",
    body = "Parent: #42\n\nScope: implement the smallest viable slice.",
    labels = { "triage" },
    dedup_key = "decompose/generic-workflow/issue/owner/x/42/v1/1/123",
    parent_comment_target = {
      repo = "owner/x",
      pr_number = 7,
    },
    source_ref = {
      kind = "external",
      ref = "owner/x#pr/7",
    },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "github_issue_create_request",
    payload = payload,
  }
end

local function archaudit_payload(extra)
  local payload = {
    schema = "github-proxy.issue-create.v1",
    repo = "owner/repo",
    title = "Archaudit: packages/archaudit/core.lua:1 SRP",
    body = table.concat({
      "Architecture doctrine violation:",
      "",
      "File: packages/archaudit/core.lua:1",
      "Rule: SRP",
      "",
      "Why:",
      "Core has one concrete issue.",
      "",
      "Suggested fix:",
      "Move the local helper.",
      "",
      "<!-- archaudit-dedup: archaudit/owner_repo/packages_archaudit_core_lua/1/SRP/123456 -->",
    }, "\n"),
    labels = {},
    dedup_key = "archaudit/owner_repo/packages_archaudit_core_lua/1/SRP/123456",
    source_ref = {
      kind = "repo-site",
      ref = "owner/repo#packages/archaudit/core.lua:1#archaudit-create-intent",
    },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

local function mock_issue_create_search(stdout)
  t.mock_command("gh issue list", {
    stdout = stdout or "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_create()
  t.mock_command("gh issue create", {
    stdout = "https://github.example/owner/x/issues/99\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_child_issue_rest_view(issue_number, child_id)
  t.mock_command("gh api repos/owner/x/issues/" .. tostring(issue_number or 99), {
    stdout = '{"id":' .. tostring(child_id or 987654321) .. ',"number":' .. tostring(issue_number or 99) .. "}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_source_issue_author(author_login)
  local author = "null"
  if author_login ~= nil then
    author = '{"login":"' .. h.json_string(author_login) .. '"}'
  end
  t.mock_command("gh api repos/owner/x/issues/42", {
    stdout = '{"number":42,"title":"Source issue","body":"Source body","state":"open","user":' .. author .. "}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_add_sub_issue(parent_number, child_id, exit_code, stderr)
  t.mock_command(
    "gh api --method POST repos/owner/x/issues/" .. tostring(parent_number or 42) .. "/sub_issues -F sub_issue_id=" .. tostring(child_id or 987654321),
    {
      stdout = "",
      stderr = stderr or "",
      exit_code = exit_code or 0,
    }
  )
end

local function mock_parent_sub_issues(parent_number, child_id)
  t.mock_command(
    "gh api --paginate --slurp repos/owner/x/issues/" .. tostring(parent_number or 42) .. "/sub_issues?per_page=100",
    {
      stdout = '[[{"id":' .. tostring(child_id or 987654321) .. ',"number":99}]]\n',
      stderr = "",
      exit_code = 0,
    }
  )
end

local function mock_parent_pr_comments(comments)
  h.mock_pr_comment_view(comments or {})
end

local function mock_parent_pr_comment_write()
  h.mock_pr_comment_write()
end

local function mock_parent_issue_comment_write()
  t.mock_command("gh issue comment 42 --repo owner/x --body-file /tmp/fkst-github-proxy-intent-", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue comment 42 --repo owner/x --body-file /tmp/fkst-github-proxy-created-", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function first_call_index(needle)
  for index, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      return index
    end
  end
  return nil
end

return {
  test_issue_create_parent_ledger_markers_have_visible_text_and_parse = function()
    local dedup_key = event().payload.dedup_key
    local created = core.issue_created_marker(dedup_key, "99")
    local intent = core.issue_create_intent_marker(dedup_key)

    t.is_true(created:find("Opened sub-issue #99 for this task.\n\n", 1, true) == 1)
    t.is_true(created:find('<!-- fkst:github-proxy:issue-created:v1 dedup="' .. dedup_key .. '" issue="99" -->', 1, true) ~= nil)
    t.is_true(intent:find("Preparing to open a sub-issue for this task.\n\n", 1, true) == 1)
    t.is_true(intent:find('<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. dedup_key .. '" -->', 1, true) ~= nil)
    t.eq(core.has_trusted_issue_created_marker({
      {
        body = created,
        author_login = "fkst-test-bot",
      },
    }, dedup_key, "fkst-test-bot"), true)
    t.eq(core.has_trusted_issue_create_intent_marker({
      {
        body = intent,
        author_login = "fkst-test-bot",
      },
    }, dedup_key, "fkst-test-bot"), true)
    t.eq(core.issue_created_marker(dedup_key, "99"), created)
    t.eq(core.issue_create_intent_marker(dedup_key), intent)
  end,

  test_issue_create_request_dry_run_does_not_search_or_create = function()
    mock_write_env("")

    local result = t.run_department("departments/github_issue_create/main.lua", event(), opts("issue-create-dry-run"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("/sub_issues"), 0)
  end,

  test_issue_create_request_dry_run_with_parent_does_not_link_native_sub_issue = function()
    mock_write_env("")

    local result = t.run_department("departments/github_issue_create/main.lua", event({
      parent = 42,
    }), opts("issue-create-dry-run-native-sub-issue"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/99"), 0)
    t.eq(count_calls("/sub_issues"), 0)
  end,

  test_issue_create_request_missing_fields_fail_closed = function()
    mock_write_env("1")
    mock_bot_env()

    local result = t.run_department("departments/github_issue_create/main.lua", event({
      title = "",
    }), opts("issue-create-missing-title", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
  end,

  test_issue_create_request_trusted_marker_skips_create = function()
    local payload = event().payload
    payload.parent_comment_target = nil
    mock_write_env("1")
    mock_bot_env()
    mock_issue_create_search(string.format(
      '[{"number":99,"title":"Existing","state":"OPEN","body":"already created\\n%s","author":{"login":"fkst-test-bot"}}]\n',
      h.json_string(core.issue_create_marker(payload.dedup_key))
    ))
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-idempotent", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 0)
  end,

  test_archaudit_issue_create_request_trusted_marker_skips_create = function()
    local payload = archaudit_payload()
    mock_write_env("1")
    mock_bot_env()
    mock_issue_create_search(string.format(
      '[{"number":99,"title":"Existing","state":"OPEN","body":"already created\\n%s","author":{"login":"fkst-test-bot"}}]\n',
      h.json_string(core.issue_create_marker(payload.dedup_key))
    ))
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-archaudit-marker-skip", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 0)
  end,

  test_issue_create_request_parent_ledger_marker_skips_create = function()
    local payload = event().payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({
      {
        body = core.issue_created_marker(payload.dedup_key, "99"),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-parent-ledger-skip", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 1)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
  end,

  test_issue_create_request_parent_intent_marker_resumes_create = function()
    local payload = event().payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_pr_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-parent-intent-skip", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 1)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 1)
    t.eq(count_calls("fkst-github-proxy-created"), 1)
  end,

  test_issue_create_request_parent_intent_reconciles_existing_issue_marker = function()
    local payload = event().payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search(string.format(
      '[{"number":99,"title":"Existing","state":"OPEN","body":"already created\\n%s","author":{"login":"fkst-test-bot"}}]\n',
      h.json_string(core.issue_create_marker(payload.dedup_key))
    ))
    mock_issue_create()
    mock_parent_pr_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-parent-intent-reconcile", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("fkst-github-proxy-created"), 1)
  end,

  test_issue_create_request_real_write_calls_gh_issue_create = function()
    local payload = event({
      assignees = { "fkst-test-bot" },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({})
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_pr_comment_write()
    mock_parent_pr_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-write", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 2)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 1)
    t.eq(count_calls("--assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("gh pr comment"), 2)
    t.is_true(first_call_index("gh pr comment") < first_call_index("gh issue create"))
  end,

  test_issue_create_request_raises_blocked_by_after_fresh_create = function()
    local payload = event({
      parent_comment_target = {
        repo = "owner/x",
        issue_number = 42,
      },
      post_create_blocked_by = {
        blocked_issue_number = 42,
        dedup_key = "decompose/generic-workflow/issue/owner/x/42/v1/1/123/blocked-by",
      },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    h.mock_comment_view({})
    h.mock_comment_view({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_issue_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-post-blocked-by", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    local raised = result.raises[1]
    t.eq(raised.queue, "github_issue_blocked_by_request")
    t.eq(raised.payload.schema, "github-proxy.issue-blocked-by.v1")
    t.eq(raised.payload.blocked_issue_number, 42)
    t.eq(raised.payload.blocking_issue_number, 99)
    t.eq(raised.payload.dedup_key, payload.post_create_blocked_by.dedup_key)
    t.eq(count_calls("gh issue list"), 1)
  end,

  test_issue_create_request_raises_blocked_by_from_existing_created_marker = function()
    local payload = event({
      parent_comment_target = {
        repo = "owner/x",
        issue_number = 42,
      },
      post_create_blocked_by = {
        blocked_issue_number = 42,
        dedup_key = "decompose/generic-workflow/issue/owner/x/42/v1/1/123/blocked-by",
      },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    h.mock_comment_view({
      {
        body = core.issue_created_marker(payload.dedup_key, "99"),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-post-blocked-by-existing", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(result.raises[1].queue, "github_issue_blocked_by_request")
    t.eq(result.raises[1].payload.blocking_issue_number, 99)
  end,

  test_issue_create_request_parent_comment_target_only_records_parent_ledger_marker_without_sub_issue_link = function()
    local payload = event().payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({})
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_pr_comment_write()
    mock_parent_pr_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-parent-ledger-write", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 1)
    t.eq(count_calls("gh pr comment"), 2)
    t.eq(count_calls("gh api repos/owner/x/issues/99"), 0)
    t.eq(count_calls("/sub_issues"), 0)
    t.eq(count_calls("fkst-github-proxy-intent"), 1)
    t.eq(count_calls("fkst-github-proxy-created"), 1)
    t.eq(core.has_trusted_issue_created_marker({
      {
        body = core.issue_created_marker(payload.dedup_key, "99"),
        author_login = "fkst-test-bot",
      },
    }, payload.dedup_key, "fkst-test-bot"), true)
  end,

  test_issue_create_request_parent_only_links_native_sub_issue_without_parent_ledger = function()
    local payload = event().payload
    payload.parent = 42
    payload.parent_comment_target = nil
    mock_write_env("1")
    mock_bot_env()
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_child_issue_rest_view(99, 987654321)
    mock_issue_add_sub_issue(42, 987654321)

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-native-sub-issue", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue create"), 1)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/42/comments?per_page=100"), 0)
    t.eq(count_calls("fkst-github-proxy-created"), 0)
    t.eq(count_calls("gh issue comment 42 --repo owner/x"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/99"), 1)
    t.eq(count_calls("gh api --method POST repos/owner/x/issues/42/sub_issues -F sub_issue_id=987654321"), 1)
  end,

  test_issue_create_request_with_parent_relinks_existing_child_on_redelivery = function()
    local payload = event({
      parent = 42,
    }).payload
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({
      {
        body = core.issue_created_marker(payload.dedup_key, "99"),
        author_login = "fkst-test-bot",
      },
    })
    mock_child_issue_rest_view(99, 987654321)
    mock_issue_add_sub_issue(
      42,
      987654321,
      1,
      "HTTP 422: Validation Failed (already linked as a sub-issue)"
    )
    mock_parent_sub_issues(42, 987654321)
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-native-sub-issue-redelivery", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/99"), 1)
    t.eq(count_calls("gh api --method POST repos/owner/x/issues/42/sub_issues -F sub_issue_id=987654321"), 1)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/42/sub_issues?per_page=100"), 1)
  end,

  test_issue_create_request_second_delivery_same_dedup_skips_create = function()
    local payload = event().payload
    local run_opts = opts("issue-create-once-dedup", {
      FKST_GITHUB_WRITE = "1",
    })
    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({})
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_pr_comment_write()
    mock_parent_pr_comment_write()

    local first = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, run_opts)
    t.eq(first.exit_code, 0)

    mock_write_env("1")
    mock_bot_env()
    mock_parent_pr_comments({
      {
        body = core.issue_create_intent_marker(payload.dedup_key),
        author_login = "fkst-test-bot",
      },
    })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_pr_comment_write()

    local second = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, run_opts)
    t.eq(second.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 3)
    t.eq(count_calls("gh issue list"), 2)
    t.eq(count_calls("gh issue create"), 1)
  end,

  test_issue_create_request_without_parent_uses_issue_search_fallback = function()
    local payload = event().payload
    payload.parent_comment_target = nil
    mock_write_env("1")
    mock_bot_env()
    mock_issue_create_search(string.format(
      '[{"number":99,"title":"Existing","state":"OPEN","body":"already created\\n%s","author":{"login":"fkst-test-bot"}}]\n',
      h.json_string(core.issue_create_marker(payload.dedup_key))
    ))
    mock_issue_create()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("issue-create-no-parent-search-fallback", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 0)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("/sub_issues"), 0)
  end,

  test_fork_issue_create_request_reauthorizes_source_author_before_writes = function()
    local payload = event({
      external_effect_saga = "fork-and-block",
      external_effect_step = "create-fork",
      parent_comment_target = {
        repo = "owner/x",
        issue_number = 42,
      },
      post_create_blocked_by = {
        blocked_issue_number = 42,
        dedup_key = "github-devloop/fork/owner/x/issue/42/v1/blocked-by",
        external_effect_saga = "fork-and-block",
        external_effect_step = "block-original",
      },
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    mock_source_issue_author("untrusted-human")
    local intent_comment = {
      body = core.issue_create_intent_marker(payload.dedup_key),
      author_login = "fkst-test-bot",
    }
    h.mock_comment_view({ intent_comment })
    h.mock_comment_view({ intent_comment })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_issue_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("fork-issue-create-unauthorized-author", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 1)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("gh issue comment 42 --repo owner/x"), 0)
  end,

  test_fork_issue_create_request_rejects_missing_source_author = function()
    local payload = event({
      dedup_key = "github-devloop/fork/owner/x/issue/42/missing-author",
      external_effect_saga = "fork-and-block",
      external_effect_step = "create-fork",
      parent_comment_target = {
        repo = "owner/x",
        issue_number = 42,
      },
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    mock_source_issue_author(nil)
    h.mock_comment_view({})

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("fork-issue-create-missing-author", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 1)
    t.eq(count_calls("gh issue list"), 0)
    t.eq(count_calls("gh issue create"), 0)
    t.eq(count_calls("gh issue comment 42 --repo owner/x"), 0)
  end,

  test_fork_issue_create_request_allows_authorized_source_author = function()
    local payload = event({
      dedup_key = "github-devloop/fork/owner/x/issue/42/authorized-author",
      external_effect_saga = "fork-and-block",
      external_effect_step = "create-fork",
      parent_comment_target = {
        repo = "owner/x",
        issue_number = 42,
      },
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    }).payload
    mock_write_env("1")
    mock_bot_env()
    mock_source_issue_author("trusted-human")
    local intent_comment = {
      body = core.issue_create_intent_marker(payload.dedup_key),
      author_login = "fkst-test-bot",
    }
    h.mock_comment_view({ intent_comment })
    h.mock_comment_view({ intent_comment })
    mock_issue_create_search("[]\n")
    mock_issue_create()
    mock_parent_issue_comment_write()

    local result = t.run_department("departments/github_issue_create/main.lua", {
      queue = "github_issue_create_request",
      payload = payload,
    }, opts("fork-issue-create-authorized-author", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 1)
    t.eq(count_calls("gh issue list"), 1)
    t.eq(count_calls("gh issue create"), 1)
    t.eq(count_calls("gh issue comment 42 --repo owner/x"), 1)
  end,
}
