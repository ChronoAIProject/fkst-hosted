local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local v_reviewing = require("devloop.validators.reviewing")
local v_fixing = require("devloop.validators.fixing")
local v_merge_ready = require("devloop.validators.merge_ready")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local json_string = h.json_string

local function run_handoff(payload, name)
  return t.run_department("departments/comment_handoff/main.lua", {
    queue = "github-proxy.github_comment_written",
    payload = payload,
  }, opts(name))
end

local function mock_marker_comment(comment_id, body, author_login)
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. tostring(comment_id) .. "'", {
    stdout = '{"body":"' .. json_string(body or "") .. '","user":{"login":"' .. tostring(author_login or "fkst-test-bot") .. '"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_comment_written_pr_open_ack_redrives_pr_observer = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_marker_comment("IC_pr_open_1", core.state_marker(proposal_id, "pr-open", version))

    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_pr_open_1",
      request_dedup_key = "pr-delegation/pr-open/github-devloop/issue/owner/repo/42/g1",
      dedup_key = "pr-delegation/pr-open/github-devloop/issue/owner/repo/42/g1/written/IC_pr_open_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.pr_open",
        proposal_id = proposal_id,
        pr_number = 7,
        version = version,
        source_ref = source_ref,
      },
    }, "comment-handoff-pr-open")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local observe = find_raise(result.raises, "devloop_observe_pr").payload
    t.eq(observe.schema, "github-proxy.v1")
    t.eq(observe.type, "pr")
    t.eq(observe.repo, "owner/repo")
    t.eq(observe.number, 7)
    t.eq(observe.source_ref.kind, source_ref.kind)
    t.eq(observe.source_ref.ref, source_ref.ref)
  end,

  test_comment_written_reviewing_ack_raises_durable_reviewing_with_verifiable_hand_off = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
    mock_marker_comment("IC_reviewing_1", core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", version))
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_reviewing_1",
      request_dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/" .. version .. "/7",
      dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/" .. version .. "/7/written/IC_reviewing_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.reviewing",
        proposal_id = "github-devloop/issue/owner/repo/42",
        pr_number = 7,
        version = version,
        source_ref = source_ref,
      },
    }, "comment-handoff-reviewing")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local reviewing = find_raise(result.raises, "devloop_reviewing").payload
    t.eq(reviewing.schema, "github-devloop.reviewing.v1")
    t.eq(reviewing.reviewing_hand_off.comment_id, "IC_reviewing_1")
    t.eq(reviewing.reviewing_hand_off.marker_version, version)
    t.eq(reviewing.reviewing_hand_off.event_version, version)
    t.eq(v_reviewing.is_supported_reviewing(core, reviewing), true)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.eq(label.expected_proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(label.expected_state, "reviewing")
    t.eq(label.expected_version, version)
  end,

  test_comment_written_reviewing_ack_skips_other_owned_issue = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = '{"assignees":[{"login":"human"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_reviewing_other_1",
      request_dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/" .. version .. "/7",
      dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/" .. version .. "/7/written/IC_reviewing_other_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.reviewing",
        proposal_id = "github-devloop/issue/owner/repo/42",
        pr_number = 7,
        version = version,
        source_ref = source_ref,
      },
    }, "comment-handoff-reviewing-other-owned")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_comment_written_reviewing_ack_allows_pr_native_proposal = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local version = "pr-native-version/fix/1"
    local proposal_id = entity_lib.pr_proposal_id("owner/repo", 7)
    mock_marker_comment("IC_pr_native_reviewing_1", core.state_marker(proposal_id, "reviewing", version))
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_pr_native_reviewing_1",
      request_dedup_key = "fix/comment/" .. proposal_id .. "/" .. version .. "/7",
      dedup_key = "fix/comment/" .. proposal_id .. "/" .. version .. "/7/written/IC_pr_native_reviewing_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.reviewing",
        proposal_id = proposal_id,
        pr_number = 7,
        version = version,
        source_ref = source_ref,
      },
    }, "comment-handoff-pr-native-reviewing")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local reviewing = find_raise(result.raises, "devloop_reviewing").payload
    t.eq(reviewing.proposal_id, proposal_id)
    t.eq(reviewing.pr_number, 7)
    t.eq(reviewing.version, version)
    t.eq(reviewing.reviewing_hand_off.comment_id, "IC_pr_native_reviewing_1")
    t.eq(v_reviewing.is_supported_reviewing(core, reviewing), true)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.eq(label.expected_proposal_id, proposal_id)
    t.eq(label.expected_state, "reviewing")
    t.eq(label.expected_version, version)
  end,

  test_comment_written_fixing_ack_raises_byte_equivalent_payload = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/v1/fix/1"
    local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/v1", "def456")
    local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
    local expected_replay = payloads_builders.build_replayed_fixing_payload(core, {
      proposal_id = proposal_id,
      impl_version = version,
    }, 7, {
      review_proposal_id = review_proposal_id,
      review_dedup_key = review_dedup_key,
      reviewed_head_sha = "def456",
      blocking_gap = "missing regression guard",
      gate_baseline_sha = "ba5e1234",
      predecessor_set = "pred-a",
      review_reason = "rollup-red",
    }, source_ref)
    mock_marker_comment("IC_fixing_1", core.state_marker(proposal_id, "fixing", version))
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_fixing_1",
      request_dedup_key = "review-result/comment/" .. proposal_id .. "/reject/" .. review_dedup_key,
      dedup_key = "review-result/comment/" .. proposal_id .. "/reject/written/IC_fixing_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.fixing",
        proposal_id = proposal_id,
        pr_number = 7,
        version = version,
        review_proposal_id = review_proposal_id,
        review_dedup_key = review_dedup_key,
        reviewed_head_sha = "def456",
        blocking_gap = "missing regression guard",
        gate_baseline_sha = "ba5e1234",
        current_head_sha = "def456",
        gate_failure_excerpt = "rollup-red",
        predecessor_set = "pred-a",
        dedup_key = expected_replay.dedup_key,
        source_ref = source_ref,
      },
    }, "comment-handoff-fixing")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local fixing = find_raise(result.raises, "devloop_fixing").payload
    local expected = payloads_builders.build_devloop_fixing_payload(core, {
      proposal_id = proposal_id,
      impl_version = version,
    }, 7, {
      review_proposal_id = review_proposal_id,
      review_dedup_key = review_dedup_key,
      reviewed_head_sha = "def456",
      blocking_gap = "missing regression guard",
      gate_baseline_sha = "ba5e1234",
      current_head_sha = "def456",
      gate_failure_excerpt = "rollup-red",
      predecessor_set = "pred-a",
    }, source_ref)
    t.eq(fixing.schema, expected.schema)
    t.eq(fixing.proposal_id, expected.proposal_id)
    t.eq(fixing.pr_number, expected.pr_number)
    t.eq(fixing.version, expected.version)
    t.eq(fixing.review_proposal_id, expected.review_proposal_id)
    t.eq(fixing.review_dedup_key, expected.review_dedup_key)
    t.eq(fixing.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(fixing.blocking_gap, expected.blocking_gap)
    t.eq(fixing.gate_baseline_sha, expected.gate_baseline_sha)
    t.eq(fixing.gate_failure_excerpt, expected.gate_failure_excerpt)
    t.eq(fixing.predecessor_set, expected.predecessor_set)
    t.eq(fixing.dedup_key, expected_replay.dedup_key)
    t.eq(fixing.source_ref.kind, expected.source_ref.kind)
    t.eq(fixing.source_ref.ref, expected.source_ref.ref)
    t.eq(v_fixing.is_supported_fixing(core, fixing), true)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.eq(label.expected_proposal_id, proposal_id)
    t.eq(label.expected_state, "fixing")
    t.eq(label.expected_version, version)
  end,

  test_comment_written_merge_ready_ack_raises_byte_equivalent_payload = function()
    local source_ref = entity_lib.pr_source_ref("owner/repo", 7)
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456")
    local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
    mock_marker_comment("IC_merge_ready_1", core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", version))
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "IC_merge_ready_1",
      request_dedup_key = "review-result/comment/github-devloop/issue/owner/repo/42/approve/" .. review_dedup_key,
      dedup_key = "review-result/comment/github-devloop/issue/owner/repo/42/approve/written/IC_merge_ready_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.merge_ready",
        proposal_id = "github-devloop/issue/owner/repo/42",
        pr_number = 7,
        version = version,
        review_proposal_id = review_proposal_id,
        review_dedup_key = review_dedup_key,
        reviewed_head_sha = "def456",
        current_head_sha = "def456",
        source_ref = source_ref,
      },
    }, "comment-handoff-merge-ready")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local merge_ready = find_raise(result.raises, "devloop_merge_ready").payload
    local expected = payloads_builders.build_devloop_merge_ready_payload(core, "github-devloop/issue/owner/repo/42", 7, version, {
      review_proposal_id = review_proposal_id,
      review_dedup_key = review_dedup_key,
      reviewed_head_sha = "def456",
      current_head_sha = "def456",
    }, source_ref)
    t.eq(merge_ready.schema, expected.schema)
    t.eq(merge_ready.proposal_id, expected.proposal_id)
    t.eq(merge_ready.pr_number, expected.pr_number)
    t.eq(merge_ready.version, expected.version)
    t.eq(merge_ready.review_proposal_id, expected.review_proposal_id)
    t.eq(merge_ready.review_dedup_key, expected.review_dedup_key)
    t.eq(merge_ready.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(merge_ready.dedup_key, expected.dedup_key)
    t.eq(merge_ready.source_ref.kind, expected.source_ref.kind)
    t.eq(merge_ready.source_ref.ref, expected.source_ref.ref)
    t.eq(v_merge_ready.is_supported_merge_ready(core, merge_ready), true)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.eq(label.expected_proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(label.expected_state, "merge-ready")
    t.eq(label.expected_version, version)
  end,
}
