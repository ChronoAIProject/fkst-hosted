local core = require("core")
local t = fkst.test

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Adopt consensus package",
    body = "Create a small flat package that asks several angles to judge a proposal.",
    context = "The package must stay silent unless all angles agree.",
    angles = { "teleology", "parsimony", "fidelity" },
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

return {
  test_ideal_rubric_never_renders_without_mode_calibration = function()
    local calibration_by_mode = {
      converge = "The IDEAL section is context only, never an abstain ground",
      gate = "the IDEAL section is context only, never a rejection ground",
    }

    for _, mode in ipairs({ "converge", "gate" }) do
      for _, angle in ipairs({ "teleology", "parsimony", "fidelity" }) do
        local prompt = core.build_angle_prompt(proposal({ verdict_mode = mode }), angle)
        local has_ideal = prompt:find("IDEAL:", 1, true) ~= nil
        local has_six_smell = prompt:find("Six-smell comparison:", 1, true) ~= nil

        t.is_true(has_ideal)
        t.is_true(has_six_smell)
        if has_ideal and has_six_smell then
          t.is_true(prompt:find(calibration_by_mode[mode], 1, true) ~= nil)
        end
      end
    end
  end,
}
