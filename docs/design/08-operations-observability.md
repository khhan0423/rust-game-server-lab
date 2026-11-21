# Design 08. Operations and Observability

**English** | [한국어](08-operations-observability.ko.md)

- Status: **Draft** (2026-11-18). 2026-11-19: Operations commands don't block the game (Section 2), DB time is measured separately at each stage (Section 3), and configuration and secrets were added (Section 8).
- Related documents: `04-database.md` 1.1 (three types of logs and their paths), `03-data-ownership.md` 9.7 (server registration in Redis)

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| M1 | Principle | Don't build a monitoring server. **Servers only expose data in standard formats**. Standard tools collect, display, and send alerts about it. |
| M2 | Admin port | Every server (Gateway, Entry, Game, combat, trade) opens a **private network only HTTP admin port** separate from its game port (`axum`). |
| M3 | Metrics | Expose `/metrics` in Prometheus format with `metrics` + `metrics-exporter-prometheus`. |
| M4 | Logs | Write operations logs and game analytics logs **only to JSON Lines files**. Fluent Bit handles collection and routing. **Never write operations logs to any DB.** |
| M5 | Tools | Free and open source. Candidates are Prometheus + Grafana + Alertmanager and Fluent Bit, with alerts via a Discord webhook. Finalize the choice **before the first live launch**. |
| M6 | Cost | 0 during development. For the first live launch, run the tools on **one operations VM** (Section 6). |

---

## 1. Four Types of Observability

| Type | Question | Method |
|---|---|---|
| Health checks | Is it alive? Can it accept work? | `/health/live`, `/health/ready` |
| Metrics | How well is it running? | `/metrics` → Prometheus → Grafana |
| Logs | What happened? | JSON files → Fluent Bit |
| Alerts | Does someone need to look at this? | Prometheus alert rules → Alertmanager → Discord |

## 2. Admin Port

| Path | Details | Consumer |
|---|---|---|
| `/health/live` | Is the process responding? Any response is enough for OK. | Process supervisor (systemd or similar). Restarts the process on failure. |
| `/health/ready` | Can the server accept work? Required connections (DB, Redis, and required interserver links) are healthy, and the server isn't draining. | **Load balancer.** Sends no connections to Gateway or Entry servers that aren't ready. |
| `/metrics` | Metrics (Prometheus text format) | Prometheus |
| `/admin/...` | Start draining, maintenance mode, change log level, Kick | Operations tool. **Authentication required.** |

- Open the admin port **only on the private network**. `/admin` requires authentication for the operations tool account (a signed token or mTLS).
- Admin HTTP runs as a task on the I/O runtime. Zone threads and player actors don't receive admin requests directly. They receive messages when needed.
- **Operations commands don't stop the game.** If an `/admin` handler needs a DB operation, the admin task calls `.await` itself and passes the result to zones and actors through messages or snapshot replacement (Document 01, Section 5.7). If even one operations command waits for the DB on a game thread, the entire server stops.
- `ready` turns on only after the startup barrier (Document 01, Section 10.0) completes.
- When the state changes, update `ready` along with Redis `server:{id}.state` (Document 03, Section 9.7). When draining begins, `ready` fails so no new connections arrive.

## 3. Metrics

```rust
metrics::counter!("tick_overrun_total", "zone" => zone_label).increment(1);
metrics::histogram!("tick_duration_ms").record(elapsed_ms);
metrics::gauge!("ccu", "shard" => shard_label).set(ccu as f64);
```

