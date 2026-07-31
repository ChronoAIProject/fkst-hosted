-- Two sessions in one deployment resolve their logical work labels through the
-- same FKST_WORK_LABEL_NAMESPACE, so the label family cannot tell them apart.
-- Ownership is the creator routing; these prove it is enforced on the observe
-- and liveness paths, not just at claim admission (#5750).
local h = require("tests.devloop_helpers")
local t = h.t
local m_claims = require("devloop.claims")

local CREATOR = "chronoai-shining"
local FOREIGN = "wanghuan-520"

local function mock_creator(login)
  for _ = 1, 8 do
    t.mock_command('printf %s "$FKST_SESSION_CREATOR"', {
      stdout = login or "",
      stderr = "",
      exit_code = 0,
    })
  end
end

return {
  test_entity_assigned_to_this_creator_is_owned = function()
    mock_creator(CREATOR)
    local owned, reason = m_claims.issue_owned_by_session({ CREATOR })
    t.is_true(owned)
    t.eq(reason, nil)
  end,

  -- The prod case: #5745 was assigned to chronoai-shining, and wanghuan-520's
  -- session redrove it anyway because only the label family was checked.
  test_entity_assigned_to_another_creator_is_not_owned = function()
    mock_creator(FOREIGN)
    local owned, reason = m_claims.issue_owned_by_session({ CREATOR })
    t.is_true(not owned)
    t.is_true(reason:find(FOREIGN, 1, true) ~= nil)
  end,

  -- GitHub logins are case-insensitive; ownership must not hinge on spelling.
  test_creator_match_is_case_insensitive = function()
    mock_creator("ChronoAI-Shining")
    t.is_true((m_claims.issue_owned_by_session({ "chronoai-shining" })))
  end,

  -- Routing requires EXACTLY one assignee: ambiguity is not ownership.
  test_multiple_assignees_are_not_owned = function()
    mock_creator(CREATOR)
    local owned = m_claims.issue_owned_by_session({ CREATOR, FOREIGN })
    t.is_true(not owned)
  end,

  test_no_assignee_is_not_owned = function()
    mock_creator(CREATOR)
    t.is_true(not (m_claims.issue_owned_by_session({})))
    t.is_true(not (m_claims.issue_owned_by_session(nil)))
  end,

  -- Standalone single-session deployments configure no creator and must keep
  -- behaving exactly as before: there is nobody to collide with.
  test_no_configured_creator_owns_everything = function()
    mock_creator("")
    t.is_true((m_claims.issue_owned_by_session({ FOREIGN })))
    t.is_true((m_claims.issue_owned_by_session({})))
  end,
}
