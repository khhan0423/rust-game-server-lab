# Rust Game Server Lab

**English** | [한국어](README.ko.md)

**An experiment to find out, by actually building one, whether an MMO-class game server can be written in Rust.**

The author has built game servers in C++ and is new to Rust. The goal is to answer these questions with code and measurements:

- Does a lock-free game logic structure (zone threads + per-player actors) come naturally in Rust?
- Can currency and items be made impossible to lose or duplicate, even when a server dies?
- Can a single process hold 10,000 concurrent players at a 10 Hz tick?
- How much of the discipline that people used to enforce by hand can the compiler and lints take over?

This is not a finished game or a framework. Design documents, plans, and code are published as the work proceeds.

## Status

The project is in its pilot stage: verifying the skeleton of the design. Progress is tracked in [docs/plan/README.md](docs/plan/README.md).

| Stage | Scope | Status |
|---|---|---|
| P0 | Workspace, quality gate, dev databases, config, logging, metrics, admin port | Done |
| P1 | Protocol and frame codec | Next |
| P2–P9 | Launcher, login and sessions, content actors, zones and lobby, cross-shard messaging, GUI test client, reconnection, load measurement | Planned |

So far it has been built and tested on Windows (MSVC) only. Linux has not been verified yet.

## Architecture at a glance

```
 Client ─ 1. log in ───► Publisher login server ── Login DB      accounts, login methods, entry tickets
   │                            ▲ verify ticket
   ├── 2. enter ───────► Entry server ── Account DB              character, home shard, login token
   │
   └── 3. play ────────► Gateway ─────────── Valkey              presence, which server hosts which space
                            │
                 ┌──────────┼──────────┐
               Game 1     Game 2     Game 3        Battle server pool (dungeons, PvP; no DB)
                 │          │          │
              GameDB 1   GameDB 2   GameDB 3
               shard 1    shard 2    shard 3

 Dev tools:  Launcher (GUI: starts, stops, and kills the server set)
             Test client (GUI: log in, walk around, buy, mail, reconnect)
             Bot (headless: scripted scenarios and load tests)
```

| Component | Role | Status |
|---|---|---|
| Publisher login server | Platform accounts and login methods (guest first, then Steam); issues entry tickets. Kept separate from the game | Planned (P3) |
| Entry server | Verifies the ticket, creates the character, assigns the home shard, issues the game login token | Planned (P3) |
| Gateway | The only thing clients connect to; routes messages and keeps sessions alive across reconnects | Process skeleton (P0) |
| Game server | Zones and player content for one shard | Startup barrier only (P0) |
| Battle server | Instanced dungeons and PvP, no database | Planned (D2) |
| Launcher | GUI tool that brings the whole server set up in order and can kill one server to test recovery | Planned (P2) |
| Test client | GUI client for trying every feature by hand | Planned (P7) |
| Bot | Headless client for automated scenarios and load tests | Planned (P3)

| Principle | What it means | Document |
|---|---|---|
| Clients connect only to the Gateway | Moving between servers is a route change inside the Gateway. A dropped connection can resume within a grace period | [Design 02](docs/design/02-session-network.md), [Session contract](docs/protocol/session-contract.md) |
| Combat runs on zone threads, content on player actors | A space (InGameSpace) is owned by one synchronous fixed-tick thread; inventory and currency are owned by one async actor per player. Game logic needs no locks | [Design 01](docs/design/01-threading.md) |
| A shard is a game DB plus its game servers | A player is assigned a home shard on first entry and it never changes. A game server connects only to its own shard's DB | [Design 04](docs/design/04-database.md), [Design 07](docs/design/07-login-account.md) |
| Cross-shard effects travel as messages | No server writes to another shard's DB. Messages are recorded before sending (outbox) and applied exactly once with idempotency keys | [Design 03](docs/design/03-data-ownership.md) |
| Currency changes only after the DB commit | SQL lives in Rust functions and is checked at build time. No stored procedures | [Guidelines §8](docs/RUST_GUIDELINES.md) |
| Every queue is bounded | When a queue is full, the sender does not wait; a defined policy applies | [Design 01 §7](docs/design/01-threading.md) |
| No traffic until startup completes | If any startup step fails, the server does not come up | [Design 01 §10](docs/design/01-threading.md) |

## Repository layout

```
crates/
  infra/      config, logging, metrics, admin port (shared by every server)
  db/         all database access; SQL lives only here
  server/     game server binary
  gateway/    gateway binary
migrations/   database migrations
dev/          PostgreSQL and Valkey for development
scripts/      quality gate scripts
docs/
  RUST_GUIDELINES.md   coding and design rules
  design/              design documents (threading, sessions, data ownership, database, space lifecycle, login, operations, payments, dev tooling)
  protocol/            session contract with the client
  plan/                execution plan, pilot stages, test scenarios
```

Crates are added in the stage that first needs them. Still to come: `protocol`, `net`, `game`, `publisher` (publisher login server), `bot`, `launcher` (GUI server launcher), and `client` (GUI test client).

Most design documents are currently written in Korean.

## Development

- Rust stable (`rust-toolchain.toml`) and a Docker-compatible runtime (PostgreSQL 17, Valkey 8)
- Setup

```sh
cp .env.example .env          # change the password; .env is never committed
docker compose --env-file .env -f dev/compose.yaml up -d --wait
```

- Quality gate (must pass before any change is considered done)

```sh
scripts/gate.sh        # Windows PowerShell: scripts/gate.ps1
# runs: cargo fmt --all -- --check
#       cargo clippy --workspace --all-targets -- -D warnings
#       cargo test --workspace
```

Tests run against a real PostgreSQL. They start the actual server processes and also check that a server refuses to start when startup cannot complete (no database, schema mismatch, missing secret).

## Branches

| Branch | Purpose |
|---|---|
| `dev` | Working branch, small commits |
| `main` | Verified work only, regrouped into commits by category |

The procedure is described in [docs/git-workflow.md](docs/git-workflow.md).

## Contributing

This is a personal experiment. Only the owner has write access, and external pull requests are not accepted. Questions and comments are welcome as issues.

## License

All rights reserved. This is a personal project. The source is published so it can be read and discussed; no license to use, modify, or redistribute it is granted.