- `metrics` uses atomics internally and takes no locks. Zone threads and player actors can record metrics every tick.
- Use `snake_case` for metric names and append the unit (`_ms`, `_bytes`, `_total`). Keep the number of distinct label values low (don't use values such as player IDs as labels).

| Area | Metrics |
|---|---|
| Connections | Concurrent users per shard and per Gateway. Login successes and failures. **Counts by disconnect reason** (Document 02, Section 2.3). TCP series carry a `listener` label: `net_connections_current`, `net_connections_accepted_total`, `net_connections_rejected_total`, `net_connections_closed_total`, `net_frames_received_total`, `net_frames_sent_total`, `net_bytes_received_total`, `net_bytes_sent_total`. Rejected and closed connections also carry `reason`. |
| Entry | Publisher API response time and failures, number of new characters created, and assignments per shard. |
| Zone thread | **Tick duration and tick overrun count**, number of active and dormant InGameSpace instances. |
| Player actor | Actor count, commands waiting in the inbox, command processing time, DB function duration, retry count, and **slow command count** (Document 01, Section 5.8). |
| Channels | Channel usage between threads and the number of times channels fill up. |
| DB | Connection pool wait time, query time, failure count, and retry count (deadlocks). |
| DB (Approach B, zone and InGameSpaceManager) | **Measure three intervals separately**: time spent waiting in the DB task channel, execution time, and time spent handling the result (`DbResult`) in the zone. When it slows down, this makes it easy to tell whether the problem is the DB or the result handling code. |
| Startup | Time per stage and number of records handled by startup recovery (Document 01, Section 10.0). |
| Redis | Response time, failure count, and lease renewal failures. |
| Sessions | Retransmission buffer usage, Replay and Resync counts. |
| Interserver | Unacknowledged outbox message count, retransmission count, and link reconnect count. |

### 3.1 Alerts (Starting Point)

| Alert | Condition (Example) |
|---|---|
| Server missing | Prometheus scrape fails for 1 minute. |
| Tick overruns | Tick overrun rate of at least 5% over 1 minute. |
| DB latency | Connection pool wait p99 of at least 1 second. |
| Outbox backlog | Unacknowledged messages keep increasing for 5 minutes. |
| Disconnect spike | At least 3 times the normal disconnect count (suspect a server network failure, `LocalNetwork` in Document 02, Section 2.3). |

## 4. Logs

The top-level runtime folders are `logs/` for file logs, `client-data/` for test client credentials, and `target/` for builds and test scratch files. Each log belongs in its process instance directory. Tests can set `log_root` to a directory under `target/`.

| Log | How It's Written | File |
|---|---|---|
| **Operations log** | `tracing` macros through the shared formatter in `infra`. | `<log_root>/<name>-<instance>/ops.<YYYY-MM-DD>.jsonl` (UTC daily rotation) |
| **Game analytics log** | Serialize a typed event enum with `serde_json`. | `<log_root>/<name>-<instance>/game-analytics.<YYYY-MM-DD>.jsonl` (UTC daily rotation) |
| Transactional log | In the same Game DB transaction (not a log file). | - |

### 4.1 Operations Log

Each program sets its identity in `[process]`: `name` (`publisher`, `game`, `gateway`, `ops`, `bot`, or `client`), `instance` (default `"1"`, or the profile name for the client), `environment` (`development`, `friends_test`, `alpha`, `beta`, or `live`), and `log_root` (default `"logs"`). The log directory and file name are derived from these settings. Programs set up logging once using the shared entry point in `infra`.

Every event is written as one JSON object on one line. Its first seven keys, in order, are `ts` (RFC 3339 UTC with six fractional digits and a `Z` suffix), `level` (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`), `env`, `server`, `instance`, `target`, and `msg`. Fields from outer spans come next, then fields from inner spans, then event fields. Inner spans override outer spans on a name clash, and the event overrides spans. A field that clashes with a standard key gets a trailing underscore. Numbers and booleans keep their JSON types; other values are strings.

Use the same field names for the same values: `conn_id`, `session_id`, `player_id`, `account_id`, `request_id`, `peer`, `reason`, `error`, and `elapsed_ms` (elapsed time in milliseconds).

```json
{"ts":"2026-11-21T12:00:00.123456Z","level":"INFO","env":"development","server":"gateway","instance":"1","target":"gateway::connection","msg":"connection closed","conn_id":69,"peer":"127.0.0.1:50000","reason":"auth_timeout","elapsed_ms":1000}
```

- `non_blocking` puts the log entry on a channel and returns immediately. One background thread writes to the file. **Disk I/O doesn't block the zone thread.**
- Drop entries when the buffer fills up so the game doesn't block. Record the number of dropped entries as a metric.

### 4.2 Game Analytics Log: Typed Events

```rust
#[derive(Serialize)]
#[serde(tag = "log_type")]
pub enum AnalyticsEvent {
    ItemAcquired { player: PlayerId, item: ItemTemplateId, source: AcquireSource },
    DungeonCleared { player: PlayerId, dungeon: DungeonId, duration_ms: u32 },
    SessionEnded { player: PlayerId, reason: CloseReason, play_ms: u64 },
}
```

- Design and analytics use these logs, so the fields must be correct. Defining events as an enum lets the compiler **check field names and types at compile time**.
- `log_type` is the routing key for Fluent Bit.
- Add common fields (timestamp, shard, server ID) to every event.
- Write these logs the same way as operations logs (channel + background thread). Drop entries when the buffer fills up and record the drops as a metric.

### 4.3 Panics and Logs

With `panic = "abort"` (Document 01, Section 10.2), a panic loses the last few lines the background thread hasn't written yet. On normal shutdown, the server writes all remaining logs before exiting. Core dumps help cover what was lost, and records that must be exact are in the Game DB transactional log.

## 5. Fluent Bit

### 5.1 Deployment

| Stage | Deployment |
|---|---|
| Development and small deployments | One Fluent Bit instance per server reads the files and sends them directly to their destination (Log DB or file archive). |
| As the server count grows | **Two tiers: agent + aggregator.** Fluent Bit on each server only reads and forwards the logs (forward). The aggregators (1 to 2) on the operations VM route logs and write to the Log DB. |

```
[Each server]   Game server → JSON files → Fluent Bit (agent)
                                                   │ forward
[Operations VM]                           Fluent Bit aggregator (routing rules only here)
                                            ├─ analytics.* ──> Log DB (pgsql output, connections = aggregators × workers)
                                            ├─ analytics.* ──> File archive (compressed, long term)
                                            └─ ops.*       ──> File archive (no DB writes)
```

- Fluent Bit's PostgreSQL output opens **one connection per worker**. With two tiers, the number of Log DB connections stays fixed regardless of the server count.
- Fluent Bit runs in a **separate process** from the game server. Set CPU and memory ceilings with systemd (`CPUQuota`, `MemoryMax`) and Fluent Bit buffer limits. Use filesystem buffers so they persist across restarts.
- Switching from one tier to two **doesn't change game code**. Only Fluent Bit configuration changes.

### 5.2 Log DB

- Fluent Bit writes one row per log entry with `tag`, `time`, and `data JSONB`.
- **Partition tables by time** (monthly or daily). Detach old partitions to archive or delete them.
- If structured tables are needed, move the data with a **periodic transformation job**. Don't use triggers (Guide, Section 8).
- Send only the tags needed for analytics to the DB and archive all original data in compressed files.
- Analytics logs use **at least once** delivery, so duplicates are possible.

## 6. Tools and Cost

| Stage | Setup | Cost |
|---|---|---|
| **Development** | Only an admin port and JSON logging in the code. Check logs in the terminal and in files. | 0 |
| When dashboards are needed during development | Prometheus + Grafana in containers on the development PC's Rancher Desktop. | 0 |
| **First live launch** | **One operations VM**: Prometheus, Grafana, Alertmanager, and a Fluent Bit aggregator. A Fluent Bit agent on each server. Alerts via a Discord webhook. | One VM |
| Alternative | Grafana Cloud free plan (check the free limits at the time). | 0 within the limits |
| Optional | Add a searchable log store such as Loki when operations log search is needed. | Operations VM resources |

All the tools are free and open source. The costs are the server they run on and the time it takes to configure them.

## 7. What the Operations Tool Shows

The operations tool (GM tool) combines information from other sources without collecting it directly.

```
Operations tool screen
 ├─ Server list and status  ← Redis server:{id} (role, address, load, Active/Draining)
 ├─ Graphs                  ← Grafana (embed or link)
 ├─ Current alerts          ← Alertmanager
 └─ Operations commands     ← /admin on each server (drain, maintenance mode, announcement, Kick)
```

## 8. Configuration and Secrets

If you commit configuration files with real DB credentials, internal hosts, and staff contact details, anyone who gets the repository can access the production DB. Keep configuration and secrets separate.

| Type | Where to Keep It | Examples |
|---|---|---|
| General configuration | Configuration files in the repository | Ports, thread counts, channel capacity, tick interval |
| Secrets | **Outside the repository.** Environment variables or a secret store (including a separate file with restricted permissions). | DB connection strings, Redis passwords, shared secrets between servers, token signing keys, alert webhook URLs |

- Keep only example files (`*.example`) with blank secret values in the repository.
- Don't print secrets in logs or error messages. Redact them from `Debug` output for configuration structs as well.

## 9. Remaining Decisions

| # | Question | Notes |
|---|---|---|
| M7 | Tools | **Confirmed (2026-11-19): One self-managed operations VM for the first live launch** (Section 6). Move to the cloud later if needed. |
| M8 | Process supervision | **Default (2026-11-19): systemd.** Final confirmation in Document 06 on the cluster. |
| M9 | Operations log retention | **Default (2026-11-19): 90 days** (compress old files). Keep them longer if required by law. |
