local security_logic = require("security_logic")
local completion = require("completion")
local records = require("records")
local blueprint = require("workflow.engine.blueprint")
local t = fkst.test

local function sample_findings_json()
  return table.concat({
    "[",
    '{"severity":"high","area":"dependency:leftpad","file":"package.json","advisory":"GHSA-aaaa-bbbb-cccc",',
    '"summary":"Vulnerable transitive dependency.","remediation":"Upgrade to 1.3.1."},',
    '{"severity":"low","area":"tests","file":"src/auth.js","summary":"Auth path lacks tests.","remediation":"Add coverage."}',
    "]",
  })
end

return {
  test_decode_findings_accepts_valid_array = function()
    local findings = security_logic.decode_findings(sample_findings_json())
    t.eq(#findings, 2)
    t.eq(findings[1].severity, "high")
    t.eq(findings[1].advisory, "GHSA-aaaa-bbbb-cccc")
    t.eq(findings[2].area, "tests")
  end,

  test_decode_findings_empty_is_empty = function()
    t.eq(#security_logic.decode_findings("[]"), 0)
    t.eq(#security_logic.decode_findings("   "), 0)
  end,

  test_decode_findings_rejects_non_array_and_malformed = function()
    t.raises(function()
      security_logic.decode_findings('{"severity":"high"}')
    end)
    t.raises(function()
      security_logic.decode_findings("[{]")
    end)
    t.raises(function()
      security_logic.decode_findings('[{"severity":"nope","area":"x","summary":"y","remediation":"z"}]')
    end)
  end,

  test_finding_dedup_key_is_deterministic_and_bounded = function()
    local finding = { severity = "high", area = "dependency:x", summary = "s", remediation = "r" }
    local first = security_logic.finding_dedup_key("owner/repo", finding)
    local second = security_logic.finding_dedup_key("owner/repo", finding)
    t.eq(first, second)
    t.is_true(#first <= 512)
    t.is_true(first:find("workflow-security/", 1, true) == 1)
  end,

  test_build_finding_issue_request_shape = function()
    local finding = {
      severity = "critical",
      area = "dependency:openssl",
      file = "Cargo.toml",
      advisory = "GHSA-1234-5678-9abc",
      summary = "Known CVE in pinned openssl.",
      remediation = "Bump to a patched release.",
    }
    local request = security_logic.build_finding_issue_request("owner/repo", finding, true)
    t.eq(request.schema, "github-proxy.issue-create.v1")
    t.eq(request.repo, "owner/repo")
    t.eq(request.labels[1], "fkst-security")
    t.eq(request.source_ref.kind, "repo-site")
    t.is_true(request.title:find("Security%[critical%]") ~= nil)
    t.is_true(request.body:find("workflow-security-dedup:", 1, true) ~= nil)
    t.is_true(request.body:find("GHSA-1234-5678-9abc", 1, true) ~= nil)
  end,

  test_build_finding_issue_request_omits_label_when_absent = function()
    local finding = { severity = "medium", area = "config", summary = "s", remediation = "r" }
    local request = security_logic.build_finding_issue_request("owner/repo", finding, false)
    t.eq(#request.labels, 0)
  end,

  test_build_finding_requests_orders_by_severity_and_caps = function()
    local findings = {
      { severity = "low", area = "a", summary = "s", remediation = "r" },
      { severity = "critical", area = "b", summary = "s", remediation = "r" },
      { severity = "medium", area = "c", summary = "s", remediation = "r" },
    }
    local requests = security_logic.build_finding_requests("owner/repo", findings, true, 2)
    t.eq(#requests, 2)
    t.is_true(requests[1].title:find("critical", 1, true) ~= nil)
    t.is_true(requests[2].title:find("medium", 1, true) ~= nil)
  end,

  test_repo_slug_validation = function()
    t.is_true(security_logic.repo_slug_ok("owner/repo"))
    t.is_true(not security_logic.repo_slug_ok("no-slash"))
    t.is_true(not security_logic.repo_slug_ok(""))
  end,

  test_completion_status_mapping = function()
    t.eq(completion.status_of_result({ state = "ready" }), "result_ready")
    t.eq(completion.status_of_result({ state = "running" }), "running")
    t.eq(completion.status_of_result({ state = "transient" }), "recoverable")
    t.eq(completion.status_of_result({ state = "malformed" }), "fatal")
    t.eq(completion.status_of_result({ state = "weird" }), "unknown")
    t.eq(completion.status_of_result(nil), "unknown")
  end,

  test_completion_reader_reads_child_ref_result = function()
    local reader = completion.reader({ origin = "issue/1" })
    t.eq(reader({ result = { state = "ready" } }), "result_ready")
    t.eq(reader({ result = { state = "running" } }), "running")
    t.eq(reader({}), "unknown")
    t.eq(reader("not-a-table"), "unknown")
  end,

  test_builtin_blueprint_validates = function()
    local ok = blueprint.validate(records.BLUEPRINT)
    t.is_true(ok)
    t.eq(records.BLUEPRINT.id, "security-review")
    t.eq(#records.BLUEPRINT.steps, 4)
    t.eq(records.BLUEPRINT.steps[4].id, records.FINAL_STEP_ID)
  end,

  test_records_provider_single_record = function()
    local all = records.records()
    t.eq(#all, 1)
    t.eq(all[1].blueprint.id, "security-review")
    local valid = blueprint.validate(all[1].blueprint)
    t.is_true(valid)
  end,
}
