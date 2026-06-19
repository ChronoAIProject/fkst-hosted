# Lua Coverage Producer Proof

Issue: `github-devloop/issue/ChronoAIProject/fkst-packages/1135`.

Established practice: a coverage ratchet should have one authoritative deterministic producer. A fallback or aggregation producer needs evidence that the existing producer cannot satisfy the ratchet contract.

## Result

The canonical `FKST_LUA_COVERAGE_JSON` producer is not sufficient for package-root Lua ratchet enforcement today.

Direct local probe:

```sh
tmpdir="$(mktemp -d)"
fkst-framework --self-test --coverage "$tmpdir"
python3 - <<'PY' "$tmpdir/coverage.json"
import json, sys
from pathlib import Path
data = json.loads(Path(sys.argv[1]).read_text())
print(data)
PY
```

Observed with `/Users/auric/fkst-substrate/target/debug/fkst-framework`:

```text
0 passed, 0 failed
{}
```

`scripts/check_repo_coverage.py` requires engine-authored Lua line metadata through `files`, top-level missing line metadata, or covered-line maps that can be mapped to repository production Lua paths. The observed canonical `--self-test --coverage` artifact has no `files`, no top-level `missing_lines` or `uncovered_lines`, and no covered-line map entries.

Therefore the exact producer limitation is: `fkst-framework --self-test --coverage` currently writes `coverage.json` without usable line metadata for package production Lua files. The package-root fallback is not justified in `scripts/run.sh`; a producer change that emits package test coverage with repository-path line metadata should be proposed separately.
