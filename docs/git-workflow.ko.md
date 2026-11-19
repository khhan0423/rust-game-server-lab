# 브랜치와 커밋 규칙

[English](git-workflow.md) | **한국어**

## 브랜치

| 브랜치 | 용도 | 누가 올리나 |
|---|---|---|
| `dev` | 작업 브랜치. 작은 단위로 자주 커밋한다 | 소유자 |
| `main` | 검증이 끝난 내용만 있다. 커밋이 분류별로 정리돼 있다 | 소유자 |

저장소에 쓰는 권한은 소유자에게만 있다. 협업자를 추가하지 않는다.

## dev에서 작업하기

1. `dev`에서 작업하고 커밋한다. 커밋은 되돌리기 쉬운 작은 단위로 한다.
2. 푸시하기 전에 품질 게이트를 통과시킨다(`scripts/gate.sh` 또는 `scripts/gate.ps1`).

## main으로 올리기

`dev`의 작업이 한 단계(예: 계획의 P1) 끝나고 게이트를 통과하면 `main`으로 올린다. 이때 `dev`의 커밋을 그대로 옮기지 않고 **분류별로 다시 묶는다.**

```sh
git switch main
# 분류마다: dev의 해당 경로를 가져와 커밋한다
git restore --source=dev -- docs/design
git commit -m "docs: ..."
git restore --source=dev -- crates/net
git commit -m "net: ..."
# ...

git diff main dev --stat        # 아무것도 나오지 않아야 한다 (main과 dev의 내용이 같다)
scripts/gate.sh                 # main에서도 게이트 통과 확인
git push origin main

git switch dev
git merge main                  # dev의 기준을 main에 맞춘다 (내용 변화 없음)
git push origin dev
```

- `main`과 `dev`의 **내용은 같고 커밋 묶음만 다르다.** `git diff main dev`가 비어 있는 것으로 확인한다.
- `dev`의 커밋이 이미 분류별로 묶여 있으면 `main`을 `dev`로 그대로 앞으로 옮겨도 된다(fast-forward).
- `main`의 이력은 다시 쓰지 않는다(강제 푸시 금지).

## 커밋 메시지

`분류: 요약` 형식으로 쓴다. 요약은 **영어로**, 무엇을 왜 했는지 한 줄로 적는다.

| 분류 | 대상 |
|---|---|
| `docs` | 설계, 계획, 가이드, README |
| `build` | Cargo 워크스페이스, 의존성, lint와 도구 설정 |
| `infra` | `crates/infra` |
| `db` | `crates/db`, `migrations/` |
| `protocol`, `net`, `game` | 같은 이름의 크레이트 |
| `server`, `gateway` | 같은 이름의 바이너리 크레이트 |
| `tools` | 런처, 테스트 클라이언트, 봇, 개발 환경, 스크립트 |
| `chore` | 저장소 설정처럼 위에 들지 않는 것 |

구현과 그 테스트는 같은 커밋에 둔다.

## 언어

저장소의 기본 언어는 영어이고 한국어는 보조다. 두 언어가 모두 있는 문서는 영어를 `이름.md`, 한국어를 `이름.ko.md`에 두고 서로 링크한다.

## 올리지 않는 것

- 비밀값: `.env`, 접속 문자열, 키. 예시는 `*.example`만 둔다.
- 빌드 산출물(`target/`)과 로그.
- 이 프로젝트 밖의 다른 프로젝트 이름, 코드, 내부 정보. 문서, 코드, 커밋 메시지 어디에도 쓰지 않는다.
