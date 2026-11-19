# 조사: 패킷 페이로드 형식

- 날짜: 2026-11-17
- 목적: `02-session-network.md` N4 결정을 위한 비교
- 결과: **Protobuf + `prost` 채택** (2026-11-17). Godot 웹 export는 하지 않으므로 GDScript 생성은 불필요
- 범위: 클라이언트 ↔ 서버 게임 패킷의 **페이로드**. 헤더 프레이밍(`len u32 | msg u16 | flags u16`)과 서버 간 제어 메시지는 이미 정해져 있으므로 제외

## 1. 요구사항

| # | 요구사항 | 이유 |
|---|---|---|
| R1 | 스키마 하나에서 Rust, C#(Unity, Godot .NET), C++(Unreal), GDScript(Godot) 코드를 **생성** | 엔진별로 직렬화를 손으로 짜면 반드시 어긋남 |
| R2 | 필드 추가/삭제 시 구버전 클라이언트와 호환 | 라이브 서비스 중 클라이언트 강제 업데이트 최소화 |
| R3 | 빈번한 작은 패킷(이동, 전투)에서 할당과 복사가 적을 것 | 동접 1만, 10Hz |
| R4 | Rust 쪽 구현이 성숙하고 safe할 것 | 가이드상 `unsafe` 금지, 패닉 = 프로세스 종료 |
| R5 | 엔진별 빌드 연동이 쉬울 것 | 특히 Unreal(외부 C++ 라이브러리 연동이 까다로움) |

### Godot에서 GDScript 지원이 왜 필요한가

Godot 4의 C# 프로젝트는 **웹으로 export할 수 없다.** 공식 문서에 "Projects written in C# using Godot 4 currently cannot be exported to the web"라고 적혀 있다. 그래서 Godot 웹 클라이언트를 생각한다면 GDScript 코드 생성이 필요하다. 웹 출시 계획이 없다면 Godot도 C#으로 처리할 수 있어서 R1의 부담이 크게 준다.

## 2. 후보별 지원 현황

