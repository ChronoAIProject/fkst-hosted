local M = {}

local function stderr_of(result)
  return type(result) == "table" and tostring(result.stderr or "") or ""
end

function M.is_rate_limited(result)
  local stderr = stderr_of(result):lower()
  for _, needle in ipairs({
    "api rate limit exceeded",
    "secondary rate limit",
    "was submitted too quickly",
    "http 429",
    "status 429",
    "429 too many requests",
    "too many requests",
  }) do
    if stderr:find(needle, 1, true) then
      return true
    end
  end
  if stderr:find("abuse", 1, true) and stderr:find("rate", 1, true) then
    return true
  end
  return false
end

function M.error_class(result)
  if M.is_rate_limited(result) then
    return "gh-rate-limited"
  end
  return "gh-command-failed"
end

function M.run(exec, cmd, timeout, context)
  local result = exec({ cmd = cmd, timeout = timeout, rate_pool = { name = "gh" } })
  if type(result) ~= "table" or tonumber(result.exit_code) ~= 0 then
    local class = M.error_class(result)
    local message = "std.github: " .. tostring(context) .. " failed: " .. class .. ": " .. stderr_of(result)
    error(setmetatable({
      class = class,
      retryable = class == "gh-rate-limited",
      result = result,
      message = message,
    }, {
      __tostring = function(err)
        return err.message
      end,
    }))
  end
  return result
end

return M
