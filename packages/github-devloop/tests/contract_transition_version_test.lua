local h = require("tests.devloop_core_helpers")
local transition_version = require("contract.transition_version")
local core = h.core
local t = h.t

local cases = {
  { value = "ready/consensus-2026-06-17T22:18:19Z/loop/12", expected = "ready-consensus-2026-06-17T22-2609426986" },
  { value = "", expected = "empty" },
  { value = nil, expected = "empty" },
  { value = "###", expected = "version" },
  { value = "/reviewing#head//fix/1/", expected = "reviewing-head-fix-1" },
  { value = "ready/consensus-owner-repo-42-2026-06-17T22:18:19Z/loop/12", expected = "ready-consensus-owner-repo-42-0920351821" },
}

local version_shapes = {
  {
    value = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    base = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
  },
  {
    value = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    base = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    loop = 2,
  },
  {
    value = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2/fix/1",
    base = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    loop = 2,
    fix = 1,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/review-loop/3",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    review_loop = 3,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/review-meta-action/4",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    review_meta_action = 4,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/ready-split/5",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    ready_split = 5,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/reimplement/6",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    reimplement = 6,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/timeout/reviewing/1",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    timeout_state = "reviewing",
    timeout = 1,
  },
  {
    value = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2/fix/1/review-loop/3/review-meta-action/4/ready-split/5/reimplement/6/timeout/reviewing/7",
    base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    loop = 2,
    fix = 1,
    review_loop = 3,
    review_meta_action = 4,
    ready_split = 5,
    reimplement = 6,
    timeout_state = "reviewing",
    timeout = 7,
  },
  {
    value = "ready/base/fix/1/review-loop/2/fix/3/timeout/ready/4/timeout/reviewing/5",
    base = "ready/base",
    fix = 3,
    review_loop = 2,
    ready_timeout = 4,
    timeout_state = "reviewing",
    timeout = 5,
  },
}

return {
  test_safe_version_segment_matches_captured_devloop_goldens = function()
    for _, case in ipairs(cases) do
      t.eq(transition_version.safe_version_segment(case.value), case.expected)
    end
  end,

  test_parse_render_round_trips_known_transition_version_shapes = function()
    for _, case in ipairs(version_shapes) do
      local parsed = transition_version.parse(case.value)
      t.eq(parsed.base, case.base)
      t.eq(transition_version.render(parsed), case.value)
    end
  end,

  test_structured_round_getters_match_devloop_public_getters = function()
    for _, case in ipairs(version_shapes) do
      local parsed = transition_version.parse(case.value)
      t.eq(transition_version.loop_round(parsed), core.version_loop_round(case.value))
      t.eq(transition_version.fix_round(parsed), core.version_fix_round(case.value))
      t.eq(transition_version.review_loop_round(parsed), core.version_review_loop_round(case.value))
      t.eq(transition_version.review_meta_action_round(parsed), core.version_review_meta_action_round(case.value))
      t.eq(transition_version.ready_split_round(parsed), core.version_ready_split_round(case.value))
      t.eq(transition_version.reimplement_round(parsed), core.version_reimplement_round(case.value))
      t.eq(transition_version.timeout_round(parsed, "reviewing"), core.version_timeout_round(case.value, "reviewing"))
      t.eq(transition_version.timeout_round(parsed, "ready"), core.version_timeout_round(case.value, "ready"))
    end
  end,

  test_structured_round_getters_return_expected_goldens = function()
    for _, case in ipairs(version_shapes) do
      local value = case.value
      t.eq(transition_version.loop_round(value), case.loop or 0)
      t.eq(transition_version.fix_round(value), case.fix or 0)
      t.eq(transition_version.review_loop_round(value), case.review_loop or 0)
      t.eq(transition_version.review_meta_action_round(value), case.review_meta_action or 0)
      t.eq(transition_version.ready_split_round(value), case.ready_split or 0)
      t.eq(transition_version.reimplement_round(value), case.reimplement or 0)
      t.eq(transition_version.timeout_round(value, "reviewing"), case.timeout or 0)
      t.eq(transition_version.timeout_round(value, "ready"), case.ready_timeout or 0)
    end
  end,

  test_recorded_loop_then_fix_shape_keeps_both_rounds = function()
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z/loop/2/fix/1"
    local parsed = transition_version.parse(version)

    t.eq(transition_version.render(parsed), version)
    t.eq(transition_version.loop_round(parsed), 2)
    t.eq(transition_version.fix_round(parsed), 1)
    t.eq(core.version_loop_round(version), 2)
    t.eq(core.version_fix_round(version), 1)
  end,

  test_max_timeout_round_uses_devloop_ordered_timeout_states = function()
    local version = "ready/base/timeout/custom-state/9/timeout/reviewing/2"

    t.eq(transition_version.timeout_round(version, "custom-state"), 9)
    t.eq(transition_version.timeout_round(version, "reviewing"), 2)
    t.eq(transition_version.max_timeout_round(version), 2)
  end,
}
