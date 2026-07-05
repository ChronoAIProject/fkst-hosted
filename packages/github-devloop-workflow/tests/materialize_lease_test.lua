local lease = require("core.materialize.lease")
local t = fkst.test

local core = {
  has_label = function()
    return false
  end,
  invalidate_entity_after_write = function()
  end,
}

local function deps_with_assignees(assignees, writes)
  return {
    read_current_issue_ownership = function()
      return {
        assignees = assignees,
        author_login = "fkst-test-bot",
        labels = {},
      }
    end,
    write_enabled = function()
      return true
    end,
    issue_unassign = function(repo, issue_number, login, timeout)
      writes[#writes + 1] = {
        repo = repo,
        issue_number = issue_number,
        login = login,
        timeout = timeout,
      }
    end,
  }
end

local tests = {
  test_done_release_removes_only_self_assignee = function()
    local writes = {}
    local ok = lease.release_done_claim(
      core,
      deps_with_assignees({ "fkst-test-bot" }, writes),
      "owner/repo",
      42,
      "github-devloop/issue/owner/repo/42"
    )
    t.is_true(ok)
    t.eq(#writes, 1)
    t.eq(writes[1].repo, "owner/repo")
    t.eq(writes[1].issue_number, 42)
    t.eq(writes[1].login, "fkst-test-bot")
  end,

  test_done_release_does_not_touch_non_self_assignee = function()
    local writes = {}
    local ok = lease.release_done_claim(
      core,
      deps_with_assignees({ "human" }, writes),
      "owner/repo",
      42,
      "github-devloop/issue/owner/repo/42"
    )
    t.eq(ok, false)
    t.eq(#writes, 0)
  end,

  test_done_release_dry_run_logs_without_write = function()
    local writes = {}
    local deps = deps_with_assignees({ "fkst-test-bot" }, writes)
    deps.write_enabled = function()
      return false
    end
    local ok = lease.release_done_claim(core, deps, "owner/repo", 42, "github-devloop/issue/owner/repo/42")
    t.is_true(ok)
    t.eq(#writes, 0)
  end,

  -- A "done" terminal (every slot merged) closes the completed origin idea issue.
  test_done_close_origin_closes_the_issue = function()
    local closes = {}
    local deps = {
      write_enabled = function()
        return true
      end,
      issue_close = function(repo, issue_number, timeout)
        closes[#closes + 1] = { repo = repo, issue_number = issue_number, timeout = timeout }
        return { exit_code = 0 }
      end,
    }
    local ok = lease.close_done_origin(core, deps, "owner/repo", 42, "github-devloop/issue/owner/repo/42")
    t.is_true(ok)
    t.eq(#closes, 1)
    t.eq(closes[1].repo, "owner/repo")
    t.eq(closes[1].issue_number, 42)
  end,

  -- Dry-run posture: without FKST_GITHUB_WRITE the close is logged, not executed.
  test_done_close_origin_dry_run_does_not_close = function()
    local closes = {}
    local deps = {
      write_enabled = function()
        return false
      end,
      issue_close = function(repo, issue_number, timeout)
        closes[#closes + 1] = { repo = repo, issue_number = issue_number }
      end,
    }
    local ok = lease.close_done_origin(core, deps, "owner/repo", 42, "github-devloop/issue/owner/repo/42")
    t.is_true(ok)
    t.eq(#closes, 0)
  end,
}

return tests
