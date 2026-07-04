return require("testkit.devloop_pr_fixtures").new({
  entity_lib = require("devloop.entity"),
  base = require("tests.devloop_base_helpers"),
  entity_read_mocks = require("tests.entity_read_mock_helpers"),
  m_builders = require("devloop.markers.builders"),
  pr_safety = require("devloop.pr_safety"),
  github_risk = require("devloop.github_risk"),
  include_high_risk_helpers = true,
})
