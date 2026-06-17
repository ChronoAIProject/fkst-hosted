local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local render_comment = h.render_comment
local entity_read_mocks = require("tests.entity_read_mock_helpers")
require("tests.cache_seed_helpers")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function run_liveness_scan(name, run_opts)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = { schema = "github-devloop.tick.v1" },
    ts = "2026-06-03T01:32:03Z",
  }, run_opts or opts(name or "lineage-watchdog-liveness-scan"))
end

local function run_timeout_reconcile(payload, name)
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_timeout_reconcile",
    payload = payload,
  }, opts(name or "lineage-watchdog-timeout-reconcile"))
end

local function mock_repo()
  t.mock_command(core.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_list()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = '[{"number":42,"state":"open","updated_at":"2026-06-03T01:02:03Z"}]' .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_empty_pr_list()
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function seed_cache(entry, run_opts)
  return t.run_department("tests/cache_seed_helpers.lua", {
    queue = "cache_seed",
    payload = entry,
  }, run_opts or opts("lineage-watchdog-cache-seed"))
end

local function encode_json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\b", "\\b")
    :gsub("\f", "\\f")
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
end

local function seed_cached_pr(comments, run_opts)
  seed_cache({
    key = core.entity_view_cache_key(repo, "pr", 7),
    value = '{"updated_at":"2026-06-04T01:02:03Z","producer":"observe_pr","stdout":"'
    .. encode_json_string(entity_read_mocks.pr_view_stdout({
      repo = repo,
      number = 7,
      head = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
      base_branch = "dev",
      state = "OPEN",
      updated_at = "2026-06-04T01:02:03Z",
      comments = comments,
    }))
    .. '"}',
  }, run_opts or opts("lineage-watchdog-pr-cache-seed"))
end

local function count_pr_view_fetches()
  local count = 0
  local expected = h.argv_rendered(core.gh_pr_view_observe_cmd(repo, 7))
  for _, call in ipairs(t.command_calls()) do
    if h.argv_rendered(tostring(call.rendered or "")) == expected then
      count = count + 1
    end
  end
  return count
end

local function mock_issue(comments)
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 42,
    title = "Issue 42",
    body = "",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled", "fkst-dev:pr-open" },
    comments = comments,
    assignees = { "fkst-test-bot" },
    times = 1,
  })
end

