local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function opts(name, write_mode)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_WRITE = write_mode or "1",
      FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES = "30",
    },
  }
end

local function event(extra)
  local payload = core.rollup_ready_payload("owner/repo", "dev", "integration/dev", 9, "def456")
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

local function observe_clean()
  return {
    schema_version = 1,
    generated_at_ms = now() * 1000,
    truncated = { deliveries = false, dead_letters = false },
    dead_letters = json.decode("[]"),
  }
end

local function run_merge(payload, run_opts)
  return h.run_department("departments/rollup_merge/main.lua", {
    queue = "devloop_rollup_ready",
    payload = payload,
  }, run_opts or opts("rollup-merge"))
end

local function mock_write_mode(value)
  for _ = 1, 2 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = value or "1", stderr = "", exit_code = 0 })
  end
end

local function mock_soak_minutes(value)
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES"', {
    stdout = tostring(value or "30"),
    stderr = "",
    exit_code = 0,
  }, 2)
end

local function mock_pr(head_sha, base, rollup_state, rollup_conclusion, mergeable, merge_state, state, merged_at, comments)
  local rendered_comments = entity_read_mocks.view_comments_json(comments or {})
  entity_read_mocks.mock_pr_view_raw_selector(t, { number = 9 }, entity_read_mocks.pr_merge_selector, {
    stdout = string.format(
      '{"headRefName":"integration/dev","headRefOid":"%s","baseRefName":"%s","baseRefOid":"abc123","state":"%s","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"name":"ci","state":"%s","conclusion":"%s","headSha":"%s","completedAt":"%s"}]}\n',
      h.json_string(head_sha or "def456"),
      h.json_string(base or "dev"),
      h.json_string(state or "OPEN"),
      h.json_string(merged_at or ""),
      rendered_comments,
      h.json_string(mergeable or "MERGEABLE"),
      h.json_string(merge_state or "CLEAN"),
      h.json_string(rollup_state or "COMPLETED"),
      h.json_string(rollup_conclusion or "SUCCESS"),
      h.json_string(head_sha or "def456"),
      h.json_string("2026-06-03T01:30:00Z")
    ),
  })
end

local function observe_sample_state_marker(head_sha, status, first_clean_observed_at_ms, sampled_at_ms)
  return '<!-- fkst:github-devloop-integration:rollup-observe-sample:v1 head_sha="'
    .. tostring(head_sha or "def456")
    .. '" status="'
    .. tostring(status or "clean")
    .. '" first_clean_observed_at_ms="'
    .. tostring(first_clean_observed_at_ms)
    .. '" sampled_at_ms="'
    .. tostring(sampled_at_ms)
    .. '" -->'
end

local function observe_sample_comment(head_sha, status, age_seconds, author_login)
  local sampled_at_ms = (now() - tonumber(age_seconds or 0)) * 1000
  return {
    body = observe_sample_state_marker(head_sha, status, sampled_at_ms, sampled_at_ms),
    author_login = author_login or core._test_bot_login,
    created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - tonumber(age_seconds or 0)),
  }
end

local function mature_clean_sample(head_sha)
  return observe_sample_comment(head_sha or "def456", "clean", 31 * 60)
end

local function fresh_clean_sample(head_sha)
  return observe_sample_comment(head_sha or "def456", "clean", 60)
end

