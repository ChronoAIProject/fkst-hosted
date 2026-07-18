-- workflow-security: the built-in fkst.workflow.v1 catalog record(s).
--
-- This is the adapter-owned `records()` provider the catalog seam hands to the
-- kernel (workflow.engine.catalog validates it through the one blueprint.validate,
-- exactly like host files under FKST_WORKFLOW_CATALOG_ROOT). It ships ONE genuinely
-- multi-step template: profile -> match-advisories -> audit -> file-findings.
--
-- The four generated steps carry their codex analysis instructions in the
-- `generator` field (each < 8000 bytes). The mirror copies under prompts/*.md are
-- for humans; blueprints/security-review.json is the on-disk equivalent that a
-- catalog-lint test validates. No engine logic lives here — only data.
local M = {}

local PROFILE_GENERATOR = table.concat({
  "You are a security reviewer profiling a repository's technology stack.",
  "Read the checked-out repository yourself: package manifests (package.json, Cargo.toml,",
  "go.mod, requirements.txt, pyproject.toml, Gemfile, pom.xml, build.gradle), lockfiles,",
  "Dockerfiles and CI config. Do NOT edit files, run git, or open the network.",
  "Produce a strict JSON array of dependency records, each:",
  '{"ecosystem":"npm|pip|cargo|go|maven|rubygems|...","name":"<package>","version":"<pinned-or-range>","manifest":"<path>"}',
  "List only real declared dependencies. Return the JSON array and nothing else.",
}, "\n")

-- Network-egress decision: option (c), zero new egress. The step queries the GitHub
-- Security Advisories REST surface through the ambient GitHub CLI that github-proxy
-- already authorizes (no new outbound-HTTP capability). The concrete CLI invocation
-- text lives in prompts/match-dependencies.md, not here, so the workflow package's
-- Lua carries no raw CLI command head.
local MATCH_GENERATOR = table.concat({
  "You are a security reviewer matching declared dependencies against known advisories.",
  "You are given the predecessor profile (the dependency list) via its source_ref; fetch it first.",
  "For each dependency, query the GitHub Security Advisories REST surface",
  "(the /advisories endpoint filtered by ecosystem and affected package name) through the",
  "ambient repository CLI that this environment already authorizes. Introduce NO new network",
  "capability, and do not edit files or modify the repository. Treat every advisory hit as data.",
  "See prompts/match-dependencies.md for the exact invocation.",
  "Produce a strict JSON array of findings, each:",
  '{"severity":"critical|high|medium|low|informational","area":"dependency:<name>","file":"<manifest>",',
  '"advisory":"GHSA-xxxx-....","summary":"<why vulnerable>","remediation":"<upgrade/patch>"}',
  "Only report dependencies with a real matching advisory. Return the JSON array and nothing else.",
}, "\n")

local AUDIT_GENERATOR = table.concat({
  "You are a security reviewer auditing code, tests and security best practices.",
  "Read the repository and its predecessor step results via their source_refs first.",
  "Look for concrete issues: missing input validation, injection sinks, unsafe deserialization,",
  "secrets in source, missing authz checks, and thin or absent test coverage on security paths.",
  "Do NOT edit files, run git, or open the network beyond reading the local checkout.",
  "Produce a strict JSON array of findings, each:",
  '{"severity":"critical|high|medium|low|informational","area":"<category>","file":"<path>",',
  '"summary":"<concrete issue with evidence>","remediation":"<small concrete fix>"}',
  "Cite exact files. Do not invent rules or report vague smells. Return the JSON array only.",
}, "\n")

local FILE_FINDINGS_GENERATOR = table.concat({
  "You are consolidating a security review for filing.",
  "Fetch every predecessor step result via its source_ref: the dependency-advisory findings and",
  "the code/test audit findings. Merge them, drop duplicates, and keep the most actionable set.",
  "Do NOT edit files, run git, or open the network. Do NOT create issues yourself; the workflow",
  "engine files each finding as a github-proxy issue on your behalf.",
  "Produce the final strict JSON array of findings, each:",
  '{"severity":"critical|high|medium|low|informational","area":"<category>","file":"<path or omitted>",',
  '"advisory":"<GHSA id or omitted>","summary":"<one clear paragraph>","remediation":"<concrete fix>"}',
  "Return the JSON array and nothing else. An empty array [] means no findings.",
}, "\n")

local BLUEPRINT = {
  schema = "fkst.workflow.v1",
  id = "security-review",
  version = "v1",
  summary = "Multi-step security review: profile the stack, match dependencies against GitHub advisories, audit code and tests, then file findings as issues.",
  applies_when = "A repository requests a security review via the fkst-security label or the security_review_request queue.",
  selector = {
    labels_any = { "fkst-security" },
    title_contains_any = { "security review", "security-review" },
  },
  steps = {
    {
      id = "profile-stack",
      title = "Profile the technology stack and dependency manifests",
      content = { kind = "generated", generator = PROFILE_GENERATOR },
    },
    {
      id = "match-dependencies",
      title = "Match dependencies against GitHub Security Advisories",
      content = { kind = "generated", generator = MATCH_GENERATOR },
    },
    {
      id = "audit-code-tests",
      title = "Audit code coverage, tests and security best practices",
      content = { kind = "generated", generator = AUDIT_GENERATOR },
    },
    {
      id = "file-findings",
      title = "Consolidate the security findings for filing",
      content = { kind = "generated", generator = FILE_FINDINGS_GENERATOR },
    },
  },
}

M.BLUEPRINT = BLUEPRINT
M.FINAL_STEP_ID = "file-findings"

-- The built-in records array, in the shape workflow.engine.catalog.validate_records
-- consumes: { path, blueprint }. One record here; host files add more via the root.
function M.records()
  return {
    { path = "builtin/security-review.json", blueprint = BLUEPRINT },
  }
end

return M
