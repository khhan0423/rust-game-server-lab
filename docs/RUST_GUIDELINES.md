# Rust Coding and Design Guide

**English** | [한국어](RUST_GUIDELINES.ko.md)

Code and design in this project conform to this document. Humans and AI agents alike follow the same rules.

Consistency is kept in three layers. The higher one wins.

1. **Rules enforced by tools**: `rustfmt`, `clippy`, compiler lints. Code that fails does not go in.
2. **Rules in this document**: design rules that tools cannot catch.
3. **Review**: checked with the checklist at the end of this document.

Even if you don't know Rust well, layer 1 catches most problems. When a rule is confusing, first check "do the tools pass it", then read this document.

---

## 0. Quality gate (must always pass)

Before committing, all three commands below must succeed at the workspace root.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Warnings are treated as errors (`-D warnings`). "Fix warnings later" is not allowed.
- When a lint must be turned off, do not use `#[allow(...)]`. Use `#[expect(lint_name, reason = "reason")]` instead. When the warning later disappears, `expect` itself emits a warning, so no dead exception remains.
- The configuration files are `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` at the root, and `[workspace.lints]` in `Cargo.toml`. Do not edit these without agreement.

---

## 1. Toolchain and environment

- The compiler version is pinned to an exact number in `rust-toolchain.toml` (currently `1.100.0`), so every PC and the Linux container build with the same compiler. To move to a newer version, change that number and the image tag of the `linux-gate` service in `dev/compose.yaml` in one commit, and fix any new lints in the same commit.
- `clippy.toml` only adjusts lint behavior (allowing `unwrap` etc. in tests). Turning lints on and off is done in `[workspace.lints]` in `Cargo.toml`.
- Edition is `2024`.
- **Do not use an IDE.** This project is developed with the CLI only (as an experiment). Basic flow:
  - `cargo check`: fast type check. Used most often
  - `cargo clippy`, `cargo fmt`, `cargo test`: the quality gate of chapter 0
  - `bacon clippy`: keep it in a terminal and it rechecks on every save (the IDE red squiggle's role)
  - `cargo doc --open`: browse project and dependency docs
  - `rustc --explain E0xxx`: detailed explanation of a compiler error code
- The development environment runs inside Rancher Desktop. The dev DB and Valkey run as containers, and builds and tests for the production target Linux are also verified in a container on the same machine (`docker compose --env-file .env -f dev/compose.yaml run --rm linux-gate`). Do not install a separate WSL distribution.
- On Windows, use the MSVC toolchain (`x86_64-pc-windows-msvc`).

---

## 2. Workspace structure

Do not put everything into one crate. Split crates by role and fix the dependency direction one way.

```
Server/
├─ Cargo.toml            # [workspace], [workspace.dependencies], [workspace.lints]
├─ crates/
│  ├─ protocol/          # Packet definitions, encoding/decoding. No I/O, no async
│  ├─ game/              # Game rules, state. No I/O, no DB (pure logic)
│  ├─ db/                # Sole owner of DB access (sqlx). Queries live only here
│  ├─ net/               # Shared network code: framing codec, link send/receive
│  ├─ infra/             # Common operational code: config loading, log init, metrics, admin port. Depends on no other crate
│  ├─ gateway/           # Gateway binary (design 02)
│  ├─ publisher/         # Publisher login server binary (design 07). Separate from game crates. Uses only the Login DB
│  ├─ ops/               # Operations tool web server (design 12). Depends only on infra; reaches data through operator APIs
│  ├─ server/            # Binary. Runs the Game server, battle server, cluster services as roles. tokio, wiring
│  ├─ bot/               # Headless test/load bot binary. Reference implementation of the session contract
│  ├─ client/            # GUI test client (Bevy + bevy_egui, design 10)
│  └─ launcher/          # GUI server launcher (eframe, design 10). Does not depend on game crates
└─ migrations/           # DB migration SQL
```

**Dependency direction rules**

```
server ──> game ──> protocol
   │                   ^
   ├──> db             │
   └──> net ────────────┤
gateway ──> net         │
bot ───────> net ───────┘
client ────> net        (protocol via net)
launcher                (does not depend on game crates)
publisher ──> db        (Login DB module only. Does not depend on game, net)
binary crates (server, gateway, bot, client, launcher) ──> infra   (infra depends on nothing)
```

- `protocol` depends on no workspace crate. It owns the pilot schemas and packet dispatch, and builds the message ID registry from the [message catalog](protocol/message-catalog.md). Its runtime and `game` do no I/O; `game` may depend only on `protocol`.
- `net` depends on `protocol` and owns the client frame and backend envelope codecs. The stage adds both libraries without wiring them into the binaries.
- `gateway` does not know `game`. The Gateway does not interpret game packet payloads (design 02 3.2).
- `game` does not know `db`. When persistence is needed, `server` takes `game`'s result and passes it to `db`.
- Dependency cycles are blocked by the compiler. But "just put it all in server" is not something the compiler can block, so review blocks it.
- External crate versions are written only in the root `[workspace.dependencies]`. Each crate only references them, like `tokio = { workspace = true }`.

As a C++ analogy, `protocol` and `game` are static libs with no header dependencies, and `server` is the exe linking them.

---

## 3. Naming rules

Follow standard Rust conventions (RFC 430) as they are. The compiler and clippy warn about most of it.

| Target | Rule | Example |
|---|---|---|
| Crates, modules, functions, variables | `snake_case` | `fn move_player`, `mod zone` |
| Types, traits, enum variants | `UpperCamelCase` | `struct PlayerId`, `enum Packet { MoveReq }` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `const MAX_PACKET_SIZE: usize` |
| Constructors | `new`, variants `with_*`, `from_*` | `Zone::new(id)`, `Config::from_file(path)` |
| Getters | field name as is, no `get_` prefix | `player.level()` |
| Conversions | `as_*` (no cost, reference), `to_*` (copy), `into_*` (consumes ownership) | `id.as_u64()`, `name.to_owned()`, `buf.into_bytes()` |
| bool returns | `is_*`, `has_*`, `can_*` | `is_alive()` |

- Treat abbreviations as words: `HttpServer`, `DbPool` (not `HTTPServer`, `DBPool`).
- No Hungarian notation (`m_`, `p`, `sz`, `n`). The compiler knows the types.

---

## 4. Type design

### 4.1 Do not return raw types as they are (newtype)

Wrap IDs, quantities, and amounts in dedicated types. Same concept as strong typedef in C++, but in Rust it costs nothing and is easy to write.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(Uuid);   // data IDs are UUID v7 (design 03, ch. 7)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(Uuid);

// fn give_item(player: Uuid, item: Uuid)    // X: compiles even with swapped order
fn give_item(player: PlayerId, item: ItemId) // O: wrong order is a compile error
```

### 4.2 Represent state with enums

Do not combine several bool flags. Enumerate the possible states in an enum. This makes "impossible states" impossible to construct.

```rust
// X
struct Session { logged_in: bool, in_zone: bool, zone_id: Option<ZoneId> }

// O
enum SessionState {
    Connected,
    Authenticated { account: AccountId },
    InZone { account: AccountId, zone: ZoneId },
}
```

### 4.3 match must be exhaustive

- When `match`ing an enum made in this project, do not use `_ =>`. List every variant so that when a variant is added, the compiler points out every place that does not handle it. This is the biggest difference from C++ `switch`, and its biggest advantage.
- `_` is allowed only for external crate enums or open-ended values like numbers.

### 4.4 Money, currency, numbers

- Do not use `f32` or `f64` for amounts, currency, or probability calculations. Use integer minimal units (`i64`) or `rust_decimal`.
- Currency and stat arithmetic uses `checked_add`, `checked_sub`, and overflow is treated as an error. Rust integer overflow panics in debug builds but silently wraps in release builds, so trust neither.

```rust
let new_gold = player.gold.checked_add(reward).ok_or(GameError::GoldOverflow)?;
```

### 4.5 Option instead of null, Result instead of exceptions

- "May be absent" is expressed as `Option<T>`. No magic values (`-1`, `0`, `INVALID_ID`).
- "May fail" is expressed as `Result<T, E>`. Details are in chapter 5.

---

## 5. Error handling

| Location | Approach |
|---|---|
| Library crates (`protocol`, `game`, `db`) | Define a per-crate error enum with `thiserror` |
| Top level of binaries (`server`, `bot`), startup code | Propagate up with `anyhow::Result`, attaching context (`.context("...")`) |

```rust
// crates/game/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum GameError {
    #[error("player {0:?} not found")]
    PlayerNotFound(PlayerId),
    #[error("not enough gold: need {need}, have {have}")]
    NotEnoughGold { need: i64, have: i64 },
    #[error("gold overflow")]
    GoldOverflow,
}
```

Rules:

- Do not use `unwrap()` or `expect()` outside test code (clippy blocks it). Only in places that truly must die, such as config loading at process start, use it with `#[expect(clippy::expect_used, reason = "...")]`.
- Do not commit `panic!`, `todo!`, `unimplemented!`.
- Do not discard errors. When ignoring a result like `let _ = send(...)`, add a comment on why ignoring it is fine.
- Propagate errors with `?`. Do not double-handle by logging an error and re-raising it. The handler logs it once, at the point of handling.
- The server must not die because one client packet is malformed. In that case, drop only the session.

