# Metrics reference

The `/metrics` endpoint (port 3333) exposes every Prometheus metric in text format, which
is the fastest way to check live state without reading logs:

```bash
curl -s http://localhost:3333/metrics | grep eink_
```

The names below are read by a test (`metrics::tests::the_metrics_the_runbook_names_are_the_ones_that_exist`)
so that renaming a metric cannot leave this table pointing at something that no longer
exists.

| Symptom | Metric to check |
|---|---|
| Sessions not submitting | `eink_sessions_submitted_total`, `eink_sessions_active` |
| OCR failures | `eink_ocr_requests_total{result="error"}` |
| OCR slow | `eink_ocr_duration_seconds` buckets |
| OCR cache not helping | `eink_ocr_cache_total{result="hit"}` / `{result="miss"}` |
| Which provider answered | `eink_ocr_provider_wins_total{provider="local"}` |
| WebSocket drops | `eink_ws_connections_active` |
| Long-poll timing out too fast | `eink_long_poll_total{result="timeout"}` |
| Webhook not firing | `eink_webhook_total{result="error"}` |
| Webhook receiver was down and recovered | `eink_webhook_total{result="retried"}` |
| Unexpected session expiry | `eink_sessions_expired_total` |
| Disk filling up | `eink_sessions_stored`, `eink_session_content_bytes` |
| A gateway route falling over | `eink_http_requests_total{status="500"}` |

Every fixed vocabulary is seeded at zero at startup, so a quiet server still answers a
query for `eink_ocr_requests_total{result="error"}` with `0` rather than "no data".

The Grafana dashboard on `odroid:3002` visualises these over time, which is usually
faster than grepping logs.
