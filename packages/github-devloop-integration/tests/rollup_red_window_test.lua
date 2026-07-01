local h = require("tests.devloop_helpers")

local core = h.core
local t = h.t

local function exec_returning(value)
  return function(_command)
    return { stdout = value, stderr = "", exit_code = 0 }
  end
end

return {
  -- Regression guard for the latent bug fixed alongside the _trim decouple: the old
  -- ambient M._trim returned two values (the trimmed string plus the chained gsub's
  -- substitution count), so `tonumber(M._trim(raw))` passed that count (0 or 1) as
  -- tonumber's base and raised "base out of range" for ANY set value — the custom red
  -- window never parsed. Resolving to contract.strings.trim (single value) fixes it.
  test_rollup_red_window_minutes_parses_a_set_value = function()
    t.eq(core.rollup_red_window_minutes(exec_returning("30")), 30)
  end,

  test_rollup_red_window_minutes_trims_surrounding_whitespace = function()
    t.eq(core.rollup_red_window_minutes(exec_returning("  45  ")), 45)
  end,

  test_rollup_red_window_minutes_rejects_non_numeric = function()
    t.raises(function()
      core.rollup_red_window_minutes(exec_returning("abc"))
    end)
  end,

  test_rollup_red_window_minutes_rejects_out_of_range = function()
    t.raises(function()
      core.rollup_red_window_minutes(exec_returning("5000"))
    end)
  end,
}
