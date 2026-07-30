local t = fkst.test

-- Per-package configuration must be readable by the package it belongs to, and
-- must be inert in every way that could surprise: absent, malformed, or carrying
-- a name the session author is not allowed to set.

local function exec_returning(value)
  return function(_)
    return { exit_code = 0, stdout = value, stderr = "" }
  end
end

local function fresh()
  local pkg_env = require("devloop.package_env")
  pkg_env._reset()
  return pkg_env
end

local BLOB = '{"github-devloop":{"FKST_DEVLOOP_AUTO_REFINE_MAX":"2"}}'

return {
  test_a_configured_value_is_served = function()
    local pkg_env = fresh()
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning(BLOB)), "2")
  end,

  test_an_absent_variable_falls_through = function()
    -- The control plane may be older than these packages, or the session may
    -- configure nothing. Either way every read must reach the process env, or the
    -- two halves of this feature could not deploy independently.
    local pkg_env = fresh()
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning("")), nil)
  end,

  test_a_name_the_author_may_not_set_is_never_served = function()
    -- The control plane already refuses platform-owned names at parse time. This
    -- is the second, independent gate: a forged or stale blob still cannot
    -- redirect a session's identity or routing.
    local pkg_env = fresh()
    local forged = '{"x":{"FKST_GITHUB_BOT_LOGIN":"attacker[bot]"}}'
    t.eq(pkg_env.get("FKST_GITHUB_BOT_LOGIN", exec_returning(forged)), nil)
    t.eq(pkg_env.is_author_settable("FKST_GITHUB_BOT_LOGIN"), false)
    t.eq(pkg_env.is_author_settable("FKST_SESSION_ID"), false)
    t.eq(pkg_env.is_author_settable("FKST_DEVLOOP_AUTO_REFINE_MAX"), true)
  end,

  test_a_value_that_is_not_a_blob_is_simply_unset = function()
    -- read_env is the funnel EVERY devloop env read passes through, so this must
    -- be total. Throwing on a value that is not a package-env object at all would
    -- take an unrelated read -- and with it the session -- down.
    local pkg_env = fresh()
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning("45")), nil)
    pkg_env._reset()
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning("not json")), nil)
  end,

  test_a_malformed_object_fails_loudly = function()
    -- It claimed to be an object and is not: a real defect, not an absent value.
    local pkg_env = fresh()
    local ok = pcall(pkg_env.get, "FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning('{"broken'))
    t.eq(ok, false)
  end,

  test_the_same_key_under_two_packages_errors = function()
    -- The control plane rejects this, so seeing it here means the blob did not
    -- come from it. Picking a winner would make one package's configuration
    -- vanish with no signal.
    local pkg_env = fresh()
    local conflicting = '{"a":{"FKST_DEVLOOP_TEST_COMMAND":"x"},"b":{"FKST_DEVLOOP_TEST_COMMAND":"y"}}'
    local ok = pcall(pkg_env.get, "FKST_DEVLOOP_TEST_COMMAND", exec_returning(conflicting))
    t.eq(ok, false)
  end,

  test_a_json_array_is_unset = function()
    local pkg_env = fresh()
    -- A JSON array is not an object, and does not start with `{`, so it is unset.
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning("[1,2]")), nil)
  end,

  test_a_non_object_block_errors = function()
    local pkg_env = fresh()
    t.eq(pcall(pkg_env.get, "FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning('{"pkg":"oops"}')), false)
  end,

  test_a_non_string_value_errors = function()
    -- The wire form is strings only; a number here means the blob was not
    -- produced by the control plane's serializer.
    local pkg_env = fresh()
    t.eq(
      pcall(pkg_env.get, "FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning('{"pkg":{"FKST_DEVLOOP_AUTO_REFINE_MAX":2}}')),
      false
    )
  end,

  test_an_empty_value_falls_through = function()
    -- An empty string is "not configured", not "configured as empty": otherwise a
    -- blank line in the trigger would silently blank a package's default.
    local pkg_env = fresh()
    local blank = '{"github-devloop":{"FKST_DEVLOOP_AUTO_REFINE_MAX":""}}'
    t.eq(pkg_env.get("FKST_DEVLOOP_AUTO_REFINE_MAX", exec_returning(blank)), nil)
  end,
}
