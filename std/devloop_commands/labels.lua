local S = {}
local support = require("std.devloop_commands.support")
local validators = require("std.devloop_commands.validators")

function S.install(M)
  function M.gh_repo_labels_list(repo, timeout)
    return support.gh_result(function()
      return support.github().api_paginate_slurp("repos/" .. tostring(repo) .. "/labels?per_page=100", timeout)
    end)
  end

  function M.gh_repo_label_create(repo, name, color, description, timeout)
    return support.gh_result(function()
      return support.github().label_rest_create(
        repo,
        validators.require_label_name(name),
        validators.require_label_color(color),
        description,
        timeout
      )
    end)
  end

  function M.gh_repo_label_update(repo, name, color, description, timeout)
    return support.gh_result(function()
      return support.github().label_rest_update(
        repo,
        validators.require_label_name(name),
        validators.require_label_color(color),
        description,
        timeout
      )
    end)
  end
end

return S
