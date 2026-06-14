local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise

local function run_handoff(payload, name)
  return t.run_department("departments/comment_handoff/main.lua", {
    queue = "github-proxy.github_comment_written",
    payload = payload,
  }, opts(name))
end

return {
  test_comment_written_ready_ack_raises_durable_ready_with_verifiable_hand_off = function()
    local source_ref = core.issue_source_ref("owner/repo", 42)
    local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "issue",
      issue_number = 42,
      comment_id = "IC_ready_1",
      request_dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/written/IC_ready_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.ready",
        proposal_id = "github-devloop/issue/owner/repo/42",
        version = version,
        marker_version = version,
        source_ref = source_ref,
      },
    }, "comment-handoff-ready")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local ready = find_raise(result.raises, "devloop_ready").payload
    t.eq(ready.schema, "github-devloop.ready.v1")
    t.eq(ready.ready_hand_off.comment_id, "IC_ready_1")
    t.eq(ready.ready_hand_off.marker_version, version)
    t.eq(ready.ready_hand_off.event_version, ready.dedup_key)
    t.eq(core.is_supported_ready(ready), true)
  end,

  test_comment_written_ready_ack_preserves_effect_version_marker_identity = function()
    local source_ref = core.issue_source_ref("owner/repo", 42)
    local event_version = "consensus:github-devloop/issue/owner/repo/42/intake/1234567890"
    local marker_version = "intake/github-devloop/issue/owner/repo/42/2026-06-03T02-02-03Z"
    local result = run_handoff({
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "issue",
      issue_number = 42,
      comment_id = "IC_ready_effect_1",
      request_dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/consensus-github-devloop/issue/owner/repo/42/intake/1234567890",
      dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/written/IC_ready_effect_1",
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.ready",
        proposal_id = "github-devloop/issue/owner/repo/42",
        version = event_version,
        marker_version = marker_version,
        source_ref = source_ref,
      },
    }, "comment-handoff-ready-effect-version")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local ready = find_raise(result.raises, "devloop_ready").payload
    t.eq(ready.dedup_key, core.build_devloop_ready_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = marker_version,
      source_ref = source_ref,
    }).dedup_key)
    t.is_true(ready.dedup_key ~= core.build_devloop_ready_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = event_version,
      source_ref = source_ref,
    }).dedup_key)
    t.eq(ready.ready_hand_off.comment_id, "IC_ready_effect_1")
    t.eq(ready.ready_hand_off.marker_version, marker_version)
    t.eq(ready.ready_hand_off.event_version, ready.dedup_key)
    t.eq(core.is_supported_ready(ready), true)
  end,

  test_comment_written_reviewing_ack_raises_durable_reviewing_with_verifiable_hand_off = function()
    local source_ref = core.pr_source_ref("owner/repo", 7)
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
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
    t.eq(#result.raises, 1)
    local reviewing = find_raise(result.raises, "devloop_reviewing").payload
    t.eq(reviewing.schema, "github-devloop.reviewing.v1")
    t.eq(reviewing.reviewing_hand_off.comment_id, "IC_reviewing_1")
    t.eq(reviewing.reviewing_hand_off.marker_version, version)
    t.eq(reviewing.reviewing_hand_off.event_version, version)
    t.eq(core.is_supported_reviewing(reviewing), true)
  end,

  test_comment_written_reviewing_ack_skips_other_owned_issue = function()
    local source_ref = core.pr_source_ref("owner/repo", 7)
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
}
