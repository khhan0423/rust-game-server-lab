# Rust 게임 서버 실험실

[English](README.md) | **한국어**

저는 오랫동안 C++로 게임 서버를 만들어 왔습니다. 이 프로젝트는 MMO급 게임 서버를 Rust로 만들 수 있는지 직접 만들고 측정해서 확인하는 실험입니다. 진행하면서 아래 질문에 답을 얻으려고 합니다.

- 게임 로직을 락 없이 돌리는 구조(존 스레드 + 플레이어 액터)가 Rust에서 자연스럽게 나오는가
- 재화와 아이템을 서버가 죽어도 잃거나 복사하지 않게 만들 수 있는가
- 한 프로세스에서 동접 1만, 틱 10Hz를 지킬 수 있는가
- 사람이 손으로 지키던 규칙을 컴파일러와 lint가 얼마나 대신해 주는가

완성된 게임도, 프레임워크도 아닙니다. 설계 문서, 계획, 코드를 진행되는 대로 공개합니다. 문서는 영어로 쓰고, 한국어판을 `이름.ko.md`로 옆에 둡니다.

## 어디까지 왔나

설계의 뼈대가 실제 코드에서 맞물리는지 확인하는 파일럿 단계입니다. 전체 계획은 [docs/plan/README.md](docs/plan/README.ko.md)에 있습니다.

지금 있는 것은 크레이트 네 개(`infra`, `db`, `server`, `gateway`)로 구성된 Cargo 워크스페이스, 데이터베이스가 없거나 스키마가 오래됐거나 비밀값이 없으면 아예 뜨지 않는 기동 절차, 개발용 데이터베이스 환경, 품질 게이트 스크립트, 테스트 14개입니다. Windows(MSVC)와 Linux에서 빌드되고 테스트를 통과합니다. Linux 쪽은 컨테이너에서 돌립니다.

다음은 프로토콜과 프레임 코덱입니다. 그 뒤로 런처, 접속과 세션이 이어집니다. 퍼블리셔 로그인 서버, Entry 서버, GUI 테스트 클라이언트, 봇, 전투 서버, 존, 컨텐츠는 그다음에 계획 문서가 정한 순서대로 만듭니다.

## 구조

파일럿이 끝났을 때의 전체 모습입니다.

```
 클라이언트 ─ 1. 로그인 ──► 퍼블리셔 로그인 서버 ── Login DB     계정, 로그인 수단, 입장 티켓
   │                              ▲ 티켓 확인
   ├── 2. 입장 ─────────► Entry 서버 ── Account DB             캐릭터, 본 샤드, 로그인 토큰
   │
   └── 3. 플레이 ───────► Gateway ─────────── Valkey           접속 상태, 장소 담당 서버
                             │
                 ┌───────────┼───────────┐
              게임서버1    게임서버2    게임서버3      전투 서버 풀 (던전, PvP. DB 없음)
                 │           │           │
              게임DB1      게임DB2      게임DB3
               샤드 1       샤드 2       샤드 3

 개발 도구:  런처 (GUI: 서버군을 띄우고, 내리고, 강제로 죽인다)
             테스트 클라이언트 (GUI: 로그인, 이동, 구매, 우편, 재접속)
             봇 (창 없음: 자동 시나리오와 부하 테스트)
```

이 그림 뒤에 있는 결정 몇 가지:

- 클라이언트는 Gateway에만 붙습니다. 서버 이동은 Gateway가 목적지를 바꿔 주는 것으로 끝나고, 연결이 끊겨도 유예 시간 안에 다시 이어 붙습니다([설계 02](docs/design/02-session-network.ko.md), [세션 계약](docs/protocol/session-contract.ko.md)).
- 전투는 존 스레드에서, 플레이어 컨텐츠는 플레이어 액터에서 돌아갑니다. 장소는 고정 틱의 동기 스레드 하나가 맡고, 인벤토리와 재화는 유저마다 async 액터 하나가 소유합니다. 그래서 게임 로직에 락이 없습니다([설계 01](docs/design/01-threading.ko.md)).
- 샤드는 게임 DB와 그 샤드의 게임서버입니다. 유저는 처음 입장할 때 샤드를 배정받고 이후 바뀌지 않으며, 게임서버는 자기 샤드 DB에만 연결합니다([설계 04](docs/design/04-database.ko.md), [설계 07](docs/design/07-login-account.ko.md)).
- 샤드를 넘는 일은 메시지로만 일어나고, 상대 DB에 직접 쓰지 않습니다. 메시지는 보내기 전에 기록하고(outbox) 멱등 키로 한 번만 반영합니다([설계 03](docs/design/03-data-ownership.ko.md)).
- 재화는 DB 커밋이 끝난 뒤에만 바뀝니다. SQL은 전부 Rust 함수 안에 있고 빌드할 때 검사하며, 저장 프로시저는 쓰지 않습니다([가이드](docs/RUST_GUIDELINES.ko.md)).
- 모든 큐에 상한이 있습니다. 큐가 가득 차면 기다리지 않고 정해진 정책대로 처리합니다.
- 기동이 끝나기 전에는 손님을 받지 않습니다. 기동 단계가 하나라도 실패하면 서버는 뜨지 않습니다.

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

크레이트는 처음 필요한 단계에서 만듭니다. 앞으로 `protocol`, `net`, `game`, `publisher`(퍼블리셔 로그인 서버), `bot`, `launcher`(GUI 서버 런처), `client`(GUI 테스트 클라이언트)가 생깁니다.

## 돌려 보기

Rust stable(`rust-toolchain.toml`)과, PostgreSQL 17과 Valkey 8을 띄울 Docker 호환 런타임이 필요합니다. 저는 Rancher Desktop을 씁니다.

```sh
cp .env.example .env          # 비밀번호를 바꾼다. .env는 저장소에 올리지 않는다
docker compose --env-file .env -f dev/compose.yaml up -d --wait
```

변경을 마칠 때마다 품질 게이트를 통과해야 합니다:

```sh
scripts/gate.sh        # Windows PowerShell: scripts/gate.ps1
# 내용: cargo fmt --all -- --check
#       cargo clippy --workspace --all-targets -- -D warnings
#       cargo test --workspace
```

테스트는 실제 PostgreSQL에서 돕니다. 서버 프로세스를 직접 띄우고, 준비에 실패하는 경우(DB 없음, 스키마 불일치, 비밀값 없음)에는 뜨지 않는지도 확인합니다.

서버는 Linux에서 돌릴 것이라, 같은 게이트를 같은 런타임의 Linux 컨테이너에서도 돌립니다.

```sh
docker compose --env-file .env -f dev/compose.yaml run --rm linux-gate
```

## 브랜치

작업은 `dev`에서 작은 커밋으로 합니다. `main`에는 검증이 끝난 작업만, 커밋을 분류별로 다시 묶어서 올립니다. 절차는 [docs/git-workflow.ko.md](docs/git-workflow.ko.md)에 있습니다.

## 기여

개인 실험 프로젝트입니다. 저장소에 쓰는 권한은 저에게만 있고 외부 Pull Request는 받지 않습니다. 의견이나 질문은 Issue로 남겨 주세요.

## 라이선스

모든 권리는 저에게 있습니다. 개인 프로젝트이며, 읽고 토론할 수 있도록 소스를 공개한 것입니다. 사용, 수정, 재배포를 허락하는 라이선스는 부여하지 않습니다.
