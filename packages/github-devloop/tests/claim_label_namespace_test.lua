local config = require("devloop.config")
local m_claims = require("devloop.claims")
local t = fkst.test

-- Label-mode ownership must read this session's EFFECTIVE claimed label, i.e. the
-- same label the claim writer adds and `ensure_repo` registers.
--
-- Reading the raw `fkst-dev:claimed` let a FOREIGN namespace's label read as this
-- session's own claim. A standalone/local substrate writes the unmapped label; a
-- namespaced cloud session then believed the issue was already claimed, so it never
-- wrote its own claim -- while the lifecycle path, which DOES map, saw no claim at
-- all and reported the item `unmanaged`. Claimed by nobody, advanced by nobody, and
-- silent: no fault, no error, health still reporting "working".

local NAMESPACE = "chronoai-fkst-cloud-test"
local OWNER = "session-owner"

local function env_values(values)
  return function(command)
    return {
      stdout = values[command] or "",
      stderr = "",
      exit_code = 0,
    }
  end
end

--- A label-mode session namespaced to NAMESPACE.
local function namespaced_label_mode()
  return env_values({
    [config.read_env_command("FKST_GITHUB_CLAIM_MODE")] = "label",
    [config.read_env_command("FKST_WORK_LABEL_NAMESPACE")] = NAMESPACE,
    [config.read_env_command("FKST_SESSION_WORK_LABEL")] = "fkst-dev-" .. NAMESPACE,
  })
end

--- A label-mode session with no namespace configured at all.
local function plain_label_mode()
  return env_values({
    [config.read_env_command("FKST_GITHUB_CLAIM_MODE")] = "label",
  })
end

return {
  test_effective_claimed_label_is_namespaced = function()
    t.eq(m_claims.effective_claimed_label(namespaced_label_mode()), "fkst-dev-" .. NAMESPACE .. ":claimed")
    -- ...and the logical constant itself is untouched, so the writer and the
    -- registration keep deriving from one source.
    t.eq(m_claims.claimed_label(), "fkst-dev:claimed")
  end,

  test_foreign_unmapped_claim_label_is_inert = function()
    -- THE REGRESSION. A stale local session's `fkst-dev:claimed` must not read as
    -- this namespaced session's claim.
    t.eq(
      m_claims.issue_claim_state({}, OWNER, { "fkst-dev:claimed", "fkst-dev-" .. NAMESPACE }, namespaced_label_mode()),
      "unassigned"
    )
  end,

  test_own_namespaced_claim_label_is_self = function()
    t.eq(
      m_claims.issue_claim_state(
        {},
        OWNER,
        { "fkst-dev-" .. NAMESPACE .. ":claimed", "fkst-dev-" .. NAMESPACE },
        namespaced_label_mode()
      ),
      "self"
    )
  end,

  test_another_namespaces_claim_label_is_inert = function()
    -- Not just the unmapped form: a DIFFERENT deployment's namespace is equally
    -- foreign and must not be mistaken for this session's claim.
    t.eq(
      m_claims.issue_claim_state({}, OWNER, { "fkst-dev-someone-elses-cloud:claimed" }, namespaced_label_mode()),
      "unassigned"
    )
  end,

  test_no_claim_label_at_all_is_unassigned = function()
    t.eq(m_claims.issue_claim_state({}, OWNER, { "fkst-dev-" .. NAMESPACE }, namespaced_label_mode()), "unassigned")
    t.eq(m_claims.issue_claim_state({}, OWNER, {}, namespaced_label_mode()), "unassigned")
    t.eq(m_claims.issue_claim_state({}, OWNER, nil, namespaced_label_mode()), "unassigned")
  end,

  test_unnamespaced_deployment_is_byte_identical = function()
    -- With no namespace configured the label maps to itself, so a deployment that
    -- never adopted namespacing behaves exactly as before this fix.
    t.eq(m_claims.effective_claimed_label(plain_label_mode()), "fkst-dev:claimed")
    t.eq(m_claims.issue_claim_state({}, OWNER, { "fkst-dev:claimed" }, plain_label_mode()), "self")
    t.eq(m_claims.issue_claim_state({}, OWNER, { "fkst-dev" }, plain_label_mode()), "unassigned")
  end,

  test_assignee_mode_ignores_claim_labels_entirely = function()
    -- The default mode must be untouched by any of this: ownership stays the sole
    -- self-assignee, and a claim label of either form is irrelevant.
    local assignee_mode = env_values({})
    t.eq(
      m_claims.issue_claim_state({ { login = OWNER } }, OWNER, { "fkst-dev:claimed" }, assignee_mode),
      "self"
    )
    t.eq(
      m_claims.issue_claim_state({ { login = "somebody-else" } }, OWNER, { "fkst-dev:claimed" }, assignee_mode),
      "other"
    )
    t.eq(m_claims.issue_claim_state({}, OWNER, { "fkst-dev:claimed" }, assignee_mode), "unassigned")
  end,
}
