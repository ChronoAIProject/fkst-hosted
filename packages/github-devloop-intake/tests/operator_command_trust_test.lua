-- Who may drive an operator command.
--
-- Before this, `operator_command_fact` trusted the bot login alone, which made
-- `fkst: reintake` -- the documented re-entry edge out of `blocked` -- reachable
-- only by the refine department writing it into its own amendment. A human
-- operator's command was dropped in silence, so an issue whose refinement budget
-- was spent could never be recovered (prod #5714). These cover both directions:
-- who the widened policy now admits, and who it must still reject.
local h = require("tests.devloop_helpers")
local t = h.t
local operator_commands = require("devloop.operator_commands")
local github_author_policy = require("devloop.github_author_policy")

local BOT = "fkst-test-bot"

local function comment(author, id)
  return {
    id = id or ("IC_" .. tostring(author)),
    body = "fkst: reintake",
    author_login = author,
    created_at = "2026-07-31T07:00:00Z",
  }
end

--- Env the policy builder reads. Counts are generous because the builder reads
--- through pcall'd env lookups whose call count is an implementation detail.
local function mock_policy_env(creator, authorized)
  for _ = 1, 8 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = BOT, stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_AUTHORIZED_LOGINS"', {
      stdout = authorized or "", stderr = "", exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_SESSION_CREATOR"', {
      stdout = creator or "", stderr = "", exit_code = 0,
    })
  end
end

local function policy_of(logins)
  return github_author_policy.from_logins(logins)
end

return {
  -- The regression that matters most: today's only working path must keep working.
  test_bot_authored_command_is_still_accepted = function()
    local fact = operator_commands.operator_command_fact(
      { comment(BOT) }, "reintake", policy_of({ BOT, "session-creator" })
    )
    t.is_true(fact ~= nil)
    t.eq(fact.command, "reintake")
  end,

  test_session_creator_command_is_accepted = function()
    local fact = operator_commands.operator_command_fact(
      { comment("session-creator") }, "reintake", policy_of({ BOT, "session-creator" })
    )
    t.is_true(fact ~= nil)
    t.eq(fact.author_login, "session-creator")
  end,

  test_unauthorized_author_command_is_rejected = function()
    local fact = operator_commands.operator_command_fact(
      { comment("random-passerby") }, "reintake", policy_of({ BOT, "session-creator" })
    )
    t.eq(fact, nil)
  end,

  -- Without a policy the old contract stands, so every existing caller and
  -- fixture that passes none keeps its bot-only behaviour.
  test_absent_policy_keeps_bot_only_contract = function()
    h.mock_bot_env()
    t.eq(operator_commands.operator_command_fact({ comment("session-creator") }, "reintake"), nil)
    t.is_true(operator_commands.operator_command_fact({ comment(BOT) }, "reintake") ~= nil)
  end,

  -- The whitelist stores folded logins, so a raw set lookup would reject both of
  -- these. They are the exact spellings GitHub hands back across its surfaces.
  test_author_matching_is_case_and_bot_suffix_insensitive = function()
    local policy = policy_of({ BOT, "Session-Creator" })
    t.is_true(operator_commands.operator_command_fact(
      { comment("session-CREATOR") }, "reintake", policy) ~= nil)
    t.is_true(operator_commands.operator_command_fact(
      { comment(BOT .. "[bot]") }, "reintake", policy) ~= nil)
  end,

  test_policy_from_env_admits_creator_and_authorized_logins = function()
    mock_policy_env("session-creator", "collaborator-one,collaborator-two")
    local policy = operator_commands.operator_author_policy()
    t.is_true(policy ~= nil)
    for _, login in ipairs({ BOT, "session-creator", "collaborator-one", "collaborator-two" }) do
      t.is_true(github_author_policy.is_authorized(policy, login))
    end
    t.is_true(not github_author_policy.is_authorized(policy, "random-passerby"))
  end,

  -- No creator configured is the standalone-package contract, not an error.
  test_policy_from_env_without_creator_still_admits_the_bot = function()
    mock_policy_env(nil, nil)
    local policy = operator_commands.operator_author_policy()
    t.is_true(policy ~= nil)
    t.is_true(github_author_policy.is_authorized(policy, BOT))
    t.is_true(not github_author_policy.is_authorized(policy, "session-creator"))
  end,
}
