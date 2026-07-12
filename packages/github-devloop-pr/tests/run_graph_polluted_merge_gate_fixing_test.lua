local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local t = h.t
local core = h.core

local repo = "owner/repo"
local pr_number = 7
local comment_id = "123456"

local function polluted_merge_gate_comments(event)
  local clean_body = "github-devloop merge gate failed: mergeable-conflicting\n"
    .. m_builders.merge_gate_marker(event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      "mergeable-conflicting"
    )
  local malformed_body = "github-devloop merge gate failed: mergeable-conflicting\n"
    .. '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. event.proposal_id
    .. '" pr="' .. tostring(event.pr_number)
    .. '" version="' .. event.version
    .. '" review_proposal="' .. event.review_proposal_id
    .. '" review_dedup="' .. event.review_dedup_key
    .. '" head_sha="' .. event.reviewed_head_sha
    .. '" gate_baseline_sha="828df8d3" reason="' .. clean_body .. '" -->'
  return {
    {
      body = clean_body,
      author_login = "fkst-test-bot",
      created_at = "2026-07-11T21:14:00Z",
    },
    {
      body = malformed_body,
      author_login = "fkst-test-bot",
      created_at = "2026-07-11T21:19:00Z",
    },
  }
end

local function mock_fix_execution(event, comments)
  local branch = devloop_base.implement_branch(repo, "42", event.version)
  local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
  local issue_comments = {
    core.state_marker(event.proposal_id, "fixing", event.version),
    comments[1],
    comments[2],
  }

  h.mock_context_bundle(event)
  h.mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, issue_comments, branch, event.version)
  h.mock_pr_fix({ origin_marker }, branch, event.reviewed_head_sha)
  h.mock_existing_fix_worktree(branch, event.reviewed_head_sha, nil, {
    sha = event.gate_baseline_sha,
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  h.mock_implement_codex(0, "fixed polluted merge-gate replay")
  h.mock_git_status(" M packages/github-devloop-pr/core.lua\n")
  h.mock_git_commit("feedface", branch)
  h.mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, issue_comments, branch, event.version)
  h.mock_git_push(branch)
  h.mock_pr_fix({ origin_marker }, branch, "feedface")
end

local function mock_runtime_and_context()
  for _ = 1, 32 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 16 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 16 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 32 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_replay_entry(event, comments)
  local branch = devloop_base.implement_branch(repo, "42", event.version)
  local pr_comments = {
    m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev"),
    core.state_marker(event.proposal_id, "fixing", event.version),
    comments[1],
    comments[2],
  }
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = branch,
    head_sha = event.reviewed_head_sha,
    base_branch = "dev",
    comments = pr_comments,
    labels = { "fkst-dev:fixing" },
    state = "OPEN",
  }, entity_read_mocks.pr_origin_selector)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = 42,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author", 4)
end

local function mock_comment_handoff(event, comments)
  local visible_comments = '[[{"id":1,"body":"'
    .. h.json_string(core.state_marker(event.proposal_id, "fixing", event.version))
    .. '","user":{"login":"fkst-test-bot"}},{"id":2,"body":"'
    .. h.json_string(comments[1].body)
    .. '","user":{"login":"fkst-test-bot"}},{"id":3,"body":"'
    .. h.json_string(comments[2].body)
    .. '","user":{"login":"fkst-test-bot"}}]]\n'
  for _, command in ipairs({
    "gh api --paginate --slurp repos/owner/repo/issues/7/comments?per_page=100",
    "gh api --paginate --slurp 'repos/owner/repo/issues/7/comments?per_page=100'",
  }) do
    for _ = 1, 4 do
      t.mock_command(command, {
        stdout = visible_comments,
        stderr = "",
        exit_code = 0,
      })
    end
  end
  t.mock_command("gh api --method POST repos/owner/repo/issues/7/comments --field 'body=", {
    stdout = '{"id":' .. comment_id .. ',"body":"created","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. comment_id .. "'", {
    stdout = '{"body":"' .. h.json_string(core.state_marker(event.proposal_id, "fixing", event.version))
      .. '","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_label_write()
  for _ = 1, 3 do
    t.mock_command("gh label list", {
      stdout = '[{"name":"fkst-dev:fixing"},{"name":"fkst-dev:reviewing"}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh pr edit", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function initial_event()
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = pr_number,
      title = "Polluted merge-gate replay",
      updated_at = "2026-07-11T21:19:00Z",
      dedup_key = "owner/repo#pr/7@2026-07-11T21:19:00Z",
      source_ref = entity_lib.pr_source_ref(repo, pr_number),
    },
    source_ref = {
      kind = "external",
      reference = "owner/repo#pr/7",
    },
  }
end

return {
  test_run_graph_polluted_merge_gate_stream_reaches_fix = function()
    local expected = h.fixing({
      gate_baseline_sha = "281c4f9e",
      gate_failure_excerpt = "mergeable-conflicting",
    })
    local comments = polluted_merge_gate_comments(expected)
    mock_runtime_and_context()
    mock_replay_entry(expected, comments)
    mock_comment_handoff(expected, comments)
    mock_label_write()
    mock_fix_execution(expected, comments)

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 12 }))
    graph.assert_covers(trace, {
      "github-proxy.github_entity_changed -> github-devloop-pr.observe_pr",
      "github-proxy.github_pr_comment_request -> github-proxy.github_pr_comment",
      "github-proxy.github_comment_written -> github-devloop-pr.comment_handoff",
      "github-devloop-pr.devloop_fixing -> github-devloop-pr.fix",
    })
    t.eq(trace.final.dead_letters, 0)
    local replay_request = graph.require_raise(trace, "github-proxy.github_pr_comment_request", function(raised)
      return raised.payload.handoff ~= nil
        and raised.payload.handoff.kind == "github-devloop.fixing"
    end)
    t.eq(replay_request.payload.handoff.gate_baseline_sha, expected.gate_baseline_sha)
    local fixing = graph.require_raise(trace, "github-devloop-pr.devloop_fixing")
    t.eq(fixing.payload.gate_baseline_sha, expected.gate_baseline_sha)
    t.eq(fixing.payload.review_proposal_id, expected.review_proposal_id)
    t.eq(fixing.payload.review_dedup_key, expected.review_dedup_key)
    t.eq(fixing.payload.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(h.count_calls("codex exec"), 1)
    local reviewing_request = graph.require_raise(trace, "github-proxy.github_pr_comment_request", function(raised)
      return raised.payload.handoff ~= nil
        and raised.payload.handoff.kind == "github-devloop.reviewing"
    end)
    t.eq(reviewing_request.payload.handoff.kind, "github-devloop.reviewing")
  end,
}
