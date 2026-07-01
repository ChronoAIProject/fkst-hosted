local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")
local entity_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core

local repo = "rollup-owner/rollup-repo"
local issue_number = 4242
local child_pr_number = 4247
local rollup_pr_number = 4249
local parent = base_ids.proposal_id(repo, issue_number)
local child_pr = entity_lib.pr_proposal_id(repo, child_pr_number)
local version = "ready/consensus-" .. parent .. "/2026-06-03T01-02-03Z"
local child_head_sha = "0123456789abcdef0123456789abcdef01234567"
local child_merge_commit_sha = "1111111111111111111111111111111111111111"
local rollup_head_sha = "fedcba9876543210fedcba9876543210fedcba98"
local integration_branch = "integration-elonsg"
local upstream_branch = "dev"
local child_branch = "devloop-rollup-owner-rollup-repo-4242-01HY"

local function issue_comments_api_cmd()
  return "gh api --paginate --slurp repos/" .. repo .. "/issues/" .. tostring(issue_number) .. "/comments?per_page=100"
end

local function quoted_issue_comments_api_cmd()
  return "gh api --paginate --slurp 'repos/" .. repo .. "/issues/" .. tostring(issue_number) .. "/comments?per_page=100'"
end

local function issue_rest_api_cmd()
  return "gh api repos/" .. repo .. "/issues/" .. tostring(issue_number)
end

local function comment(body, created_at)
  return {
    id = tostring(created_at or body):gsub("[^%w_%-]", "_"):sub(1, 60),
    body = body,
    author_login = core._test_bot_login,
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function parent_comments()
  return {
    comment(core.state_marker(parent, "awaiting-pr", version), "2026-06-03T01:02:03Z"),
    comment(m_builders.pr_delegation_marker(core, parent, child_pr, child_pr_number, version, "g1"), "2026-06-03T01:03:03Z"),
  }
end

local function child_pr_comments(state)
  local child_state = state or "merged"
  local body = m_builders.pr_origin_marker(core, parent, issue_number, child_branch, version, integration_branch)
    .. "\n" .. core.state_marker(parent, child_state, version)
  if child_state == "merged" then
    body = body .. "\n" .. m_builders.merged_marker(core, parent, child_pr_number, version, child_head_sha)
  end
  return {
    comment(body, "2026-06-03T01:04:03Z"),
  }
end

local function status_rollup_success()
  return '[{"name":"ci","state":"COMPLETED","conclusion":"SUCCESS"}]'
end

local function initial_event()
  return {
    queue = "github-devloop-integration.devloop_rollup_ready",
    payload = core.rollup_ready_payload(
      repo,
      upstream_branch,
      integration_branch,
      rollup_pr_number,
      rollup_head_sha
    ),
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(rollup_pr_number),
    },
  }
end

local function mock_common_env()
  for _ = 1, 12 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = core._test_bot_login,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
      stdout = repo,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_CLAIM_MODE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
      stdout = upstream_branch,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
      stdout = integration_branch,
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_rollup_merge_success()
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = rollup_pr_number,
    head = integration_branch,
    head_sha = rollup_head_sha,
    base_branch = upstream_branch,
    state = "OPEN",
    head_repo = repo,
    status_check_rollup_json = status_rollup_success(),
  }, entity_mocks.pr_merge_selector)
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = rollup_pr_number,
    head = integration_branch,
    head_sha = rollup_head_sha,
    base_branch = upstream_branch,
    state = "OPEN",
    head_repo = repo,
    status_check_rollup_json = status_rollup_success(),
  }, entity_mocks.pr_merge_selector)
  t.mock_command(
    "gh pr merge '" .. tostring(rollup_pr_number) .. "' --repo '" .. repo .. "' --merge --match-head-commit '" .. rollup_head_sha .. "'",
    {
      stdout = "merged\n",
      stderr = "",
      exit_code = 0,
    }
  )
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = rollup_pr_number,
    head = integration_branch,
    head_sha = rollup_head_sha,
    base_branch = upstream_branch,
    state = "MERGED",
    head_repo = repo,
    merged_at = "2026-06-03T02:03:04Z",
    status_check_rollup_json = status_rollup_success(),
  }, entity_mocks.pr_merge_selector)
