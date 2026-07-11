local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core
local repo = "owner/repo"
local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local pr_number = 7
local branch = "devloop-owner-repo-42-01HY"
local h1 = "def456"
local h2 = "feedface"

local function origin_marker(version)
  return m_builders.pr_origin_marker(issue_proposal_id, "42", branch, version, "dev")
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
  }
end

local function reviewing_comments(version)
  return {
    origin_marker(version),
    trusted_comment(core.state_marker(issue_proposal_id, "reviewing", version)),
  }
end

local function mock_repo()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_config()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_open_pr_list()
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = '[{"number":7,"state":"open","updated_at":"2026-06-04T01:02:03Z"}]\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_claim()
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 42,
    title = "Issue 42",
    body = "",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    labels = { "fkst-dev:enabled", "fkst-dev:reviewing" },
    comments = {},
    assignees = { "fkst-test-bot" },
    register_all_views = true,
    times = 1,
  })
end

local function mock_pr_state(comments, head_sha)
  local fields = {
    repo = repo,
    number = pr_number,
    head = branch,
    head_sha = head_sha,
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = { "fkst-dev:reviewing" },
    times = 1,
  }
  entity_read_mocks.mock_pr_read_forms(t, fields)
end

local function run_liveness_scan(name)
  return h.run_department("departments/liveness_scan/main.lua", {
    queue = "github-devloop-pr.devloop_liveness_tick",
    payload = { schema = "github-devloop.tick.v1" },
    ts = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
    now_seconds = now(),
  }, h.opts(name))
end

local function run_observe_pr(payload, name)
  mock_branch_config()
  return h.run_department("departments/observe_pr/main.lua", {
    queue = "github-devloop-pr.devloop_observe_pr",
    payload = payload,
    now_seconds = now(),
  }, h.opts(name))
end

return {
  test_stale_review_result_self_heals_through_liveness_to_current_head_decision = function()
    local initial_version = h.reviewing().version
    local stale_result = h.review_reached()
    local comments = reviewing_comments(initial_version)
    t.eq(stale_result.proposal_id, devloop_base.pr_review_proposal_id(repo, pr_number, initial_version, h1))

    h.mock_pr_origin(comments, branch, h2)
    local stale_drop = h.run_review_result(stale_result, h.opts("stale-head-review-result"))
    t.eq(stale_drop.exit_code, 0)
    t.eq(#stale_drop.raises, 0)

    mock_repo()
    mock_open_pr_list()
    mock_issue_claim()
    mock_pr_state(comments, h2)
    local scanned = run_liveness_scan("stale-head-liveness-scan")
    t.eq(scanned.exit_code, 0)
    local observe_raise = h.find_raise(scanned.raises, "devloop_observe_pr")
    t.is_true(observe_raise ~= nil)
    t.eq(observe_raise.payload.source, "liveness-scan")
    t.eq(observe_raise.payload.source_ref.ref, "owner/repo#pr/7")

    mock_issue_claim()
    mock_pr_state(comments, h2)
    local observed = run_observe_pr(observe_raise.payload, "stale-head-observe-current-pr")
    t.eq(observed.exit_code, 0)
    local reviewing_request = h.find_raise(observed.raises, "github-proxy.github_pr_comment_request")
    t.is_true(reviewing_request ~= nil)
    t.eq(reviewing_request.payload.handoff.kind, "github-devloop.reviewing")
    local fresh_version = reviewing_request.payload.handoff.version
    t.eq(fresh_version, core.next_review_loop_version(initial_version))

    local handoff = h.run_comment_handoff_from_request(
      reviewing_request.payload,
      "IC_stale_head_rereview_1",
      "stale-head-reviewing-comment-handoff"
    )
    t.eq(handoff.exit_code, 0)
    local fresh_reviewing = h.find_raise(handoff.raises, "devloop_reviewing")
    t.is_true(fresh_reviewing ~= nil)
    t.eq(fresh_reviewing.payload.version, fresh_version)

    h.mock_issue_review({ "fkst-dev:reviewing" }, {
      reviewing_request.payload.body,
    }, {
      title = "Implement decision recorder",
      body = "Issue context",
    })
    h.mock_pr_origin({ origin_marker(initial_version) }, branch, h2)
    local review = h.run_review_pr(fresh_reviewing.payload, h.opts("stale-head-fresh-review"))
    t.eq(review.exit_code, 0)
    local proposal = h.find_raise(review.raises, "consensus.proposal").payload
    local expected_review_id = devloop_base.pr_review_proposal_id(repo, pr_number, fresh_version, h2)
    t.eq(proposal.proposal_id, expected_review_id)
    t.is_true(proposal.body:find("Reviewed PR head: " .. h2, 1, true) ~= nil)

    local fresh_result = {
      schema = "consensus.consensus_reached.v1",
      proposal_id = proposal.proposal_id,
      decision = "approve",
      body = "Review consensus approves the current diff.",
      dedup_key = "consensus:" .. proposal.dedup_key,
      source_ref = entity_lib.pr_source_ref(repo, pr_number),
    }
    h.mock_pr_origin({ origin_marker(initial_version) }, branch, h2)
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      reviewing_request.payload.body,
    })
    local applied = h.run_review_result(fresh_result, h.opts("stale-head-fresh-review-result"))
    t.eq(applied.exit_code, 0)
    local decision_request = h.find_raise(applied.raises, "github-proxy.github_pr_comment_request")
    t.is_true(decision_request ~= nil)
    t.is_true(decision_request.payload.body:find('state="merge-ready"', 1, true) ~= nil)
    t.is_true(decision_request.payload.body:find('proposal="' .. expected_review_id .. '"', 1, true) ~= nil)

    local current = core.current_state({
      reviewing_request.payload.body,
      decision_request.payload.body,
    }, issue_proposal_id)
    t.eq(current.state, "merge-ready")
    t.eq(current.version, fresh_version)
    t.eq(h.find_causal_raise(applied, "devloop_fixing"), nil)
  end,
}
