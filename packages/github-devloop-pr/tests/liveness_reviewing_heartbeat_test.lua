local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local convergence_shared = require("devloop.convergence.shared")
local h = require("tests.devloop_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local conv_attempts = require("devloop.convergence.attempts")
local t = h.t
local core = h.core
local opts = h.opts
local replay_fields = require("devloop.replay_fields")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function restart_transition_row(state_name)
  return replay_fields.restart_transition_row(core.restart_transition_table(), state_name)
end

local function encode_json_string(value)
  return h.encode_json_string(value)
end

local function run_liveness_scan(name)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = { schema = "github-devloop.tick.v1" },
    ts = "2026-06-03T01:32:03Z",
  }, opts(name or "liveness-reviewing-heartbeat"))
end

local function mock_branch_config_env()
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

local function run_observe_pr(name)
  mock_branch_config_env()
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = 7,
      state = "open",
      updated_at = "2026-06-04T01:02:03Z",
      dedup_key = "liveness-scan/owner/repo/pr/7",
      source = "liveness-scan",
      source_ref = entity_lib.pr_source_ref(repo, 7),
    },
  }, opts(name or "observe-pr-reviewing-heartbeat"))
end

local function mock_repo()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_list()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_list()
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

local function mock_pr_state(comments)
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    times = 1,
  })
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = {},
  }, entity_read_mocks.pr_origin_selector)
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T00:00:00Z",
  }
end

local function state_comment(state_name, state_version, created_at)
  return trusted_comment(core.state_marker(proposal_id, state_name, state_version), created_at)
end

local function timeout_attempt_comment(state_version, round)
  return trusted_comment(conv_attempts.timeout_attempt_marker(core, proposal_id, state_version, "reviewing", round, entity_lib.pr_source_ref(repo, 7)))
end

local function review_round_comment(created_at)
  local source_ref = entity_lib.pr_source_ref(repo, 7)
  local review_proposal_id = devloop_base.pr_review_proposal_id(repo, 7, version, "def456")
  return trusted_comment(conv_rounds.review_converge_round_marker(core,
    review_proposal_id,
    proposal_id,
    version,
    "def456",
    convergence_shared.source_ref_digest(source_ref),
    1,
    "consensus:" .. review_proposal_id .. "/review/loop/1",
    "Still reviewing",
    { { angle = "minimal", verdict = "continue", digest = "recent" } }
  ), created_at)
end

local function run_with_pr_comments(name, comments)
  mock_repo()
  mock_issue_list()
  mock_pr_list()
  mock_issue_claim()
  mock_pr_state(comments)
  return run_liveness_scan(name)
end

local function run_observe_with_pr_comments(name, comments)
  mock_issue_claim()
  mock_pr_state(comments)
  return run_observe_pr(name)
end

local function find_raise(result, queue)
  return h.find_raise(result.raises, queue)
end

local function find_pr_comment_with(result, needle)
  return h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload and payload.body or ""):find(needle, 1, true) ~= nil
  end)
end

return {
  test_liveness_scan_reviewing_recent_pr_converge_round_does_not_timeout_count = function()
    local result = run_with_pr_comments("liveness-scan-reviewing-pr-heartbeat-live", {
      m_builders.pr_origin_marker(core, proposal_id, "42", "devloop-owner-repo-42-01HY", version, "dev"),
      state_comment("reviewing", version, "2026-06-03T00:00:00Z"),
      review_round_comment(os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 60)),
    })
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result, "devloop_timeout_reconcile"), nil)
    t.eq(find_raise(result, "devloop_reviewing"), nil)
    t.eq(find_pr_comment_with(result, "fkst:github-devloop:timeout-attempt:v1"), nil)
  end,

  test_liveness_scan_reviewing_stale_past_budget_pr_converge_round_climbs_to_blocked = function()
    local timeout_version = version .. "/timeout/reviewing/3"
    local result = run_with_pr_comments("liveness-scan-reviewing-pr-heartbeat-stale", {
      m_builders.pr_origin_marker(core, proposal_id, "42", "devloop-owner-repo-42-01HY", version, "dev"),
      state_comment("reviewing", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment(version, 1),
      timeout_attempt_comment(version, 2),
      review_round_comment("2026-06-03T00:00:00Z"),
    })
    t.eq(result.exit_code, 0)
    local reconcile = find_raise(result, "devloop_timeout_reconcile")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.state, "reviewing")
    t.eq(reconcile.payload.issue_version, timeout_version)
    t.eq(reconcile.payload.round, 3)
    t.eq(reconcile.payload.source_ref.ref, "owner/repo#pr/7")
  end,

  test_liveness_scan_reviewing_issue_side_heartbeat_is_not_read_as_pr_live = function()
    local row = restart_transition_row("reviewing")
    local source_ref = entity_lib.pr_source_ref(repo, 7)
    local review_proposal_id = devloop_base.pr_review_proposal_id(repo, 7, version, "def456")
    local signal = core.restart_row_liveness_signal(row, {
      state = "reviewing",
      version = version,
      proposal_id = proposal_id,
      marker_created_at = "2026-06-03T00:00:00Z",
    }, {
      proposal_id = proposal_id,
      source_ref = source_ref,
      review_proposal_id = review_proposal_id,
      head_sha = "def456",
      current = {
        comments = {
          review_round_comment(os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 60)),
        },
      },
      current_pr = {
        comments = {},
      },
      now_seconds = now(),
    }, now())
    t.eq(signal.live, false)
  end,

  test_observe_pr_reviewing_recent_pr_converge_round_does_not_escalate_timeout = function()
    local timeout_version = version .. "/timeout/reviewing/2"
    local result = run_observe_with_pr_comments("observe-pr-reviewing-pr-heartbeat-live", {
      m_builders.pr_origin_marker(core, proposal_id, "42", "devloop-owner-repo-42-01HY", version, "dev"),
      state_comment("reviewing", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment(version, 1),
      timeout_attempt_comment(version, 2),
      review_round_comment(os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 60)),
    })
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result, "devloop_timeout_reconcile"), nil)
    t.eq(find_raise(result, "devloop_reviewing"), nil)
    t.eq(find_pr_comment_with(result, "fkst:github-devloop:timeout-attempt:v1"), nil)
  end,

  test_observe_pr_reviewing_stale_past_budget_pr_converge_round_escalates_timeout = function()
    local timeout_version = version .. "/timeout/reviewing/2"
    local result = run_observe_with_pr_comments("observe-pr-reviewing-pr-heartbeat-stale", {
      m_builders.pr_origin_marker(core, proposal_id, "42", "devloop-owner-repo-42-01HY", version, "dev"),
      state_comment("reviewing", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment(version, 1),
      timeout_attempt_comment(version, 2),
      review_round_comment("2026-06-03T00:00:00Z"),
    })
    t.eq(result.exit_code, 0)
    local reconcile = find_raise(result, "devloop_timeout_reconcile")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.state, "reviewing")
    t.eq(reconcile.payload.issue_version, timeout_version)
    t.eq(reconcile.payload.round, 3)
    t.eq(reconcile.payload.source_ref.ref, "owner/repo#pr/7")
  end,
}
