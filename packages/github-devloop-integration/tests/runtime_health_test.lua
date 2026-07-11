local t = fkst.test
local runtime_health = require("core.rollup_health")

local function verdict(snapshot)
  return runtime_health.verdict(snapshot, {
    now_seconds = 1781832600,
    stall_seconds = 1800,
  })
end

local function assert_clean(snapshot)
  local result = verdict(snapshot)
  t.eq(result.clean, true)
  t.eq(result.reason, "clean")
end

local function assert_dirty(snapshot, reason)
  local result = verdict(snapshot)
  t.eq(result.clean, false)
  t.eq(result.reason, reason)
end

local generated_at_ms = 1781832600000

local function promotion_snapshot(dead_letters, extra)
  local snapshot = {
    schema_version = 1,
    generated_at_ms = generated_at_ms,
    truncated = { deliveries = false, dead_letters = false },
    dead_letters = dead_letters or {},
  }
  for key, value in pairs(extra or {}) do
    snapshot[key] = value
  end
  return snapshot
end

local function promotion_verdict(snapshot, window_start_ms)
  return runtime_health.promotion_verdict(snapshot, window_start_ms or (generated_at_ms - 1800000), {
    stall_seconds = 1800,
  })
end

local function assert_promotion_clean(snapshot, window_start_ms)
  local result = promotion_verdict(snapshot, window_start_ms)
  t.eq(result.clean, true)
  t.eq(result.reason, "clean")
end

local function assert_promotion_dirty(snapshot, reason, window_start_ms)
  local result = promotion_verdict(snapshot, window_start_ms)
  t.eq(result.clean, false)
  t.eq(result.reason, reason)
end

