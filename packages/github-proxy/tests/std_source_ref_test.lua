-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local source_ref = require("std.source_ref")
local t = fkst.test

return {
  test_bounded_source_ref_requires_kind_and_ref_under_limit = function()
    t.is_true(source_ref.has_bounded_source_ref({
      kind = "external",
      ref = "owner/repo#issue/42",
    }, 200))
    t.eq(source_ref.has_bounded_source_ref({
      kind = "external",
      ref = string.rep("x", 201),
    }, 200), false)
    t.eq(source_ref.has_bounded_source_ref({
      kind = "external",
    }, 200), false)
  end,

  test_version_order_key_preserves_version_cas_total_order_contract = function()
    t.eq(source_ref.version_order_key("consensus:plain-version"), "plain-version")
    t.eq(source_ref.version_order_key("ready/consensus-plain-version"), "plain-version")
    t.eq(
      source_ref.version_order_key("consensus:github-devloop/issue/1/intake/2026-06-17T22:18:19Z"),
      "2026-06-17T22-18-19Z/loop/000000000000"
    )
    t.eq(
      source_ref.version_order_key("ready/consensus-2026-06-17T22:18:19Z/loop/12"),
      "2026-06-17T22-18-19Z/loop/000000000012"
    )
    t.eq(
      source_ref.version_order_key("ready/plain-version"),
      "plain-version"
    )
    t.eq(
      source_ref.version_order_key("first/2026-06-17T01:02:03Z/then/2026-06-17T04-05-06Z/loop/7"),
      "2026-06-17T04-05-06Z/loop/000000000007"
    )
    t.eq(
      source_ref.version_order_key("2026-06-17T22:18:19Z/review-meta-action/2"),
      "2026-06-17T22-18-19Z/loop/000000000000/review-meta-action/2"
    )
    t.eq(source_ref.version_order_key(nil), "")
  end,
}
