-- devloop library behavior tests are hosted in github-devloop because the
-- engine test runner only scans package tests and department tests.
local devloop_base = require("devloop.base")
local t = fkst.test

local expected_write_error =
  "github-devloop: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1 (trusted_bot_login)"

local function call_trusted_bot_login(env_login, write_mode, configured_login)
  devloop_base.configure_trusted_bot_login(nil)
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
    stdout = env_login or "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
    stdout = write_mode or "",
    stderr = "",
    exit_code = 0,
  })
  if configured_login ~= nil then
    devloop_base.configure_trusted_bot_login(configured_login)
  end

  local ok, result = pcall(devloop_base.trusted_bot_login)
  local configured = devloop_base.configured_trusted_bot_login()
  devloop_base.configure_trusted_bot_login(nil)
  return ok, result, configured
end

return {
  test_write_mode_without_bot_login_fails_closed = function()
    local ok, err, configured = call_trusted_bot_login("", "1")
    t.eq(ok, false)
    t.is_true(tostring(err):find(expected_write_error, 1, true) ~= nil)
    t.is_nil(configured)
  end,

  test_write_mode_with_whitespace_bot_login_fails_closed = function()
    local ok, err, configured = call_trusted_bot_login("   ", "1")
    t.eq(ok, false)
    t.is_true(tostring(err):find(expected_write_error, 1, true) ~= nil)
    t.is_nil(configured)
  end,

  test_write_mode_with_suffix_only_bot_login_fails_closed = function()
    local ok, err, configured = call_trusted_bot_login("[bot]", "1")
    t.eq(ok, false)
    t.is_true(tostring(err):find(expected_write_error, 1, true) ~= nil)
    t.is_nil(configured)
  end,

  test_write_mode_lazy_loads_bot_login_from_env = function()
    local ok, login, configured = call_trusted_bot_login("production-bot", "1")
    t.eq(ok, true)
    t.eq(login, "production-bot")
    t.eq(configured, "production-bot")
  end,

  test_dry_run_without_bot_login_uses_test_default = function()
    local ok, login, configured = call_trusted_bot_login("", "")
    t.eq(ok, true)
    t.eq(login, devloop_base._test_bot_login)
    t.is_nil(configured)
  end,

  test_explicit_bot_login_wins_in_every_posture = function()
    local real_ok, real_login, real_configured = call_trusted_bot_login("", "1", "injected-bot")
    t.eq(real_ok, true)
    t.eq(real_login, "injected-bot")
    t.eq(real_configured, "injected-bot")

    local dry_ok, dry_login, dry_configured = call_trusted_bot_login("", "", "injected-bot")
    t.eq(dry_ok, true)
    t.eq(dry_login, "injected-bot")
    t.eq(dry_configured, "injected-bot")
  end,
}
