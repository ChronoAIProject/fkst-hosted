local angle_answers = require("angle_answers")
local core = require("core")
local t = fkst.test

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local gap_label = "⟦FKST:GAP⟧"

local function inline_answer(verdict, reply)
  return verdict_label .. " " .. verdict .. " " .. reply_label .. " " .. reply
end

local function failure(output, mode)
  local parsed, violation = core.parse_angle_output(output, mode)
  t.is_nil(parsed)
  return violation
end

local function proposal()
  return {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Adopt consensus package",
    body = "Judge the proposed implementation.",
    context = "The implementation must remain fail closed.",
    angles = { "teleology", "parsimony", "fidelity" },
    dedup_key = "proposal-42-v1",
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/42",
    },
  }
end

return {
  test_accepts_production_inline_verdict_reply_shape = function()
    local parsed, violation = core.parse_angle_output(
      "⟦FKST:STANCE⟧ defend\n"
        .. inline_answer("abstain", "The current evidence does not resolve the ownership concern.")
    )

    t.eq(parsed.verdict, "abstain")
    t.eq(parsed.reply, "The current evidence does not resolve the ownership concern.")
    t.is_nil(violation)
  end,

  test_inline_pair_preserves_prompt_echo_neutrality_and_reply_bound = function()
    local prompt = core.build_angle_prompt(proposal(), "teleology")
    local parsed = core.parse_angle_output(
      prompt .. "\n" .. inline_answer("approve", string.rep("x", 2100))
    )

    t.eq(parsed.verdict, "approve")
    t.eq(#parsed.reply, 2000)
  end,

  test_pairs_reject_duplicate_orphan_and_embedded_components = function()
    t.eq(failure(
      inline_answer("approve", "real answer") .. "\n" .. reply_label .. " orphan reply"
    ), "duplicate_component")
    t.eq(failure(
      verdict_label .. " approve\n" .. inline_answer("abstain", "real answer")
    ), "duplicate_component")
    t.eq(failure(
      inline_answer("approve", "answer " .. reply_label .. " injected reply")
    ), "duplicate_component")
    t.eq(failure(
      inline_answer("approve", "answer " .. verdict_label .. " abstain")
    ), "duplicate_component")
    t.eq(failure(
      verdict_label .. " approve\n" .. reply_label .. " answer " .. verdict_label .. " abstain"
    ), "duplicate_component")
    t.eq(failure(
      verdict_label .. " approve\n" .. reply_label .. " "
        .. string.rep("x", 2100) .. verdict_label .. " abstain"
    ), "duplicate_component")
  end,

  test_inline_reject_requires_adjacent_bounded_gap = function()
    local parsed = core.parse_angle_output(
      inline_answer("reject", "The diff is unsafe.") .. "\n" .. gap_label .. " missing regression test",
      "gate"
    )
    t.eq(parsed.verdict, "reject")
    t.eq(parsed.blocking_gap, "missing regression test")

    t.eq(failure(inline_answer("reject", "The diff is unsafe."), "gate"), "gap_rules_violated")
    t.eq(failure(
      inline_answer("reject", "The diff is unsafe.")
        .. "\nintervening line\n" .. gap_label .. " missing regression test",
      "gate"
    ), "gap_rules_violated")
    t.eq(failure(
      inline_answer("reject", "The diff is unsafe.")
        .. "\n" .. gap_label .. " missing test " .. verdict_label .. " approve",
      "gate"
    ), "duplicate_component")
  end,

  test_returns_typed_protocol_violations = function()
    t.eq(failure("plain prose"), "verdict_marker_missing")
    t.eq(failure(verdict_label .. " maybe\n" .. reply_label .. " answer"), "verdict_word_invalid")
    t.eq(failure(verdict_label .. " approve"), "reply_marker_missing")
    t.eq(failure(verdict_label .. " approve " .. reply_label), "reply_empty")
    t.eq(failure(
      verdict_label .. " approve\nintervening line\n" .. reply_label .. " answer"
    ), "reply_not_adjacent")
    t.eq(failure(
      verdict_label .. " approve\n" .. reply_label .. " answer\n" .. gap_label .. " stray gap",
      "gate"
    ), "gap_rules_violated")
  end,

  test_protocol_telemetry_aggregates_sorted_typed_counts = function()
    local ok, err = pcall(angle_answers.assert_all_valid, {
      { exit_code = 0, protocol_violation = "verdict_word_invalid" },
      { exit_code = 0, protocol_violation = "reply_not_adjacent" },
      { exit_code = 0, protocol_violation = "reply_not_adjacent" },
    }, "rebuttal")

    t.eq(ok, false)
    t.is_true(tostring(err):find(
      "protocol_violations=reply_not_adjacent:2,verdict_word_invalid:1",
      1,
      true
    ) ~= nil)
  end,

  test_prompts_require_two_separate_physical_lines = function()
    local angle_prompt = core.build_angle_prompt(proposal(), "teleology")
    local rebuttal_prompt = core.build_rebuttal_prompt(proposal(), {
      angle = "teleology",
      stdout = verdict_label .. " approve\n" .. reply_label .. " initial answer",
    }, {
      {
        angle = "parsimony",
        stdout = verdict_label .. " abstain\n" .. reply_label .. " peer answer",
      },
    })

    t.is_true(angle_prompt:find("two separate physical lines", 1, true) ~= nil)
    t.is_true(rebuttal_prompt:find("two separate physical lines", 1, true) ~= nil)
  end,
}
