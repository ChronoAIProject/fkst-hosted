local M = {}

function M.fail(path, code, message, meta)
  local err = {
    path = path,
    code = code,
    message = message,
  }
  if meta ~= nil then
    err.meta = meta
  end
  return err
end

return M
