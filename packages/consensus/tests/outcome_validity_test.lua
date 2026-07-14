local angle_answers = require("angle_answers")
local core = require("core")
local rebuttal = require("departments.decide.rebuttal")
local t = fkst.test

local function proposal(extra)
  local value = {
    proposal_id = "proposal-42",
    dedup_key = "proposal-42-v1",
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/42",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function answer(angle, verdict)
  return {
    angle = angle,
    verdict = verdict,
    reply = angle .. " reply",
    exit_code = 0,
  }
end

return {
  test_angle_answer_validity_requires_success_and_a_parseable_verdict = function()
    t.eq(angle_answers.is_valid(answer("teleology", "approve")), true)
    t.eq(angle_answers.is_valid({
      angle = "teleology",
      verdict = "approve",
      reply = "Reply.",
      exit_code = 7,
    }), false)
    t.eq(angle_answers.is_valid({
      angle = "teleology",
      reply = "Reply.",
      exit_code = 0,
    }), false)

    local ok, err = pcall(angle_answers.assert_all_valid, {
      answer("teleology", "approve"),
      { angle = "parsimony", stderr = "worker unavailable", exit_code = 7 },
    }, "blind")
    t.eq(ok, false)
    t.is_true(tostring(err):find("codex-failed", 1, true) ~= nil)
  end,

  test_build_converge_payload_rejects_an_incomplete_panel = function()
    local ok, err = pcall(
      core.build_converge_payload,
      proposal({ round = 2, dedup_key = "proposal-42-v1/loop/2" }),
      "Narrow the disagreement.",
      {
        answer("teleology", "approve"),
        answer("parsimony", "abstain"),
        { angle = "fidelity", exit_code = 7 },
      }
    )

    t.eq(ok, false)
    t.is_true(tostring(err):find("codex-failed", 1, true) ~= nil)
  end,

  test_outcome_builders_reject_zero_valid_angle_answers = function()
    local malformed = {
      { angle = "teleology", stdout = "no verdict", exit_code = 0 },
    }
    local converge_ok, converge_err = pcall(
      core.build_converge_payload,
      proposal(),
      "Narrow the disagreement.",
      malformed
    )
    t.eq(converge_ok, false)
    t.is_true(tostring(converge_err):find("angle-output-unparseable", 1, true) ~= nil)

    local reached_ok, reached_err = pcall(
      core.build_reached_payload,
      proposal(),
      "approve",
      malformed,
      "Use the synthesis framing."
    )
    t.eq(reached_ok, false)
    t.is_true(tostring(reached_err):find("angle-output-unparseable", 1, true) ~= nil)

    local degraded_ok, degraded_err = pcall(
      core.build_reached_payload,
      proposal(),
      "approve",
      {
        answer("teleology", "approve"),
        { angle = "parsimony", stderr = "worker unavailable", exit_code = 7 },
      },
      "Use the synthesis framing."
    )
    t.eq(degraded_ok, false)
    t.is_true(tostring(degraded_err):find("codex-failed", 1, true) ~= nil)
  end,

  test_post_rebuttal_reached_rejects_a_degraded_blind_panel = function()
    local reached_ok, reached_err = pcall(
      rebuttal.post_rebuttal_reached,
      proposal(),
      {
        answer("teleology", "approve"),
        { angle = "parsimony", stderr = "worker unavailable", exit_code = 7 },
      },
      {
        answer("teleology", "approve"),
        answer("parsimony", "approve"),
      },
      "converge",
      {
        aggregate = function()
          return "approve"
        end,
        assert_all_angle_answers_valid = function(results, phase)
          return angle_answers.assert_all_valid(results, phase)
        end,
        build_reached_payload = function()
          return {}
        end,
      }
    )

    t.eq(reached_ok, false)
    t.is_true(tostring(reached_err):find("codex-failed", 1, true) ~= nil)
  end,

  test_reached_payload_requires_gate_reject_gap_but_allows_premise_refutation = function()
    local gate_ok, gate_err = pcall(core.build_reached_payload, proposal({ verdict_mode = "gate" }), {
      decision = "reject",
    }, {
      answer("teleology", "reject"),
    })
    t.eq(gate_ok, false)
    t.is_true(tostring(gate_err):find("blocking-gap-invalid", 1, true) ~= nil)

    local premise = core.build_reached_payload(proposal(), {
      decision = "reject",
      decision_reason = "premise-refuted",
    }, {
      answer("teleology", "abstain"),
      answer("parsimony", "approve"),
    })
    t.eq(premise.decision, "reject")
    t.eq(premise.decision_reason, "premise-refuted")
    t.is_nil(premise.blocking_gap)
  end,
}
