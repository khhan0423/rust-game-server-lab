# Research: Design Data, Live Operations, and Admin Tools

- Status: **proposal** (2026-11-20). Nothing here is binding until it is moved into `docs/design/`.
- Related: [admin-web-platform.ko.md](admin-web-platform.ko.md) (security research for the web tools, Korean), `docs/design/01-threading.md` §5.7 and §10.0, `docs/design/03-data-ownership.md` §3, `docs/design/08-operations-observability.md`

This note turns what was learned from earlier live game services into concrete proposals for this project. It covers three things: how planner-authored tables reach the server, how operators change a running service, and what the web tools that drive those changes should look like.

## 1. What earlier live services taught us

**Design data**

- Static tables were loaded only at startup. Everything that changes while the service is live (content switches, shop item switches, events, notices) lived in a database instead.
- Validation depth varied. Where cross-references and ranges were checked and a failure stopped startup, bad data never reached players. Where a failed check only logged, it did.
- Client and server agreement on data was weak: a version string comparison, or a content hash that was received and only logged.
- String keys looked up at runtime, hand-written parsing per table, and no single schema made every new table expensive and easy to get wrong.

**Operations**

- Operator changes reached servers by polling a database (1 s to 10 min depending on the item) or by a publish/subscribe signal relayed through a coordinating server.
- A tool's "success" meant "the signal was sent". Nobody confirmed that each game server had actually applied the change.
- Grants (items, currency, mail) had no business key that prevents a second submission from paying twice.
- The audit record and the change itself were written by separate calls, so one could exist without the other.
- Admin tools wrote directly into game databases.

## 2. Proposal A: design data pipeline

| Step | Proposal |
|---|---|
| Schema | One schema file per table is the source of truth. It declares each column's type, unit, required/optional, valid range, the table it references, and whether the client may see it |
| Code generation | Rust row types and typed IDs (`ItemId`, `MonsterId`, ...) are generated from the schema. The same schema generates the client's types |
| Authoring | Planners edit spreadsheets. An export tool converts them into a bundle (one file per table plus a manifest) and runs validation |
| Validation | At export and in CI: duplicate keys, ranges, cross-table references. Errors name the table, row, and column. At startup the server validates the bundle it actually received and refuses to start on failure (startup barrier) |
| Lookup | String aliases are resolved at export. At runtime rows are found by typed ID in an index built at load |
| Version | App version and content version are separate. The manifest carries a content hash; the client sends the hash it loaded at login and a mismatch is rejected with a clear reason |
| Live reload | Load the new bundle, validate, build indexes, then swap one `Arc` snapshot. Requests in flight keep the generation they started with. If anything fails, the old generation stays and the failure is reported |

Static tables answer "what exists in the game". They are never edited by the ops tool.

## 3. Proposal B: live-ops data and the control path

**Where live-ops state lives.** Notices, maintenance windows, content and shop switches, event schedules, and sanctions live in the database (Common DB for game-wide items, Account DB for account sanctions). Each record carries a generation number, a target (all, shard, platform), an effective period, the operator, and a reason.

**How a change reaches servers.**

```
ops tool ──► admin API of the owning service ──► DB commit (generation N+1, audit row in the same transaction)
                                              └─► notify (hint only)
game servers: on notify, on reconnect, and on a periodic check → read up to the latest generation → apply → report "applied N+1"
ops tool: shows requested / applied / failed per server
```

- The notification is only a hint. A server that missed it catches up from the generation number, so nothing depends on the signal arriving.
- The tool distinguishes "request stored", "applied by server X", and "failed on server Y".
- The tool never writes game databases. It calls the admin API of the service that owns the state.

**Grants and compensation.**

- Every grant has a request ID, and every recipient within it has an idempotency key. Execution uses the existing grant path to the player's home shard, so a retry sends only what failed and never pays twice.
- Bulk grants require a target preview, a reason, and a second approver above a threshold.

**Maintenance.** A staged flow: block new entries → announce → drain or disconnect → confirm empty → patch → verify readiness → reopen in stages.

**Monitoring additions.** Age of each server's status sample, lag between a policy's commit and its application per server, partial-failure rate of grants.

## 4. Proposal C: the web tools

- **Two separate tools**, deployed and authenticated separately: a game operations tool and a publisher back-office tool. They share no database and no session.
- **Stack (primary):** server-rendered Rust: `axum` + compile-time templates (Askama), with htmx for partial updates. No Node server and no npm build, which removes the attack paths behind the 2025–2026 incidents summarized in the security research. **Fallback:** a static SPA with a same-origin Rust API, only for a tool whose screens prove too complex for server rendering.
- **Baseline regardless of stack:** not reachable from the public internet; single sign-on with phishing-resistant MFA (passkeys); short sessions; role-based least privilege; every operator action audited with before/after values; two-person approval for large grants; CSRF protection and a strict content security policy; pinned and audited dependencies.

| Tool | First screens |
|---|---|
| Game operations | server status and applied generations, notices, maintenance schedule, content and shop switches, kick, sanctions, player lookup (read-only role for support), grants with approval, audit log |
| Publisher back-office | account lookup, login methods and linking, platform sanctions, payments and refunds, support history, audit log |

## 5. Decisions needed

| # | Question | Recommendation |
|---|---|---|
| R1 | Bundle format for design data | Start with JSON per table plus a manifest (easy to diff and review); switch to a binary format only if load time becomes a problem |
| R2 | Schema language | A small TOML schema per table with generated Rust, rather than a third-party schema system |
| R3 | Notification transport for live-ops changes | A single publish command on Valkey as a hint, with generation catch-up as the guarantee |
| R4 | Web stack for both tools | `axum` + Askama + htmx. Askama and htmx need dependency approval |
| R5 | When to build the first ops tool | A minimal one (status, notice, maintenance, kick, grant) right after pilot stage P6, instead of waiting for stage D8 |
| R6 | Approval threshold for grants | To be set with the game's economy; start with "any bulk grant" |
