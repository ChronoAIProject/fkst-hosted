local graph = require("testkit.graph")
local t = fkst.test

local known_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

return {
  test_fire_raiser_git_ref_poll_routes_real_tick_to_ref_detect = function()
    t.mock_command('printf %s "$FKST_GIT_WATCH_REFS"', {
      stdout = "origin#main",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("git ls-remote 'origin' 'refs/heads/main'", {
      stdout = known_sha .. "\trefs/heads/main\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("git ls-remote origin refs/heads/main", {
      stdout = known_sha .. "\trefs/heads/main\n",
      stderr = "",
      exit_code = 0,
    })

    local trace = t.fire_raiser("git_ref_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "git_ref_poll")
    t.eq(trace.routed_to[1], "ref_detect")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 1)
    t.eq(trace.raised[1].queue, "git_ref_changed")
    t.eq(trace.raised[1].payload.schema, "git-branch-detector.ref-changed.v1")
    t.eq(trace.raised[1].payload.source_ref.kind, "git-ref")
    t.eq(trace.raised[1].payload.source_ref.ref, "origin#main")
    t.eq(trace.raised[1].payload.sha, known_sha)
    t.eq(trace.raised[1].payload.dedup_key, "git-ref/origin#main#" .. known_sha)
    graph.assert_covers(trace, {})
  end,
}