local function mock_integration_head(head_sha, committed_at_seconds)
  local selected_head = tostring(head_sha or "def456")
  t.mock_command("git fetch origin integration/dev", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("FETCH_HEAD", { stdout = selected_head .. "\n", stderr = "", exit_code = 0 })
end

local function mock_runtime_gate(snapshot, head_sha, committed_at_seconds)
  t.mock_observe(snapshot or observe_clean())
  mock_soak_minutes("30")
  mock_integration_head(head_sha or "def456", committed_at_seconds)
end

local function mock_merge_command(head_sha, result)
  local command_result = result or { stdout = "merged\n", stderr = "", exit_code = 0 }
  t.mock_command("gh pr merge '9' --repo 'owner/repo' --merge --match-head-commit '" .. tostring(head_sha or "def456") .. "'", command_result)
end

local function mock_successful_merge()
  mock_write_mode("1")
  mock_pr()
  mock_runtime_gate(observe_clean())
  mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
  mock_merge_command()
  mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
end

return {
  test_rollup_merge_green_mergeable_identity_match_merges = function()
    mock_successful_merge()
    local result = run_merge(event(), opts("rollup-merge-success", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
    t.is_true(h.has_call("--match-head-commit 'def456'"))
  end,

  test_rollup_merge_red_or_pending_ci_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "FAILURE")
    h.mock_required_check_runs_for("def456", "success")
    local red = run_merge(event(), opts("rollup-merge-red", "1"))
    t.eq(red.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)

    mock_write_mode("1")
    mock_pr("def456", "dev", "IN_PROGRESS", "")
    local pending = run_merge(event(), opts("rollup-merge-pending", "1"))
    t.eq(pending.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_neutral_ci_does_not_merge = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "NEUTRAL")
    h.mock_required_check_runs_for("def456", "success")
    local result = run_merge(event(), opts("rollup-merge-neutral", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_unmergeable_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "CONFLICTING", "DIRTY")
    local result = run_merge(event(), opts("rollup-merge-unmergeable", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_uses_fresh_current_head_for_match_commit = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_runtime_gate(observe_clean(), "aaaa1111")
    mock_pr("aaaa1111", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("aaaa1111") })
    mock_merge_command("aaaa1111")
    mock_pr("aaaa1111", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-fresh-head", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
    t.is_true(h.has_call("--match-head-commit 'aaaa1111'"))
    t.eq(h.count_calls("--match-head-commit 'def456'"), 0)
  end,

  test_rollup_merge_retries_head_modified_with_fresh_head = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_runtime_gate(observe_clean(), "def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_merge_command("def456", {
      stdout = "",
      stderr = "GraphQL: Head branch was modified. Review and try the merge again. (mergePullRequest)",
      exit_code = 1,
    })
    mock_runtime_gate(observe_clean(), "aaaa1111")
    mock_pr("aaaa1111", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("aaaa1111") })
    mock_merge_command("aaaa1111")
    mock_pr("aaaa1111", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-head-modified-retry", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 2)
    t.eq(h.count_calls("--match-head-commit 'def456'"), 1)
    t.eq(h.count_calls("--match-head-commit 'aaaa1111'"), 1)
  end,

  test_rollup_merge_does_not_retry_other_merge_errors = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_runtime_gate(observe_clean(), "def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_merge_command("def456", {
      stdout = "",
      stderr = "GraphQL: Repository rule violation",
      exit_code = 1,
    })
    local result = run_merge(event(), opts("rollup-merge-non-head-error", "1"))
    t.is_true(result.exit_code ~= 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_clean_observe_and_soaked_head_merges = function()
    mock_successful_merge()
    local result = run_merge(event(), opts("rollup-merge-clean-soaked", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_uses_env_bot_identity_for_mature_soak_sample = function()
    local prod_bot = "prod-bot"
    h.mock_author_policy_configure(core._test_bot_login)
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = prod_bot, stderr = "", exit_code = 0 })
    mock_write_mode("1")
    mock_pr("def456")
    mock_runtime_gate(observe_clean(), "def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", {
      observe_sample_comment("def456", "clean", 31 * 60, prod_bot),
    })
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local run_opts = opts("rollup-merge-prod-bot-soak", "1")
    run_opts.env.FKST_GITHUB_BOT_LOGIN = prod_bot
    local result = t.run_department("departments/rollup_merge/main.lua", {
      queue = "devloop_rollup_ready",
      payload = event(),
    }, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_dirty_observe_holds_without_merge = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_runtime_gate({
      schema_version = 1,
      generated_at_ms = now() * 1000,
      truncated = { deliveries = false, dead_letters = false },
      queues = {
        { queue = "devloop_ready", ready = 0, leased = 0, retry = 0, dlq = 1 },
      },
      dead_letters = {
        { delivery_id = "dead-1", queue = "devloop_ready", dead_at_ms = now() * 1000 - 1000 },
      },
    }, "def456")
    local result = run_merge(event(), opts("rollup-merge-runtime-dirty", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_stale_dead_letter_audit_still_merges = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_runtime_gate({
      schema_version = 1,
      generated_at_ms = now() * 1000,
      truncated = { deliveries = false, dead_letters = false },
      queues = {
        { queue = "devloop_ready", ready = 0, leased = 0, retry = 0, dlq = 1 },
      },
      dead_letters = {
        {
          delivery_id = "dead-1",
          queue = "devloop_ready",
          dead_at_ms = now() * 1000 - 32 * 60 * 1000,
          permanent = true,
          replayable = false,
        },
      },
    }, "def456")
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-stale-dead-letter-audit", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_missing_observe_holds_fail_closed = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_soak_minutes("30")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    local result = run_merge(event(), opts("rollup-merge-observe-missing", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_malformed_observe_holds_fail_closed = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_runtime_gate("not a snapshot", "def456")
    local result = run_merge(event(), opts("rollup-merge-observe-malformed", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_clean_observe_with_only_fresh_sample_holds_under_soak = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { fresh_clean_sample("def456") })
    mock_runtime_gate(observe_clean(), "def456", now() - 60 * 60)
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-fresh-head-holds", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_dirty_sample_newer_than_clean_resets_soak_window = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", {
      observe_sample_comment("def456", "clean", 40 * 60),
      observe_sample_comment("def456", "dirty", 60),
    })
    mock_runtime_gate(observe_clean(), "def456", now() - 60 * 60)
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-dirty-sample-reset", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_old_head_sample_does_not_soak_current_head = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", {
      mature_clean_sample("aaaa1111"),
    })
    mock_runtime_gate(observe_clean(), "def456", now() - 60 * 60)
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-old-head-sample", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_old_commit_newly_head_without_mature_sample_holds = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456")
    mock_runtime_gate(observe_clean(), "def456", now() - 60 * 60)
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-old-commit-no-sample", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_head_change_resets_soak_window = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_runtime_gate(observe_clean(), "aaaa1111", now() - 60 * 60)
    local result = run_merge(event(), opts("rollup-merge-head-change-resets-soak", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_same_sha_return_holds_until_current_head_soak_restarts = function()
    local mature_a = observe_sample_comment("def456", "clean", 40 * 60)
    local after_b_then_returned_to_a = {
      body = observe_sample_state_marker("def456", "clean", now() * 1000, now() * 1000),
      author_login = core._test_bot_login,
      created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
    }

    mock_write_mode("1")
    mock_pr("def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", {
      mature_a,
      after_b_then_returned_to_a,
    })
    mock_runtime_gate(observe_clean(), "def456", now() - 60 * 60)
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-same-sha-return-holds", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_expected_transient_observe_still_merges = function()
    mock_write_mode("1")
    mock_pr("def456")
    mock_runtime_gate({
      schema_version = 1,
      generated_at_ms = now() * 1000,
      truncated = { deliveries = false, dead_letters = false },
      entities = {
        {
          entity = "github-devloop/issue/owner/repo/623",
          events = {
            { queue = "devloop_ready", outcome = "retry-pending", error_class = "retry-pending" },
            { queue = "devloop_merge_ready", error_class = "marker-lag" },
          },
        },
      },
      queues = {
        { queue = "devloop_ready", ready = 0, leased = 0, retry = 1, dlq = 0 },
      },
      dead_letters = json.decode("[]"),
    }, "def456")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "OPEN", "", { mature_clean_sample("def456") })
    mock_merge_command()
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
    local result = run_merge(event(), opts("rollup-merge-transients-clean", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_base_mismatch_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "main")
    local result = run_merge(event(), opts("rollup-merge-base-mismatch", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_does_not_require_issue_review_markers = function()
    mock_successful_merge()
    local result = run_merge(event(), opts("rollup-merge-no-markers", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh issue comment"), 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_dry_run_never_merges = function()
    mock_write_mode("")
    local result = run_merge(event(), opts("rollup-merge-dry-run", ""))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_malformed_owned_queue_payload_fails_closed = function()
    local payload = event({ schema = "github-devloop.bad.v1" })
    local result = run_merge(payload, opts("rollup-merge-malformed", "1"))
    t.is_true(result.exit_code ~= 0)
    t.is_true(tostring(result.error or ""):find(
      "github-devloop: rollup-ready-payload-invalid: rollup_merge unsupported devloop_rollup_ready payload",
      1,
      true
    ) ~= nil)
    t.is_true(tostring(result.error or ""):find(payload.dedup_key, 1, true) ~= nil)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,
}
