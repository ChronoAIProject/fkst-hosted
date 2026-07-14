local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local github_fake = require("forge.github_fake")
local git_fake = require("forge.git_fake")
local testing = require("testkit.testing")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local devloop_state = require("devloop.state")
local sync_conflict = require("departments.sync_conflict.main")

local t = h.t
local core = h.core

local REPO = "owner/repo"
local ISSUE_NUMBER = 2275
local PR_NUMBER = 2281
local BOT = "fkst-test-bot"
local INTEGRATION_BRANCH = "integration/dev"
local INTEGRATION_SHA = "1111111111111111111111111111111111111111"
local PR_HEAD_SHA = "2222222222222222222222222222222222222222"
local ADVANCED_SHA = "3333333333333333333333333333333333333333"
local PROPOSAL_ID = "github-devloop/issue/owner/repo/2275"
local ROOT_VERSION = "ready/consensus-github-devloop/issue/owner/repo/2275/2026-07-14T01-02-03Z"
local PR_PROPOSAL_ID = entity_lib.pr_proposal_id(REPO, PR_NUMBER)
local ORIGINAL_BRANCH = devloop_base.implement_branch(REPO, ISSUE_NUMBER, ROOT_VERSION)
local REPLACEMENT_VERSION = ROOT_VERSION .. "/reimplement/1"
local REPLACEMENT_BRANCH = devloop_base.implement_branch(REPO, ISSUE_NUMBER, REPLACEMENT_VERSION)
local UNMERGED = "100644 abcdef 1\tpackages/github-devloop/core.lua\n"

local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local result = {}
  for key, field in pairs(value) do
    result[copy(key)] = copy(field)
  end
  return result
end

local function comment(body, created_at, author)
  return {
    id = tostring(created_at or body):gsub("[^%w]", "_"):sub(1, 60),
    body = body,
    author_login = author or BOT,
    created_at = created_at or "2026-07-14T01:03:00Z",
  }
end

local function json_string(value)
  return h.json_string(tostring(value or ""))
end

local function render_comment(value)
  return table.concat({
    '{"id":"', json_string(value.id),
    '","body":"', json_string(value.body),
    '","author":{"login":"', json_string(value.author_login),
    '"},"createdAt":"', json_string(value.created_at), '"}',
  })
end

local function render_pr(pr)
  local comments = {}
  for _, value in ipairs(pr.comments or {}) do
    table.insert(comments, render_comment(value))
  end
  return table.concat({
    '{"headRefName":"', json_string(pr.head_ref_name),
    '","headRefOid":"', json_string(pr.head_sha),
    '","baseRefName":"', json_string(pr.base_ref_name),
    '","state":"', json_string(pr.state),
    '","updatedAt":"2026-07-14T01:04:00Z"',
    ',"isDraft":', pr.is_draft and "true" or "false",
    ',"comments":[', table.concat(comments, ","), "]",
    ',"labels":[]',
    ',"headRepository":{"nameWithOwner":"', json_string(pr.head_repository), '"}',
    ',"headRepositoryOwner":{"login":"', json_string((pr.head_repository or ""):match("^([^/]+)")), '"}',
    ',"isCrossRepository":', pr.is_cross_repository and "true" or "false",
    ',"mergeable":"CONFLICTING","mergeStateStatus":"DIRTY","statusCheckRollup":[]}',
  })
end

local function parent_comments(version, delegation)
  return {
    comment(core.state_marker(PROPOSAL_ID, "awaiting-pr", version), "2026-07-14T01:03:00Z"),
    comment(m_builders.pr_delegation_marker(
      PROPOSAL_ID,
      PR_PROPOSAL_ID,
      PR_NUMBER,
      version,
      delegation or "g1"
    ), "2026-07-14T01:03:01Z"),
  }
end