In C++ terms: there are no exceptions, and every failure shows in the return type. `?` is "return immediately on failure" written in one character.

---

## 6. Ownership, concurrency, async

This chapter is the hardest part of learning Rust, and the most important.

### 6.1 Basic principle: one task owns state (actor model)

- Game state (Zone, player list) is **owned exclusively by the single task** responsible for that Zone.
- Other places do not touch the state directly; they send messages over a `tokio::sync::mpsc` channel.
- Then no locks are needed inside game logic. It is the same structure as C++ MMO servers where each Zone has a job queue processed by a single thread.

```rust
enum ZoneCommand {
    Enter { player: PlayerId, reply: oneshot::Sender<Result<(), GameError>> },
    Move { player: PlayerId, to: Position },
    Leave { player: PlayerId },
}

async fn run_zone(mut zone: Zone, mut rx: mpsc::Receiver<ZoneCommand>) {
    while let Some(cmd) = rx.recv().await {
        zone.handle(cmd); // &mut zone: only this task accesses it, so no lock
    }
}
```

### 6.2 When sharing is truly needed

| Situation | Tool |
|---|---|
| Read-only sharing (config, design data tables) | `Arc<T>` (immutable, no lock needed) |
| Rarely written, often read | `Arc<RwLock<T>>` |
| Short critical section, no `.await` inside | `Arc<std::sync::Mutex<T>>` |
| Must `.await` inside the critical section | Rethink the design. Converting to an actor is usually right |

