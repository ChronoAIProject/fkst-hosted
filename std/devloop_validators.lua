local S = {}
local registry = require("std.registry")

local validators_index = require("std.devloop_validators.index")
local validators = {
  require("std.devloop_validators.fixing"),
  require("std.devloop_validators.intake_candidate"),
  require("std.devloop_validators.issue"),
  require("std.devloop_validators.merge_ready"),
  require("std.devloop_validators.pr"),
  require("std.devloop_validators.pr_review_unresolved"),
  require("std.devloop_validators.ready"),
  require("std.devloop_validators.result"),
  require("std.devloop_validators.review_meta"),
  require("std.devloop_validators.review_result"),
  require("std.devloop_validators.reviewing"),
  require("std.devloop_validators.unresolved"),
  require("std.devloop_validators.validate_proposal"),
}

function S.install(M)
  registry.install_indexed_installers("std.devloop_validators.index", validators_index, validators, M, M.restart_package_name or "github-devloop")
end

return S
