local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

-- Direct unit coverage for the opt-in stale-tolerant read cache primitive. The
-- integration tests use a unique issue number per scenario with a single read
-- per run, so every gh_exec_cached call is a cold miss that delegates to
-- M.gh_exec with argv -- the hit / expiry / encoding logic is otherwise invisible (the
-- #550/#551 harness-fidelity lesson: a primitive whose value-add never runs in
-- the suite would pass even if broken).

local MULTILINE = '{"title":"cached","body":"line1\\nline2","author":{"login":"fkst-test-bot"},"comments":[]}\n'

local function mock_author_policy_env()
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', { stdout = "fkst-test-bot,ElonSG", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_AUTHORIZED_LOGINS"', { stdout = "trusted-human", stderr = "", exit_code = 0 })
end

return {
  test_second_call_within_ttl_returns_cached_without_re_exec = function()
    mock_author_policy_env()
    local key = "github-devloop/ghread-v2/unittest/owner/repo/9001"
    cache_set(key, "")
    local calls = 0
    local function exec()
      calls = calls + 1
      return { stdout = MULTILINE, exit_code = 0 }
    end
    local first = require("devloop.github_proxy_entity_view").gh_exec_cached({ argv = { "gh", "issue", "view", "9001" } }, key, 90, exec)
    t.eq(first.stdout, MULTILINE)
    t.eq(calls, 1)
    local second = require("devloop.github_proxy_entity_view").gh_exec_cached({ argv = { "gh", "issue", "view", "9001" } }, key, 90, exec)
    t.eq(second.stdout, MULTILINE, "multi-line stdout round-trips intact from cache")
    t.eq(second.cached, true)
    t.eq(calls, 1, "second call within TTL must NOT re-exec gh")
  end,

  test_failed_read_is_not_cached = function()
    mock_author_policy_env()
    local key = "github-devloop/ghread-v2/unittest/owner/repo/9002"
    cache_set(key, "")
    local calls = 0
    local function exec()
      calls = calls + 1
      return { stdout = "", stderr = "boom", exit_code = 1 }
    end
    local first = require("devloop.github_proxy_entity_view").gh_exec_cached({ argv = { "gh", "issue", "view", "9002" } }, key, 90, exec)
    t.eq(first.exit_code, 1)
    local second = require("devloop.github_proxy_entity_view").gh_exec_cached({ argv = { "gh", "issue", "view", "9002" } }, key, 90, exec)
    t.eq(calls, 2, "a non-zero exit read must not be memoized")
  end,

  test_expired_entry_refetches = function()
    mock_author_policy_env()
    local key = "github-devloop/ghread-v2/unittest/owner/repo/9003"
    -- Seed an entry whose expiry epoch (1) is far in the past.
    cache_set(key, "1\nstale-body")
    local calls = 0
    local function exec()
      calls = calls + 1
      return { stdout = '{"title":"fresh","author":{"login":"fkst-test-bot"}}', exit_code = 0 }
    end
    local result = require("devloop.github_proxy_entity_view").gh_exec_cached({ argv = { "gh", "issue", "view", "9003" } }, key, 90, exec)
    t.eq(result.stdout, '{"title":"fresh","author":{"login":"fkst-test-bot"}}', "an expired cache entry must re-fetch, not serve stale")
    t.eq(calls, 1)
  end,

  test_read_cache_key_encodes_variant_and_keeps_repo_path = function()
    t.eq(
      require("devloop.github_proxy_entity_view").gh_read_cache_key("intake-scan", "owner/repo", 42),
      "github-devloop/ghread-v2/intake-scan/owner/repo/42"
    )
  end,
}