- Do not hold a `std::sync::Mutex` guard across `.await` (clippy `await_holding_lock`).
- Do not use `Rc` and `RefCell` in async server code. They are not `Send`, so they cannot cross tasks.

### 6.3 async rules

- Do not make blocking calls (`std::thread::sleep`, synchronous file I/O, heavy CPU work) inside async functions. If unavoidable, send it to `tokio::task::spawn_blocking`.
- Channels are **bounded** (`mpsc::channel(N)`) by default. An unbounded channel lets one slow client eat memory without bound. Same reason as capping the send queue in C++.
- Do not just drop the `JoinHandle` of a `tokio::spawn`ed task. Decide in advance who handles its completion and errors.
- Server shutdown propagates through a `CancellationToken` (tokio-util) or a `shutdown` channel and is handled gracefully.

### 6.4 How not to fight the borrow checker

- At first, do not store references (`&'a T`) in structs. Let structs own their data. Use structs with lifetime parameters only after getting used to the language.
- Do not plaster `.clone()` everywhere to silence compile errors. Such errors usually mean "redecide who owns this". That said, `Arc::clone` and small `Copy` types can be copied freely.
- For graph structures where objects point at each other (player ↔ party ↔ guild), do not link with pointers. Reference by ID (`PartyId`) and look up in `HashMap<PartyId, Party>`. The C++ `shared_ptr`/`weak_ptr` cycle problem disappears at the root.

---

## 7. Network and protocol

- All packets are defined in the `protocol` crate. One enum per direction (`ClientPacket`, `ServerPacket`).
- Framing uses a length-header scheme, implemented with `Decoder` and `Encoder` from `tokio_util::codec`.
- Keep a maximum packet size constant (`MAX_PACKET_SIZE`), and when a length header exceeds it, drop the session immediately.
- **Validate at the boundary, trust inside.** At the point of decoding bytes into typed structs, finish all validation (length, range, enum values). Values that entered the `game` crate are assumed valid.
- Each session has one read task and one write task. Writes come from a bounded channel.