local function mock_issue_reconcile(comments)
  t.mock_command(core.gh_issue_view_loop_cmd(repo, 42), {
    stdout = string.format(
      '{"title":"Issue 42","updatedAt":"2026-06-03T01:02:03Z","labels":[{"name":"fkst-dev:enabled"},{"name":"fkst-dev:pr-open"}],"comments":[%s],"state":"OPEN"}\n',
      table.concat((function()
        local rendered = {}
        for _, comment in ipairs(comments or {}) do
          table.insert(rendered, render_comment(comment))
        end
        return rendered
      end)(), ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_linked_pr(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, render_comment(comment))
  end
  entity_read_mocks.mock_pr_view_raw_selector(t, { repo = repo, number = 7 }, entity_read_mocks.pr_origin_selector, {
    stdout = string.format(
      '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-04T01:02:03Z","comments":[%s]}\n',
      table.concat(rendered, ",")
    ),
    stderr = "",
    exit_code = 0,
  }, 1)
end

local function issue_pr_open_comments(state_version)
  return {
    {
      body = core.state_marker(proposal_id, "pr-open", state_version or version),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:00:00Z",
    },
    {
      body = core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:01:00Z",
    },
  }
end

local function pr_reviewing_comments()
  local review_version = version .. "/review-loop/1"
  return {
    {
      body = core.pr_origin_marker(proposal_id, 42, "devloop-owner-repo-42-01HY", version, "dev"),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:02:00Z",
    },
    {
      body = core.state_marker(proposal_id, "reviewing", review_version),
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T00:03:00Z",
    },
    {
      body = core.review_converge_round_marker(
        core.pr_review_proposal_id(repo, 7, review_version, "def456"),
        proposal_id,
        core.safe_version_segment(review_version),
        "def456",
        core.source_ref_digest(core.pr_source_ref(repo, 7)),
        1,
        core._dedup_key({ "review", "round", proposal_id, review_version, "def456" }),
        "continue",
        "digest",
        {}
      ),
      author_login = "fkst-test-bot",
      created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 60),
    },
  }
end

return {
  test_issue_surface_watchdog_uses_linked_pr_state_before_timeout = function()
    local run_opts = opts("lineage-watchdog-issue-surface-sees-pr-state")
    mock_repo()
    mock_issue_list()
    mock_issue(issue_pr_open_comments())
    seed_cached_pr(pr_reviewing_comments(), run_opts)
    mock_empty_pr_list()

    local result = run_liveness_scan("lineage-watchdog-issue-surface-sees-pr-state", run_opts)
    t.eq(result.exit_code, 0)
    t.eq(count_pr_view_fetches(), 0)
    t.eq(find_raise(result.raises, "devloop_timeout_reconcile"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    local changed = find_raise(result.raises, "github-proxy.github_entity_changed")
    t.is_true(changed ~= nil)
    t.eq(changed.payload.type, "issue")
  end,

  test_issue_surface_watchdog_still_times_out_genuinely_stuck_pr_open = function()
    local timeout_version = version .. "/timeout/pr-open/3"
    local run_opts = opts("lineage-watchdog-stuck-pr-open-times-out")
    mock_repo()
    mock_issue_list()
    mock_issue(issue_pr_open_comments(timeout_version))
    seed_cached_pr({}, run_opts)
    mock_empty_pr_list()

    local result = run_liveness_scan("lineage-watchdog-stuck-pr-open-times-out", run_opts)
    t.eq(result.exit_code, 0)
    t.eq(count_pr_view_fetches(), 0)
    local reconcile = find_raise(result.raises, "devloop_timeout_reconcile")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.state, "pr-open")
    t.eq(reconcile.payload.issue_version, timeout_version)
    t.eq(reconcile.payload.round, 3)
    t.eq(reconcile.payload.source_ref.ref, "owner/repo#issue/42")
  end,

  test_issue_surface_watchdog_deadline_defers_when_pr_surface_uncached = function()
    local timeout_version = version .. "/timeout/pr-open/3"
    local run_opts = opts("lineage-watchdog-uncached-pr-surface-defers")
    mock_repo()
    mock_issue_list()
    mock_issue(issue_pr_open_comments(timeout_version))
    mock_empty_pr_list()

    local result = run_liveness_scan("lineage-watchdog-uncached-pr-surface-defers", run_opts)
    t.eq(result.exit_code, 0)
    t.eq(count_pr_view_fetches(), 0)
    t.eq(find_raise(result.raises, "devloop_timeout_reconcile"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_entity_changed"), nil)
  end,

  test_issue_targeted_timeout_reconcile_noops_when_pr_stream_advanced = function()
    local timeout_version = version .. "/timeout/pr-open/3"
    local payload = core.build_devloop_timeout_reconcile_payload(
      core.restart_transition_row("pr-open"),
      {
        state = "pr-open",
        version = timeout_version,
        proposal_id = proposal_id,
      },
      proposal_id,
      core.issue_source_ref(repo, 42),
      3
    )
    mock_issue_reconcile(issue_pr_open_comments(timeout_version))
    mock_linked_pr(pr_reviewing_comments())

    local result = run_timeout_reconcile(payload, "lineage-watchdog-reconcile-advanced-noop")
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,

  test_pr_gone_timeout_reconcile_falls_back_to_issue_surface_and_terminalizes = function()
    local timeout_version = version .. "/timeout/pr-open/3"
    local payload = core.build_devloop_timeout_reconcile_payload(
      core.restart_transition_row("pr-open"),
      {
        state = "pr-open",
        version = timeout_version,
        proposal_id = proposal_id,
      },
      proposal_id,
      core.pr_source_ref(repo, 7),
      3
    )
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = repo,
      number = 42,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
      times = 1,
    })
    entity_read_mocks.mock_issue_view_selector(t, {
      repo = repo,
      number = 42,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    }, "assignees,author", 1)
    t.mock_command(core.gh_pr_view_origin_cmd(repo, 7), {
      stdout = "",
      stderr = "GraphQL: Could not resolve to a PullRequest with the number of 7. (repository.pullRequest) 404 not found",
      exit_code = 1,
    })
    t.mock_command(core.gh_pr_view_origin_cmd(repo, 7), {
      stdout = "",
      stderr = "GraphQL: Could not resolve to a PullRequest with the number of 7. (repository.pullRequest) 404 not found",
      exit_code = 1,
    })
    mock_issue_reconcile(issue_pr_open_comments(timeout_version))

    local result = run_timeout_reconcile(payload, "lineage-watchdog-pr-gone-timeout-reconcile")
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.eq(tonumber(comment.payload.issue_number), 42)
    t.is_true(comment.payload.body:find('state="blocked"', 1, true) ~= nil)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label ~= nil)
  end,
}
