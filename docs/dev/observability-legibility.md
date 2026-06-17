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

## Topology surface contract

This spec amendment follows PDCA/OODA closed-loop control and SRE topology-dashboard practice: an operator dashboard may improve orientation, but it must remain a projection of an authoritative artifact instead of becoming a second source of topology truth.

The accepted machine contract is unchanged. A package that opts in with `M.spec.graph_json = true` exposes topology through `graph_json()`, and that function returns the canonical `fkst.graph.v1` artifact. Any board topology view derives from that artifact; dashboard code must not invent, persist, or infer topology from prose, labels, or runtime layout.

The board must not render an inline `## System topology` section merely because `graph_json()` exists. Inline rendering is allowed only when the implementing spec or patch carries documented evidence that the `fkst-dev board` workflow needs inline topology to close an operator feedback loop, and that linking or reusing the authoritative artifact is insufficient for that workflow. Without that evidence, the correct board behavior is to link to or reuse the authoritative artifact and keep the dashboard as a thin consumer.

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
