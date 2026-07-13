local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local convergence_shared = require("devloop.convergence.shared")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local reviewed_version = transition_version.safe_version_segment(h.reviewing().version)
local reviewed_head_sha = "def456"
local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, reviewed_version, reviewed_head_sha)
local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function pr_source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function initial_event(queue, payload)
  return {
    queue = queue,
    payload = payload,
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(pr_number),
    },
  }
end

local function state_marker(state, version)
  return core.state_marker(issue_proposal_id, state, version or reviewed_version)
end

local function pr_origin_marker(version)
  return m_builders.pr_origin_marker(issue_proposal_id, tostring(issue_number), "devloop-owner-repo-42-01HY", version or reviewed_version, "dev")
end

local function mock_env(times)
  for _ = 1, times or 8 do
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
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_origin_read(comments)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    base_branch = "dev",
    head_sha = reviewed_head_sha,
    state = "OPEN",
    comments = comments,
  }, entity_read_mocks.pr_origin_selector)
end

local function seed_pr_and_issue_reads(state, extra_comments)
  local comments = { pr_origin_marker(), state_marker(state or "reviewing") }
  for _, comment in ipairs(extra_comments or {}) do
    table.insert(comments, comment)
  end
  mock_pr_origin_read(comments)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author")
  h.mock_issue_review(
    { "fkst-dev:reviewing" },
    { state_marker(state or "reviewing") },
    { assignees = { "fkst-test-bot" }, author_login = "fkst-test-bot" }
  )
end

local function unresolved_angle_digests()
  return {
    { angle = "minimal", verdict = "comment", digest = "needs another pass" },
  }
end

local function narrowed_question()
  return "Is the PR ready to merge?"
end

local function unresolved_payload()
  return {
    schema = "consensus.consensus_converge.v1",
    proposal_id = review_proposal_id,
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref(),
    round = 1,
    narrowed_question = narrowed_question(),
    angle_digests = unresolved_angle_digests(),
  }
end

local function reached_payload()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = review_proposal_id,
    decision = "approve",
    body = "Review consensus approves the diff.",
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref(),
  }
end

local function refused_reject_payload(proposal_id)
  local selected_proposal_id = proposal_id or review_proposal_id
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = selected_proposal_id,
    decision = "reject",
    body = "Review consensus rejects the diff without an actionable gap.",
    dedup_key = "consensus:" .. devloop_base.pr_review_proposal_dedup_key(selected_proposal_id),
    source_ref = pr_source_ref(),
  }
end

local function mock_consensus_approval()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-pr-lifecycle-replay/runtime",
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

local function mock_reviewing_liveness_replay(version)
  local comments = {
    {
      body = pr_origin_marker(version),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:00:00Z",
    },
    {
      body = state_marker("reviewing", version),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:00:00Z",
    },
  }

  mock_env(32)
  for _ = 1, 8 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = '[{"number":7,"state":"open","updated_at":"2026-06-04T01:02:03Z"}]\n',
    stderr = "",
    exit_code = 0,
  })
  h.mock_default_issue_claim(repo, issue_number)
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    head_sha = reviewed_head_sha,
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = { "fkst-dev:reviewing" },
    times = 8,
  })
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    head_sha = reviewed_head_sha,
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = { "fkst-dev:reviewing" },
  }, entity_read_mocks.pr_origin_selector, 8)
  h.mock_issue_review({ "fkst-dev:reviewing" }, {
    state_marker("reviewing", version),
  }, {
    repo = repo,
    number = issue_number,
    title = "Implement decision recorder",
    body = "Issue context",
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  })
  h.mock_context_bundle({
    proposal_id = issue_proposal_id,
    pr_number = pr_number,
    source_ref = pr_source_ref(),
  })
  local implementation_worktree = devloop_base.implement_worktree_path(
    "/tmp/fkst-packages-test/github-devloop/runtime",
    repo,
    issue_number,
    version
  )
  t.mock_command(core.path_is_directory_cmd(implementation_worktree), {
    stdout = "",
    stderr = "",
    exit_code = 1,
  })
  t.mock_command("gh pr diff '7' --repo 'owner/repo' --name-only", {
    stdout = "file.lua\n",
    stderr = "",
    exit_code = 0,
  })
  mock_consensus_approval()
end

local function review_converge_round_marker()
  return conv_rounds.review_converge_round_marker(core,
    review_proposal_id,
    issue_proposal_id,
    reviewed_version,
    reviewed_head_sha,
    convergence_shared.source_ref_digest(pr_source_ref()),
    1,
    review_dedup_key,
    narrowed_question(),
    unresolved_angle_digests()
  )
end

