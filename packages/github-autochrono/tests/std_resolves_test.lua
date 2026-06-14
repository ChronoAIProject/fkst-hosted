-- Resolution probe: fail-closed if the package's `std` symlink is missing or
-- broken, so a forgotten/unresolved std vendoring is caught, not silently degraded.
local std = require("std")
local t = fkst.test

return {
  test_std_root_resolves = function()
    t.eq(type(std), "table")
    t.eq(std.version, "0")
  end,
}
