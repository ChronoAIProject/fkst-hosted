local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

return {
  test_intake_parser_is_strict_and_conservative = function()
    local parsed = core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Clear bounded task.")
    t.eq(parsed.action, "enable")
    t.eq(parsed.service_class, "standard")
    t.eq(parsed.reason, "Clear bounded task.")

    local tracked = core.parse_intake_action("⟦FKST:INTAKE⟧ track\n⟦FKST:CLASS⟧ background\n⟦FKST:REASON⟧ Umbrella tracking issue with independent waves.")
    t.eq(tracked.action, "track")
    t.eq(tracked.service_class, "background")
    t.eq(tracked.reason, "Umbrella tracking issue with independent waves.")

    local escalated = core.parse_intake_action("⟦FKST:INTAKE⟧ escalate-to-class\n⟦FKST:CLASS⟧ expedite\n⟦FKST:REASON⟧ Third widget-sync recurrence; class-level retry policy is required.")
    t.eq(escalated.action, "escalate-to-class")
    t.eq(escalated.service_class, "expedite")
    t.eq(escalated.reason, "Third widget-sync recurrence; class-level retry policy is required.")

    t.is_nil(core.parse_intake_action("prefix\n⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Clear bounded task."))
    t.is_nil(core.parse_intake_action("⟦FKST:INTAKE⟧ enable extra\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Clear bounded task."))
    t.is_nil(core.parse_intake_action("⟦FKST:INTAKE⟧ park\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Unknown values must fail closed."))
    t.is_nil(core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ urgent\n⟦FKST:REASON⟧ Invalid class facts must fail closed."))
    t.is_nil(core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:REASON⟧ Clear bounded task.\n⟦FKST:INTAKE⟧ decline"))
  end,
}
