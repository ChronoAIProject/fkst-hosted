return require("testkit.devloop_helpers_fixtures").new({
  entity_lib = require("devloop.entity"),
  base = require("tests.devloop_base_helpers"),
  pr = require("tests.devloop_pr_helpers"),
  worktree = require("tests.devloop_worktree_helpers"),
  entity_read_mocks = require("tests.entity_read_mock_helpers"),
  mock_review_result_pr_name_only = true,
  hydrate_handoff_state_comment = true,
  include_ready_causal_raise = false,
})
