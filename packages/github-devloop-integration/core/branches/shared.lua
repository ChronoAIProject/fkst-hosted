local git_mechanics = require("devloop.git_mechanics")
local base_ids = require("devloop.base_ids")
local strings = require("contract.strings")
local Shared = {}
local forge_validators = require("devloop.forge_validators")

function Shared.install(M)
  local helpers = {}

  local function require_safe_branch(name, branch)
    if not forge_validators.is_git_ref_safe(branch) then
      error("github-devloop: invalid " .. tostring(name))
    end
    return tostring(branch)
  end

  local function require_safe_sha(name, sha)
    if not forge_validators.is_git_sha(sha) then
      error("github-devloop: invalid " .. tostring(name))
    end
    return tostring(sha)
  end

  local function require_safe_repo(repo)
    local value = tostring(repo or "")
    if value == "" or base_ids.safe_repo(value) ~= value then
      error("github-devloop: invalid branch sync repo")
    end
    return value
  end

  local function require_sync_result(result)
    if result ~= "clean" and result ~= "resolved" then
      error("github-devloop: invalid branch sync result")
    end
    return result
  end

  local function runtime_root_path(runtime_root)
    local root = strings.trim(runtime_root)
    if root == "" or root:find("[\r\n]") ~= nil then
      error("github-devloop: invalid FKST_RUNTIME_ROOT")
    end
    return root:gsub("/+$", "")
  end

  local git_handle = nil

  local function git()
    if git_handle == nil then
      if type(exec_argv) ~= "function" then
        error("github-devloop: git adapter requires exec_argv")
      end
      git_handle = require("forge.git").new(exec_argv)
    end
    return git_handle
  end

  local function run_git(fn, label)
    local ok, result_or_error = pcall(fn)
    if ok then
      return result_or_error
    end
    if type(result_or_error) == "table" and result_or_error.result ~= nil then
      return result_or_error.result
    end
    error(tostring(label or "git-adapter operation") .. " failed: " .. tostring(result_or_error))
  end

  local function run_git_ok(fn, label)
    local result = run_git(fn, label)
    if result.exit_code ~= 0 then
      return nil, tostring(label or "git-adapter operation") .. " failed: " .. tostring(result.stderr)
    end
    return result
  end

  function git_mechanics.repo_ref_store_lock_key(repo)
    local key = "github-devloop/git/"
      .. base_ids.safe_repo(require_safe_repo(repo))
      .. "/fetch"
    if not strings.is_path_safe_key(key, M._max_key_len) then
      error("github-devloop: invalid git ref-store lock key")
    end
    return key
  end

  function git_mechanics.with_repo_ref_store_lock(repo, fn)
    return with_lock(git_mechanics.repo_ref_store_lock_key(repo), fn)
  end

  helpers.require_safe_branch = require_safe_branch
  helpers.require_safe_sha = require_safe_sha
  helpers.require_safe_repo = require_safe_repo
  helpers.require_sync_result = require_sync_result
  helpers.runtime_root_path = runtime_root_path
  helpers.git = git
  helpers.run_git = run_git
  helpers.run_git_ok = run_git_ok

  return helpers
end

return Shared
