local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function mock_bot(login, write_mode, write_reads)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = login or "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, write_reads or 2 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = write_mode or "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function ownership_json(logins, author_login)
  local rendered = {}
  for _, login in ipairs(logins or {}) do
    table.insert(rendered, string.format('{"login":"%s"}', tostring(login)))
  end
  return '{"assignees":[' .. table.concat(rendered, ",") .. '],"author":{"login":"'
    .. tostring(author_login or "fkst-test-bot") .. '"}}\n'
end

local function state(name, created_at)
  return {
    state = name or "thinking",
    version = "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    marker_created_at = created_at or "1970-01-01T00:00:00Z",
  }
end

local function self_current(extra)
  local fields = extra or {}
  return {
    assignees = fields.assignees or {},
    title = fields.title or "Implement fork isolation",
    author_login = fields.author_login or "fkst-test-bot",
    comments = fields.comments or {},
  }
end

local function capture_raises(fn)
  local old_raise = raise
  local raised = {}
  raise = function(queue, payload)
    table.insert(raised, {
      queue = queue,
      payload = payload,
    })
  end
  local ok, result = pcall(fn)
  raise = old_raise
  if not ok then
    error(result)
  end
  return result, raised
end

return {
  test_issue_claim_state_is_current_assignees_only = function()
    t.eq(core.issue_claim_state({}, "fkst-test-bot"), "unassigned")
    t.eq(core.issue_claim_state({ { login = "fkst-test-bot" } }, "fkst-test-bot"), "self")
    t.eq(core.issue_claim_state({ { login = "human" } }, "fkst-test-bot"), "other")
    t.eq(core.issue_claim_state({ { login = "fkst-test-bot" }, { login = "other-bot" } }, "fkst-test-bot"), "other")
  end,

  test_is_self_owned_issue_allows_self_assignee_or_unassigned_self_author = function()
    t.eq(core.is_self_owned_issue(nil, "fkst-test-bot"), false)
    t.eq(core.is_self_owned_issue({ assignees = { "fkst-test-bot" }, author_login = "human" }, "fkst-test-bot"), true)
    t.eq(core.is_self_owned_issue({ assignees = {}, author_login = "fkst-test-bot" }, "fkst-test-bot"), true)
    t.eq(core.is_self_owned_issue({ assignees = {}, author_login = "human" }, "fkst-test-bot"), false)
    t.eq(core.is_self_owned_issue({ assignees = { "human" }, author_login = "fkst-test-bot" }, "fkst-test-bot"), false)
  end,

  test_dry_run_claim_proceeds_without_assigning = function()
    mock_bot("fkst-test-bot", "")

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      self_current(),
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, true)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_claim_assigns_then_verifies_self_only_winner = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      self_current(),
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, true)
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 0)
  end,

  test_claim_loss_unassigns_only_self_and_skips = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "other-bot" }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      self_current(),
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, false)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--remove-assignee 'other-bot'"), 0)
  end,

  test_non_self_assignee_is_never_touched = function()
    mock_bot("fkst-test-bot", "1")

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "human" } } },
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, false)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_other_author_unassigned_issue_raises_self_assigned_fork_without_assigning = function()
    mock_bot("fkst-test-bot", "1")

    local ok, raised = capture_raises(function()
      return core.claim_issue_for_management(
        "claim_contract",
        "owner/repo",
        42,
        self_current({ author_login = "human" }),
        "github-devloop/issue/owner/repo/42"
      )
    end)

    t.eq(ok, false)
    t.eq(count_calls("gh issue edit"), 0)
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(raised[1].payload.schema, "github-proxy.issue-create.v1")
    t.eq(raised[1].payload.assignees[1], "fkst-test-bot")
    t.eq(raised[1].payload.dedup_key, core.fork_issue_dedup_key("owner/repo", 42))
    t.eq(raised[1].payload.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(raised[1].payload.post_create_blocked_by.dedup_key, core.fork_issue_dedup_key("owner/repo", 42) .. "/blocked-by")
  end,

  test_missing_author_unassigned_issue_skips_without_assigning_or_forking = function()
    mock_bot("fkst-test-bot", "1")

    local ok, raised = capture_raises(function()
      return core.claim_issue_for_management(
        "claim_contract",
        "owner/repo",
        42,
        { assignees = {}, title = "Unknown author", comments = {} },
        "github-devloop/issue/owner/repo/42"
      )
    end)

    t.eq(ok, false)
    t.eq(#raised, 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_existing_fork_parent_ledger_skips_duplicate_fork = function()
    mock_bot("fkst-test-bot", "1")
    local dedup_key = core.fork_issue_dedup_key("owner/repo", 42)

    local ok, raised = capture_raises(function()
      return core.claim_issue_for_management(
        "claim_contract",
        "owner/repo",
        42,
        self_current({
          author_login = "human",
          comments = {
            {
              body = '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. dedup_key .. '" issue="99" -->',
              author_login = "fkst-test-bot",
            },
          },
        }),
        "github-devloop/issue/owner/repo/42"
      )
    end)

    t.eq(ok, false)
    t.eq(#raised, 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_existing_fork_parent_intent_skips_duplicate_fork = function()
    mock_bot("fkst-test-bot", "1")
    local dedup_key = core.fork_issue_dedup_key("owner/repo", 42)

    local ok, raised = capture_raises(function()
      return core.claim_issue_for_management(
        "claim_contract",
        "owner/repo",
        42,
        self_current({
          author_login = "human",
          comments = {
            {
              body = '<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. dedup_key .. '" -->',
              author_login = "fkst-test-bot",
            },
          },
        }),
        "github-devloop/issue/owner/repo/42"
      )
    end)

    t.eq(ok, false)
    t.eq(#raised, 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_forged_fork_parent_intent_does_not_suppress_fork = function()
    mock_bot("fkst-test-bot", "1")
    local dedup_key = core.fork_issue_dedup_key("owner/repo", 42)

    local ok, raised = capture_raises(function()
      return core.claim_issue_for_management(
        "claim_contract",
        "owner/repo",
        42,
        self_current({
          author_login = "human",
          comments = {
            {
              body = '<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. dedup_key .. '" -->',
              author_login = "human",
            },
          },
        }),
        "github-devloop/issue/owner/repo/42"
      )
    end)

    t.eq(ok, false)
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(raised[1].payload.dedup_key, dedup_key)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_timeout_release_requires_fresh_self_only_read = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local released = core.maybe_release_stale_self_claim(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "fkst-test-bot" } } },
      "github-devloop/issue/owner/repo/42",
      state("thinking")
    )

    t.eq(released, true)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
  end,

  test_timeout_release_can_be_followed_by_reclaim = function()
    mock_bot("fkst-test-bot", "1", 4)
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })

    local released = core.maybe_release_stale_self_claim(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "fkst-test-bot" } } },
      "github-devloop/issue/owner/repo/42",
      state("thinking")
    )
    local reclaimed = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      self_current(),
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(released, true)
    t.eq(reclaimed, true)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 1)
  end,

  test_timeout_release_skips_when_fresh_read_is_not_self_only = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({ "fkst-test-bot", "human" }),
      stderr = "",
      exit_code = 0,
    })

    local released = core.maybe_release_stale_self_claim(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "fkst-test-bot" } } },
      "github-devloop/issue/owner/repo/42",
      state("thinking")
    )

    t.eq(released, false)
    t.eq(count_calls("--remove-assignee"), 0)
  end,

  test_verify_pr_review_issue_claim_predicate_contract = function()
    mock_bot("fkst-test-bot", "")
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = { "fkst-test-bot" },
      author_login = "human",
    }, "github-devloop/issue/owner/repo/42"), true)
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = { "human" },
      author_login = "fkst-test-bot",
    }, "github-devloop/issue/owner/repo/42"), false)
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = {},
      author_login = "fkst-test-bot",
    }, "github-devloop/issue/owner/repo/42"), true)
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = {},
      author_login = "human",
    }, "github-devloop/issue/owner/repo/42"), false)
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", nil, nil, "github-devloop/pr/owner/repo/7"), false)
  end,

  test_verify_pr_review_issue_claim_uses_configured_claim_owner_before_assert = function()
    mock_bot("real-bot", "")

    local self_owned = core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = { "real-bot" },
      author_login = "human",
    }, "github-devloop/issue/owner/repo/42")
    local other_owned = core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = { "fkst-test-bot" },
      author_login = "human",
    }, "github-devloop/issue/owner/repo/42")
    core.configure_trusted_bot_login(nil)

    t.eq(self_owned, true)
    t.eq(other_owned, false)
  end,

  test_verify_pr_review_issue_claim_rederives_missing_ownership_and_fails_closed = function()
    mock_bot("fkst-test-bot", "")
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = ownership_json({}, "fkst-test-bot"),
      stderr = "",
      exit_code = 0,
    })
    t.eq(core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, {
      assignees = {},
    }, "github-devloop/issue/owner/repo/42"), true)

    mock_bot("fkst-test-bot", "")
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = "",
      stderr = "forced failure",
      exit_code = 1,
    })
    local ok = pcall(function()
      core.verify_pr_review_issue_claim("claim_contract", "owner/repo", 42, nil, "github-devloop/issue/owner/repo/42")
    end)
    t.eq(ok, false)
  end,
}
