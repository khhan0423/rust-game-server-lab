# Design 10. Development Tools: Unit Tests, Test Client, and Server Launcher

**English** | [한국어](10-dev-tooling.ko.md)

- Status: **Draft** (2026-11-19). The owner set the next tasks: "Prepare unit tests, prepare a test client, and prepare a server launcher."
- Related documents: `docs/plan/README.md` (execution plan: pilot stage, development stages, test scenarios), `docs/RUST_GUIDELINES.md` Section 2 (workspace), Section 11 (tests), Section 13 (crates), `docs/protocol/session-contract.md` (contract the test client must follow), `08-operations-observability.md` (health checks, logs)

## 0. Decision Summary

| # | Decision | Details |
|---|---|---|
| T1 | Order | ① Workspace skeleton and unit tests → ② Server launcher → ③ Test client. The test client comes last because it needs a server to connect to. |
| T2 | Quality gate | All work must pass `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test` before it's complete (AGENTS.md). |
| T3 | Development DB and Valkey | Run them in **Rancher Desktop** (dockerd/moby container engine). Keep using the Docker CLI and `docker compose` as before. Migrate from Docker Desktop (owner's decision, 2026-11-19). Run Linux build and test checks in a container (`linux-gate`) in the same Rancher Desktop environment. Don't maintain a separate WSL distribution (owner's decision, 2026-11-20). |
| T4 | Test client form | **A Windows GUI application with a window** (owner's decision). Scenario and load modes that run multiple bots can also run without a window for automated tests. |
| T5 | Server launcher form | **A GUI application written in Rust** (owner's decision). Add `crates/launcher`. |
| T6 | GUI and graphics | **Test client: Bevy (0.20) + bevy_egui.** It's the most advanced graphics engine in Rust (based on wgpu, with support up to real-time path tracing). Draw the field, players, NPCs, and AOI cells in 2D/3D. Use egui for the panels (login, inventory, packet log). **Server launcher: eframe (egui, wgpu renderer).** It's best suited to tool screens such as status boards, log windows, and graphs. The two programs share the same egui widget code. The default egui font doesn't include Korean glyphs, so add a Korean font (owner's decision: "Use the most advanced GUI or graphics," 2026-11-19). |
| T7 | Crate layout | `crates/launcher` (eframe GUI), `crates/client` (Bevy GUI test client), `crates/bot` (headless scenario and load test bot). Dependency direction: `client -> net -> protocol`, `bot -> net`. `launcher` doesn't depend on game crates and only handles processes and `/health`. Use GUI crates only in these tool crates. Keep them out of server binaries. |

---

## 1. Current State

The repository contains only design documents and tool configuration (`rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`), with no Rust code. All three tasks therefore start with the **workspace skeleton**.

## 2. Workspace Skeleton and Unit Tests

### 2.1 What to Build

Follow the layout in Section 2 of the guide, but create each crate at the stage when it's first needed (don't create empty crates ahead of time). Start with only `infra`, `db`, `server`, and `gateway`.

```
Server/
├─ Cargo.toml        [workspace], [workspace.dependencies], [workspace.lints] (Guide, Appendix A)
├─ crates/
│  ├─ protocol/      Packet definitions (prost)
│  ├─ game/          Game rules (no I/O)
│  ├─ db/            sqlx queries
│  ├─ net/           Framing codec, links
│  ├─ gateway/       Gateway binary
│  ├─ server/        Binaries by role, such as Game/combat/Entry
│  └─ bot/           Test client
└─ migrations/
```

Once the skeleton exists, the three quality gate commands run for the first time.

### 2.2 Types of Tests

| Type | Target | Method | External Dependencies |
|---|---|---|---|
| Unit tests | `protocol`, `game` (pure logic) | `#[cfg(test)] mod tests` in the same file. Encoding round trips and boundary values. | None |
| Actor tests | Player actor, zone logic | Send a command through a channel and check the result message. Use `tokio::time::pause()` for virtual time (Guide, Section 11). | None |
| DB integration tests | Functions in the `db` crate | `sqlx::test` creates an empty DB for each test, applies the migrations, then runs the test. | **PostgreSQL required** |
| Server integration tests | Flows that involve multiple processes | Start the processes with the launcher and check them with test client scenarios (Sections 3 and 4). | PostgreSQL, Valkey |

- Don't use `sleep` to align timing in tests. Wait for an event or a channel.
- Use real PostgreSQL for DB integration tests. A fake DB can't verify core behavior such as transactions, constraints, and idempotency keys.

## 3. Server Launcher

### 3.1 What It Does

Start or stop the entire server cluster on a development PC with a single command. Since systemd handles production (Document 08, M8), the launcher is **for development and testing**.

```
launcher                   Open the window with service controls, status, and recent output
launcher --headless up     Start in dependency order and stop in reverse order on Ctrl+C
launcher --headless check  Start in dependency order, then stop in reverse order and report the result
```

Each start waits for `ready admin=<address> ...` on the child's stdout before starting the next service. Graceful stop asks the admin port to shut down using a credential generated for the child; the launcher kills the child if the deadline expires. The window can also restart or kill one service and open the operations tool at the address it announced.

### 3.2 Topology File (Example)

The default is `dev/config/launcher.toml`. Use `--topology <file>` to select another topology. Paths are relative to the working directory, which the run scripts set to the workspace root.

```toml
bin_dir = "target/debug"
env_file = ".env"
ready_timeout_seconds = 60
stop_timeout_seconds = 15

[process]
name = "launcher"
environment = "development"
log_level = "info"

[[services]]
name = "publisher"
binary = "publisher"
config = "dev/config/publisher.toml"
args = ["run"]

[[services]]
name = "server"
binary = "server"
config = "dev/config/server.toml"
args = ["run"]
depends_on = ["publisher"]
```

- Don't put secrets such as DB connection details in this file. Pass them through environment variables (Document 08, Section 8).
- The default topology also starts `gateway` and `ops`, with `ops` last. Names must be unique and dependencies must be known and acyclic.

### 3.3 Each Server Runs on Its Own; the Launcher Only Helps (owner decision, 2026-11-20)

The launcher is a convenience on top of servers that already start by themselves. It is never the only way to start one.

- **Every server has its own config file.** One TOML file per server under `dev/config/` (`publisher.toml`, `server.toml`, `gateway.toml`, `ops.toml`). A server reads only its own file. Secrets stay in environment variables.
- **Every server has its own run script.** `scripts/run-<server>.sh` and `.ps1` load the environment and start that one server with its config file. A server must start with no other server running, and report what it is waiting for instead of failing in a confusing way.
- **The launcher uses the same config files and the same commands as the scripts.** It adds order, status, logs, and the buttons to stop, restart, and kill.

Beyond starting servers, the launcher is where these jobs live:

| Job | What it means |
|---|---|
| Patch design data | Apply a new design data bundle (design 11) to running servers: validate it, tell the servers to reload, and show which servers took it |
| Start with data from a chosen folder | Start the server set against a design data folder picked in the launcher, so a planner's working copy or an older bundle can be tried without touching the default one |
| Start a chosen version | Keep several built versions side by side and start the server set from one of them, for example to reproduce a bug on last week's build |

These three depend on design data (design 11) and on built versions being kept in separate folders, so they arrive after the basic launcher.

## 4. Test Client (`bot` Crate)

### 4.1 What It Does

- **It is the reference implementation of the session contract.** It connects, authenticates, reconnects, and recovers from disconnects as specified in the session contract. Client developers (Unity or similar) can use it to check, "Does the server really behave this way?"
- Provide two modes.

| Mode | Purpose | Interface |
|---|---|---|
| Interactive | Developers send one packet at a time to check behavior. | Text command window. Commands such as `login`, `enter`, `buy 1001`, `move 10 20`, and `disconnect`. |
| Scenario | Repeat the same flow automatically, run multiple bots concurrently, and load test. | Write scenarios in Rust and run them with N bots. Results show latency distributions (`hdrhistogram`) and failure counts. |

### 4.2 First Scenario

Reproduce the approved shard simulation (Document 04, Section 2) as a scenario.

1. Connect 3 bots for the first time → verify that each is assigned to a different shard.
2. Have each bot buy an item → verify that each item is saved to its bot's Game DB.
3. Have bot 1 send mail to bot 2 → verify that it arrives only once in bot 2's mailbox (send the message twice on purpose to check idempotency as well).
4. Click **Kill** on bot 2's Game server → verify after restart that the mail is still there.

## 5. Completion Criteria for Each Stage

| Stage | Completion Criteria |
|---|---|
| ① Skeleton and unit tests | **Complete (2026-11-19).** 4 crates (`infra`, `db`, `server`, `gateway`), passing quality gate, and passing DB and server startup tests with real PostgreSQL. |
| ② Server launcher | The window shows service status and output and can start, stop, restart, and kill processes. `launcher --headless check` verifies readiness and clean shutdown. |
| ③ Test client | Connect and authenticate in interactive mode. Connect N bots and print a latency distribution in scenario mode. |

## 6. Decisions to Make

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q1 | Development container environment | Confirmed: Rancher Desktop (T3) | - |
| Q2 | Test client form | Confirmed: Windows GUI (T4) | - |
| Q3 | Server launcher form | Confirmed: Rust GUI (T5) | - |
| Q4 | GUI library | Confirmed: T6 | - |
| Q5 | Test client language | Confirmed: Rust (Bevy) | - |
