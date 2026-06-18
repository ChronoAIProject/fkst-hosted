local h = require("tests.devloop_helpers")
require("tests.cache_seed_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = tostring(value),
    },
  }, run_opts)
end

local function mock_repo_env()
  h.mock_bot_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_FORK_GRACE_HOURS"', { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_scan_other_authored_unassigned_issue_inside_grace_does_not_fork = function()
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, opts("fork-intake-scan-other-author"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,

  test_scan_other_authored_unassigned_issue_after_grace_raises_fork_request_only = function()
    local run_opts = opts("fork-intake-scan-other-author-stale")
    local seeded = seed_cache(core.fork_first_observed_key("owner/repo", 42, "2026-06-03T01:02:03Z"), now() - (3 * 60 * 60) - 1, run_opts)
    t.eq(seeded.exit_code, 0)
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local request = find_raise(result.raises, "github-proxy.github_issue_create_request").payload
    t.eq(request.external_effect_saga, "fork-and-block")
    t.eq(request.external_effect_step, "create-fork")
    t.eq(request.assignees[1], "fkst-test-bot")
    t.eq(request.parent_comment_target.issue_number, 42)
    t.eq(request.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(request.post_create_blocked_by.external_effect_saga, "fork-and-block")
    t.eq(request.post_create_blocked_by.external_effect_step, "block-original")
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,

  test_scan_stale_open_issue_revalidates_closed_issue_before_fork = function()
    local run_opts = opts("fork-intake-scan-stale-open-author-closed")
    local seeded = seed_cache(core.fork_first_observed_key("owner/repo", 42, "2026-06-03T01:02:03Z"), now() - (3 * 60 * 60) - 1, run_opts)
    t.eq(seeded.exit_code, 0)
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","updatedAt":"2026-06-03T01:02:03Z","state":"CLOSED","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,

  test_scan_other_authored_closed_issue_after_grace_does_not_fork = function()
    local run_opts = opts("fork-intake-scan-other-author-closed")
    local seeded = seed_cache(core.fork_first_observed_key("owner/repo", 42, "2026-06-03T01:02:03Z"), now() - (3 * 60 * 60) - 1, run_opts)
    t.eq(seeded.exit_code, 0)
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"CLOSED","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,

  test_scan_progress_after_old_observation_restarts_fork_grace = function()
    local run_opts = opts("fork-intake-scan-progress-restarts-grace")
    local seeded = seed_cache(core.fork_first_observed_key("owner/repo", 42, "2026-06-03T01:02:03Z"), now() - (3 * 60 * 60) - 1, run_opts)
    t.eq(seeded.exit_code, 0)
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T02:00:00Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","updatedAt":"2026-06-03T02:00:00Z","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,
}
