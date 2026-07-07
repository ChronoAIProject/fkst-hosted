local core = require("core")
local t = fkst.test

return {
  test_debate_phase_names_returns_ordered_canonical_labels = function()
    local phases = core.debate_phase_names()

    t.eq(#phases, 3)
    t.eq(phases[1], "blind")
    t.eq(phases[2], "rebuttal")
    t.eq(phases[3], "synthesis")
  end,

  test_debate_phase_names_returns_fresh_plain_list = function()
    local phases = core.debate_phase_names()
    phases[1] = "changed"

    local fresh = core.debate_phase_names()
    t.eq(fresh[1], "blind")
    t.eq(fresh[2], "rebuttal")
    t.eq(fresh[3], "synthesis")
  end,
}