---

## 8. Database

- DB access code lives only in the `db` crate. No other crate may contain SQL strings. The same applies to Redis-compatible store access (all external store access goes through the `db` crate).
- **The Redis-compatible store is "a bulletin board all servers look at"** (design 03, ch. 9). Rules:
  - Do not put **source-of-truth data** in it: currency, items, trade state, outbox, idempotency keys. The DB is the source of truth.
  - **Single commands only.** No Lua scripts, no `MULTI`/`EXEC`, no `WATCH`.
  - When "read, decide, write" is needed, the **actor that owns that decision** does it, not Redis.
  - Every key must have a TTL or an explicit deletion path.
  - If a gap between two commands (for example `GET` check then `PEXPIRE`) can corrupt data, **fence it in the DB with a fencing number** (rejecting writes carrying an old number).
  - Redis commands are wrapped in the `db` crate as per-key-kind functions (for example `claim_space`); other crates do not send arbitrary commands.
  - **Confirmed 2026-11-20:** using a single `PUBLISH` command as a hint, such as for live operations-change notifications, is allowed (design 12, ch. 2). PUBLISH is also a single command. However, correctness must be guaranteed by generation-number catching-up, not by the hint, and no logic may depend on the notification arriving.
- **The DB is PostgreSQL.** `sqlx`'s compile-time checking (column types, nullability inference) is the most accurate, and you can use the native `uuid` type and CTEs (multiple operations in one statement). Data IDs are stored in `uuid` columns (design 03, ch. 7).
- **No stored procedures (SP) or triggers.** All DB logic is written as `sqlx::query!`/`query_as!` inside Rust functions in the `db` crate. Like Pro*C in C, SQL is written directly in code and checked against the real schema at build time. One Rust function plays the role one SP used to.
  - Accepted trade-off: the app DB account needs table-alter privileges, and bundling multiple statements adds a round trip per statement.
- Commit `.sqlx/` offline data with `cargo sqlx prepare` so builds work in CI and environments without a DB.
- **Consistency is enforced by the DB.** Use unique constraints, `CHECK` constraints, conditional writes (`UPDATE ... WHERE expected state`), and `INSERT ... ON CONFLICT`. For conditional writes, **always check rows affected (`rows_affected`)** and treat a count other than expected as rejection.
- **DB functions are written once, as async.** There are three ways to call them (design 01 5.7): async code calls directly with `.await` (A), synchronous code such as zone threads uses request/response messages (B), startup stages, tools, batches, and tests use a blocking wrapper (C). **Do not use C inside zone threads or async code.** Do not add a separate sync-only DB driver.
- **Transactions are opened in Rust code** (`pool.begin()` → `tx.commit()`). The DB guarantees atomicity and rollback; leaving the function without committing rolls back automatically in `Drop`.
  - Inside a transaction, do not wait for anything other than DB statements (other server responses, Redis, channels). Run the statements and commit immediately.
  - Query functions that need a transaction take `&mut Transaction`, so they cannot be called outside a transaction by mistake.
  - Deadlock and serialization failures rerun the **entire** transaction. Idempotency keys make the retry safe.
  - The player actor changes in-memory state and notifies the client **after** the commit is confirmed.

Example: selling an item (item deletion + gold increase in one transaction)

```rust
pub async fn sell_item(pool: &PgPool, player: PlayerId, item: ItemInstanceId, price: Gold)
    -> Result<SellOutcome, DbError>
{
    let mut tx = pool.begin().await?;                                  // BEGIN

    let deleted = sqlx::query!(
        "DELETE FROM inventory WHERE item_instance_id = $1 AND player_id = $2 AND locked = false",
        item.as_uuid(), player.as_uuid()
    ).execute(&mut *tx).await?.rows_affected();
    if deleted != 1 { return Ok(SellOutcome::ItemNotFound); }          // return → automatic ROLLBACK

    let updated = sqlx::query!(
        "UPDATE wallet SET gold = gold + $2 WHERE player_id = $1 AND gold <= $3 - $2",
        player.as_uuid(), price.as_i64(), Gold::MAX.as_i64()
    ).execute(&mut *tx).await?.rows_affected();
    if updated != 1 { return Ok(SellOutcome::GoldOverflow); }          // deletion rolls back too

    tx.commit().await?;                                                // COMMIT: both tables applied together
    Ok(SellOutcome::Sold)
}
```

