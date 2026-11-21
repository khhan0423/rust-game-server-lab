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

- Code review covers design, not only behavior. Reusable building blocks are types (`struct` + `impl`, with a `trait` for the part that varies) in library crates; binaries only wire them together. Nothing is written twice. Apply the design review list in `docs/RUST_GUIDELINES.md` section 16 to every change, and when delegating work, name the types and the crate they belong to in the brief.
- Crate dependency direction: `server -> game -> protocol`, `server -> db`, `server -> net -> protocol`, `gateway -> net`, `bot -> net`, `client -> net`. Binary crates (`server`, `gateway`, `bot`, `client`, `launcher`) may depend on `infra` (config, logging, metrics, admin port); `infra` depends on no workspace crate; `testkit` is permitted only in `[dev-dependencies]`. `gateway` never depends on `game`. `launcher` never depends on `game`, `db`, or `net`. `publisher` (publisher login server) depends only on `db` (its Login DB module) and `infra`; game-side crates never touch the Login DB. `ops` (operations tool web server) depends only on `infra`; it holds no database credentials and reaches data only through the owning service's operator API. Every server starts on its own from its config file under `dev/config/` and its `scripts/run-<server>` script; the launcher is a helper on top, never the only way to start a server. `protocol` and `game` do no I/O. SQL and Redis-compatible store access live only in `db`. Never store currency, items, trade state, outbox or idempotency keys in Redis. Redis is a shared bulletin board: single commands only (no Lua, no `MULTI`/`EXEC`, no `WATCH`); read-decide-write logic belongs to the owning actor; races between two commands are fenced in the DB (see `docs/design/03-data-ownership.md` section 9).
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

## Reporting to the owner

The owner's report is a status report of the whole project, in Korean and plain words, grouped by state. Use it whenever the owner asks where things stand, and after finishing a stage.

1. **진행한 것** (done): things that exist and were seen working. Each line says what it is and how it was checked.
2. **진행 중인 것** (in progress): things that were started and are not finished. Each line says what works today and what is missing.
3. **계획만 있는 것** (planned only): things with a design or a plan and no code. Each line names the document.
4. **정해 주실 것** (decisions needed): each question with the reason it must be decided and the recommendation.

Rules: an item appears in exactly one group. "Done" means verified working, never "code written". A skeleton that starts but does nothing useful is "in progress". Keep each item to one line and the whole report within one screen; details live in the documents. Do not use coded labels. End with the date and the latest commit on `main`.
