local h = require("tests.devloop_helpers")
local m_shared = require("devloop.markers.shared")
local t = h.t
local core = h.core

return {
  test_invalid_or_missing_intake_service_class_fails_closed = function()
    local parsed = core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ urgent\n⟦FKST:REASON⟧ Invalid class values normalize to standard.")
    t.is_nil(parsed)
    t.is_nil(core.parse_intake_action("⟦FKST:INTAKE⟧ enable\n⟦FKST:REASON⟧ Missing class facts fail closed."))
    t.eq(m_shared.normalize_intake_service_class(nil), "standard")
    t.eq(m_shared.normalize_intake_service_class("EXPEDITE"), "expedite")
  end,

  test_intake_service_class_labels_are_display_only_projection = function()
    local add, remove = core.intake_service_class_label_changes("expedite")
    t.eq(add[1], "fkst-class:expedite")
    t.eq(remove[1], "fkst-class:standard")
    t.eq(remove[2], "fkst-class:background")
    t.eq(core.intake_service_class_label("unknown"), "fkst-class:standard")
  end,

  test_intake_service_class_label_request_binds_intake_fact_identity = function()
    local request = core.build_intake_service_class_label_request("owner/repo", 42, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "intake/github-devloop/issue/owner/repo/42/v1",
      service_class = "background",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    })

    t.eq(request.schema, "github-proxy.label.v1")
    t.eq(request.add_labels[1], "fkst-class:background")
    t.eq(request.remove_labels[1], "fkst-class:expedite")
    t.eq(request.remove_labels[2], "fkst-class:standard")
    t.is_nil(request.label_colors)
    t.is_true(request.dedup_key:find("class-label", 1, true) ~= nil)
    t.eq(request.source_ref.ref, "owner/repo#issue/42")
  end,
}
