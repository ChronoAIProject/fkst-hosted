local testing = require("testkit.testing")
local git_fake = require("forge.git_fake")
local core = require("core")
local ref_detect = require("departments.ref_detect.main")
local t = fkst.test

local observed_at = "2026-07-06T10:00:00Z"
local known_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
local next_sha = "cccccccccccccccccccccccccccccccccccccccc"

local function event()
  return {
    queue = "git-branch-detector.git_ref_poll_tick",
    ts = observed_at,
    payload = {
      schema = "git-branch-detector.ref-poll-tick.v1",
      source_ref = {
        kind = "cron",
        ref = "git-branch-detector/git_ref_poll/" .. observed_at,
      },
    },
  }
end

local function fake_git_with_responses(responses)
  local model = git_fake.model({})
  local git = git_fake.new(model)
  local default_exec = git._exec
  git._exec = function(argv, timeout, context)
    default_exec(argv, timeout, context)
    t.eq(argv[1], "git")
    t.eq(argv[2], "ls-remote")
    local response = table.remove(responses, 1)
    if response == nil then
      return { stdout = "", stderr = "unexpected git argv", exit_code = 1 }
    end
    t.eq(argv[3], response.remote)
    t.eq(argv[4], "refs/heads/" .. response.branch)
    return {
      stdout = response.stdout or "",
      stderr = response.stderr or "",
      exit_code = response.exit_code or 0,
    }
  end
  return git, model
end

local function response(remote, branch, opts)
  opts = opts or {}
  local stdout = opts.stdout
  if stdout == nil and opts.sha ~= nil then
    stdout = opts.sha .. "\trefs/heads/" .. branch .. "\n"
  end
  return {
    remote = remote,
    branch = branch,
    stdout = stdout,
    stderr = opts.stderr,
    exit_code = opts.exit_code,
  }
end

local function fake_git_with_branch(remote, branch, sha)
  return fake_git_with_responses({
    response(remote, branch, { sha = sha }),
  })
end

local function department_with(watch_refs, git)
  return ref_detect.make_department({
    git = git,
    read_env = function(name)
      if name == "FKST_GIT_WATCH_REFS" then
        return watch_refs
      end
      return nil
    end,
    now = function()
      return observed_at
    end,
  })
end

local function capture_warns(fn)
  local previous_warn = log.warn
  local warnings = {}
  log.warn = function(message)
    table.insert(warnings, tostring(message))
  end
  local ok, result = pcall(fn)
  log.warn = previous_warn
  if not ok then
    error(result, 0)
  end
  return result, warnings
end

local function run_with_logs(dept)
  local result, warnings = capture_warns(function()
    return testing.run_fake(dept, event())
  end)
  return result, warnings
end

local function contains(value, fragment)
  return tostring(value or ""):find(tostring(fragment), 1, true) ~= nil
end

