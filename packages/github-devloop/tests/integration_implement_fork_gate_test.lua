local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local content_filter = require("forge.github.content_filter")
local h = require("tests.devloop_helpers")
local forks = require("devloop.forks")
local parsers_issue = require("devloop.parsers.issue")
local t = h.t
local core = h.core
local ready = h.ready
local run_implement = h.run_implement
local opts = h.opts
local mock_issue_implement = h.mock_issue_implement
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local count_calls = h.count_calls
local find_raise = h.find_raise
local author_policy = require("testkit.github_author_policy")

local original_issue = 1663
local canonical_issue = 1715

local function original_view_with_fork_ledger()
  local dedup_key = forks.fork_issue_dedup_key("owner/repo", original_issue)
  local marker = '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. dedup_key
    .. '" issue="' .. tostring(canonical_issue) .. '" -->'
  return '{"title":"Original","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[{"body":"'
    .. marker:gsub('"', '\\"')
    .. '","author":{"login":"fkst-test-bot"}}],"assignees":[],"author":{"login":"human"}}\n'
end

local function original_view_with_peer_fork_ledger(author_login)
  local dedup_key = forks.fork_issue_dedup_key("owner/repo", original_issue)
  local marker = '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. dedup_key
    .. '" issue="' .. tostring(canonical_issue) .. '" -->'
  return '{"title":"Original","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[{"body":"'
    .. marker:gsub('"', '\\"')
    .. '","author":{"login":"' .. tostring(author_login or "ElonSG") .. '"}}],"assignees":[],"author":{"login":"human"}}\n'
end

local function redacted_original_state_for(logins, author_login)
  local filtered = content_filter.filter_gh_content_json(
    original_view_with_peer_fork_ledger(author_login),
    content_filter.build_whitelist(logins),
    {}
  )
  return parsers_issue.parse_issue_view_state(core, filtered), filtered
end

local function find_duplicate_comment(raises)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find("Duplicate fork for owner/repo#" .. tostring(original_issue), 1, true) ~= nil
      and tostring(payload.body or ""):find("canonical fork is #" .. tostring(canonical_issue), 1, true) ~= nil
  end)
end

local function find_duplicate_label(raises)
  return find_raise(raises, "github-proxy.github_issue_label_request", function(payload)
    for _, label in ipairs(payload.add_labels or {}) do
      if label == "fkst:duplicate-fork" then
        return true
      end
    end
    return false
  end)
end

local function command_count_snapshot()
  return {
    codex = count_calls("codex exec"),
    worktree_list = count_calls("git worktree list"),
    git_c = count_calls("git -C"),
    issue_close = count_calls("gh issue close"),
  }
end

return {
  test_noncanonical_fork_exits_before_implementation = function()
    local event = ready()
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
      forks.fork_origin_marker("owner/repo", original_issue, "human", entity_lib.issue_source_ref("owner/repo", original_issue)),
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", original_issue), {
      stdout = original_view_with_fork_ledger(),
      stderr = "",
      exit_code = 0,
    })

    local before = command_count_snapshot()
    local result = run_implement(event, opts("implement-duplicate-fork", {
      FKST_GITHUB_WRITE = "",
    }))

    t.eq(result.exit_code, 0)
    t.is_true(find_duplicate_comment(result.raises) ~= nil)
    t.is_true(find_duplicate_label(result.raises) ~= nil)
    t.eq(count_calls("codex exec") - before.codex, 0)
    t.eq(count_calls("git worktree list") - before.worktree_list, 0)
    t.eq(count_calls("git -C") - before.git_c, 0)
    t.eq(count_calls("gh issue close") - before.issue_close, 0)
  end,

  test_peer_bot_authored_noncanonical_fork_exits_before_implementation = function()
    local event = ready()
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      author_login = "ElonSG",
      body = forks.fork_issue_body("owner/repo", original_issue, "human", entity_lib.issue_source_ref("owner/repo", original_issue)),
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", original_issue), {
      stdout = original_view_with_peer_fork_ledger("ElonSG"),
      stderr = "",
      exit_code = 0,
    })
    author_policy.mock_env(t, {
      env = {
        FKST_GITHUB_BOT_LOGIN = "loning",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "loning,ElonSG",
      },
    }, {
      configure_trusted_bot_login = h.mock_author_policy_configure,
    })

    local before = command_count_snapshot()
    local result = run_implement(event, opts("implement-peer-duplicate-fork", {
      FKST_GITHUB_BOT_LOGIN = "loning",
      FKST_GITHUB_WRITE = "",
    }))

    t.eq(result.exit_code, 0)
    t.is_true(find_duplicate_comment(result.raises) ~= nil)
    t.is_true(find_duplicate_label(result.raises) ~= nil)
    t.eq(count_calls("codex exec") - before.codex, 0)
    t.eq(count_calls("git worktree list") - before.worktree_list, 0)
    t.eq(count_calls("git -C") - before.git_c, 0)
    t.eq(count_calls("gh issue close") - before.issue_close, 0)
  end,

  test_untrusted_authored_noncanonical_fork_marker_is_redacted_and_ignored = function()
    local event = ready()
    local branch = devloop_base.implement_branch("owner/repo", "42", event.dedup_key)
    local redacted_original, redacted_stdout = redacted_original_state_for({ "loning", "ElonSG" }, "mallory")
    t.is_true(redacted_stdout:find("[fkst:blocked-github-content:v1", 1, true) ~= nil)
    t.is_nil(forks.trusted_issue_created_number(
      core,
      redacted_original.comments,
      forks.fork_issue_dedup_key("owner/repo", original_issue),
      "loning",
      { loning = true, elonsg = true }
    ))
    mock_issue_implement({ "fkst-dev:ready" }, {
      {
        body = core.state_marker(event.proposal_id, "ready", event.dedup_key),
        author_login = "loning",
      },
    }, {
      author_login = "loning",
      body = forks.fork_issue_body("owner/repo", original_issue, "human", entity_lib.issue_source_ref("owner/repo", original_issue)),
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", original_issue), {
      stdout = original_view_with_peer_fork_ledger("mallory"),
      stderr = "",
      exit_code = 0,
    })
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)

    local before = command_count_snapshot()
    local result = run_implement(event, opts("implement-untrusted-duplicate-fork", {
      FKST_GITHUB_BOT_LOGIN = "loning",
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "loning,ElonSG",
      FKST_GITHUB_WRITE = "",
    }))

    t.eq(result.exit_code, 0)
    t.is_nil(find_duplicate_comment(result.raises))
    t.is_nil(find_duplicate_label(result.raises))
    t.is_true(count_calls("codex exec") - before.codex > 0)
  end,
}
