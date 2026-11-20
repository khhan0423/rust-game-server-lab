# Rust Game Server Lab

**English** | [한국어](README.ko.md)

I spent years writing game servers in C++. This project is my way of finding out whether a server of MMO scale can be built in Rust, by building one and measuring it. Along the way I want answers to a few concrete questions.

- Does a lock-free game logic structure (zone threads plus per-player actors) come naturally in Rust?
- Can currency and items be made impossible to lose or duplicate, even when a server dies mid-operation?
- Can a single process hold 10,000 concurrent players at a 10 Hz tick?
- How much of the discipline people used to enforce by hand can the compiler and lints take over?

This is not a finished game or a framework. Design documents, plans, and code are published as the work proceeds. Most design documents are currently written in Korean.

## Where it stands

The project is at the pilot stage: verifying the skeleton of the design before building content on top of it. The full plan is in [docs/plan/README.md](docs/plan/README.md).

What exists today is a Cargo workspace with four crates (`infra`, `db`, `server`, `gateway`), a startup sequence that refuses to come up when the database is missing, the schema is out of date, or a secret is missing, a dev environment for the databases, a quality gate script, and 14 tests. So far everything has been built and tested on Windows (MSVC) only; Linux is not verified.

Next up is the protocol and frame codec, followed by the launcher, then login and sessions. The publisher login server, entry server, GUI test client, bot, battle server, zones, and actual content come after that, in the order laid out in the plan.

## How it is put together

The target shape of the whole system, when the pilot is done:

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

A few decisions behind that picture:

- Clients connect only to the Gateway. Moving between servers is a route change inside the Gateway, and a dropped connection can resume within a grace period ([design 02](docs/design/02-session-network.md), [session contract](docs/protocol/session-contract.md)).
- Combat runs on zone threads, player content on per-player actors. A space is owned by one synchronous fixed-tick thread; inventory and currency are owned by one async actor per player. Game logic needs no locks ([design 01](docs/design/01-threading.md)).
- A shard is a game database plus its game servers. A player is assigned a home shard on first entry and it never changes; a game server connects only to its own shard's DB ([design 04](docs/design/04-database.md), [design 07](docs/design/07-login-account.md)).
- Cross-shard effects travel as messages, never direct writes to another shard's DB. Messages are recorded before sending (outbox) and applied exactly once with idempotency keys ([design 03](docs/design/03-data-ownership.md)).
- Currency changes only after the DB commit. All SQL lives in Rust functions and is checked at build time; there are no stored procedures ([guidelines](docs/RUST_GUIDELINES.md)).
- Every queue is bounded. When a queue is full, the sender does not wait; a defined policy applies.
- A server accepts no traffic until startup completes. If any startup step fails, it does not come up.

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

## Running it

You need Rust stable (`rust-toolchain.toml`) and a Docker-compatible runtime (PostgreSQL 17, Valkey 8).

```sh
cp .env.example .env          # change the password; .env is never committed
docker compose --env-file .env -f dev/compose.yaml up -d --wait
```

The quality gate must pass before any change is considered done:

```sh
scripts/gate.sh        # Windows PowerShell: scripts/gate.ps1
# runs: cargo fmt --all -- --check
#       cargo clippy --workspace --all-targets -- -D warnings
#       cargo test --workspace
```

Tests run against a real PostgreSQL. They start the actual server processes, and they also check that a server refuses to start when startup cannot complete: no database, schema mismatch, or a missing secret.

## Branches

Work happens on `dev`, in small commits. `main` receives verified work only, regrouped into commits by category. The procedure is in [docs/git-workflow.md](docs/git-workflow.md).

## Contributing

This is a personal experiment. Only I have write access, and I do not accept pull requests. Questions and comments are welcome as issues.

## License

All rights reserved. This is a personal project. The source is published so it can be read and discussed; no license to use, modify, or redistribute it is granted.
