---
"fkst-hosted": patch
---

Add the dormant controller↔worker wire types for the engine→worker execution move (#161, increment 1 of #151): `ControlMessage::ResolvedDispatch` + `ResolvedDispatch`/`DispatchGoal`/`CloneSpec`/`OrnnPlan` (the controller resolves credentials/config and dispatches a self-contained payload), `CredentialRefreshRequest`/`CredentialRefreshResponse` (the fence-guarded mid-run GitHub-App token mint RPC, so the worker never holds the App private key), and `StatusReport`/`SessionStatus` (worker→controller status). Secret-bearing fields are zeroizing `SecretString`s serialized via dedicated helpers — `Debug` never leaks them. No handler constructs or consumes the types yet (the worker only gains a no-op match arm), so behaviour is unchanged; `PROTOCOL_VERSION` stays 1.