end

local function mock_liveness_scan_inputs(child_state)
  local effective_child_state = child_state or "merged"
  entity_mocks.mock_issue_list_command(t, core.gh_issue_list_observe_cmd(repo), {
    {
      number = issue_number,
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    },
  })
  entity_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    title = "Awaiting delegated PR",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = parent_comments(),
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "title,body,comments,labels,state,createdAt,updatedAt,assignees,author")
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = child_pr_number,
    comments = child_pr_comments(child_state),
    head = child_branch,
    head_sha = child_head_sha,
    merge_commit_sha = child_merge_commit_sha,
    state = effective_child_state == "merged" and "MERGED" or "OPEN",
    base_branch = integration_branch,
    merged_at = effective_child_state == "merged" and "2026-06-03T02:03:04Z" or nil,
    labels = {},
  }, entity_mocks.pr_origin_selector, 2)
end

local function mock_rollup_landing(exit_code)
  t.mock_command(core.git_fetch_branch_cmd("origin", upstream_branch), {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.git_remote_branch_head_cmd("origin", upstream_branch), {
    stdout = rollup_head_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor " .. child_merge_commit_sha .. " " .. rollup_head_sha, {
    stdout = "",
    stderr = "",
    exit_code = exit_code,
  })
end

local function mock_observe_issue_inputs(child_state, landed)
  local effective_child_state = child_state or "merged"
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":0,"pageInfo":{"hasNextPage":false},"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  entity_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    title = "Awaiting delegated PR",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = parent_comments(),
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "title,body,comments,labels,state,createdAt,updatedAt,assignees,author")
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = child_pr_number,
    comments = child_pr_comments(child_state),
    head = child_branch,
    head_sha = child_head_sha,
    merge_commit_sha = child_merge_commit_sha,
    state = effective_child_state == "merged" and "MERGED" or "OPEN",
    base_branch = integration_branch,
    merged_at = effective_child_state == "merged" and "2026-06-03T02:03:04Z" or nil,
    labels = {},
  }, entity_mocks.pr_origin_selector, 2)
  if effective_child_state == "merged" then
    mock_rollup_landing(landed == false and 1 or 0)
  end
  t.mock_command("gh issue close " .. tostring(issue_number) .. " --repo " .. repo, {
    stdout = "closed\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_github_proxy_writes()
  for _, command in ipairs({
    issue_comments_api_cmd(),
    quoted_issue_comments_api_cmd(),
  }) do
    t.mock_command(command, {
      stdout = "[[]]\n",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 2 do
    t.mock_command(issue_rest_api_cmd(), {
      stdout = '{"labels":[{"name":"fkst-dev:awaiting-pr"}],"assignees":[{"login":"fkst-test-bot"}]}\n',
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("gh api --method POST repos/" .. repo .. "/issues/" .. tostring(issue_number) .. "/comments --field 'body=", {
    stdout = '{"id":123456,"body":"created","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh label list --repo " .. repo .. " --limit 1000 --json name", {
    stdout = '[{"name":"fkst-dev:awaiting-pr"},{"name":"fkst-dev:merged"}]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue edit " .. tostring(issue_number) .. " --repo " .. repo, {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_everything(child_state, landed)
  mock_common_env()
  mock_rollup_merge_success()
  mock_liveness_scan_inputs(child_state)
  mock_observe_issue_inputs(child_state, landed)
  mock_github_proxy_writes()
end

return {
  test_rollup_merge_nudges_issue_liveness_scan_after_rollup_lands_on_dev = function()
    mock_everything()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 10 }))
    graph.assert_covers(trace, {
      "github-devloop-integration.devloop_rollup_ready -> github-devloop-integration.rollup_merge",
      "github-devloop.devloop_liveness_tick -> github-devloop.liveness_scan",
      "github-devloop.devloop_observe_issue -> github-devloop.observe_issue",
    })

    local tick = graph.require_raise(trace, "github-devloop.devloop_liveness_tick")
    t.eq(tick.payload.reason, "rollup-merged")
    t.eq(tick.payload.repo, repo)
    t.eq(tick.payload.source_ref.ref, repo .. "#pr/" .. tostring(rollup_pr_number))
    t.eq(tick.payload.state, nil)

    local observe = graph.require_raise(trace, "github-devloop.devloop_observe_issue")
    t.eq(observe.payload.repo, repo)
    t.eq(tonumber(observe.payload.number), issue_number)
    t.eq(observe.payload.source, "liveness-scan")
    local resume = graph.require_raise(trace, "github-proxy.github_issue_comment_request")
    t.is_true(resume.payload.body:find('state="merged"', 1, true) ~= nil)
    local label = graph.require_raise(trace, "github-proxy.github_issue_label_request")
    t.eq(label.payload.add_labels[1], "fkst-dev:merged")
    t.eq(h.count_calls("gh pr merge"), 1)
    t.eq(h.count_calls("gh issue close " .. tostring(issue_number) .. " --repo " .. repo), 1)
  end,

  test_rollup_merge_wake_does_not_close_parent_when_child_pr_is_nonterminal = function()
    mock_everything("merge-ready")

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 10 }))
    graph.assert_covers(trace, {
      "github-devloop-integration.devloop_rollup_ready -> github-devloop-integration.rollup_merge",
      "github-devloop.devloop_liveness_tick -> github-devloop.liveness_scan",
      "github-devloop.devloop_observe_issue -> github-devloop.observe_issue",
    })

    local tick = graph.require_raise(trace, "github-devloop.devloop_liveness_tick")
    t.eq(tick.payload.reason, "rollup-merged")

    local observe = graph.require_raise(trace, "github-devloop.devloop_observe_issue")
    t.eq(observe.payload.source, "liveness-scan")
    t.eq(tonumber(observe.payload.number), issue_number)
    local merged_comment = graph.find_raise(trace, "github-proxy.github_issue_comment_request", function(raised)
      return tostring(raised.payload and raised.payload.body or ""):find('state="merged"', 1, true) ~= nil
    end)
    local merged_label = graph.find_raise(trace, "github-proxy.github_issue_label_request", function(raised)
      local add = raised.payload and raised.payload.add_labels or {}
      return add[1] == "fkst-dev:merged"
    end)
    t.eq(merged_comment, nil)
    t.eq(merged_label, nil)
    t.eq(h.count_calls("gh issue close " .. tostring(issue_number) .. " --repo " .. repo), 0)
  end,

  test_rollup_merge_wake_does_not_close_parent_before_child_merge_commit_lands = function()
    mock_everything("merged", false)

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 10 }))
    graph.assert_covers(trace, {
      "github-devloop-integration.devloop_rollup_ready -> github-devloop-integration.rollup_merge",
      "github-devloop.devloop_liveness_tick -> github-devloop.liveness_scan",
      "github-devloop.devloop_observe_issue -> github-devloop.observe_issue",
    })

    local merged_comment = graph.find_raise(trace, "github-proxy.github_issue_comment_request", function(raised)
      return tostring(raised.payload and raised.payload.body or ""):find('state="merged"', 1, true) ~= nil
    end)
    local merged_label = graph.find_raise(trace, "github-proxy.github_issue_label_request", function(raised)
      local add = raised.payload and raised.payload.add_labels or {}
      return add[1] == "fkst-dev:merged"
    end)
    t.eq(merged_comment, nil)
    t.eq(merged_label, nil)
    t.eq(h.count_calls("git merge-base --is-ancestor " .. child_merge_commit_sha .. " " .. rollup_head_sha), 1)
    t.eq(h.count_calls("git merge-base --is-ancestor " .. child_head_sha .. " " .. rollup_head_sha), 0)
    t.eq(h.count_calls("gh issue close " .. tostring(issue_number) .. " --repo " .. repo), 0)
  end,
}