- The price comes from the server's design data. Do not use values sent by the client.
- To prevent duplicate processing of retransmitted requests, add one more idempotency-key `INSERT` line into the same transaction (design 03, ch. 3).
- Schema changes go only through migration files in `migrations/`. Do not run DDL manually.
- Operations that change multiple rows, such as currency transfers, trades, and mail, must be handled in a single transaction. Leave the transaction scope (what succeeds or fails together) in the function's doc comment.
- Retriable requests (grants, payments) carry an idempotency key.
- Do not await DB calls directly inside a game logic task (Zone actor). The Zone tick would be tied to DB latency. Send DB work to a separate task and receive the result as a message.

---

## 9. Logging and observability

- No `println!`/`eprintln!`/`dbg!` (clippy blocks them). Use `tracing` macros.
- Do not concatenate strings into logs; use structured fields.

```rust
tracing::info!(player = ?player_id, zone = ?zone_id, "player entered zone");
```

- For per-session work, attach `session_id` with `#[tracing::instrument]` or spans so logs are traceable.
- Level criteria:
  - `error`: an operator must look at it.
  - `warn`: abnormal but handled. A client sending a malformed packet falls here.
  - `info`: connect, disconnect, major state changes.
  - `debug`/`trace`: for development.

---

## 10. unsafe

- `unsafe` code is forbidden (`unsafe_code = "forbid"`).
- If it truly becomes necessary (FFI etc.), isolate it in a separate crate and grant the exception only there. Every `unsafe` block carries a `// SAFETY:` comment explaining why it is safe.

---

## 11. Tests

- Unit tests live in `#[cfg(test)] mod tests` at the bottom of the same file. Integration tests live in the crate's `tests/` directory.
- `protocol` and `game` are pure logic, so write plenty of tests. Round-trip tests (encode then decode gives back the original) and boundary-value tests are the baseline.
- Do not use `sleep` to align timing in tests. For time-dependent logic, use virtual time with `tokio::time::pause()`, or wait on channels and events.
- Test names are sentences saying what is verified: `move_rejected_when_target_outside_zone`.
- `unwrap`/`expect` are allowed inside test code (configured in `clippy.toml`).

---

## 12. Documentation and comments

- `pub` items get `///` doc comments. Write what it does and when it fails.
- At the top of each module file, write the module's responsibility in one or two lines with a `//!` comment.
- Comments say "why", not "what". Do not write comments that read the code aloud.

---

## 13. External crates (dependencies)

This is the default allow list. Adding a crate not here requires leaving a reason and reaching agreement.

| Purpose | Crate |
|---|---|
| async runtime | `tokio` |
| Buffers, codecs | `bytes`, `tokio-util` |
| Serialization | `serde` (+ per-format `serde_json`, `toml`) |
| Packet serialization | `prost` (confirmed in design 02 8.2) |
| Data IDs | `uuid` (`v7` feature) |
| TLS | `rustls` (`tokio-rustls`) |
| Errors | `thiserror` (libraries), `anyhow` (binaries) |
| Logging | `tracing`, `tracing-subscriber` (JSON feature), `tracing-appender` (file rotation, async writes) |
| Metrics | `metrics`, `metrics-exporter-prometheus` |
| HTTP (admin port, Entry server) | `axum` |
| DB | **PostgreSQL** + `sqlx` (`postgres` feature) |
| Redis-compatible store | `redis` (decided 2026-11-19. Sufficient for single-command use) |
| Money amounts (when needed) | `rust_decimal` |
| HTTP client (external calls such as the Steam Web API) | `reqwest` (`rustls` feature). Approved 2026-11-19 |
| HTML templates (admin web tools) | `askama` (game operations tool in design 12, publisher back office). Approved 2026-11-20. htmx is not an npm dependency; the tool itself serves one file with the version and hash recorded |
| Dev runtime observation | `console-subscriber` (tokio-console). **Enable only as a feature flag on dev builds; do not ship in release**. Approved 2026-11-19 |
| Latency distribution measurement (load testing) | `hdrhistogram` (only in measurement tools such as the `bot` crate). Approved 2026-11-19 |

