# Design 11. Design Data

- Status: **approved** (2026-11-20)
- Related: [12-live-ops-and-admin-tools.md](12-live-ops-and-admin-tools.md), `01-threading.md` 5.7 and 10.0, `04-database.md`, `docs/protocol/session-contract.md`

**English** | [한국어](11-design-data.ko.md)

This document defines how planner-authored tables reach the server and the client. Decisions below were approved by the owner on 2026-11-20. Earlier live services showed what happens without this pipeline: hand-written parsing per table, string keys at runtime, weak client-server agreement on data, and validation that sometimes only logged. This design removes those failure modes.

## 1. What is design data

Design data answers "what exists in the game": items, shops, monsters, rewards, dungeons, field capacity, schedules. It is authored by planners, versioned with the build, and read-only at runtime.

Two kinds of data must not be confused:

| Kind | Question it answers | Where it lives | Who changes it |
|---|---|---|---|
| Static tables (design data) | What exists in the game | Bundle files shipped with the build | Planners, through export and review |
| Live-ops data | What is turned on right now | Database (Common DB, Account DB) | Operators, through the operations tool |

The operations tool never edits static tables. Turning a shop item on or off is a live-ops change in the database, not an edit to the item's definition. See design 12 for that path.

## 2. Decisions

All six open questions from the research proposal were decided on 2026-11-20.

- The bundle is one JSON file per table plus a manifest. JSON is easy to diff and review. If load time ever becomes a problem, the bundle format can change behind the same manifest and loader.
- Each table has a small TOML schema. It is the source of truth for column types and rules. Rust types are generated from it, not hand-written.
- Live-ops changes are announced with a single publish command on Valkey as a hint. Servers are guaranteed to catch up by generation number, never by the hint. See design 12.

## 3. Schema

One TOML file per table, in the repository next to the exported data. The schema declares, for each column:

- type (integer, decimal, string, bool, enum, reference)
- unit where relevant (milliseconds, gold, permille)
- required or optional
- valid range or allowed values
- which table a reference column points to
- whether the client may see the column

The schema also declares the table's ID type name, so code generation can produce a typed ID (`ItemId`, `MonsterId`) instead of a raw number. These are newtypes in the sense of the guidelines section 4.1: passing an `ItemId` where a `MonsterId` is expected is a compile error, not a live bug.

## 4. Code generation

A build-time generator reads the schemas and produces:

- one typed ID per table
- one row struct per table, with fields matching the columns
- the enum types declared in the schema

The generator runs in `build.rs` or as a checked-in tool invoked by the build, similar to how protocol types come from `.proto`. The same schemas can later generate client-side types, so both sides agree by construction rather than by review.

Generated code lands in the `game` crate. Loading and parsing live in the server layer, because `game` does no I/O.

## 5. Authoring and export

Planners work in spreadsheets. The export tool converts the sheets into a bundle:

```
tables/
  items.json
  monsters.json
  rewards.json
  ...
  manifest.json
```

String aliases that planners find convenient are resolved to typed IDs at export time. After export, no runtime lookup by string name exists.

The manifest carries the table list, the content version, and a content hash over all table files.

## 6. Validation

The same validation runs at three points: export, CI, and server startup.

Checks:

- duplicate keys within a table
- values outside the declared range
- references to rows that do not exist in the target table
- required columns missing, unknown columns present
- enum values outside the declared set

Errors name the table, row, and column, so a planner can fix the sheet without reading server code.

At export and in CI, a failed check fails the export or the build. At startup, the server validates the bundle it actually received and refuses to start on failure. This is part of the startup barrier in design 01 section 10.0: a bad bundle never reaches players. A check whose failure is only logged is not acceptable; earlier live services showed that bad data does reach players that way.

## 7. Version and content hash

App version and content version are separate. The manifest carries the content hash.

At login, the client sends the hash of the bundle it loaded. If it does not match the server's hash, login is rejected with a clear reason telling the player to update. A mismatch that is only logged is not enough: the client would then simulate with different data than the server, which produces support tickets no one can reproduce.

## 8. Loading and lookup

The server loads the bundle during the startup barrier, validates it, and builds indexes by typed ID. Runtime lookups go through those indexes. There is no per-table hand-written parsing and no string lookup.

## 9. Reload

Static tables change rarely, but the server must be able to pick up a new bundle without a restart. The reload follows the snapshot swap in design 01 section 5.7:

1. The admin task loads the new bundle and validates it, using the async direct style (A). The game keeps running on the old data.
2. If validation fails, the old generation stays and the failure is reported. Nothing is swapped.
3. On success, the new data becomes a read-only `Arc` snapshot and is delivered by message to the zone threads and actors that use it.
4. Each recipient swaps its reference in turn. A request in flight keeps the generation it started with. No state exists where half the old and half the new bundle is visible.

Blocking DB wrappers (style C in design 01 5.7) are never used for reload. The reload happens while the game is running.

## 10. What is not in this document

Live-ops data, the control path, and the operations tool are defined in [12-live-ops-and-admin-tools.md](12-live-ops-and-admin-tools.md).
