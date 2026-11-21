# Rust Game Server Lab

**English** | [한국어](README.ko.md)

I spent years writing game servers in C++. This project is my way of finding out whether a server of MMO scale can be built in Rust, by building one and measuring it. Along the way I want answers to a few concrete questions.

- Does a lock-free game logic structure (zone threads plus per-player actors) come naturally in Rust?
- Can currency and items be made impossible to lose or duplicate, even when a server dies mid-operation?
- Can a single process hold 10,000 concurrent players at a 10 Hz tick?
- How much of the discipline people used to enforce by hand can the compiler and lints take over?

This is not a finished game or a framework. Design documents, plans, and code are published as the work proceeds. Documents are written in English, and each has a Korean version next to it as `name.ko.md`.

## Where it stands

The project is at the pilot stage: verifying the skeleton of the design before building content on top of it. The full plan is in [docs/plan/README.md](docs/plan/README.md).

What exists today is a Cargo workspace with ten crates. The game server is still a skeleton, but it already refuses to come up when the database is missing, the schema is out of date, or a secret is missing. The gateway accepts client connections, reads and writes frames, and closes connections that do not authenticate in time. It cannot let anyone in yet, because the entry server that issues login tokens does not exist. A headless bot opens thousands of connections against it for smoke and stress runs, and the gateway reports its connections and runtime threads as metrics. The publisher login server works: it creates guest accounts, logs them in, and hands out short-lived single-use entry tickets that the game side can verify. The GUI test client opens a window where you can create a guest account and log in against that server. The operations tool is a web page that shows which servers are up, lists accounts, sanctions or restores an account, and keeps a record of every operator action. Around these sit a dev environment for the databases and a quality gate. It builds and passes its tests on Windows (MSVC) and on Linux. The Linux run happens in a container.

The message definitions and the frame codec are written and tested, but not yet wired into the servers. Next up are the entry server and gateway sessions, so that a ticket from the login server actually gets a client into a game server. The launcher, bot, battle server, zones, and actual content come after that, in the order laid out in the plan.

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
  protocol/   message definitions (.proto) and the message ID table
  net/        frame codec and server-to-server envelope codec
  gateway/    gateway binary
  publisher/  publisher login server binary (guest login, entry tickets)
  ops/        operations tool, a web server (server status, connections, accounts, audit)
  bot/        headless bot that opens many connections (smoke and stress runs)
  client/     GUI test client (Bevy and egui)
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

## Where things are

- `crates/` holds the source, `docs/` the design and guides, and `migrations/` the database migrations.
- `dev/` holds local configuration, and `scripts/` holds commands to run and check the programs.
- `logs/` holds logs by process instance, `client-data/` holds test client credentials, and `target/` holds builds and test scratch files.

## Running it

You need Rust (the version is pinned in `rust-toolchain.toml`, and rustup installs it for you) and a Docker-compatible runtime for PostgreSQL 17 and Valkey 8. I use Rancher Desktop. Building also needs the Protocol Buffers compiler (`protoc`) on the PATH.

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

To try the login flow, start the login server and the test client in two terminals. Every server has its own config file under `dev/config/` and its own run script under `scripts/` (`.sh` and `.ps1`), and starts without the others. The scripts load `.env` for you. Run the `migrate` command once before the first start.

```sh
scripts/run-publisher.sh migrate     # once
scripts/run-publisher.sh run
scripts/run-ops.sh                   # operations tool at http://127.0.0.1:18300
cargo run -p client -- --profile alice
```

In the client window, press "Create guest account" and then "Log in". Each profile keeps its own guest account under `client-data/`, so several clients can run side by side.

A quick check that the connection path still works, and a heavier one:

```sh
scripts/smoke.sh       # starts the gateway, connects 70 bots, checks the counters, stops everything
scripts/stress.sh      # 2,000 connections by default; prints the bot summary and the gateway's metrics
```

The servers are meant to run on Linux, so the same gate also runs in a Linux container on the same runtime:

```sh
docker compose --env-file .env -f dev/compose.yaml run --rm linux-gate
```

## Branches

Work happens on `dev`, in small commits. `main` receives verified work only, regrouped into commits by category. The procedure is in [docs/git-workflow.md](docs/git-workflow.md).

## Contributing

This is a personal experiment. Only I have write access, and I do not accept pull requests. Questions and comments are welcome as issues.

## License

All rights reserved. This is a personal project. The source is published so it can be read and discussed; no license to use, modify, or redistribute it is granted.
