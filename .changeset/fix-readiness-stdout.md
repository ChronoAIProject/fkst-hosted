---
"fkst-hosted": patch
---

Fix the session runner never reaching `running`: `supervise` writes its readiness markers to stdout, so the runner now drains both stdout and stderr into one merged buffer for the marker/panic scans (#50).
