local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function err(message)
  return "github-devloop: fix codex failed: " .. message
end

return {
  test_wrapped_codex_failure_extracts_narrow_error_class = function()
    t.eq(core.error_fact_class(err("exit 1")), "codex-failed")
    t.eq(core.error_fact_class({ message = err("exit 1") }), "codex-failed")
  end,

  test_fingerprint_is_stable_across_timestamp_sha_and_tmp_path_noise = function()
    local first = core.error_fact_fingerprint({
      queue = "devloop_fixing",
      error_class = "codex-failed",
      message = err("at 2026-06-11T20:57:25Z in /tmp/fkst-a/worktree sha 81bb199f4a3eda6d736d11100856a12230030b0e"),
    })
    local second = core.error_fact_fingerprint({
      queue = "devloop_fixing",
      error_class = "codex-failed",
      message = err("at 2026-06-12T01:02:03Z in /tmp/fkst-b/worktree sha 7d9c0a1b2c3d4e5f678901234567890abcdef123"),
    })
    t.eq(first, second)
  end,

  test_fingerprint_separates_error_class_and_queue = function()
    local base = {
      queue = "devloop_fixing",
      error_class = "codex-failed",
      message = err("exit 1"),
    }
    local same_error_different_queue = {
      queue = "devloop_ready",
      error_class = "codex-failed",
      message = err("exit 1"),
    }
    local same_queue_different_error = {
      queue = "devloop_fixing",
      error_class = "git-command-failed",
      message = err("exit 1"),
    }

    t.is_true(core.error_fact_fingerprint(base) ~= core.error_fact_fingerprint(same_error_different_queue))
    t.is_true(core.error_fact_fingerprint(base) ~= core.error_fact_fingerprint(same_queue_different_error))
  end,

  test_build_error_fact_omits_unowned_context_fields = function()
    local fact = core.build_error_fact({
      queue = "devloop_fixing",
      message = err("exit 1"),
    })

    t.eq(fact.schema, "github-devloop.error-fact.v1")
    t.eq(fact.queue, "devloop_fixing")
    t.eq(fact.error_class, "codex-failed")
    t.is_nil(fact.source_ref)
    t.is_nil(fact.attempt)
    t.is_nil(fact.terminal)
  end,

  test_build_error_fact_preserves_explicit_owned_context_fields = function()
    local source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    }
    local fact = core.build_error_fact({
      queue = "devloop_fixing",
      error_class = "codex-failed",
      message = err("exit 1"),
      source_ref = source_ref,
      attempt = 3,
      terminal = false,
    })

    t.eq(fact.source_ref.kind, "external")
    t.eq(fact.source_ref.ref, "owner/repo#issue/42")
    t.eq(fact.attempt, 3)
    t.eq(fact.terminal, false)
    t.eq(core.error_fact_source_ref_digest(source_ref), "external:owner/repo#issue/42")
  end,
}
