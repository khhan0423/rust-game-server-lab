# Rust Game Server Lab

**Rust로 MMO급 게임 서버를 만들 수 있는지 직접 만들어 보며 검증하는 실험 프로젝트입니다.**

C++로 게임 서버를 만들어 온 개발자가 Rust를 처음 쓰면서, 아래 질문에 코드와 측정으로 답하는 것이 목표입니다.

- 게임 로직을 락 없이 돌리는 구조(존 스레드 + 플레이어 액터)가 Rust에서 자연스럽게 나오는가
- 재화와 아이템을 서버가 죽어도 잃거나 복사하지 않게 만들 수 있는가
- 한 프로세스에서 동접 1만, 틱 10Hz를 지킬 수 있는가
- 컴파일러와 lint가 사람이 지키던 규칙을 얼마나 대신해 주는가

완성된 게임이나 프레임워크가 아닙니다. 설계 문서, 계획, 코드가 진행되는 대로 모두 공개합니다.

## 현재 상태

파일럿(설계의 뼈대 검증) 단계입니다. 진행표는 [docs/plan/README.md](docs/plan/README.md)에 있습니다.

| 단계 | 내용 | 상태 |
|---|---|---|
| P0 | 워크스페이스, 품질 게이트, 개발용 DB, 설정, 로그, 지표, 관리 포트 | 완료 |
| P1 | 프로토콜과 프레임 코덱 | 다음 |
| P2~P9 | 런처, 접속과 세션, 컨텐츠 액터, 존과 로비, 샤드 간 메시지, GUI 테스트 클라이언트, 재접속, 부하 측정 | 예정 |

지금까지는 Windows(MSVC)에서만 빌드하고 테스트했습니다. Linux 확인은 아직 하지 않았습니다.

## 구조 요약

```
 클라이언트 ──► Gateway ─────────── Valkey (접속 상태, 장소 담당 서버)
                  │
      ┌───────────┼───────────┐
   게임서버1    게임서버2    게임서버3       Entry 서버 ── Account DB
      │           │           │           전투 서버 풀 (던전, PvP. DB 없음)
   게임DB1      게임DB2      게임DB3
    샤드 1       샤드 2       샤드 3
```

| 원칙 | 내용 | 문서 |
|---|---|---|
| 클라이언트는 Gateway에만 붙는다 | 서버 이동은 Gateway가 목적지를 바꿔 준다. 끊겨도 유예 시간 안에 이어 붙는다 | [설계 02](docs/design/02-session-network.md), [세션 계약](docs/protocol/session-contract.md) |
| 전투는 존 스레드, 컨텐츠는 플레이어 액터 | 장소(InGameSpace)는 고정 틱의 동기 스레드 하나가, 인벤토리와 재화는 유저마다 async 액터 하나가 소유한다. 그래서 게임 로직에 락이 없다 | [설계 01](docs/design/01-threading.md) |
| 샤드 = 게임 DB + 그 샤드의 게임서버 | 유저는 처음 입장할 때 샤드를 배정받고 바뀌지 않는다. 게임서버는 자기 샤드 DB에만 연결한다 | [설계 04](docs/design/04-database.md), [설계 07](docs/design/07-login-account.md) |
| 샤드를 넘는 일은 메시지로 | 상대 DB에 직접 쓰지 않는다. 보내기 전에 기록하고(outbox) 멱등 키로 한 번만 반영한다 | [설계 03](docs/design/03-data-ownership.md) |
| 재화는 DB 커밋 후에만 바뀐다 | SQL은 Rust 함수 안에 쓰고 빌드할 때 검사한다. 저장 프로시저는 쓰지 않는다 | [가이드 8장](docs/RUST_GUIDELINES.md) |
| 모든 큐에 상한이 있다 | 가득 차면 기다리지 않고 정해진 정책대로 처리한다 | [설계 01 7장](docs/design/01-threading.md) |
| 준비가 끝나야 손님을 받는다 | 기동 단계가 하나라도 실패하면 서버가 뜨지 않는다 | [설계 01 10장](docs/design/01-threading.md) |

## 저장소 구성

```
crates/
  infra/      설정, 로그, 지표, 관리 포트 (모든 서버 공용)
  db/         DB 접근 전담. SQL은 여기에만 둔다
  server/     Game 서버 바이너리
  gateway/    Gateway 바이너리
migrations/   DB 마이그레이션
dev/          개발용 PostgreSQL, Valkey 구성
scripts/      품질 게이트 스크립트
docs/
  RUST_GUIDELINES.md   코딩과 설계 규칙
  design/              설계 문서 (스레드, 세션, 데이터 소유, DB, 장소 수명, 로그인, 운영, 결제, 개발 도구)
  protocol/            클라이언트와의 세션 계약
  plan/                실행 계획, 파일럿 단계, 테스트 시나리오
```

크레이트는 처음 필요한 단계에서 추가합니다. 앞으로 `protocol`, `net`, `game`, `bot`, `launcher`(GUI 서버 런처), `client`(GUI 테스트 클라이언트)가 생깁니다.

## 개발 환경

- Rust stable (`rust-toolchain.toml`), Docker 호환 환경(PostgreSQL 17, Valkey 8)
- 준비

```sh
cp .env.example .env          # 비밀번호를 바꾼다. .env는 저장소에 올리지 않는다
docker compose --env-file .env -f dev/compose.yaml up -d --wait
```

- 품질 게이트 (변경을 마칠 때마다 모두 통과해야 한다)

```sh
scripts/gate.sh        # Windows PowerShell: scripts/gate.ps1
# 내용: cargo fmt --all -- --check
#       cargo clippy --workspace --all-targets -- -D warnings
#       cargo test --workspace
```

테스트는 실제 PostgreSQL에서 돕니다. 서버 프로세스를 직접 띄우고, 준비에 실패하는 경우(DB 없음, 스키마 불일치, 비밀값 없음)에 뜨지 않는지도 확인합니다.

## 브랜치

| 브랜치 | 용도 |
|---|---|
| `dev` | 작업 브랜치. 작은 단위로 커밋한다 |
| `main` | 검증이 끝난 것만 올린다. 커밋을 분류별로 다시 묶어서 올린다 |

절차는 [docs/git-workflow.md](docs/git-workflow.md)에 있습니다.

## 기여

개인 실험 프로젝트라 저장소에 쓰는 권한은 소유자에게만 있고, 외부 Pull Request는 받지 않습니다. 의견이나 질문은 Issue로 남겨 주세요.
