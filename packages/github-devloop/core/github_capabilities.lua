local S = {}

function S.install(M)
local gh_program = table.concat({ "g", "h" })

local function shell_words(command)
  local words = {}
  for word in tostring(command or ""):gmatch("%S+") do
    local cleaned = word:gsub("^'", ""):gsub("'$", "")
    table.insert(words, cleaned)
  end
  return words
end

local function shell_env_assignment(name, value)
  if value == false then
    return name .. "="
  end
  return name .. "=${" .. tostring(value) .. ":-}"
end

local function repo_from_flag(command)
  return tostring(command or ""):match("%-%-repo%s+'([^']+)'")
    or tostring(command or ""):match("%-%-repo%s+([^%s]+)")
end

local function repo_from_api_path(command)
  return tostring(command or ""):match("'repos/([^/]+/[^/'%s]+)")
    or tostring(command or ""):match("%srepos/([^/]+/[^/%s]+)")
end

local function stage_for_command(command)
  local words = shell_words(command)
  local resource = words[2]
  local action = words[3]
  local method = tostring(command or ""):match("%-%-method%s+([A-Z]+)")
  if resource == "pr" and action == "merge" then
    return "merge"
  end
  if resource == "pr" and action == "create" then
    return "open-pr"
  end
  if resource == "pr" and action == "ready" then
    return "pr-ready"
  end
  if resource == "issue" and action == "close" then
    return "issue-close"
  end
  if resource == "issue" and action == "edit" then
    return "assignee"
  end
  if resource == "pr" and action == "close" then
    return "pr-close"
  end
  if (resource == "issue" or resource == "pr") and action == "comment" then
    return "comment"
  end
  if resource == "api" and method ~= nil then
    return "api-" .. method:lower()
  end
  return "read-audit"
end

local function command_scope(command)
  return {
    repo = repo_from_flag(command) or repo_from_api_path(command),
    branch = tostring(command or ""):match("%-%-head%s+'([^']+)'")
      or tostring(command or ""):match("%-%-head%s+([^%s]+)")
      or tostring(command or ""):match("HEAD:refs/heads/'([^']+)'")
      or tostring(command or ""):match("HEAD:refs/heads/([^%s]+)"),
    stage = stage_for_command(command),
  }
end

local function is_write_command(command)
  local words = shell_words(command)
  local resource = words[2]
  local action = words[3]
  local method = tostring(command or ""):match("%-%-method%s+([A-Z]+)")
  return resource == "api" and method ~= nil and method ~= "GET"
    or resource == "pr" and (
      action == "merge"
      or action == "create"
      or action == "ready"
      or action == "close"
      or action == "comment"
    )
    or resource == "issue" and (
      action == "edit"
      or action == "close"
      or action == "comment"
    )
end

local function is_merge_command(command)
  local words = shell_words(command)
  return words[2] == "pr" and words[3] == "merge"
end

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

function M.github_command_capability(command)
  local scope = command_scope(command)
  if is_merge_command(command) then
    return {
      role = "merge",
      token_env = "FKST_GITHUB_MERGE_TOKEN",
      scope = scope,
      write = true,
    }
  end
  if is_write_command(command) then
    return {
      role = "write",
      token_env = "FKST_GITHUB_WRITE_TOKEN",
      scope = scope,
      write = true,
    }
  end
  return {
    role = "read-audit",
    token_env = "FKST_GITHUB_READ_TOKEN",
    scope = scope,
    write = false,
  }
end

function M.github_capability_env_prefix(capability)
  local cap = capability or {}
  if cap.role == "read-audit" then
    return table.concat({
      shell_env_assignment("GH_TOKEN", "FKST_GITHUB_READ_TOKEN"),
      shell_env_assignment("GITHUB_TOKEN", "FKST_GITHUB_READ_TOKEN"),
    }, " ")
  end
  if cap.role == "write" then
    return table.concat({
      shell_env_assignment("GH_TOKEN", "FKST_GITHUB_WRITE_TOKEN"),
      shell_env_assignment("GITHUB_TOKEN", "FKST_GITHUB_WRITE_TOKEN"),
    }, " ")
  end
  if cap.role == "merge" then
    return table.concat({
      shell_env_assignment("GH_TOKEN", "FKST_GITHUB_MERGE_TOKEN"),
      shell_env_assignment("GITHUB_TOKEN", "FKST_GITHUB_MERGE_TOKEN"),
    }, " ")
  end
  error("github-devloop: unsupported GitHub capability role")
end

local function validate_write_scope(capability)
  local scope = capability and capability.scope or {}
  if capability.write ~= true then
    return
  end
  if tostring(scope.repo or "") == "" then
    error("github-devloop: GitHub write capability requires explicit repo scope")
  end
  if tostring(scope.stage or "") == "" or tostring(scope.stage) == "read-audit" then
    error("github-devloop: GitHub write capability requires explicit stage scope")
  end
end

local function token_split_enabled(opts)
  if opts.github_token_split == "force" then
    return true
  end
  if type(fkst) == "table" and type(fkst.test) == "table" then
    return false
  end
  return true
end

function M.github_capability_exec_opts(opts)
  local command = tostring(opts and opts.cmd or "")
  if shell_words(command)[1] ~= gh_program then
    return opts
  end
  local capability = M.github_command_capability(command)
  validate_write_scope(capability)
  local prepared = {}
  for key, value in pairs(opts or {}) do
    prepared[key] = value
  end
  prepared.github_capability = capability
  prepared.github_write_denied = capability.write ~= true
  if token_split_enabled(prepared) then
    prepared.cmd = M.github_capability_env_prefix(capability) .. " " .. command
  end
  return prepared
end

function M.github_prompt_injection_canary_result(observation)
  local seen = observation or {}
  return {
    secret_leaked = seen.secret_leaked == true,
    unintended_write = seen.unintended_write == true,
    false_success_without_tests = seen.false_success_without_tests == true,
    passed = seen.secret_leaked ~= true
      and seen.unintended_write ~= true
      and seen.false_success_without_tests ~= true,
  }
end
end

return S
