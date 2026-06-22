local S = {}
local registry = require("std.registry")

local validators_index = require("devloop.validators.index")
local validators = {
  require("devloop.validators.fixing"),
  require("devloop.validators.intake_candidate"),
  require("devloop.validators.issue"),
  require("devloop.validators.merge_ready"),
  require("devloop.validators.pr"),
  require("devloop.validators.pr_review_unresolved"),
  require("devloop.validators.ready"),
  require("devloop.validators.result"),
  require("devloop.validators.review_meta"),
  require("devloop.validators.review_result"),
  require("devloop.validators.reviewing"),
  require("devloop.validators.unresolved"),
  require("devloop.validators.validate_proposal"),
}

function S.install(M)
  registry.install_indexed_installers("devloop.validators.index", validators_index, validators, M, M.restart_package_name or "github-devloop")
end

return S
