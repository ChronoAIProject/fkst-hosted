local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
local opts = h.opts
local run_fix = h.run_fix
local mock_pr_native_fix = h.mock_pr_native_fix
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise
local json_string = h.json_string
local render_comment = h.render_comment

local function pr_native_review_reached(extra)
  local version = "pr-native-version"
  local proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456")
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal_id,
    decision = "reject",
    body = "Review consensus rejects the PR-native diff.",
    blocking_gap = "missing regression guard",
    dedup_key = "consensus:" .. proposal_id .. "/review",
    source_ref = h.pr_source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function pr_native_fixing(extra)
  local event = pr_native_review_reached()
  local proposal_id = entity_lib.pr_proposal_id("owner/repo", 7)
  local value = {
    schema = "github-devloop.fixing.v1",
    proposal_id = proposal_id,
    pr_number = 7,
    version = core.fix_version_from_review_version("pr-native-version"),
    review_proposal_id = event.proposal_id,
    review_dedup_key = event.dedup_key,
    reviewed_head_sha = "def456",
    dedup_key = "fixing/" .. proposal_id .. "/v1",
    source_ref = h.pr_source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_pr_native_fix_write_pushes_and_skips_issue_side_effects = function()
    local event = pr_native_fixing()
    local branch = "pr-native-branch"
    local reject_comment = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      nil,
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because the PR-native parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = h.pr_source_ref(),
      },
      event.source_ref
    ).body
    mock_bot_env()
    mock_write_env("1")
    mock_pr_native_fix({
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    mock_existing_fix_worktree(branch, "def456")
    mock_implement_codex(0, "fixed PR-native review feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_pr_native_fix({
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, "def456")
    mock_git_push(branch)
    mock_pr_native_fix({}, branch, "feedface")

    local result = run_fix(event, opts("fix-pr-native-write", { FKST_GITHUB_WRITE = "1" }))
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    local expected_version = core.next_fix_version(event.version)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(reviewing_raise, nil)
    t.eq(comment_raise.payload.pr_number, 7)
    t.eq(comment_raise.payload.issue_number, nil)
    t.is_true(comment_raise.payload.body:find(m_builders.fix_marker(core, event.proposal_id, event.review_proposal_id, event.review_dedup_key, "def456", "feedface"), 1, true) ~= nil)
    t.eq(core.current_state({ comment_raise.payload.body }, event.proposal_id).state, "reviewing")
    t.eq(core.current_state({ comment_raise.payload.body }, event.proposal_id).version, expected_version)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.reviewing")
    t.eq(comment_raise.payload.handoff.proposal_id, event.proposal_id)
    t.eq(comment_raise.payload.handoff.pr_number, event.pr_number)
    t.eq(comment_raise.payload.handoff.version, expected_version)
    t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/IC_pr_native_fix_reviewing_1'", {
      stdout = '{"body":"' .. json_string(core.state_marker(event.proposal_id, "reviewing", expected_version)) .. '","user":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
    local handoff = t.run_department("departments/comment_handoff/main.lua", {
      queue = "github-proxy.github_comment_written",
      payload = {
        schema = "github-proxy.comment-written.v1",
        repo = comment_raise.payload.repo,
        target = "pr",
        pr_number = comment_raise.payload.pr_number,
        comment_id = "IC_pr_native_fix_reviewing_1",
        request_dedup_key = comment_raise.payload.dedup_key,
        dedup_key = comment_raise.payload.dedup_key .. "/written/IC_pr_native_fix_reviewing_1",
        source_ref = comment_raise.payload.source_ref,
        handoff = comment_raise.payload.handoff,
      },
    }, opts("fix-pr-native-write-comment-handoff"))
    t.eq(handoff.exit_code, 0)
    local handoff_reviewing = find_raise(handoff.raises, "devloop_reviewing")
    t.eq(handoff_reviewing.payload.proposal_id, event.proposal_id)
    t.eq(handoff_reviewing.payload.version, expected_version)
    t.eq(find_raise(handoff.raises, "github-proxy.github_issue_label_request").payload.expected_state, "reviewing")
    t.eq(count_calls("git push origin"), 1)
  end,

  test_fix_pre_spawn_write_gate_skips_stale_pr_head_before_codex = function()
    local event = pr_native_fixing()
    local branch = "pr-native-branch"
    local reject_comment = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      nil,
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because the PR-native parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = h.pr_source_ref(),
      },
      event.source_ref
    ).body
    local comments = {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }

    mock_bot_env()
    mock_write_env("1")
    entity_read_mocks.mock_pr_view_raw_selector(t, {}, entity_read_mocks.pr_fix_precheck_selector, {
      stdout = string.format(
        '{"headRefName":"%s","headRefOid":"feedface","baseRefName":"dev","state":"OPEN","comments":[%s,%s],"headRepository":{"nameWithOwner":"owner/repo"},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false}\n',
        json_string(branch),
        render_comment(comments[1]),
        render_comment(comments[2])
      ),
      stderr = "",
      exit_code = 0,
    })
    mock_pr_native_fix(comments, branch, "def456")

    local result = run_fix(event, opts("fix-stale-before-codex", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git worktree list"), 0)
    t.eq(count_calls("git push origin"), 0)
  end,
}
