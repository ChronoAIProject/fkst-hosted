-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local strings = require("std.strings")
local t = fkst.test

return {
  test_trim_strips_both_ends = function()
    t.eq(strings.trim("  hi  "), "hi")
    t.eq(strings.trim(nil), "")
  end,
}
