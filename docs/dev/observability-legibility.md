# Observability legibility: local health verdict

`scripts/run.sh health` is the one-command local health check for `github-devloop` dogfood runs. Its first and only line is either:

- `HEALTHY`
- `N ANOMALIES NEEDING ATTENTION`

The governing practice is SRE-style health checking over structured telemetry: producers emit facts, and the reader-facing command aggregates them into a low-noise verdict. The script consumes `fkst-framework observe --json` and reuses the same cache as `scripts/run.sh board`; `scripts/run.sh board` prints the same verdict as its first line and then renders the full local board.

## Classification contract

`scripts/board.py` counts only explicit attention facts as anomalies:

- `terminal=true`, `disposition=terminal`, or `tag=DEAD_LETTER`
- queue/DLQ counts from generic observe data
- producer-owned terminal `failure_facts` with `error_class` and `fingerprint`
- explicit safety violations
- non-terminal entity dwell beyond the configured stall threshold

Expected transients are shown as informational activity, not anomalies. Current explicit signals include `outcome=retry-pending`, `error_class=retry-pending`, `error_class=marker-lag`, `outcome=deadline-defer`, `outcome=skip-foreign`, and `disposition=expected-transient`.

The renderer does not infer new package semantics from prose logs or GitHub labels. New department or engine disposition meanings must be emitted as structured facts by their producers before the board can render them as first-class classifications.

## Operator reading

Use:

```sh
scripts/run.sh health
```

For details, use:

```sh
scripts/run.sh board
```

If the first line is `HEALTHY`, expected transients may still be visible in the full board, but they are classified as self-healing or intentionally skipped work. If the first line reports anomalies, the `Anomalies needing attention` section contains the type, queue or entity, and producer-owned context such as `error_class`, `fingerprint`, `terminal`, or `tag`.

`--refresh`, `--ttl`, and `--stall` match `scripts/run.sh board`. `--refresh` bypasses the local TTL cache; `--stall` controls the non-terminal dwell budget used for stall suspects.
