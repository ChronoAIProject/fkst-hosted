local testing = require("testkit.testing")
local git_fake = require("forge.git_fake")
local ref_detect = require("departments.ref_detect.main")
local t = fkst.test

local observed_at = "2026-07-06T10:00:00Z"
local known_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

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

local function fake_git_with_branch(remote, branch, sha)
  local model = git_fake.model({})
  local git = git_fake.new(model)
  local default_exec = git._exec
  git._exec = function(argv, timeout, context)
    default_exec(argv, timeout, context)
    t.eq(argv[1], "git")
    if argv[2] == "ls-remote" and argv[3] == remote and argv[4] == "refs/heads/" .. branch then
      return {
        stdout = sha .. "\trefs/heads/" .. branch .. "\n",
        stderr = "",
        exit_code = 0,
      }
    end
    return { stdout = "", stderr = "unexpected git argv", exit_code = 1 }
  end
  return git, model
end

return {
  test_ref_detect_emits_git_ref_changed_for_one_configured_remote_branch = function()
    local git, model = fake_git_with_branch("origin", "main", known_sha)
    local dept = ref_detect.make_department({
      git = git,
      read_env = function(name)
        if name == "FKST_GIT_WATCH_REFS" then
          return "origin#main"
        end
        return nil
      end,
      now = function()
        return observed_at
      end,
    })

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
}
