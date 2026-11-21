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

지금 있는 것은 크레이트 열 개로 된 Cargo 워크스페이스입니다. 게임 서버는 아직 뼈대지만, 데이터베이스가 없거나 스키마가 오래됐거나 비밀값이 없으면 아예 뜨지 않습니다. Gateway는 클라이언트 접속을 받고, 프레임을 읽고 쓰고, 제시간에 인증하지 않는 접속을 끊습니다. 아직 누구도 들여보내지는 못합니다. 로그인 토큰을 발급하는 Entry 서버가 없기 때문입니다. 창 없는 봇이 여기에 접속 수천 개를 만들어 스모크와 스트레스 시험을 하고, Gateway는 접속 수와 런타임 스레드 상태를 지표로 내보냅니다. 퍼블리셔 로그인 서버는 동작합니다. 게스트 계정을 만들고, 로그인시키고, 게임 쪽이 확인할 수 있는 짧은 수명의 일회용 입장 티켓을 내줍니다. GUI 테스트 클라이언트는 창을 띄우고, 그 서버에 게스트 계정을 만들어 로그인할 수 있습니다. 운영 도구는 웹 페이지입니다. 어떤 서버가 떠 있는지 보여 주고, 계정을 조회하고, 계정을 정지하거나 풀고, 운영자가 한 모든 조작을 기록합니다. 여기에 개발용 데이터베이스 환경과 품질 게이트가 있습니다. Windows(MSVC)와 Linux에서 빌드되고 테스트를 통과합니다. Linux 쪽은 컨테이너에서 돌립니다.

메시지 정의와 프레임 코덱은 만들어 테스트까지 했지만 아직 서버에 연결하지 않았습니다. 다음은 Entry 서버와 Gateway 세션입니다. 로그인 서버가 준 티켓으로 클라이언트가 실제로 게임 서버에 들어가게 하는 부분입니다. 전투 서버, 존, 컨텐츠는 계획 문서 순서대로 만듭니다.

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
  infra/      설정, 로그, 지표, 프로세스 기동 및 종료, HTTP 리스너
  db/         DB 접근 전담. SQL은 여기에만 둔다
  server/     Game 서버 바이너리
  protocol/   메시지 정의(.proto)와 메시지 번호 표
  net/        TCP 리스너, 연결별 쓰기 큐, 프레임 및 봉투 코덱
  testkit/    통합 테스트용 서버 프로세스와 임시 디렉터리
  gateway/    Gateway 바이너리
  publisher/  퍼블리셔 로그인 서버 바이너리 (게스트 로그인, 입장 티켓)
  ops/        운영 도구, 웹 서버 (서버 현황, 접속, 계정, 조작 기록)
  bot/        접속을 여러 개 만드는 창 없는 봇 (스모크, 스트레스 시험)
  client/     GUI 테스트 클라이언트 (Bevy와 egui)
  launcher/   GUI와 헤드리스 모드가 있는 로컬 서버 관리 도구
migrations/   DB 마이그레이션
dev/          개발용 PostgreSQL, Valkey 구성
scripts/      품질 게이트 스크립트
docs/
  RUST_GUIDELINES.md   코딩과 설계 규칙
  design/              설계 문서 (스레드, 세션, 데이터 소유, DB, 장소 수명, 로그인, 운영, 결제, 개발 도구)
  protocol/            클라이언트와의 세션 계약
  plan/                실행 계획, 파일럿 단계, 테스트 시나리오
```

`game`은 아직 계획 단계입니다.

## 파일 위치

- `crates/`에는 소스 코드, `docs/`에는 설계와 가이드, `migrations/`에는 DB 마이그레이션 파일이 있다.
- `dev/`에는 로컬 개발 설정, `scripts/`에는 프로그램 실행과 검증에 쓰는 스크립트가 있다.
- `logs/`에는 프로세스 인스턴스별 로그, `client-data/`에는 테스트 클라이언트 인증 정보, `target/`에는 빌드 결과와 테스트 임시 파일이 들어간다.

## 돌려 보기

Rust(버전은 `rust-toolchain.toml`에 고정돼 있고 rustup이 알아서 설치합니다)와, PostgreSQL 17과 Valkey 8을 띄울 Docker 호환 런타임이 필요합니다. 저는 Rancher Desktop을 씁니다. 빌드하려면 Protocol Buffers 컴파일러(`protoc`)도 PATH에 있어야 합니다.

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

로그인 흐름을 해 보려면 터미널 두 개에서 로그인 서버와 테스트 클라이언트를 띄웁니다. 서버마다 `dev/config/` 아래에 자기 설정 파일이, `scripts/` 아래에 자기 실행 스크립트(`.sh`와 `.ps1`)가 있고, 다른 서버 없이 혼자 뜹니다. 스크립트가 `.env`를 알아서 읽습니다. 처음 띄우기 전에 `migrate` 명령을 한 번 실행합니다.

```sh
scripts/run-publisher.sh migrate     # 처음 한 번
scripts/run-publisher.sh run
scripts/run-ops.sh                   # 운영 도구: http://127.0.0.1:18300
cargo run -p client -- --profile alice
```

클라이언트 창에서 "Create guest account"를 누르고 "Log in"을 누릅니다. 프로필마다 게스트 계정을 `client-data/` 아래에 따로 보관하므로 클라이언트 여러 개를 동시에 띄울 수 있습니다.

개발 DB를 띄우고 마이그레이션을 마쳤다면 런처로 서버 전체를 띄울 수 있습니다. 창에서 **Start all**을 누르거나, 헤드리스 모드로 Ctrl+C까지 실행하거나 기동과 정상 종료를 확인할 수 있습니다. PowerShell에서는 같은 인수로 `scripts/run-launcher.ps1`을 실행합니다.

```sh
bash scripts/run-launcher.sh
bash scripts/run-launcher.sh --headless up
bash scripts/run-launcher.sh --headless check
```

접속 경로가 살아 있는지 빠르게 보는 시험과 무거운 시험입니다.

```sh
scripts/smoke.sh       # Gateway를 띄우고, 봇 70개를 접속시키고, 수치를 확인하고, 모두 내린다
scripts/stress.sh      # 기본 접속 2,000개. 봇의 집계와 Gateway의 지표를 출력한다
```

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
