local std = require("std")
local t = fkst.test

return {
  test_std_root_resolves = function()
    t.eq(type(std), "table")
    t.eq(std.version, "0")
  end,
}
