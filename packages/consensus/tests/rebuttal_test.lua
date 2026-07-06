local core = require("core")
local rebuttal = require("departments.decide.rebuttal")
local t = fkst.test

local stance_label = "⟦FKST:STANCE⟧"
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function answer(stance, verdict, reply, rest)
  local line = stance_label .. " " .. stance
  if rest ~= nil then
    line = line .. " " .. rest
  end
  return line .. "\n" .. verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply
end

local caps = {
  parse_angle_output = core.parse_angle_output,
  stance_label = stance_label,
}

return {
  test_parse_stance_accepts_update_with_peer_claim = function()
    local parsed = rebuttal.parse_output(answer("update", "approve", "Moved by peer evidence.", "because parsimony named the missing CAS claim"), nil, caps)
    t.eq(parsed.stance, "update")
    t.eq(parsed.peer_claim, "parsimony named the missing CAS claim")
    t.eq(parsed.verdict, "approve")
    t.eq(parsed.reply, "Moved by peer evidence.")
  end,

  test_parse_stance_accepts_defend = function()
    local parsed = rebuttal.parse_output(answer("defend", "abstain", "The peer claim does not resolve the blocker."), nil, caps)
    t.eq(parsed.stance, "defend")
    t.is_nil(parsed.peer_claim)
    t.eq(parsed.verdict, "abstain")
  end,

  test_parse_stance_rejects_missing_or_unknown_stance = function()
    t.is_nil(rebuttal.parse_output(verdict_label .. " approve\n" .. reply_label .. " ok", nil, caps))
    t.is_nil(rebuttal.parse_output(answer("maybe", "approve", "ok"), nil, caps))
  end,

  test_parse_stance_rejects_update_without_named_peer_claim = function()
    t.is_nil(rebuttal.parse_output(answer("update", "approve", "ok"), nil, caps))
    t.is_nil(rebuttal.parse_output(answer("update", "approve", "ok", "because   "), nil, caps))
  end,
}
