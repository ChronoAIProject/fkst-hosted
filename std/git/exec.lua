local M = {}

local function misuse_error(argv, context)
  local bad_program
  if type(argv) == "table" then
    bad_program = argv[1]
  end
  local message = "std.git: " .. tostring(context) .. " adapter misuse: expected git argv, got "
    .. tostring(bad_program)
  error(setmetatable({
    class = "git-adapter-misuse",
    expected_program = "git",
    bad_program = bad_program,
    message = message,
  }, {
    __tostring = function(err)
      return err.message
    end,
  }))
end

function M.run(exec, argv, timeout, context)
  if type(argv) ~= "table" or #argv < 1 or argv[1] ~= "git" then
    misuse_error(argv, context)
  end
  local result = exec({ argv = argv, timeout = timeout })
  if type(result) ~= "table" or tonumber(result.exit_code) ~= 0 then
    error({
      class = "git-command-failed",
      result = result,
      message = "std.git: " .. tostring(context) .. " failed",
    })
  end
  return result
end

return M
