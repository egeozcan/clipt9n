# Latency benchmark reports

Latency evidence belongs in dated Markdown reports named `YYYY-MM-DD.md` in this directory. Run the repository's real-provider harness with either an Anthropic API key or an approved config file:

```bash
ANTHROPIC_API_KEY='...' scripts/bench.sh
# or
CLIPT9N_BENCH_CONFIG=/absolute/path/to/config.toml scripts/bench.sh
```

The harness builds the release binary, submits 20 representative samples, and writes the per-sample durations plus p50 and p95 latency. Before committing a report, replace the network placeholder and record, without exposing secrets:

- date and clipt9n commit;
- operating system and CPU architecture;
- provider and model;
- network context (Wi-Fi/Ethernet/VPN and relevant region);
- exact non-secret configuration;
- every sample duration, p50, and p95;
- failures, throttling, and timeouts;
- limitations that affect comparison with another report.

If provider credentials, network access, permission to incur provider charges, or an environment whose latency is representative are unavailable, record the run as `BLOCKED` in `TESTING.md`. Do not create a result report from mock-provider or estimated measurements.