| | Rust (서버) | Unity (C#) | Unreal (C++) | Godot GDScript | Godot C# |
|---|---|---|---|---|---|
| **Protobuf** | `prost` (tokio 진영, 사실상 표준) + Google 공식 `protobuf` v4 crate (2025-06 출시) | 공식 `Google.Protobuf` (IL2CPP 코드 제거(stripping) 대비 `link.xml` 필요할 수 있음) | 공식 지원 없음. libprotobuf를 서드파티 모듈로 연동해야 함. 커뮤니티 플러그인 다수 | 커뮤니티 `godobuf` (Asset Library 등록) | 공식 `Google.Protobuf` |
| **FlatBuffers** | 공식 crate. 과거 safe 코드에서 메모리 오류가 나는 soundness 문제 권고(RUSTSEC 2019~2021)가 여러 번 있었음 | 공식 | 공식 (헤더 전용이라 연동 쉬움) | 커뮤니티 (순수 GDScript 구현, GDExtension 구현 2종) | 공식 C# |
| **자체 IDL + 생성기** | 직접 생성 | 직접 생성 | 직접 생성 (엔진 타입 `FString`, `TArray`, `USTRUCT` 직접 생성 가능) | 직접 생성 | 직접 생성 |

## 3. 후보별 평가

### 3.1 Protobuf (proto3)

- **장점**
  - 지원 언어와 도구가 가장 넓다. 네 가지 대상 모두 실사용 사례가 있다.
  - R2(호환성)가 형식 자체에 내장되어 있다. 필드 번호만 지키면 필드를 추가해도 구버전이 무시하고 넘어간다.
  - Rust `prost`가 성숙했고 safe하다. 생성 코드가 평범한 Rust struct라 읽기 쉽다.
- **단점**
  - 디코딩 시 메시지 객체를 만든다. C#에서는 GC 부담이 있다. 메시지 재사용(`MergeFrom`)으로 줄일 수 있다.
  - Unreal 연동이 가장 번거롭다. 플랫폼별 libprotobuf 빌드와 엔진 매크로 충돌 처리가 필요해서 보통 플러그인을 쓴다.
  - 생성 코드가 엔진 고유 타입이 아니다. Unreal Blueprint에 노출하려면 변환 계층이 필요하다.

### 3.2 FlatBuffers

- **장점**
  - 역직렬화 없이 버퍼에서 바로 읽는다(제로카피). 크고 읽기 위주인 데이터에 강하다.
  - Unreal 연동이 쉽다.
- **단점**
  - 쓰기 A​PI(builder)가 번거롭다. 작은 패킷을 자주 만드는 게임 서버 코드에서 생산성이 떨어진다.
  - Rust crate의 soundness 문제 이력이 R4에 걸린다. 최신 버전에서는 verifier로 검증한 뒤 읽는 것이 전제다.
  - GDScript 지원은 커뮤니티 구현뿐이다.
  - 작은 패킷에서는 제로카피의 이득이 작다. 이 형식이 빛나는 곳은 **기획 데이터 테이블**처럼 큰 정적 데이터다.

### 3.3 자체 IDL + 자체 코드 생성기

한국 게임사에서 흔한 방식이다. 스키마를 자체 형식(TOML, YAML, 또는 전용 문법)으로 정의하고, Rust 생성기 하나가 네 언어의 코드를 출력한다.

- **장점**
  - 완전한 통제. 와이어 포맷이 직접 정의한 바이너리라서 C++ 엔진 경험을 그대로 쓸 수 있다.
  - 엔진 고유 타입을 직접 생성할 수 있다. Unreal `USTRUCT`/Blueprint, Godot `Dictionary`/`Resource` 같은 것들이다. 클라이언트 개발자의 사용성이 가장 좋다.
  - 외부 라이브러리 의존이 없어서 Unreal 연동 문제가 사라진다.
- **단점**
  - 생성기 4종(Rust, C#, C++, GDScript)과 각 런타임을 **직접 유지보수**해야 한다.
  - 호환성 규칙(R2)을 직접 설계해야 한다. 필드 번호나 태그, 길이 접두를 두어 모르는 필드를 건너뛰게 하는 식이다.
  - 초기 비용이 크다. 서버 설계 학습이라는 이 프로젝트의 목적에서는 곁가지가 커진다.

### 3.4 제외

- **MessagePack, JSON**: 스키마가 없어서 R1(코드 생성)을 만족하지 않는다.
- **Cap'n Proto**: GDScript 구현을 찾지 못했다.

## 4. 추천

**1순위: Protobuf (proto3) + Rust `prost`**

- R1~R4를 모두 만족한다. R5는 Unreal만 번거롭고, 이는 플러그인으로 해결할 수 있다.
- 이 프로젝트의 핵심은 서버 설계와 Rust 학습이다. 직렬화 도구를 만드는 데 시간을 쓰지 않는다.
- 구조상 교체 비용이 작다. 페이로드 해석은 `protocol` 크레이트 안에만 있으므로(02 문서 8.2), 나중에 자체 형식으로 바꿔도 Gateway, 봉투, 게임 로직은 바뀌지 않는다.

**2순위: 자체 IDL + 생성기.** Unreal이 주력 클라이언트가 되고 Blueprint 사용성이 중요해지면 그때 전환을 검토한다. `.proto` 파일을 그대로 스키마 원본으로 쓰고, Unreal용 래퍼만 자체 생성하는 절충안도 있다.

**FlatBuffers는 패킷용이 아니라 기획 데이터 테이블용 후보**로 따로 남겨 둔다.

## 5. Protobuf로 갈 경우의 규칙 (초안)

- 스키마 파일은 `proto/` 디렉터리 하나에 두고, 서버와 모든 클라이언트가 이것을 원본으로 쓴다.
- 메시지 ID(`msg u16`)와 메시지 타입의 대응표도 스키마에서 생성한다. 손으로 관리하지 않는다.
- 필드 번호는 재사용하지 않는다. 삭제한 필드는 `reserved`로 막는다.
- 이동처럼 빈번한 메시지는 필드를 최소화하고, 좌표는 정수로 양자화(예: cm 단위 `sint32`)한다.
- 서버의 `protocol` 크레이트는 디코딩 직후 `prost` 생성 타입을 **검증된 도메인 타입**으로 바꾼다(가이드 7장 "경계에서 검증"). 생성 타입이 `game` 크레이트로 들어가지 않게 한다.

## 출처

- Godot 4 C# 웹 export 미지원: https://docs.godotengine.org/en/latest/tutorials/export/exporting_for_web.html
- Google 공식 Rust protobuf crate: https://crates.io/crates/protobuf/4.33.4-release , https://groups.google.com/g/protobuf/c/Ur70AGjgHnw
- prost와 Google protobuf 벤치마크 논의: https://github.com/tokio-rs/prost/issues/1469
- GDScript Protobuf (godobuf): https://github.com/oniksan/godobuf , https://godotengine.org/asset-library/asset/3734
- GDScript FlatBuffers: https://github.com/splatterfacegames/godot-flatbuffers , https://github.com/enetheru/godot-flatbuffers
- Unreal Protobuf 플러그인 예: https://github.com/DarkestLink-Dev/UE-LinkProtobuf , https://github.com/JGameEngine/UE-Protobuf-Plugin
- FlatBuffers Rust soundness 권고: https://rustsec.org/advisories/RUSTSEC-2021-0122.html , https://rustsec.org/advisories/RUSTSEC-2020-0009.html
- Unity IL2CPP stripping 관련: https://issuetracker.unity3d.com/issues/ios-link-dot-xml-does-not-prevent-il2cpp-stripping-of-generic-classs-constructor-in-a-library
