-- Locus dyad seats (natural-ownership <-> proportional-containment) participate in the
-- default philosophy core as first-class whole-picture seats: they are in the fixed default
-- angle set, render their own universal lens (not the bleak unknown-angle fallback), and take
-- part in Phase R rebuttal alongside the existing seats. This is the additive "altitude"
-- adversarial dimension: ownership pulls the fix toward the layer that owns the invariant,
-- containment resists over-reaching past the natural owner layer; the two clash in rebuttal.
local core = require("core")
local rebuttal = require("departments.decide.rebuttal")
local t = fkst.test

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-locus-1",
    title = "Where does this invariant belong?",
    body = "A fix touches several downstream sites; decide the layer that naturally owns it.",
    content_fetch = "fetch-source --ref demo/locus/1 --full",
    context = "Judge whether the remedy lives at the owning layer without over-reaching.",
    dedup_key = "proposal-locus-1-v1",
    source_ref = { kind = "proposal", ref = "demo/locus/1" },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function contains(text, sub)
  return text:find(sub, 1, true) ~= nil
end

return {
  test_default_core_includes_locus_dyad = function()
    -- No proposal.angles -> the fixed default core, which now carries the locus dyad.
    local angles = core.angles(proposal())
    local seen = {}
    for _, angle in ipairs(angles) do
      seen[angle] = true
    end
    t.is_true(seen["teleology"])
    t.is_true(seen["parsimony"])
    t.is_true(seen["fidelity"])
    t.is_true(seen["natural-ownership"])
    t.is_true(seen["proportional-containment"])
    t.is_true(#angles == 5)
  end,

  test_natural_ownership_renders_real_lens_not_fallback = function()
    local prompt = core.build_angle_prompt(proposal(), "natural-ownership")
    t.is_true(contains(prompt, "Seat: natural-ownership"))
    t.is_true(contains(prompt, "which locus naturally owns"))
    -- Must NOT degrade to the bleak unknown-angle fallback.
    t.is_nil(prompt:find("Judge from this named perspective", 1, true))
  end,

  test_proportional_containment_renders_real_lens_not_fallback = function()
    local prompt = core.build_angle_prompt(proposal(), "proportional-containment")
    t.is_true(contains(prompt, "Seat: proportional-containment"))
    -- The anti-over-reach convergence target is baked into the lens.
    t.is_true(contains(prompt, "not the highest layer imaginable"))
    t.is_nil(prompt:find("Judge from this named perspective", 1, true))
  end,

  test_rebuttal_admits_the_five_seat_core = function()
    -- Phase R admission derives from the actual seat count, so the five-seat core (and the
    -- two-seat dyad clash within it) get a rebuttal round; a single seat has no peer to clash.
    t.is_true(rebuttal.can_run({ 1, 2, 3, 4, 5 }))
    t.is_true(rebuttal.can_run({ 1, 2 }))
    t.is_true(not rebuttal.can_run({ 1 }))
    t.is_true(not rebuttal.can_run("not-a-table"))
  end,
}
