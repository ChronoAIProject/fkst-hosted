local t = fkst.test

-- G-PRODUCER-LIVENESS: a raiser that nothing ever fires is a producer nobody can
-- prove works. This drives the real engine end to end — the cron raiser fires,
-- the event routes to `run_execute`, and the department accepts it. A raiser
-- whose queue name drifted from the department's `consumes` would otherwise fail
-- silently on every session.
--
-- The harness refuses UNMOCKED external commands, which is the right default: a
-- test that reached the real `gh` would be nondeterministic and would talk to
-- GitHub from CI. So every command this path takes is mocked explicitly, and the
-- mocks describe the case being asserted — a session with no scheduled run.

return {
  test_fire_raiser_run_start_routes_a_real_tick_to_run_execute = function()
    -- The session's identity. `FKST_GITHUB_REPO` and `FKST_GITHUB_BOT_LOGIN` are
    -- what `claim_identity.read` needs before any GitHub read is attempted.
    t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
      stdout = "owner/repo",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_SESSION_CREATOR"', {
      stdout = "alice",
      stderr = "",
      exit_code = 0,
    })
    -- No open issues: this session has no scheduled run waiting for it.
    t.mock_command("gh issue list", { stdout = "[]", stderr = "", exit_code = 0 })

    local trace = t.fire_raiser("run_start")

    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "run_start")
    t.eq(trace.routed_to[1], "run_execute")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")

    -- Nothing raised, and that is the assertion that matters most: an ordinary
    -- session composing this package must boot, find no run issue, and do
    -- nothing at all. A package that acted on an unrelated work issue would be
    -- executing something nobody asked for.
    t.eq(#trace.raised, 0)
  end,
}
