# Branch and Commit Rules

**English** | [한국어](git-workflow.ko.md)

## Branches

| Branch | Purpose | Who pushes |
|---|---|---|
| `dev` | Working branch. Commit often, in small units | Owner |
| `main` | Verified work only. Commits are grouped by category | Owner |

Only the owner has write access to the repository. No collaborators are added.

## Working on dev

1. Work and commit on `dev`. Keep commits small enough to revert easily.
2. Pass the quality gate before pushing (`scripts/gate.sh` or `scripts/gate.ps1`).

## Promoting to main

When a stage of work on `dev` is finished (for example P1 of the plan) and the gate passes, promote it to `main`. The `dev` commits are not copied as they are; they are **regrouped by category**.

```sh
git switch main
# for each category: take the paths from dev and commit
git restore --source=dev -- docs/design
git commit -m "docs: ..."
git restore --source=dev -- crates/net
git commit -m "net: ..."
# ...

git diff --stat main dev --       # must print nothing (main and dev have identical content)
scripts/gate.sh                 # the gate must pass on main as well
git push origin main

git switch dev
git merge main                  # align dev with main (no content change)
git push origin dev
```

- `main` and `dev` have **the same content and differ only in how commits are grouped.** Verify with an empty `git diff main dev --`.
- If the `dev` commits are already grouped by category, `main` may simply fast-forward to `dev`.
- The history of `main` is never rewritten (no force push).

## Commit messages

Use `category: summary`. Write the summary in English, one line saying what changed and why.

| Category | Covers |
|---|---|
| `docs` | design, plan, guidelines, README |
| `build` | Cargo workspace, dependencies, lint and tool settings |
| `infra` | `crates/infra` |
| `db` | `crates/db`, `migrations/` |
| `protocol`, `net`, `game` | the crate of the same name |
| `server`, `gateway` | the binary crate of the same name |
| `tools` | launcher, test client, bot, dev environment, scripts |
| `chore` | repository settings and anything not covered above |

An implementation and its tests go in the same commit.

## Language

English is the primary language of the repository and Korean is secondary. A document that has both versions keeps the English text in `name.md` and the Korean text in `name.ko.md`, each linking to the other.

## Never committed

- Secrets: `.env`, connection strings, keys. Only `*.example` files are kept.
- Build output (`target/`) and logs.
- Names, code, or internal details of any project outside this repository, whether in documents, code, or commit messages.