return {
  test_ref_detect_emits_git_ref_changed_for_one_configured_remote_branch = function()
    local git, model = fake_git_with_branch("origin", "main", known_sha)
    local dept = department_with("origin#main", git)

    local result = testing.run_fake(dept, event())

    t.eq(#result.raises, 1)
    local raised = result.raises[1]
    t.eq(raised.queue, "git_ref_changed")
    t.eq(raised.payload.schema, "git-branch-detector.ref-changed.v1")
    t.eq(raised.payload.source_ref.kind, "git-ref")
    t.eq(raised.payload.source_ref.ref, "origin#main")
    t.eq(raised.payload.sha, known_sha)
    t.eq(raised.payload.observed_at, observed_at)
    t.eq(raised.payload.dedup_key, "git-ref/origin#main#" .. known_sha)
    t.eq(#model.writes, 1)
    t.eq(table.concat(model.writes[1].argv, " "), "git ls-remote origin refs/heads/main")
  end,

  test_parse_watch_refs_accepts_empty_and_mixed_separators = function()
    t.eq(#core.parse_watch_refs(nil), 0)
    t.eq(#core.parse_watch_refs(" \n\t "), 0)

    local targets = core.parse_watch_refs("origin#main,upstream#dev\nbackup#release")
    t.eq(#targets, 3)
    t.eq(targets[1].ref, "origin#main")
    t.eq(targets[2].ref, "upstream#dev")
    t.eq(targets[3].ref, "backup#release")
  end,

  test_ref_detect_empty_watch_list_is_clean_noop = function()
    local result, warnings = run_with_logs(department_with(" \n\t ", nil))

    t.eq(#result.raises, 0)
    t.eq(#warnings, 0)
  end,

  test_same_sha_observations_keep_same_delivery_dedup_key = function()
    local git = fake_git_with_responses({
      response("origin", "main", { sha = known_sha }),
      response("origin", "main", { sha = known_sha }),
    })
    local first = testing.run_fake(department_with("origin#main", git), event())
    local second = testing.run_fake(department_with("origin#main", git), event())

    t.eq(#first.raises, 1)
    t.eq(#second.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "git-ref/origin#main#" .. known_sha)
    t.eq(second.raises[1].payload.dedup_key, first.raises[1].payload.dedup_key)
  end,

  test_new_sha_observation_uses_fresh_delivery_dedup_key = function()
    local git = fake_git_with_responses({
      response("origin", "main", { sha = known_sha }),
      response("origin", "main", { sha = next_sha }),
    })
    local first = testing.run_fake(department_with("origin#main", git), event())
    local second = testing.run_fake(department_with("origin#main", git), event())

    t.eq(first.raises[1].payload.dedup_key, "git-ref/origin#main#" .. known_sha)
    t.eq(second.raises[1].payload.dedup_key, "git-ref/origin#main#" .. next_sha)
    t.is_true(first.raises[1].payload.dedup_key ~= second.raises[1].payload.dedup_key)
  end,

  test_ref_detect_lookup_failure_fails_closed_for_that_target = function()
    local git = fake_git_with_responses({
      response("origin", "main", { stderr = "remote unavailable", exit_code = 128 }),
    })

    local result, warnings = run_with_logs(department_with("origin#main", git))

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(contains(warnings[1], "tag=FAIL_CLOSED"))
    t.is_true(contains(warnings[1], "error_class=git-ref-lookup-failed"))
    t.is_true(contains(warnings[1], "source_ref=git-ref:origin#main"))
  end,

  test_ref_detect_missing_git_result_fails_closed_without_department_failure = function()
    local git = {
      ls_remote_branch = function(remote, branch, timeout)
        t.eq(remote, "origin")
        t.eq(branch, "main")
        t.eq(timeout, 30)
        return nil
      end,
    }

    local result, warnings = run_with_logs(department_with("origin#main", git))

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(contains(warnings[1], "error_class=git-ref-lookup-failed"))
    t.is_true(contains(warnings[1], "source_ref=git-ref:origin#main"))
  end,

  test_ref_detect_empty_branch_output_fails_closed_without_department_failure = function()
    local git = fake_git_with_responses({
      response("origin", "missing", { stdout = "" }),
    })

    local result, warnings = run_with_logs(department_with("origin#missing", git))

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(contains(warnings[1], "error_class=git-ref-branch-not-found"))
    t.is_true(contains(warnings[1], "source_ref=git-ref:origin#missing"))
  end,

  test_ref_detect_malformed_output_fails_closed_without_emit = function()
    local git = fake_git_with_responses({
      response("origin", "main", { stdout = known_sha .. "\trefs/tags/main\n" }),
    })

    local result, warnings = run_with_logs(department_with("origin#main", git))

    t.eq(#result.raises, 0)
    t.eq(#warnings, 1)
    t.is_true(contains(warnings[1], "error_class=git-ref-lookup-malformed"))
    t.is_true(contains(warnings[1], "source_ref=git-ref:origin#main"))
  end,

  test_ref_detect_partial_failure_continues_other_targets = function()
    local git, model = fake_git_with_responses({
      response("origin", "main", { stderr = "remote unavailable", exit_code = 128 }),
      response("backup", "dev", { sha = next_sha }),
    })

    local result, warnings = run_with_logs(department_with("origin#main backup#dev", git))

    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.source_ref.ref, "backup#dev")
    t.eq(result.raises[1].payload.sha, next_sha)
    t.eq(result.raises[1].payload.dedup_key, "git-ref/backup#dev#" .. next_sha)
    t.eq(#warnings, 1)
    t.is_true(contains(warnings[1], "source_ref=git-ref:origin#main"))
    t.eq(#model.writes, 2)
  end,
}
