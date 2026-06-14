-- std.strings: small, dependency-free string utilities shared across packages.
local S = {}

function S.trim(value)
  return (tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

return S
