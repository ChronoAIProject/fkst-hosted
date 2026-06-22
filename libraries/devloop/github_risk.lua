local S = {}

function S.install(M)
local high_risk_patterns = {
  "^%.github/workflows/",
  "^%.github/actions/",
  "^%.github/dependabot%.yml$",
  "^%.github/CODEOWNERS$",
  "^Cargo%.toml$",
  "^Cargo%.lock$",
  "^package%.json$",
  "^package%-lock%.json$",
  "^pnpm%-lock%.yaml$",
  "^yarn%.lock$",
  "^requirements%.txt$",
  "^requirements/",
  "^pyproject%.toml$",
  "^poetry%.lock$",
  "^scripts/",
  "^%.github/",
}

function M.github_high_risk_path(path)
  local text = tostring(path or "")
  for _, pattern in ipairs(high_risk_patterns) do
    if text:find(pattern) ~= nil then
      return true
    end
  end
  return false
end

function M.github_high_risk_paths(paths)
  local result = {}
  for _, path in ipairs(paths or {}) do
    if M.github_high_risk_path(path) then
      table.insert(result, tostring(path))
    end
  end
  return result
end
end

return S
