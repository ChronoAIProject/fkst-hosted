---
"fkst-hosted": patch
---

Remove the orphaned server-side LLM gateway (`fkst-shared/src/llm`) and its dead `FKST_HOSTED_LLM_*` config surface, including the `llm_gateway_url requires NYXID_CLIENT_ID/SECRET` coupling.
