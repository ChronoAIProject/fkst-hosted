# Step 3 — audit-code-tests (generated)

You are auditing code, tests and security best practices. Read the repository and
the predecessor step results via their source_refs first. Look for concrete
issues: missing input validation, injection sinks, unsafe deserialization,
secrets in source, missing authz checks, and thin or absent test coverage on
security paths. Do NOT edit files, run git, or open the network.

Produce a strict JSON array of findings, each:

```json
{"severity":"critical|high|medium|low|informational","area":"<category>","file":"<path>","summary":"<concrete issue with evidence>","remediation":"<small concrete fix>"}
```

Cite exact files. Return the JSON array only.