return {
  test_run_graph_pr_consensus_converge_routes_to_review_loop = function()
    mock_env()
    seed_pr_and_issue_reads("reviewing", { review_converge_round_marker() })

    local trace = graph.require_quiescent(graph.run(
      initial_event("consensus.consensus_converge", unresolved_payload()),
      { max_steps = 4 }
    ))
    graph.assert_covers(trace, {
      "consensus.consensus_converge -> github-devloop-pr.review_loop",
    })

    local step = graph.require_delivery(trace, {
      queue = "consensus.consensus_converge",
      consumer = "github-devloop-pr.review_loop",
    })
    t.eq(step.exit_code, 0)
  end,

  test_run_graph_pr_consensus_reached_routes_to_review_result = function()
    mock_env()
    seed_pr_and_issue_reads("merge-ready")
    t.mock_command("gh pr diff '7' --repo 'owner/repo' --name-only", {
      stdout = "file.lua\n",
      stderr = "",
      exit_code = 0,
    })

    local trace = graph.require_quiescent(graph.run(
      initial_event("consensus.consensus_reached", reached_payload()),
      { max_steps = 4 }
    ))
    graph.assert_covers(trace, {
      "consensus.consensus_reached -> github-devloop-pr.review_result",
    })

    local step = graph.require_delivery(trace, {
      queue = "consensus.consensus_reached",
      consumer = "github-devloop-pr.review_result",
    })
    t.eq(step.exit_code, 0)
  end,

  test_run_graph_owned_source_mismatch_fails_loud = function()
    local source_mismatch = refused_reject_payload()
    source_mismatch.blocking_gap = "missing regression guard"
    source_mismatch.source_ref = entity_lib.pr_source_ref(repo, 8)
    local trace = graph.run(
      initial_event("consensus.consensus_reached", source_mismatch),
      { max_steps = 4 }
    )
    local step = graph.require_delivery(trace, {
      queue = "consensus.consensus_reached",
      consumer = "github-devloop-pr.review_result",
    })
    t.is_true(step.exit_code ~= 0)
    t.is_true(tostring(step.error):find("review-result-invalid", 1, true) ~= nil)
  end,

  test_run_graph_owned_gapless_reject_recovers_through_liveness_replay = function()
    local replay_version = core.next_review_loop_version(reviewed_version)
    local replay_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, replay_version, reviewed_head_sha)
    local refused_trace = graph.run(
      initial_event("consensus.consensus_reached", refused_reject_payload(replay_proposal_id)),
      { max_steps = 4 }
    )
    local refused_step = graph.require_delivery(refused_trace, {
      queue = "consensus.consensus_reached",
      consumer = "github-devloop-pr.review_result",
    })
    t.is_true(refused_step.exit_code ~= 0)
    t.is_true(tostring(refused_step.error):find("review-result-invalid", 1, true) ~= nil)
    t.is_nil(graph.find_raise(refused_trace, "devloop_fix_reconcile"))
    t.is_nil(graph.find_raise(refused_trace, "github-devloop-decompose.devloop_decompose"))

    mock_reviewing_liveness_replay(replay_version)
    local liveness_tick = initial_event("devloop_liveness_tick", {
      schema = "github-devloop.tick.v1",
    })
    -- run_graph raisers use ts=0; a distinct poll generation prevents cross-test list-cache reuse.
    liveness_tick.ts = 1
    local replay_trace = graph.require_quiescent(graph.run(liveness_tick, { max_steps = 12 }))
    graph.assert_covers(replay_trace, {
      "github-devloop-pr.devloop_liveness_tick -> github-devloop-pr.liveness_scan",
      "github-devloop-pr.devloop_reviewing -> github-devloop-pr.review_pr",
      "consensus.proposal -> consensus.decide",
    })

    local redrive = graph.require_raise(replay_trace, "github-devloop-pr.devloop_reviewing")
    t.eq(redrive.payload.review_delivery_dedup_key, redrive.payload.dedup_key)
    t.eq(
      devloop_base.pr_review_proposal_id_from_redrive_delivery_dedup_key(redrive.payload.dedup_key),
      replay_proposal_id
    )
    t.is_true(redrive.payload.dedup_key ~= devloop_base.pr_review_proposal_dedup_key(replay_proposal_id))

    local proposal = graph.require_raise(replay_trace, "consensus.proposal")
    t.eq(proposal.payload.proposal_id, replay_proposal_id)
    t.eq(proposal.payload.dedup_key, redrive.payload.dedup_key)
    local decide_step = graph.require_delivery(replay_trace, {
      queue = "consensus.proposal",
      consumer = "consensus.decide",
    })
    t.eq(decide_step.exit_code, 0)
    t.is_nil(graph.find_raise(replay_trace, "devloop_timeout_reconcile"))
    t.is_nil(graph.find_raise(replay_trace, "github-devloop-decompose.devloop_decompose"))
  end,
}
