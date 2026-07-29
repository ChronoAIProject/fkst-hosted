local t = fkst.test

-- A ready-redrive must not change which lineage a version belongs to. It did:
-- `redrive/<state>/<n>` was the one transition suffix nothing stripped, so a
-- redriven item's dependency-origin lookup compared two unrelated lineages and
-- blocked as `dependency-origin-version-mismatch` with hold_kind=unresolvable --
-- permanently, since nothing later re-derives the origin marker.

local BASE = "github-devloop/issue/acme/site/42/intake/2026-07-29T18-27-36Z/3463752446"

return {
  test_ready_redrive_reduces_to_the_lineage_it_was_redriven_from = function()
    local v = require("contract.transition_version")

    t.eq(v.strip_suffixes(BASE .. "/redrive/ready/1"), BASE)
    -- The observed live shape: a redrive that then took a ready split.
    t.eq(v.strip_suffixes(BASE .. "/redrive/ready/1/ready-split/1"), BASE)
    -- Repeated redrives collapse too; the loop runs until stable.
    t.eq(v.strip_suffixes(BASE .. "/redrive/ready/1/redrive/ready/2"), BASE)
  end,

  test_redrive_stripping_is_state_generic_and_hyphen_form_too = function()
    local v = require("contract.transition_version")

    t.eq(v.strip_suffixes(BASE .. "/redrive/thinking/3"), BASE)
    t.eq(v.strip_suffixes(BASE .. "-redrive-ready-1"), BASE)
  end,

  test_a_bare_lineage_is_unchanged = function()
    -- Guards the other direction: stripping must not eat a version that never
    -- carried a suffix, or every comparison would collapse to a common prefix.
    local v = require("contract.transition_version")
    t.eq(v.strip_suffixes(BASE), BASE)
  end,
}
