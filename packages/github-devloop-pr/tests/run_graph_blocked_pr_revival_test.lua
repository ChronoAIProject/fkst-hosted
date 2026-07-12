local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local operator_commands = require("devloop.operator_commands")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local t = h.t
local core = h.core

local repo = "owner/repo"
local issue_number = 42
local pr_number = 2201
local proposal_id = "github-devloop/issue/owner/repo/42"
local pushed_head_sha = "feedface"
local rereview_comment_id = "123456"
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function source_ref(number)
  return entity_lib.pr_source_ref(repo, number or pr_number)
end

local function entity_changed_event(updated_at, number)
  local selected_pr_number = number or pr_number
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = selected_pr_number,
      updated_at = updated_at,
      dedup_key = repo .. "#pr/" .. tostring(selected_pr_number) .. "@" .. tostring(updated_at),
      source_ref = source_ref(selected_pr_number),
    },
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(selected_pr_number),
    },
  }
end

local function mock_env(times)
  for _ = 1, times or 10 do
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
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_comment_write(number)
  t.mock_command("gh api --method POST repos/owner/repo/issues/" .. tostring(number or pr_number) .. "/comments --field 'body=", {
    stdout = '{"id":' .. rereview_comment_id .. ',"body":"created","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_reviewing_marker_visibility(version)
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. rereview_comment_id .. "'", {
    stdout = '{"body":"' .. h.json_string(core.state_marker(proposal_id, "reviewing", version))
      .. '","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_label_write()
  t.mock_command("gh label list", {
    stdout = '[{"name":"fkst-dev:reviewing"},{"name":"fkst-dev:blocked"}]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr edit", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_consensus_approval()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/blocked-pr-rereview/runtime",
    stderr = "",
    exit_code = 0,
  })
  for _, angle in ipairs({ "teleology", "parsimony", "fidelity", "natural-ownership", "proportional-containment" }) do
    t.mock_command("mkdir -p", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("codex exec", {
      stdout = verdict_label .. " approve\n" .. reply_label .. " " .. angle .. " approves.\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_origin_read(comments, head_sha, labels, number)
  local fields = {
    repo = repo,
    number = number or pr_number,
    head = "devloop-owner-repo-42-01HY",
    head_sha = head_sha,
    base_branch = "dev",
    comments = comments,
    labels = labels or { "fkst-dev:blocked" },
    state = "OPEN",
    updated_at = "2026-06-03T02:00:00Z",
    times = 1,
  }
  entity_read_mocks.mock_pr_read_forms(t, fields)
  entity_read_mocks.mock_pr_view_selector(t, fields, entity_read_mocks.pr_origin_selector)
end

local function reconcile_drop_comments()
  local reconcile = h.review_reconcile()
  local blocked_version = conv_reconcile.review_reconcile_terminal_state_version(
    reconcile.issue_version,
    reconcile.round
  )
  local drop = core.build_review_reconcile_comment_request(
    repo,
    tostring(issue_number),
    reconcile,
    "drop",
    "no-semantic-progress-after-3-review-rounds",
    blocked_version
  ).body
  return {
    m_builders.pr_origin_marker(
      proposal_id,
      tostring(issue_number),
      "devloop-owner-repo-42-01HY",
      reconcile.issue_version,
      "dev"
    ),
    drop,
  }, blocked_version
end

return {
  test_run_graph_reconcile_dropped_blocked_pr_head_push_is_inert = function()
    local blocked_comments = reconcile_drop_comments()

    mock_env()
    h.mock_default_issue_claim()
    mock_pr_origin_read(blocked_comments, pushed_head_sha)

    local head_push_trace = graph.require_quiescent(graph.run(
      entity_changed_event("2026-06-03T02:00:00Z"),
      { max_steps = 3 }
    ))
    graph.assert_covers(head_push_trace, {
      "github-proxy.github_entity_changed -> github-devloop-pr.observe_pr",
    })
    t.eq(graph.find_raise(head_push_trace, "github-proxy.github_pr_comment_request"), nil)
    t.eq(graph.find_raise(head_push_trace, "github-devloop-pr.devloop_reviewing"), nil)
    t.eq(graph.find_raise(head_push_trace, "consensus.proposal"), nil)
  end,

  test_run_graph_reconcile_dropped_blocked_pr_rereview_uses_fresh_head_identity = function()
    local rereview_pr_number = 2202
    local blocked_comments, blocked_version = reconcile_drop_comments()
    local command_comments = { table.unpack(blocked_comments) }
    table.insert(command_comments, "fkst: rereview")
    local expected_version = operator_commands.operator_rereview_version(blocked_version, pushed_head_sha)
    local reviewing_comments = { table.unpack(command_comments) }
    table.insert(reviewing_comments, {
      body = core.state_marker(proposal_id, "reviewing", expected_version),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T02:01:01Z",
    })
    mock_env(20)
    h.mock_default_issue_claim()
    entity_read_mocks.mock_pr_read_forms(t, {
      repo = repo,
      number = rereview_pr_number,
      comments = command_comments,
      head = "devloop-owner-repo-42-01HY",
      head_sha = pushed_head_sha,
      state = "OPEN",
      base_branch = "dev",
      labels = { "fkst-dev:blocked" },
    })
    h.mock_pr_origin_for({
      repo = repo,
      number = rereview_pr_number,
      comments = reviewing_comments,
      head = "devloop-owner-repo-42-01HY",
      head_sha = pushed_head_sha,
      state = "OPEN",
      base_branch = "dev",
      labels = { "fkst-dev:reviewing" },
    })
    h.mock_issue_review({ "fkst-dev:reviewing" }, {}, {
      repo = repo,
      number = issue_number,
      title = "Blocked PR rereview",
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    })
    mock_pr_comment_write(rereview_pr_number)
    mock_reviewing_marker_visibility(expected_version)
    mock_pr_label_write()
    h.mock_context_bundle({
      proposal_id = proposal_id,
      pr_number = rereview_pr_number,
    })
    t.mock_command("gh pr diff " .. tostring(rereview_pr_number) .. " --repo " .. repo .. " --name-only", {
      stdout = "file.lua\n",
      stderr = "",
      exit_code = 0,
    })
    local implementation_worktree = devloop_base.implement_worktree_path(
      "/tmp/fkst-packages-test/github-devloop/runtime",
      repo,
      issue_number,
      h.reviewing().version
    )
    t.mock_command(core.path_is_directory_cmd(implementation_worktree), {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })
    mock_consensus_approval()

    local reentry_trace = graph.require_quiescent(graph.run(
      entity_changed_event("2026-06-03T02:01:00Z", rereview_pr_number),
      { max_steps = 12 }
    ))
    graph.assert_covers(reentry_trace, {
      "github-proxy.github_entity_changed -> github-devloop-pr.observe_pr",
      "github-proxy.github_pr_comment_request -> github-proxy.github_pr_comment",
      "github-proxy.github_comment_written -> github-devloop-pr.comment_handoff",
      "github-devloop-pr.devloop_reviewing -> github-devloop-pr.review_pr",
      "consensus.proposal -> consensus.decide",
    })

    local comment_request = graph.require_raise(reentry_trace, "github-proxy.github_pr_comment_request")
    t.eq(comment_request.payload.handoff.kind, "github-devloop.reviewing")
    t.eq(comment_request.payload.handoff.version, expected_version)
    t.is_true(comment_request.payload.handoff.version ~= blocked_version)

    local reviewing = graph.require_raise(reentry_trace, "github-devloop-pr.devloop_reviewing")
    t.eq(reviewing.payload.version, expected_version)

    local proposal = graph.require_raise(reentry_trace, "consensus.proposal").payload
    t.eq(
      proposal.proposal_id,
      devloop_base.pr_review_proposal_id(repo, rereview_pr_number, expected_version, pushed_head_sha)
    )
    t.is_true(proposal.body:find("Reviewed PR head: " .. pushed_head_sha, 1, true) ~= nil)
    t.eq(proposal.source_ref.ref, repo .. "#pr/" .. tostring(rereview_pr_number))
  end,
}
