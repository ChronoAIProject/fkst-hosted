return require("testkit.devloop_worktree_fixtures").new({
  devloop_base = require("devloop.base"),
  base_ids = require("devloop.base_ids"),
  base = require("tests.devloop_base_helpers"),
  enable_substrate_pin_refresh = true,
  include_head_ref_push = true,
  include_branch_diff_paths = true,
})
