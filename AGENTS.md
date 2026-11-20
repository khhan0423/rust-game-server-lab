# AGENTS.md

Rust MMO-style game server. The project owner is an experienced C++ game-server developer who is new to Rust, so every change must follow the project rules exactly; do not introduce idioms or structure the rules do not cover without asking.

## Source of truth

- `docs/RUST_GUIDELINES.md` defines naming, crate layout, error handling, concurrency, protocol, DB, logging, testing, and dependency rules. Read it before writing code.
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, and the root `Cargo.toml` `[workspace.lints]` enforce those rules. Do not edit them without the owner's approval.

## Gate (must pass before reporting work as done)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The dev environment runs inside Rancher Desktop: PostgreSQL and Valkey as containers (`dev/compose.yaml`), and the Linux check as `docker compose --env-file .env -f dev/compose.yaml run --rm linux-gate`. Do not install a separate WSL distribution. Run the Linux gate before promoting work to `main`. Builds use the committed `.sqlx/` query data (`SQLX_OFFLINE=true`); after changing any SQL or migration, run `scripts/sqlx-prepare.sh` and commit the result. Building needs `protoc` on the PATH. Message IDs are generated from the tables in `docs/protocol/message-catalog.md`; change a message there (and in the `.ko.md` twin) first, then the `.proto` files.

## Hard rules

- Crate dependency direction: `server -> game -> protocol`, `server -> db`, `server -> net -> protocol`, `gateway -> net`, `bot -> net`, `client -> net`. Binary crates (`server`, `gateway`, `bot`, `client`, `launcher`) may depend on `infra` (config, logging, metrics, admin port); `infra` depends on no workspace crate. `gateway` never depends on `game`. `launcher` never depends on `game`, `db`, or `net`. `publisher` (publisher login server) depends only on `db` (its Login DB module) and `infra`; game-side crates never touch the Login DB. `protocol` and `game` do no I/O. SQL and Redis-compatible store access live only in `db`. Never store currency, items, trade state, outbox or idempotency keys in Redis. Redis is a shared bulletin board: single commands only (no Lua, no `MULTI`/`EXEC`, no `WATCH`); read-decide-write logic belongs to the owning actor; races between two commands are fenced in the DB (see `docs/design/03-data-ownership.md` section 9).
- Design docs in `docs/design/` (01 threading, 02 session/network, 03 data ownership, 04 database, 07 login/account, 08 operations/observability) and the client contract `docs/protocol/session-contract.md` are binding. Code that deviates needs the doc changed first, with the owner's approval.
- No `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`println!`/`dbg!` outside tests. Use `Result` + `?` and `tracing`.
- Lint suppression only via `#[expect(lint, reason = "...")]`; never `#[allow]`.
- No stored procedures or triggers. All DB logic is `sqlx::query!` inside `db` crate functions; transactions are opened and committed in Rust; check `rows_affected` on every conditional write.
- No `unsafe`.
- IDs and money are newtypes; currency arithmetic uses `checked_*`; never floats for money.
- No `_ =>` arm when matching enums defined in this workspace.
- Game state is owned by one task (actor); communicate over bounded `mpsc` channels. Never hold a lock guard across `.await`.
- New external crates require the owner's approval and an update to the dependency table in the guide.
- Git: work and commit on `dev`. `main` receives only verified work, regrouped into category commits (`docs/git-workflow.md`). Never force-push `main`. Only the owner (and agents acting for the owner) write to the repository.
- Language: English is primary and Korean is secondary for repository-facing text. README, commit messages, and repository metadata are written in English; a Korean version of a document lives next to it as `name.ko.md`.
- Never name or reference any project outside this repository (names, code identifiers, internal details) in docs, code, or commit messages.
- When you introduce a Rust concept the owner may not know (lifetimes, trait objects, `Pin`, macros), explain it briefly in your report, relating it to the C++ equivalent.
