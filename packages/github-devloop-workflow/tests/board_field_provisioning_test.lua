-- Regression: the workflow adapter provisions the default intake surface onto its
-- core. Every default_intake host builds
-- an intake context bundle whose board digest (devloop.payloads.board's
-- board_digest_block) reads fields off the core (M): is_state_label,
-- safe_updated_at, comment_bodies, state_marker, and the untrusted/body-length
-- fields. If install does not provision one of them, the board digest crashes
-- ONLY under a real supervise intake path (tests otherwise never render the
-- board), stalling the whole workflow cascade because a materialized child issue
-- can never be enabled for implementation.
--
-- Found by real supervise dogfood 2026-07-03: child issue #90 (a workflow
-- scaffold slot) crashed intake with `board.lua:64: attempt to call a nil value
-- (field 'is_state_label')` because install_state omitted is_state_label. This
-- test guards the whole class of board-field omissions on the workflow core.
local core = require("core")
local t = fkst.test

-- Fields board.lua's board_digest_block calls on M. Keep in sync with the M.<field>
-- references in libraries/devloop/payloads/board.lua.
local BOARD_FUNCTION_FIELDS = {
  "is_state_label",
  "safe_updated_at",
  "comment_bodies",
  "state_marker",
}
local BOARD_VALUE_FIELDS = {
  "_untrusted_issue_data_begin",
  "_untrusted_issue_data_end",
  "_max_body_len",
}

return {
  test_workflow_core_provisions_board_digest_function_fields = function()
    for _, field in ipairs(BOARD_FUNCTION_FIELDS) do
      t.eq(type(core[field]), "function")
    end
  end,

  test_workflow_core_provisions_board_digest_value_fields = function()
    for _, field in ipairs(BOARD_VALUE_FIELDS) do
      t.is_true(core[field] ~= nil)
    end
  end,

  -- Exercise the exact crash path: board.lua's state_label() calls
  -- core.is_state_label(label) to pick the lifecycle state label from an issue's
  -- labels. It must classify a real state label and reject a non-state label.
  test_is_state_label_classifies_state_labels_on_workflow_core = function()
    t.is_true(core.is_state_label("fkst-dev:thinking"))
    t.is_true(core.is_state_label("fkst-dev:ready"))
    t.is_true(not core.is_state_label("fkst-class:standard"))
    t.is_true(not core.is_state_label("some-unrelated-label"))
  end,
}
