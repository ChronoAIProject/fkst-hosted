---
"fkst-hosted": patch
---

Add a per-worker outbound control-message queue to the controller's `WorkerRegistry`, drained by the heartbeat handler into `HeartbeatResponse.control` (#189, #151 increment 7a). This is the point-to-point dispatch channel the activation increment uses to deliver a `ResolvedDispatch` to a specific worker on its next heartbeat. The queue is kept separate from the liveness map so a `WorkerEntry` snapshot never carries a secret-bearing dispatch payload; `enqueue_control` never logs the payload; `take_control` drains FIFO exactly once; and an expired worker's queue is cleared. Dormant: nothing enqueues yet (7b does), so the heartbeat response stays `control: vec![]` and behaviour is byte-identical.
