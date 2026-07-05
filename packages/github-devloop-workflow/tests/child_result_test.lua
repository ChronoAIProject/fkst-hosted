local child_result = require("core.child_result")
local t = fkst.test

local child = {
  proposal_id = "github-devloop/issue/owner/repo/42",
  source_ref = {
    kind = "external",
    ref = "owner/repo#issue/42",
  },
}

local function deps(extra)
  local value = {
    has_merged_marker = function() return false end,
    github_closed_with_merged_pr = function() return false end,
    irreversible_terminal = function() return false end,
    impl_failed_non_retryable = function() return false end,
    recovery_in_progress = function() return false end,
    impl_failed_retryable = function() return false end,
  }
  for key, fn in pairs(extra or {}) do
    value[key] = fn
  end
  return value
end

local tests = {
  test_trusted_merged_marker_is_result_ready = function()
    local status = child_result.child_result_status(deps({
      has_merged_marker = function(ref)
        t.eq(ref.proposal_id, child.proposal_id)
        return true
      end,
    }), child)
    t.eq(status, "result_ready")
  end,

  test_github_closed_with_linked_merged_pr_is_result_ready = function()
    local status = child_result.child_result_status(deps({
      github_closed_with_merged_pr = function()
        return { ok = true, pr_number = 108 }
      end,
    }), child)
    t.eq(status, "result_ready")
  end,

  test_closed_unmerged_is_fatal = function()
    local status = child_result.child_result_status(deps({
      irreversible_terminal = function() return true end,
    }), child)
    t.eq(status, "fatal")
  end,

  test_blocked_terminal_fact_is_fatal = function()
    local status = child_result.child_result_status(deps({
      irreversible_terminal = function() return { ok = true, reason = "blocked" } end,
    }), child)
    t.eq(status, "fatal")
  end,

  test_impl_failed_non_retryable_is_fatal = function()
    local status = child_result.child_result_status(deps({
      impl_failed_retryable = function() return false end,
      impl_failed_non_retryable = function() return true end,
    }), child)
    t.eq(status, "fatal")
  end,

  test_no_changes_impl_failed_is_fatal = function()
    local status, detail = child_result.child_result_status(deps({
      impl_failed_retryable = function() return false end,
      impl_failed_non_retryable = function() return true end,
      impl_failed_reason = function() return "no-changes" end,
    }), child)
    t.eq(status, "fatal")
    t.eq(detail.impl_failed_reason, "no-changes")
  end,

  test_codex_failed_impl_failed_is_fatal = function()
    local status, detail = child_result.child_result_status(deps({
      impl_failed_retryable = function() return false end,
      impl_failed_non_retryable = function() return true end,
      impl_failed_reason = function() return "codex-failed" end,
    }), child)
    t.eq(status, "fatal")
    t.eq(detail.impl_failed_reason, "codex-failed")
  end,

  test_impl_failed_retryable_is_recoverable = function()
    local status = child_result.child_result_status(deps({
      impl_failed_retryable = function() return true end,
    }), child)
    t.eq(status, "recoverable")
  end,

  test_recovery_in_progress_is_recoverable = function()
    local status = child_result.child_result_status(deps({
      recovery_in_progress = function() return { ok = true } end,
    }), child)
    t.eq(status, "recoverable")
  end,

  test_flowing_child_is_running = function()
    local status = child_result.child_result_status(deps({}), child)
    t.eq(status, "running")
  end,

  test_at_or_after_merged_state_marker_without_exact_merge_fact_is_not_ready = function()
    local status = child_result.child_result_status(deps({
      current_entity = function()
        return {
          proposal_id = child.proposal_id,
          comments = {
            '<!-- fkst:github-devloop:state:v1 proposal="' .. child.proposal_id
              .. '" state="merged" version="2026-07-02T00-00-00Z" stage_rank="900" -->',
          },
        }
      end,
    }), child)
    t.eq(status, "running")
  end,

  test_unreadable_child_is_unknown = function()
    local status = child_result.child_result_status(deps({
      has_merged_marker = function() error("read failed") end,
    }), child)
    t.eq(status, "unknown")
  end,
}

return tests
