local h = require("tests.devloop_core_helpers")
local transition_version = require("contract.transition_version")
local devloop_base = require("devloop.base")
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

local ordering_base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local ordering_cases = {
  {
    name = "newer updated_at wins",
    left = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z",
    right = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-05T01-02-03Z",
    expected = -1,
  },
  {
    name = "timestamped version wins over timestampless fallback",
    left = "consensus:github-devloop/issue/owner/repo/42/v1",
    right = "consensus:github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z",
    expected = -1,
  },
  {
    name = "safe-equivalent slash and hyphen bases tie",
    left = "ready/consensus/v1",
    right = "ready-consensus-v1",
    expected = 0,
  },
  {
    name = "loop round orders within the same base",
    left = ordering_base .. "/loop/1",
    right = ordering_base .. "/loop/2",
    expected = -1,
  },
  {
    name = "fix round orders after the same loop round",
    left = ordering_base .. "/loop/2",
    right = ordering_base .. "/loop/2/fix/1",
    expected = -1,
  },
  {
    name = "fix field outranks later review-meta-action field",
    left = ordering_base .. "/review-meta-action/9/fix/1",
    right = ordering_base .. "/fix/2",
    expected = -1,
  },
  {
    name = "fix field outranks later reimplement field",
    left = ordering_base .. "/reimplement/9",
    right = ordering_base .. "/fix/1",
    expected = -1,
  },
  {
    name = "timeout max round orders across timeout states",
    left = ordering_base .. "/timeout/ready/1",
    right = ordering_base .. "/timeout/reviewing/2",
    expected = -1,
  },
  {
    name = "review-meta-action field outranks review-loop field",
    left = ordering_base .. "/review-loop/9",
    right = ordering_base .. "/review-meta-action/1",
    expected = -1,
  },
  {
    name = "review-loop field outranks ready-split field",
    left = ordering_base .. "/ready-split/9",
    right = ordering_base .. "/review-loop/1",
    expected = -1,
  },
  {
    name = "explicit loop round 0 equals an absent loop suffix (missing is numeric 0)",
    left = ordering_base .. "/loop/0",
    right = ordering_base,
    expected = 0,
  },
  {
    name = "same timeout max round is equal regardless of timeout state",
    left = ordering_base .. "/timeout/ready/2",
    right = ordering_base .. "/timeout/reviewing/2",
    expected = 0,
  },
  {
    name = "higher timeout max round wins regardless of timeout state",
    left = ordering_base .. "/timeout/ready/3",
    right = ordering_base .. "/timeout/reviewing/2",
    expected = 1,
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

  test_compare_matches_captured_devloop_transition_order_goldens = function()
    for _, case in ipairs(ordering_cases) do
      t.eq(transition_version.compare(case.left, case.right), case.expected, case.name)
      t.eq(transition_version.compare(case.right, case.left), -case.expected, case.name .. " reverse")
    end
  end,

  test_marker_order_key_matches_captured_devloop_stage_rank_goldens = function()
    t.eq(
      transition_version.marker_order_key(ordering_base .. "/loop/2/fix/1", 700),
      "2026-06-04T01-02-03Z/000000000002/000000000001/000000000000/000000000000/000000000000/000000000000/000000000000/000000000700"
    )
    t.is_true(
      transition_version.marker_order_key(ordering_base, 690)
        < transition_version.marker_order_key(ordering_base, 700)
    )
  end,

  test_next_constructors_preserve_recorded_transition_version_shapes = function()
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local expected = {
      transition_version.next_loop(base),
      transition_version.next_fix(base .. "/loop/2"),
      transition_version.next_fix(base .. "/loop/2/fix/1"),
      transition_version.next_review_loop(base .. "/fix/2"),
      transition_version.next_review_meta_action(base .. "/review-loop/3"),
      transition_version.next_ready_split(base .. "/loop/2/fix/1"),
      transition_version.next_reimplement(base .. "/ready-split/5"),
      transition_version.next_timeout(base .. "/timeout/reviewing/2", "reviewing"),
      transition_version.next_rereview(base .. "/review-loop/3", "feedface"),
    }

    t.eq(expected[1], base .. "/loop/1")
    t.eq(expected[2], base .. "/loop/2/fix/1")
    t.eq(expected[3], base .. "/loop/2/fix/1/fix/2")
    t.eq(expected[4], base .. "/fix/2/review-loop/1")
    t.eq(expected[5], base .. "/review-loop/3/review-meta-action/1")
    t.eq(expected[6], base .. "/ready-split/1")
    t.eq(expected[7], base .. "/ready-split/5/reimplement/1")
    t.eq(expected[8], base .. "/timeout/reviewing/3")
    t.eq(expected[9], base .. "/review-loop/3/review-loop/4/rereview/4/feedface")
    for _, value in ipairs(expected) do
      t.eq(transition_version.render(transition_version.parse(value)), value)
    end
  end,

  test_suffix_strippers_preserve_requested_scope = function()
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"

    t.eq(transition_version.strip_timeout_suffixes(base .. "/fix/1/timeout/reviewing/2"), base .. "/fix/1")
    t.eq(transition_version.strip_timeout_suffixes(base .. "/fix/1/timeout/reviewing/2/timeout/ready/3"), base .. "/fix/1")
    t.eq(transition_version.strip_trailing_loop(base .. "/loop/2"), base)
    t.eq(transition_version.strip_trailing_loop(base .. "/loop/2/fix/1"), base .. "/loop/2/fix/1")
    t.eq(transition_version.strip_trailing_reimplement(base .. "/ready-split/5/reimplement/2"), base .. "/ready-split/5")
    t.eq(transition_version.trailing_reimplement_round(base .. "/ready-split/5/reimplement/2"), 2)
    t.eq(transition_version.trailing_reimplement_round(base .. "/reimplement/2/fix/1"), 0)
    t.eq(transition_version.has_review_loop(base .. "/fix/2"), false)
    t.eq(transition_version.has_review_loop(base .. "/fix/2/review-loop/3/review-meta-action/1"), true)
    t.eq(transition_version.strip_before_review_loop(base .. "/fix/2/review-loop/3/review-meta-action/1"), base .. "/fix/2")
  end,

  test_pr_review_consensus_dedup_canonicalizer_rejects_malformed_loop_rounds = function()
    local review = devloop_base.pr_review_proposal_id("owner/repo", 7, "ready/v1", "def456")
    local base = devloop_base.pr_review_consensus_dedup_key(review)

    t.eq(devloop_base.canonical_pr_review_consensus_dedup_key(base .. "/loop/1.5"), nil)
    t.eq(devloop_base.canonical_pr_review_consensus_dedup_key(base .. "/loop/1e3"), nil)
    t.eq(devloop_base.canonical_pr_review_consensus_dedup_key(base .. "/loop/0x2"), nil)
    t.eq(devloop_base.canonical_pr_review_consensus_dedup_key(base .. "/loop/2"), base)
    t.eq(devloop_base.canonical_pr_review_consensus_dedup_key(base .. "/loop/2/loop/3"), nil)
  end,

  test_devloop_state_builders_delegate_to_byte_exact_transition_constructors = function()
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"

    t.eq(core.next_fix_version(base .. "/loop/2"), base .. "/loop/2/fix/1")
    t.eq(core.next_fix_version(base .. "/loop/2/fix/1"), base .. "/loop/2/fix/1/fix/2")
    t.eq(core.next_review_loop_version(base .. "/fix/2"), base .. "/fix/2/review-loop/1")
    t.eq(core.next_review_meta_action_version(base .. "/review-loop/3"), base .. "/review-loop/3/review-meta-action/1")
  end,
}
