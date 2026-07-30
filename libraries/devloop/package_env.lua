-- Per-package configuration delivered by the control plane.
--
-- The control plane renders the session's effective package configuration --
-- the trigger's `### Package Env` merged with the manifest's `packageEnv` -- as
-- ONE variable, `FKST_SESSION_PACKAGE_ENV_JSON`, shaped `{package: {KEY: value}}`.
-- This module turns that into a flat name -> value lookup that `config.read_env`
-- consults BEFORE the process environment.
--
-- `local C`, not `local M`: the godlib ratchet counts `M.<sym>` writes across
-- `libraries/devloop/**` against a shrink-only baseline, so a conventional
-- `local M = {}` module fails CI outright.
--
-- The variable is OPTIONAL by design. An older control plane does not set it, and
-- an unconfigured session renders no key at all, so every read must fall through
-- to the process environment. That is what makes the two halves of this feature
-- deployable in either order.
local C = {}

local env = require("workflow.env")

-- Names a session author may set from `### Package Env`. Everything else in the
-- blob is IGNORED rather than trusted: the control plane already refuses
-- platform-owned names at parse time, and this is the second, independent gate
-- so a forged or stale blob still cannot redirect a session's identity, routing,
-- or credentials.
local author_settable_env = {
  FKST_DEVLOOP_AUTO_REFINE_MAX = true,
  FKST_DEVLOOP_ROLLUP_MERGE = true,
  FKST_DEVLOOP_ROLLUP_AUTOFIX = true,
  FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = true,
  FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES = true,
  FKST_DEVLOOP_TEST_COMMAND = true,
  FKST_DEVLOOP_MAX_INFLIGHT = true,
  FKST_DEVLOOP_FORK_GRACE_HOURS = true,
}

C.PACKAGE_ENV_VAR = "FKST_SESSION_PACKAGE_ENV_JSON"

function C.is_author_settable(name)
  return author_settable_env[tostring(name)] == true
end

-- Flatten `{package: {KEY: value}}` into `{KEY: value}`.
--
-- The control plane rejects the same key under two packages, so a conflict here
-- means the blob did not come from it. Erroring beats picking a winner: a silent
-- choice would make one package's configuration vanish with no signal.
local function flatten(decoded)
  local flat, owner = {}, {}
  if type(decoded) ~= "table" then
    error("devloop.package_env: package env is not an object")
  end
  for package, keys in pairs(decoded) do
    if type(keys) ~= "table" then
      error("devloop.package_env: package env block for " .. tostring(package) .. " is not an object")
    end
    for key, value in pairs(keys) do
      if type(value) ~= "string" then
        error("devloop.package_env: " .. tostring(key) .. " is not a string")
      end
      if owner[key] ~= nil and owner[key] ~= package then
        error(
          "devloop.package_env: "
            .. tostring(key)
            .. " is set by both "
            .. tostring(owner[key])
            .. " and "
            .. tostring(package)
        )
      end
      owner[key] = package
      flat[key] = value
    end
  end
  return flat
end

-- Memoized per process: the value is fixed for a session's lifetime, and every
-- read otherwise costs a subprocess.
local cached = nil

local read_raw = env.read_env(function(name)
  if name ~= C.PACKAGE_ENV_VAR then
    error("devloop.package_env: unexpected env name")
  end
  return 'printf %s "$' .. name .. '"'
end)

function C.load(exec)
  if cached ~= nil then
    return cached
  end
  local raw = read_raw(C.PACKAGE_ENV_VAR, exec)
  raw = tostring(raw or ""):gsub("^%s+", ""):gsub("%s+$", "")
  -- Only text that actually looks like a package-env object is treated as one.
  -- `read_env` is the funnel EVERY devloop env read passes through, so this
  -- function must be total: throwing on a value that is not a blob at all would
  -- take an unrelated read -- and with it the session -- down.
  if raw == "" or raw:sub(1, 1) ~= "{" then
    cached = {}
    return cached
  end
  local ok, decoded = pcall(json.decode, raw)
  if not ok or type(decoded) ~= "table" then
    -- It claimed to be an object and is not. That is a real defect rather than an
    -- absent value, so it is loud: silently ignoring it would run the session on
    -- defaults while the author believes their configuration applied.
    error("devloop.package_env: package env is not valid JSON")
  end
  cached = flatten(decoded)
  return cached
end

-- The configured value for `name`, or nil to fall through to the process env.
function C.get(name, exec)
  if not C.is_author_settable(name) then
    return nil
  end
  local value = C.load(exec)[tostring(name)]
  if value == nil or value == "" then
    return nil
  end
  return value
end

-- Test seam: drop the memo so a test can vary the blob within one process.
function C._reset()
  cached = nil
end

return C
