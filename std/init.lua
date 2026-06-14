-- std: repo-level shared library for fkst-packages.
--
-- Tier S (substrate-contract: saga department shape, idempotency oracle,
-- source_ref / version-CAS helpers) and Tier R (repo-domain utilities) modules
-- live here and are required as `std.<module>`. The library is vendored into
-- every package via a committed `packages/<pkg>/std -> ../../std` symlink, so the
-- engine's owner-scoped `package.path` (= package root) resolves it. Sharing is
-- one-way and hierarchical (all packages -> std); peer cross-package require
-- (package A -> package B internals) stays forbidden (check_repo.py G9).
return {
  version = "0",
}