local function baseline(overrides)
  local fields = overrides or {}
  local impl_version = fields.impl_version or ROOT_VERSION
  local branch = fields.branch or ORIGINAL_BRANCH
  local origin = fields.origin
  if origin == nil and fields.omit_origin ~= true then
    origin = comment(m_builders.pr_origin_marker(
      fields.origin_proposal_id or PROPOSAL_ID,
      fields.origin_issue_number or ISSUE_NUMBER,
      fields.origin_branch or branch,
      impl_version,
      fields.origin_base_branch or INTEGRATION_BRANCH
    ), "2026-07-14T01:02:00Z", fields.origin_author)
  end
  local pr_comments = {}
  if origin ~= nil then
    table.insert(pr_comments, origin)
  end
  for _, value in ipairs(fields.extra_pr_comments or {}) do
    table.insert(pr_comments, value)
  end
  local current_parent_comments = fields.parent_comments
    or parent_comments(impl_version, fields.delegation)
  return {
    expect_initial_valid = fields.expect_initial_valid == true or next(fields) == nil,
    pr = {
      number = PR_NUMBER,
      head_ref_name = fields.pr_head_ref_name or branch,
      head_sha = fields.pr_head_sha or PR_HEAD_SHA,
      base_ref_name = fields.pr_base_ref_name or INTEGRATION_BRANCH,
      state = fields.pr_state or "OPEN",
      is_draft = fields.pr_is_draft == true,
      comments = pr_comments,
      head_repository = fields.head_repository or REPO,
      is_cross_repository = fields.is_cross_repository == true,
    },
    parent = {
      repo = REPO,
      number = ISSUE_NUMBER,
      title = "Abandon exhausted original PR and reimplement",
      body = "Issue body",
      state = fields.parent_state or "OPEN",
      updated_at = "2026-07-14T01:03:02Z",
      labels = fields.parent_labels or { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
      comments = current_parent_comments,
      assignees = fields.assignees or { BOT },
      author_login = BOT,
    },
    event = {
      schema = "github-devloop.v1",
      repo = REPO,
      upstream_branch = INTEGRATION_BRANCH,
      integration_branch = fields.event_branch or branch,
      upstream_sha = INTEGRATION_SHA,
      integration_sha = fields.event_head_sha or PR_HEAD_SHA,
      dedup_key = core.pr_freshness_dedup_key(REPO, fields.event_branch or branch, INTEGRATION_SHA),
      source_ref = fields.source_ref or core.pr_freshness_source_ref(REPO, PR_NUMBER),
    },
    final_mutation = fields.final_mutation,
  }
end

local function record(model, kind, fields)
  local row = fields or {}
  row.kind = kind
  table.insert(model.writes, row)
end

local function make_git(fixture)
  local model = git_fake.model({})
  local git = git_fake.new(model)
  local upstream_reads = 0
  local unmerged_reads = 0
  function git.fetch_branch(remote, branch, timeout)
    record(model, "fetch", { remote = remote, branch = branch, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.remote_branch_head(_remote, branch, _timeout)
    if branch == INTEGRATION_BRANCH then
      upstream_reads = upstream_reads + 1
      if fixture.final_mutation == "integration-head" and upstream_reads >= 2 then
        return { stdout = ADVANCED_SHA .. "\n", stderr = "", exit_code = 0 }
      end
      return { stdout = INTEGRATION_SHA .. "\n", stderr = "", exit_code = 0 }
    end
    return { stdout = PR_HEAD_SHA .. "\n", stderr = "", exit_code = 0 }
  end
  function git.is_ancestor(ancestor, descendant, timeout)
    record(model, "is_ancestor", { ancestor = ancestor, descendant = descendant, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 1 }
  end
  function git.worktree_add_detached(worktree, sha, timeout)
    record(model, "worktree_add", { worktree = worktree, sha = sha, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.merge_no_ff(worktree, sha, timeout)
    record(model, "merge", { worktree = worktree, sha = sha, timeout = timeout })
    return { stdout = "", stderr = "conflict", exit_code = 1 }
  end
  function git.unmerged_paths(worktree, timeout)
    record(model, "unmerged", { worktree = worktree, timeout = timeout })
    unmerged_reads = unmerged_reads + 1
    if fixture.resolve_after_codex == true and unmerged_reads > 1 then
      return { stdout = "", stderr = "", exit_code = 0 }
    end
    return { stdout = UNMERGED, stderr = "", exit_code = 0 }
  end
  function git.worktree_remove(worktree, timeout)
    record(model, "worktree_remove", { worktree = worktree, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.add_all(worktree, timeout)
    record(model, "add_all", { worktree = worktree, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.diff_check(worktree, cached, timeout)
    record(model, "diff_check", { worktree = worktree, cached = cached, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.commit_message_file(worktree, message_file, timeout)
    record(model, "commit", { worktree = worktree, message_file = message_file, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function git.push_worktree_branch_update(worktree, branch, timeout)
    record(model, "push", { worktree = worktree, branch = branch, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return git, model
end

local function make_github(fixture)
  local model = github_fake.model({
    issues = {
      [REPO .. "#issue/" .. tostring(ISSUE_NUMBER)] = fixture.parent,
    },
  })
  local github = github_fake.new(model)
  local original_read_issue = github.read_issue
  local original_close = github.pr_close
  local pr_reads = 0
  local parent_reads = 0
  function github.pr_cli_view(repo, pr_number, fields, timeout)
    pr_reads = pr_reads + 1
    if pr_reads >= 2 then
      if fixture.final_mutation == "pr-head" then
        fixture.pr.head_sha = ADVANCED_SHA
      elseif fixture.final_mutation == "pr-draft" then
        fixture.pr.is_draft = true
      elseif fixture.final_mutation == "pr-base" then
        fixture.pr.base_ref_name = "dev"
      end
    end
    record(model, "pr_view", {
      repo = repo,
      pr_number = pr_number,
      fields = fields,
      timeout = timeout,
      read = pr_reads,
    })
    return { stdout = render_pr(fixture.pr), stderr = "", exit_code = 0 }
  end
  function github.read_issue(source_ref, opts)
    parent_reads = parent_reads + 1
    if parent_reads >= 2 then
      if fixture.final_mutation == "parent-state" then
        model.issues[source_ref.ref].comments = {
          comment(core.state_marker(PROPOSAL_ID, "blocked", ROOT_VERSION .. "/blocked/concurrent"), "2026-07-14T01:05:00Z"),
        }
      elseif fixture.final_mutation == "claim" then
        model.issues[source_ref.ref].assignees = { "another-bot" }
      end
    end
    record(model, "issue_read", { source_ref = copy(source_ref), opts = copy(opts), read = parent_reads })
    return original_read_issue(source_ref, opts)
  end
  function github.pr_close(repo, pr_number, timeout)
    local result = original_close(repo, pr_number, timeout)
    fixture.pr.state = "CLOSED"
    return result
  end
  return github, model
end

local function mock_env(write_mode)
  local values = {
    FKST_GITHUB_WRITE = write_mode or "1",
    FKST_GITHUB_BOT_LOGIN = BOT,
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = INTEGRATION_BRANCH,
  }
  for name, value in pairs(values) do
    for _ = 1, 12 do
      t.mock_command(devloop_base.read_env_command(name), {
        stdout = value,
        stderr = "",
        exit_code = 0,
      })
    end
  end
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-integration/2275",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
end

local function count_rows(rows, kind, predicate)
  local count = 0
  for _, row in ipairs(rows or {}) do
    if row.kind == kind and (predicate == nil or predicate(row)) then
      count = count + 1
    end
  end
  return count
end

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
end

local function count_raises(raises, queue)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function run_fixture(fixture, write_mode)
  mock_env(write_mode)
  local normalized_parent = require("forge.github.issue").normalize_issue(
    fixture.parent,
    entity_lib.issue_source_ref(REPO, ISSUE_NUMBER)
  )
  local origin = m_facts.pr_origin_fact(fixture.pr.comments)
  if origin ~= nil and origin.repo == REPO and origin.issue_number == tostring(ISSUE_NUMBER) then
    local delegation = m_facts.pr_delegation_fact(
      normalized_parent.comments,
      origin.proposal_id,
      origin.impl_version
    )
    if fixture.expect_initial_valid and delegation ~= nil then
      local delegated_repo, delegated_pr = entity_lib.parse_pr_proposal_id(delegation.pr_proposal_id)
      if delegated_repo ~= REPO or tonumber(delegated_pr) ~= PR_NUMBER
        or tonumber(delegation.pr_number) ~= PR_NUMBER then
        error("valid recovery fixture normalized an invalid pr-delegation identity: "
          .. tostring(delegated_repo) .. "#" .. tostring(delegated_pr))
      end
    end
    if fixture.expect_initial_valid then
      local awaits = devloop_state.has_state_marker(
        normalized_parent.comments,
        origin.proposal_id,
        "awaiting-pr",
        origin.impl_version
      )
      if fixture.pr.state ~= "OPEN" or normalized_parent.state ~= "OPEN"
        or not awaits or delegation == nil then
        error("valid recovery fixture preflight failed: pr_state=" .. tostring(fixture.pr.state)
          .. " parent_state=" .. tostring(normalized_parent.state)
          .. " awaits=" .. tostring(awaits)
          .. " delegation=" .. tostring(delegation ~= nil))
      end
    end
  end
  local fingerprint = core.sync_conflict_fingerprint(fixture.event, UNMERGED)
  if fixture.resolve_after_codex ~= true then
    core.record_sync_conflict_attempt(fixture.event, fingerprint, core.max_sync_conflict_attempts())
  end
  local github, github_model = make_github(fixture)
  local git, git_model = make_git(fixture)
  local department = sync_conflict.make_department({ github = github, git = git })
  local lock_keys = {}
  local previous_with_lock = with_lock
  local previous_spawn_codex_sync = spawn_codex_sync
  with_lock = function(key, fn)
    table.insert(lock_keys, key)
    return fn()
  end
  if fixture.resolve_after_codex == true then
    spawn_codex_sync = function()
      return { stdout = "resolved", stderr = "", exit_code = 0 }
    end
  end
  local ok, result = pcall(testing.run_fake, department, {
    queue = "devloop_sync_conflict",
    payload = fixture.event,
  })
  with_lock = previous_with_lock
  spawn_codex_sync = previous_spawn_codex_sync
  if not ok then
    error(result, 0)
  end
  result.github_model = github_model
  result.git_model = git_model
  result.lock_keys = lock_keys
  return result
end

local function assert_no_resolution_effects(result)
  t.eq(h.count_calls("codex exec"), 0)
  t.eq(count_rows(result.git_model.writes, "add_all"), 0)
  t.eq(count_rows(result.git_model.writes, "commit"), 0)
  t.eq(count_rows(result.git_model.writes, "push"), 0)
end

local function assert_declined(result)
  t.eq(count_rows(result.github_model.writes, "exec", function(row)
    return row.context == "gh pr close"
  end), 0)
  assert_no_resolution_effects(result)
  t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 1)
end

local function assert_recovered(result)
  t.eq(count_rows(result.github_model.writes, "exec", function(row)
    return row.context == "gh pr close"
  end), 1)
  assert_no_resolution_effects(result)
  t.is_nil(find_raise(result.raises, "github-proxy.github_issue_create_request"))
  t.eq(result.lock_keys[1], core.pr_freshness_lock_key(REPO, ORIGINAL_BRANCH))
end

return {
  test_successful_resolution_stages_with_injected_git_fake = function()
    local fixture = baseline()
    fixture.event.integration_branch = ORIGINAL_BRANCH .. "-success"
    fixture.event.dedup_key = core.pr_freshness_dedup_key(
      REPO,
      fixture.event.integration_branch,
      INTEGRATION_SHA
    )
    fixture.resolve_after_codex = true
    local result = run_fixture(fixture, "")

    t.eq(count_rows(result.git_model.writes, "add_all"), 1, "injected fake stages the resolved worktree")
    t.eq(count_rows(result.git_model.writes, "commit"), 1, "injected fake commits the resolution")
    t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 0,
      "successful resolution does not escalate")
  end,

  test_exhausted_original_pr_closes_once_without_codex_resolution_or_escalation = function()
    local fixture = baseline()
    local result = run_fixture(fixture, "1")
    assert_recovered(result)

    fixture.expect_initial_valid = false
    local replay = run_fixture(fixture, "1")
    t.eq(count_rows(result.github_model.writes, "exec", function(row)
      return row.context == "gh pr close"
    end) + count_rows(replay.github_model.writes, "exec", function(row)
      return row.context == "gh pr close"
    end), 1)
    assert_no_resolution_effects(replay)
    t.is_nil(find_raise(replay.raises, "github-proxy.github_issue_create_request"))
  end,

  test_exhausted_merged_pr_is_idempotent_without_close_or_escalation = function()
    local result = run_fixture(baseline({
      pr_state = "MERGED",
      expect_initial_valid = false,
    }), "1")

    t.eq(count_rows(result.github_model.writes, "exec", function(row)
      return row.context == "gh pr close"
    end), 0)
    assert_no_resolution_effects(result)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 0)
  end,

  test_decline_matrix_preserves_the_existing_single_escalation = function()
    local historical = parent_comments(ROOT_VERSION)
    table.insert(historical, comment(
      core.state_marker(PROPOSAL_ID, "blocked", ROOT_VERSION .. "/blocked/superseded"),
      "2026-07-14T01:06:00Z"
    ))
    local cases = {
      { name = "missing-origin", fields = { omit_origin = true } },
      { name = "untrusted-origin", fields = { origin_author = "mallory" } },
      {
        name = "pr-native-origin",
        fields = {
          origin_proposal_id = PR_PROPOSAL_ID,
          origin_issue_number = PR_NUMBER,
        },
      },
      {
        name = "wrong-repo-origin",
        fields = {
          origin_proposal_id = "github-devloop/issue/other/repo/2275",
          origin_issue_number = ISSUE_NUMBER,
        },
      },
      {
        name = "replacement-generation",
        fields = {
          impl_version = REPLACEMENT_VERSION,
          branch = REPLACEMENT_BRANCH,
          event_branch = REPLACEMENT_BRANCH,
        },
      },
      { name = "cross-repo-head", fields = { head_repository = "fork/repo", is_cross_repository = true } },
      { name = "base-branch", fields = { pr_base_ref_name = "dev" } },
      { name = "head-branch", fields = { pr_head_ref_name = ORIGINAL_BRANCH .. "-changed" } },
      { name = "head-sha", fields = { pr_head_sha = ADVANCED_SHA } },
      { name = "parent-closed", fields = { parent_state = "CLOSED" } },
      {
        name = "parent-wrong-version",
        fields = { parent_comments = parent_comments(ROOT_VERSION .. "/other") },
      },
      {
        name = "historical-awaiting-pr-superseded",
        fields = { parent_comments = historical },
      },
      {
        name = "delegation-mismatch",
        fields = {
          parent_comments = {
            comment(core.state_marker(PROPOSAL_ID, "awaiting-pr", ROOT_VERSION), "2026-07-14T01:03:00Z"),
            comment(m_builders.pr_delegation_marker(
              PROPOSAL_ID,
              entity_lib.pr_proposal_id(REPO, PR_NUMBER + 1),
              PR_NUMBER + 1,
              ROOT_VERSION,
              "g1"
            ), "2026-07-14T01:03:01Z"),
          },
        },
      },
      { name = "lost-claim", fields = { assignees = { "another-bot" } } },
      { name = "unknown-pr-state", fields = { pr_state = "UNKNOWN" } },
    }
    for _, case in ipairs(cases) do
      local result = run_fixture(baseline(case.fields), "1")
      local ok, err = pcall(assert_declined, result)
      if not ok then
        error("decline case failed: " .. case.name .. ": " .. tostring(err))
      end
    end
  end,

  test_dry_run_logs_would_close_without_pr_write_or_escalation = function()
    local result = run_fixture(baseline(), "")
    t.eq(count_rows(result.github_model.writes, "exec", function(row)
      return row.context == "gh pr close"
    end), 0)
    assert_no_resolution_effects(result)
    t.is_nil(find_raise(result.raises, "github-proxy.github_issue_create_request"))
  end,

  test_non_pr_conflict_and_replacement_conflict_keep_existing_escalation = function()
    local non_pr = baseline({
      source_ref = core.branch_sync_source_ref(REPO, INTEGRATION_BRANCH, ORIGINAL_BRANCH),
    })
    non_pr.event.dedup_key = core.branch_sync_dedup_key(
      REPO,
      INTEGRATION_BRANCH,
      ORIGINAL_BRANCH,
      INTEGRATION_SHA
    )
    assert_declined(run_fixture(non_pr, "1"))

    local replacement = baseline({
      impl_version = REPLACEMENT_VERSION,
      branch = REPLACEMENT_BRANCH,
      event_branch = REPLACEMENT_BRANCH,
    })
    assert_declined(run_fixture(replacement, "1"))
  end,

  test_original_retry_two_is_classified_by_original_branch_lineage = function()
    local retry_version = ROOT_VERSION .. "/reimplement/2"
    local result = run_fixture(baseline({
      impl_version = retry_version,
      branch = ORIGINAL_BRANCH,
      event_branch = ORIGINAL_BRANCH,
      parent_comments = parent_comments(retry_version),
      expect_initial_valid = true,
    }), "1")
    assert_recovered(result)
  end,

  test_write_before_rederive_declines_every_changed_close_precondition = function()
    for _, mutation in ipairs({ "pr-head", "pr-draft", "pr-base", "parent-state", "claim", "integration-head" }) do
      local result = run_fixture(baseline({
        final_mutation = mutation,
        expect_initial_valid = true,
      }), "1")
      assert_declined(result)
    end
  end,
}
