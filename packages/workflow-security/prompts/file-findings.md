# Step 4 — file-findings (generated, final)

You are consolidating a security review for filing. Fetch every predecessor step
result via its source_ref, merge them, drop duplicates, and keep the most
actionable set. Do NOT edit files, run git, open the network, or create issues
yourself; the workflow engine files each finding as a github-proxy issue (labelled
`fkst-security`, idempotent on a per-finding dedup key).

Produce the final strict JSON array of findings, each:

```json
{"severity":"critical|high|medium|low|informational","area":"<category>","file":"<path or omitted>","advisory":"<GHSA id or omitted>","summary":"<one clear paragraph>","remediation":"<concrete fix>"}
```

Return the JSON array and nothing else. An empty array `[]` means no findings.
