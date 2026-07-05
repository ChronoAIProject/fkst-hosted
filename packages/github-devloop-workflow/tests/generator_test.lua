local generator = require("core.generator")
local t = fkst.test

local static_slot = {
  id = "first",
  title = "Static step",
  content = {
    kind = "static",
    intent = "Implement the static step.",
  },
}

local generated_slot = {
  id = "second",
  title = "Generated step",
  content = {
    kind = "generated",
    generator = "Use the predecessor result to produce a follow-up issue.",
  },
}

local predecessor = {
  source_ref = {
    kind = "external",
    ref = "owner/repo#issue/42",
  },
}

local tests = {
  test_static_literal_emits_title_and_body_without_codex = function()
    local called = false
    local spec, reason = generator.run_slot_generator({
      spawn_codex = function()
        called = true
        return { exit_code = 0, stdout = "{}" }
      end,
    }, {}, static_slot, nil)
    t.is_nil(reason)
    t.eq(spec.title, "Static step")
    t.eq(spec.body, "Implement the static step.")
    t.eq(called, false)
  end,

  test_generated_slot_uses_codex_and_source_ref_not_content_payload = function()
    local seen_prompt = nil
    local spec, reason = generator.run_slot_generator({
      content_fetch = function(ref)
        t.eq(ref.source_ref.ref, "owner/repo#issue/42")
        return "runtime-cache:workflow/predecessor"
      end,
      spawn_codex = function(prompt)
        seen_prompt = prompt
        return {
          exit_code = 0,
          stdout = '{"title":"Next bounded issue","body":"Implement the generated follow-up."}',
        }
      end,
    }, {
      origin_proposal_id = "github-devloop/issue/owner/repo/9",
      workflow_id = "workflow-one",
    }, generated_slot, predecessor)
    t.is_nil(reason)
    t.eq(spec.title, "Next bounded issue")
    t.eq(spec.body, "Implement the generated follow-up.")
    t.is_true(seen_prompt:find("owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(seen_prompt:find("runtime-cache:workflow/predecessor", 1, true) ~= nil)
    t.is_nil(seen_prompt:find("Implement the generated follow%-up%."))
  end,

  -- Regression (found by real dogfood): a pre-fetch failure must NOT block
  -- generation. The codex still has the predecessor source_ref (+ full access)
  -- and is instructed to fetch it directly, so content_fetch throwing (e.g. an
  -- unavailable devloop board/context-bundle in one-shot run) falls back to the
  -- source_ref instead of hard-erroring predecessor-content-fetch-failed.
  test_generated_slot_content_fetch_failure_falls_back_to_source_ref = function()
    local seen_prompt = nil
    local spec, reason = generator.run_slot_generator({
      content_fetch = function()
        error("board unavailable")
      end,
      spawn_codex = function(prompt)
        seen_prompt = prompt
        return { exit_code = 0, stdout = '{"title":"Next","body":"From source_ref."}' }
      end,
    }, {
      origin_proposal_id = "github-devloop/issue/owner/repo/9",
      workflow_id = "workflow-one",
    }, generated_slot, predecessor)
    t.is_nil(reason)
    t.eq(spec.title, "Next")
    t.eq(spec.body, "From source_ref.")
    t.is_true(seen_prompt:find("owner/repo#issue/42", 1, true) ~= nil)
  end,

  test_generated_invalid_output_returns_reason_code = function()
    local spec, reason = generator.run_slot_generator({
      spawn_codex = function()
        return {
          exit_code = 0,
          stdout = '{"title":"","body":"body"}',
        }
      end,
    }, {}, generated_slot, predecessor)
    t.is_nil(spec)
    t.eq(reason, "invalid-title")
  end,

  test_generated_missing_predecessor_fails_closed = function()
    local spec, reason = generator.run_slot_generator({
      spawn_codex = function()
        return {
          exit_code = 0,
          stdout = '{"title":"x","body":"y"}',
        }
      end,
    }, {}, generated_slot, nil)
    t.is_nil(spec)
    t.eq(reason, "missing-predecessor-result")
  end,

  test_generated_missing_runner_fails_closed = function()
    local spec, reason = generator.run_slot_generator({}, {}, generated_slot, predecessor)
    t.is_nil(spec)
    t.eq(reason, "missing-generator-runner")
  end,
}

return tests
