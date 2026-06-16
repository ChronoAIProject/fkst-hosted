local gh_exec = require("std.github.exec")
local git_exec = require("std.git.exec")

return {
  test_nested_std_require_resolves = function()
    assert(type(gh_exec.run) == "function", "std.github.exec must resolve")
    assert(type(git_exec.run) == "function", "std.git.exec must resolve")
  end,
}
