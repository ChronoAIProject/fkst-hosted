local t = fkst.test

-- The refinement must AMEND, not merely retry. These pin the two properties that
-- distinguish the two: the amendment and the re-entry command travel in ONE
-- comment, and a failure to author an amendment must NOT re-enter.

local refine = require("devloop.convergence.refine")

local RECONCILE = {
  proposal_id = "github-devloop/issue/acme/site/42",
  base_version = "github-devloop/issue/acme/site/42/intake/7",
  round = 1,
  terminal_cause = "no-semantic-progress",
  source_ref = { kind = "external", ref = "acme/site#issue/42" },
}

return {
  test_a_refine_payload_round_trips_its_own_validator = function()
    local payload = refine.build_refine_payload(RECONCILE, 1, 2)
    t.is_true(refine.is_supported_refine(payload))
    t.eq(payload.schema, refine.SCHEMA)
    t.eq(payload.refine_round, 1)
    t.eq(payload.budget, 2)
  end,

  test_a_round_beyond_the_budget_is_rejected = function()
    -- The queue is durable, so a payload can outlive the configuration that
    -- created it. A stale event must not spend a budget the session no longer
    -- grants, which is why the check is here and not only at the raise site.
    local payload = refine.build_refine_payload(RECONCILE, 3, 2)
    t.eq(refine.is_supported_refine(payload), false)
  end,

  test_a_foreign_payload_is_rejected = function()
    t.eq(refine.is_supported_refine(nil), false)
    t.eq(refine.is_supported_refine({ schema = "something.else" }), false)
    local bad_cause = refine.build_refine_payload(RECONCILE, 1, 2)
    bad_cause.terminal_cause = "not-a-cause"
    t.eq(refine.is_supported_refine(bad_cause), false)
  end,

  test_the_prompt_carries_the_recorded_objection = function()
    -- Refining without the previous round's feedback is just a retry, which is
    -- the bug being fixed. The narrowed question must reach the model.
    local prompt = refine.build_prompt({
      repo = "acme/site",
      issue_number = 42,
      terminal_cause = "no-semantic-progress",
      narrowed_question = "rule 5 contradicts the manual-run invariant",
      findings_record = "angle: parsimony abstained",
    })
    t.is_true(prompt:find("rule 5 contradicts the manual-run invariant", 1, true) ~= nil)
    t.is_true(prompt:find("angle: parsimony abstained", 1, true) ~= nil)
    -- It must ask for an amendment, not advice: the reply is pasted into the
    -- issue and read as specification by the next lap.
    t.is_true(prompt:find("specification prose, not as advice", 1, true) ~= nil)
    -- And it must never resolve a contradiction by deleting the test.
    t.is_true(prompt:find("weaken or delete a required test", 1, true) ~= nil)
  end,

  test_the_command_and_the_amendment_are_one_comment = function()
    -- This is the whole fix. parse_command reads only the first non-empty line,
    -- so the command must be line 1 -- and the amendment must be in the SAME
    -- comment, or the command fires against content that does not exist yet.
    local body = refine.build_comment_body({
      ai_sentinel = "SENTINEL",
      proposal_id = RECONCILE.proposal_id,
      refine_round = 1,
      budget = 2,
      terminal_cause = "no-semantic-progress",
      round = 1,
      amendment = "Rule 5 now reads `latest_scheduled_run`.",
    })
    t.eq(body:match("^([^\n]*)"), "fkst: reintake")
    t.is_true(body:find("Rule 5 now reads", 1, true) ~= nil)
    -- The durable budget counter must close the body.
    t.is_true(body:find("fkst:github%-devloop:auto%-refine:v1") ~= nil)
  end,

  test_a_failed_authoring_does_not_re_enter = function()
    -- Re-entering with no amendment is exactly the defect this replaces, and it
    -- would spend a budget lap for nothing. The item must stay blocked.
    local body = refine.build_failure_body({
      ai_sentinel = "SENTINEL",
      refine_round = 1,
      budget = 2,
      terminal_cause = "no-semantic-progress",
      reason = "the refinement run exited non-zero",
    })
    t.eq(body:match("^([^\n]*)") == "fkst: reintake", false)
    t.is_true(body:find("stays blocked", 1, true) ~= nil)
    -- It must tell a human how to proceed by hand.
    t.is_true(body:find("fkst: reintake", 1, true) ~= nil)
  end,

  test_an_unusable_model_reply_is_rejected = function()
    -- Never paste a half-parsed reply into a specification.
    t.eq(refine.parse_amendment("not json"), nil)
    t.eq(refine.parse_amendment('{"amendment": 7}'), nil)
    t.eq(refine.parse_amendment('{"amendment": "   "}'), nil)
    t.eq(refine.parse_amendment('{"other": "x"}'), nil)
    t.eq(refine.parse_amendment('{"amendment": "  ok  "}'), "ok")
  end,
}
