local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

return {
  test_invalid_or_missing_intake_service_class_normalizes_to_standard = function()
    local parsed = core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ urgent\n⟦FKST:REASON⟧ Invalid class values normalize to standard.")
    t.eq(parsed.action, "enable")
    t.eq(parsed.service_class, "standard")
    t.eq(core.normalize_intake_service_class(nil), "standard")
    t.eq(core.normalize_intake_service_class("EXPEDITE"), "expedite")
  end,

  test_intake_service_class_labels_are_display_only_projection = function()
    local add, remove = core.intake_service_class_label_changes("expedite")
    t.eq(add[1], "fkst-class:expedite")
    t.eq(remove[1], "fkst-class:standard")
    t.eq(remove[2], "fkst-class:background")
    t.eq(core.intake_service_class_label("unknown"), "fkst-class:standard")
  end,
}
