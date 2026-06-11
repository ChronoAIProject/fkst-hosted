local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local action_label = h.action_label
local reason_label = h.reason_label
local opts = h.opts
local reviewing = h.reviewing
local review_unresolved = h.review_unresolved
local review_meta_event = h.review_meta_event
local run_observe_pr = h.run_observe_pr
local run_review_loop = h.run_review_loop
local run_review_meta = h.run_review_meta
local mock_issue_review = h.mock_issue_review
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review_meta = h.mock_issue_review_meta
local mock_bot_env = h.mock_bot_env
local mock_pr_origin = h.mock_pr_origin
local find_raise = h.find_raise

local function pr_event(updated_at)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@" .. tostring(updated_at or "2026-06-04T01:02:06Z"),
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function mock_review_loop_state(impl_version)
  local origin_marker = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev")
  mock_bot_env()
  mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")
  mock_issue_review({ "fkst-dev:reviewing" }, {
    core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
  })
end

return {
  test_review_loop_mixed_comment_abstain_converges_before_meta = function()
    local event = review_unresolved({
      round = 1,
      narrowed_question = "Which review finding should narrow?",
      angle_digests = {
        { angle = "minimal", verdict = "comment", digest = "needs narrower test" },
        { angle = "delete", verdict = "abstain", digest = "unclear scope" },
      },
    })
    local impl_version = reviewing().version
    mock_review_loop_state(impl_version)

    local first = run_review_loop(event, opts("review-v2-mixed-first-pass"))
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)
    t.is_true(find_raise(first.raises, "consensus.proposal") ~= nil)
    t.eq(find_raise(first.raises, "devloop_review_meta"), nil)

    local loop_event = review_unresolved({
      dedup_key = event.dedup_key .. "/loop/2",
      round = 2,
      narrowed_question = event.narrowed_question,
      angle_digests = event.angle_digests,
    })
    mock_review_loop_state(impl_version)

    local second = run_review_loop(loop_event, opts("review-v2-mixed-bounded-pass-meta"))
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 3)
    t.eq(find_raise(second.raises, "consensus.proposal"), nil)
    t.is_true(find_raise(second.raises, "devloop_review_meta") ~= nil)
  end,

  test_review_loop_abstain_approve_boundary_converges = function()
    local event = review_unresolved({
      round = 1,
      narrowed_question = "Does the approval resolve the concern?",
      angle_digests = {
        { angle = "minimal", verdict = "approve", digest = "acceptable" },
        { angle = "delete", verdict = "abstain", digest = "unclear" },
      },
    })
    mock_review_loop_state(reviewing().version)

    local result = run_review_loop(event, opts("review-v2-abstain-approve-boundary"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.is_true(find_raise(result.raises, "consensus.proposal") ~= nil)
    t.eq(find_raise(result.raises, "devloop_review_meta"), nil)
  end,

  test_review_meta_fix_without_gap_blocks_fail_closed = function()
    local event = review_meta_event()
    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    h.mock_context_bundle()
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("mkdir -p", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("codex exec", {
      stdout = action_label .. " fix\n" .. reason_label .. " Run another fix pass.",
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_meta(event, opts("review-v2-meta-fix-missing-gap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:blocked")
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
  end,

  test_review_meta_spec_amendment_extra_line_blocks_fail_closed = function()
    local event = review_meta_event()
    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    h.mock_context_bundle()
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("codex exec", {
      stdout = action_label .. " spec-amendment\n" .. reason_label .. " The agreed framing is defective.\ngarbage",
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_meta(event, opts("review-v2-meta-spec-extra-line"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:blocked")
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,

  test_observe_pr_fixing_self_heal_recovers_structured_gap = function()
    local impl_version = reviewing().version
    local fix_version = core.next_fix_version(impl_version)
    local review_id = core.pr_review_proposal_id("owner/repo", 7, impl_version, "def456")
    local review_dedup_key = "consensus:" .. review_id .. "/review"
    local expected = core.build_devloop_fixing_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      impl_version = fix_version,
    }, 7, {
      review_proposal_id = review_id,
      review_dedup_key = review_dedup_key,
      reviewed_head_sha = "def456",
      blocking_gap = "missing retry guard",
    }, {
      kind = "external",
      ref = "owner/repo#pr/7",
    })
    mock_issue_reviewing({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
      core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "reject", review_dedup_key, 1, "missing retry guard"),
    })

    local result = run_observe_pr(pr_event(), opts("review-v2-observe-pr-gap-self-heal"))
    t.eq(result.exit_code, 0)
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.is_true(fixing_raise ~= nil)
    t.eq(fixing_raise.payload.dedup_key, expected.dedup_key)
    t.eq(fixing_raise.payload.blocking_gap, "missing retry guard")
  end,

  test_observe_pr_fixing_self_heal_dedup_matches_original_reject_transition = function()
    local review = h.review_reached({
      decision = "reject",
      blocking_gap = "missing retry guard",
    })
    local impl_version = reviewing().version
    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local original = h.run_review_result(review, opts("review-v2-fixing-original-transition"))
    t.eq(original.exit_code, 0)
    local original_fixing = find_raise(original.raises, "devloop_fixing")
    t.is_true(original_fixing ~= nil)

    mock_issue_reviewing({ "fkst-dev:fixing" }, {
      find_raise(original.raises, "github-proxy.github_pr_comment_request").payload.body,
    })
    local healed = run_observe_pr(pr_event(), opts("review-v2-fixing-self-heal-dedup"))
    t.eq(healed.exit_code, 0)
    local healed_fixing = find_raise(healed.raises, "devloop_fixing")
    t.is_true(healed_fixing ~= nil)
    t.eq(healed_fixing.payload.dedup_key, original_fixing.payload.dedup_key)
    t.eq(healed_fixing.payload.review_dedup_key, original_fixing.payload.review_dedup_key)
  end,

  test_observe_pr_fixing_self_heal_fails_closed_without_reject_fact = function()
    local impl_version = reviewing().version
    local fix_version = core.next_fix_version(impl_version)
    mock_issue_reviewing({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
    })

    local result = run_observe_pr(pr_event(), opts("review-v2-fixing-self-heal-no-reject"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
  end,

  test_observe_pr_fixing_self_heal_fails_closed_when_head_advanced = function()
    local impl_version = reviewing().version
    local fix_version = core.next_fix_version(impl_version)
    local review_id = core.pr_review_proposal_id("owner/repo", 7, impl_version, "def456")
    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
      core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. review_id .. "/review", 1, "missing retry guard"),
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("review-v2-fixing-self-heal-head-advanced"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
  end,

  test_observe_pr_fixing_self_heal_fails_closed_when_pr_closed = function()
    local impl_version = reviewing().version
    local fix_version = core.next_fix_version(impl_version)
    local review_id = core.pr_review_proposal_id("owner/repo", 7, impl_version, "def456")
    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
      core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. review_id .. "/review", 1, "missing retry guard"),
    }, "devloop-owner-repo-42-01HY", "def456", "CLOSED")

    local result = run_observe_pr(pr_event(), opts("review-v2-fixing-self-heal-pr-closed"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
  end,
}