-- Golden-master fixtures for the rollup merge runtime gate. Keep these exact
-- expected verdicts in sync with scripts/board.py:172-180 and the anomaly
-- selection below that drives the board.py health first line.
return {
  test_runtime_health_board_parity_clean_snapshot_is_clean = function()
    assert_clean({
      schema_version = 1,
      generated_at_ms = 1781832600000,
      queues = {
        { queue = "devloop_ready", depth = 1, pending = 1, in_flight = 0, retrying = 0 },
      },
      deliveries = {
        { delivery_id = "delivery-1", queue = "devloop_ready", status = "pending" },
      },
      dead_letters = {},
    })
  end,

  test_runtime_health_board_parity_retry_pending_transient_is_clean = function()
    assert_clean({
      entities = {
        {
          entity = "github-devloop/issue/owner/repo/623",
          events = {
            {
              queue = "devloop_ready",
              outcome = "retry-pending",
              error_class = "retry-pending",
              ts = "2026-06-14T09:59:30Z",
            },
          },
        },
      },
      queues = {
        { queue = "devloop_ready", ready = 0, leased = 0, retry = 1, dlq = 0 },
      },
      failure_facts = {
        {
          origin_queue = "devloop_ready",
          origin_dept = "github-devloop.implement",
          error_class = "retry-pending",
          fingerprint = "retry-pending:abc",
          attempt = 1,
        },
      },
    })
  end,

  test_runtime_health_board_parity_expected_transients_are_clean = function()
    assert_clean({
      entities = {
        {
          entity = "github-devloop/issue/owner/repo/623",
          events = {
            {
              queue = "devloop_observe_tick",
              outcome = "deadline-defer",
              ts = "2026-06-14T09:00:00Z",
            },
            {
              queue = "devloop_merge_ready",
              error_class = "marker-lag",
              ts = "2026-06-14T09:00:00Z",
            },
            {
              queue = "github-proxy.github_entity_changed",
              outcome = "skip-foreign",
              ts = "2026-06-14T09:00:00Z",
            },
          },
        },
      },
    })
  end,

  test_runtime_health_board_parity_dead_letters_snapshot_is_dirty = function()
    assert_dirty({
      dead_letters = {
        { delivery_id = "dead-1", queue = "devloop_ready", tag = "DEAD_LETTER" },
      },
    }, "dead-letter:devloop_ready")
  end,

  test_runtime_health_board_parity_queue_dlq_snapshot_is_dirty = function()
    assert_dirty({
      queues = {
        { queue = "devloop_ready", ready = 0, leased = 0, retry = 0, dlq = 1 },
      },
    }, "queue-dlq:devloop_ready:count=1")
  end,

  test_promotion_health_ignores_stale_permanent_dead_letter_audit = function()
    assert_promotion_clean(promotion_snapshot({
      {
        delivery_id = "dead-1",
        queue = "devloop_ready",
        dead_at_ms = generated_at_ms - 1800001,
        permanent = true,
        replayable = false,
      },
    }, {
      queues = {
        { queue = "devloop_ready", dlq = 1 },
      },
    }))
  end,

  test_promotion_health_blocks_dead_letter_at_window_boundary = function()
    assert_promotion_dirty(promotion_snapshot({
      {
        delivery_id = "dead-1",
        queue = "devloop_ready",
        dead_at_ms = generated_at_ms - 1800000,
        permanent = true,
      },
    }), "dead-letter:devloop_ready")
  end,

  test_promotion_health_missing_dead_letter_timestamp_fails_closed = function()
    assert_promotion_dirty(promotion_snapshot({
      { delivery_id = "dead-1", queue = "devloop_ready" },
    }), "dead-letter-time-invalid:devloop_ready")
  end,

  test_promotion_health_truncated_dead_letter_detail_fails_closed = function()
    assert_promotion_dirty(promotion_snapshot({}, {
      truncated = { deliveries = false, dead_letters = true },
    }), "dead-letter-detail-truncated")
  end,

  test_promotion_health_aggregate_only_queue_dlq_fails_closed = function()
    assert_promotion_dirty(promotion_snapshot({}, {
      queues = {
        { queue = "devloop_ready", dlq = 1 },
      },
    }), "dead-letter-detail-inconsistent:queue=devloop_ready:count=1:detail=0")
  end,

  test_runtime_health_board_parity_terminal_fact_snapshot_is_dirty = function()
    assert_dirty({
      failure_facts = {
        {
          origin_queue = "devloop_fixing",
          origin_dept = "github-devloop.fix",
          error_class = "framework_child_nonzero",
          fingerprint = "framework_child_nonzero:ghi",
          terminal = true,
        },
      },
    }, "terminal-failure:devloop_fixing")
  end,

  test_runtime_health_board_parity_stalled_entity_snapshot_is_dirty = function()
    assert_dirty({
      entities = {
        {
          entity = "github-devloop/issue/owner/repo/623",
          terminal = false,
          events = {
            {
              queue = "devloop_ready",
              ts = "2026-06-14T09:20:00Z",
            },
          },
        },
      },
    }, "stalled-entity:github-devloop/issue/owner/repo/623")
  end,

  test_runtime_health_board_parity_empty_entities_uses_fallback_timeline = function()
    assert_dirty({
      entities = {},
      entity_timeline = {
        {
          entity = "github-devloop/issue/owner/repo/624",
          terminal = false,
          events = {
            {
              queue = "devloop_ready",
              ts = "2026-06-14T09:20:00Z",
            },
          },
        },
      },
    }, "stalled-entity:github-devloop/issue/owner/repo/624")
  end,

  test_runtime_health_malformed_snapshot_fails_closed = function()
    assert_dirty("not a snapshot", "observe-malformed")
  end,

  test_runtime_observe_gate_maps_unavailable_and_malformed_to_visible_hold_reason = function()
    local unavailable_ok, unavailable_reason, unavailable_detail = runtime_health.runtime_observe_gate({
      clean = false,
      reason = "observe-unavailable",
    })
    t.eq(unavailable_ok, false)
    t.eq(unavailable_reason, "observe-unavailable")
    t.eq(unavailable_detail, "observe-unavailable")

    local malformed_ok, malformed_reason, malformed_detail = runtime_health.runtime_observe_gate({
      clean = false,
      reason = "observe-malformed",
    })
    t.eq(malformed_ok, false)
    t.eq(malformed_reason, "observe-unavailable")
    t.eq(malformed_detail, "observe-malformed")
  end,
}
