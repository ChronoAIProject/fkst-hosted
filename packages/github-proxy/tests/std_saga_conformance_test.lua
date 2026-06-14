local conformance = require("std.saga_conformance")
local t = fkst.test

local function run_write()
  exec_sync({
    cmd = "gh issue comment '42' --repo 'owner/x' --body-file '/tmp/std-saga.md'",
    timeout = 30,
  })
end

local function run_read()
  exec_sync({
    cmd = "gh issue view '42' --repo 'owner/x' --json title",
    timeout = 30,
  })
end

return {
  test_write_class_classifier_is_explicit = function()
    t.eq(conformance.is_write_class("gh issue comment '42' --repo 'owner/x'"), true)
    t.eq(conformance.is_write_class("gh pr merge '7' --repo 'owner/x'"), true)
    t.eq(conformance.is_write_class("gh pr ready '7' --repo 'owner/x'"), true)
    t.eq(conformance.is_write_class("gh label create 'fkst-dev:ready' --repo 'owner/x'"), true)
    t.eq(conformance.is_write_class("gh workflow run 'ci.yml' --repo 'owner/x'"), true)
    t.eq(conformance.is_write_class("git push origin HEAD:branch"), true)
    t.eq(conformance.is_write_class("gh api --method POST 'repos/owner/x/issues/42/comments'"), true)
    t.eq(conformance.is_write_class("gh api graphql\nmutation { addLabelsToLabelable(input: {}) { clientMutationId } }"), true)
    t.eq(conformance.is_write_class("gh issue view '42' --repo 'owner/x'"), false)
    t.eq(conformance.is_write_class("gh pr diff '7' --repo 'owner/x'"), false)
    t.eq(conformance.is_write_class("gh api 'repos/owner/x/issues/42'"), false)
    t.eq(conformance.is_write_class("gh api graphql\nquery { viewer { login } }"), false)
  end,

  test_assert_progress_passes_when_first_writes = function()
    t.mock_command("gh issue comment '42'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    conformance.assert_progress(t, {
      first = run_write,
    })
  end,

  test_assert_progress_fails_when_first_only_reads = function()
    t.mock_command("gh issue view '42'", {
      stdout = "{}",
      stderr = "",
      exit_code = 0,
    })

    t.raises(function()
      conformance.assert_progress(t, {
        first = run_read,
      })
    end)
  end,

  test_assert_idempotent_passes_when_second_only_reads = function()
    t.mock_command("gh issue comment '42'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue view '42'", {
      stdout = "{}",
      stderr = "",
      exit_code = 0,
    })

    conformance.assert_idempotent(t, {
      first = run_write,
      second = run_read,
    })
  end,

  test_assert_idempotent_fails_when_second_writes = function()
    t.mock_command("gh issue comment '42'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    t.raises(function()
      conformance.assert_idempotent(t, {
        first = run_write,
        second = run_write,
      })
    end)
  end,
}
