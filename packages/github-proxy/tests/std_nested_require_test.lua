local gh_probe = require("std.github.probe")
local git_probe = require("std.git.probe")

return {
  test_nested_std_require_resolves = function()
    assert(gh_probe.ok == true, "std.github.probe must resolve")
    assert(git_probe.ok == true, "std.git.probe must resolve")
  end,
}
