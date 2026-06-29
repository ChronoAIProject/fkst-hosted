---
"fkst-hosted": minor
---

The GitHub App webhook now auto-triggers a pod-per-session run on a qualifying
`issues.opened`: parse the issue, mint creds from the stored NyxID binding,
build the SessionSpec + Secret, and launch the Job — the token-less entrypoint
that completes milestone #9.
