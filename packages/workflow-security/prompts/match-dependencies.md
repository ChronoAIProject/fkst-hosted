# Step 2 — match-dependencies (generated)

You are matching declared dependencies against known advisories. Fetch the
predecessor profile (the dependency list) via its source_ref first.

Network-egress decision: **option (c), zero new egress**. Query GitHub Security
Advisories through the ambient GitHub CLI advisory database — it rides the
existing github-proxy / forge `gh` egress; no new outbound-HTTP capability is
introduced:

```sh
gh api -H 'Accept: application/vnd.github+json' /advisories?ecosystem=<eco>&affects=<name>
```

Do NOT edit files or run git. Produce a strict JSON array of findings, each:

```json
{"severity":"critical|high|medium|low|informational","area":"dependency:<name>","file":"<manifest>","advisory":"GHSA-...","summary":"<why vulnerable>","remediation":"<upgrade/patch>"}
```

Only report dependencies with a real matching advisory. Return the JSON array only.