| GUI, graphics (tools only) | `bevy` + `bevy_egui` (`client` crate), `eframe`/`egui` (`launcher` crate). **Do not put in server binaries**. Approved 2026-11-19 |
| Protocol code generation (build only) | `prost-build`. Approved 2026-11-19 |
| Token signing | `ed25519-dalek` (Entry signs, Gateway verifies with the public key). Approved 2026-11-19 |
| Random numbers (session keys etc.) | `rand`. Approved 2026-11-19 |
| Command-line arguments | `clap` (binary crates only). Approved 2026-11-19 |

- On hold: the global allocator `mimalloc` is decided after load tests confirm the benefit.

- Do not add two crates that do the same job (for example `log` and `tracing`, `async-std` and `tokio`).
- Enable only the feature flags needed (`tokio = { features = ["full"] }` is allowed only in `server` and `bot`).

---

## 14. Traps C++ developers often fall into

| C++ habit | In Rust |
|---|---|
| Polymorphism via inheritance | For things that keep growing and each is large (for example InGameSpace modes), `trait`; for fixed small sets (for example events, disconnect reasons), `enum` + `match` |
| Common fields in a base class | Extract the common part into a struct and use composition |
| Function overloading | Does not exist. Give different names or use traits |
| Copy by default, move when needed | Move by default. Copy is explicit via `Clone`/`Copy` |
| RAII via destructors | `Drop` does the same job. Lock guards, sockets, transactions all clean up when leaving scope |
| `nullptr` checks | Match on `Option` or handle with `?`. No null pointers exist |
| Exceptions | `Result` and `?` |
| Global singleton manager | Construct it and pass it as an argument (`Arc<Context>`). If truly needed, `std::sync::OnceLock` |
| Header/implementation split | Does not exist. Control visibility with `pub`. Default is private |
| Code generation via macros | Solve with generics and traits first; `macro_rules!` is a last resort |

---

## 15. Visibility

- Default is private. Open only what outsiders must use with `pub`.
- Share within one crate with `pub(crate)`.
- Keep struct fields private and protect invariants with constructors and methods. However, types without invariants, such as plain data structs in `protocol`, may expose fields.

---

## 16. Review checklist

Check after finishing each change.

- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` all pass
- [ ] New lint exceptions are `#[expect(..., reason = "...")]` with a valid reason
- [ ] No `unwrap`, `expect`, `panic!`, `todo!` outside tests
- [ ] IDs and amounts are newtypes, and currency arithmetic uses `checked_*`
- [ ] No `_ =>` when matching enums made in this workspace
- [ ] Dependency direction (ch. 2) respected; SQL and Redis access only in the `db` crate
- [ ] Only single commands used with Redis (no Lua, `MULTI`, `WATCH`), and no source-of-truth data stored
- [ ] No lock guard held across `.await`, and channels are bounded
- [ ] External input (packets, config) validated at the boundary
- [ ] If a new external crate was added, the list in ch. 13 was updated
- [ ] `pub` items have doc comments

---

## Appendix A. Root Cargo.toml template

Start from this content when creating the workspace. Each member crate's `Cargo.toml` must include `[lints] workspace = true`.

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
edition = "2024"
version = "0.1.0"
publish = false

[workspace.dependencies]
# External crate versions are managed only here (see ch. 13)

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"
unreachable_pub = "warn"

[workspace.lints.clippy]
# Group: turn on stricter suggestions (priority -1 = applied before individual settings)
pedantic = { level = "warn", priority = -1 }
# Among pedantic, turn off only the noisiest for early learning
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
# Forbid panic paths
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
# Forbid debug output (use tracing)
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"
# Lint exceptions only via #[expect(..., reason = "...")]
allow_attributes = "deny"
allow_attributes_without_reason = "deny"
# Integer/cast safety
cast_possible_truncation = "warn"
cast_sign_loss = "warn"
```

Example member crate (`crates/game/Cargo.toml`):

```toml
[package]
name = "game"
edition.workspace = true
version.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
protocol = { path = "../protocol" }
thiserror = { workspace = true }
```
