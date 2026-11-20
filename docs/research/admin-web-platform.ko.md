# 한 줄 요약

[English](admin-web-platform.md) | **한국어**

**2026-11-19 기준, 두 도구 모두 비공개망의 axum + Askama로 시작하고 htmx는 필요한 화면에만 쓴다. 복잡한 화면이 핵심이 되면 Vue 정적 SPA + Rust API를 대안으로 선택한다.** 안전성을 좌우하는 것은 프레임워크 이름보다 접근 경로, 작업별 권한, 감사, 피해 범위다.

## 최근 보안 사고

날짜는 사고 발생·관측일 또는 공식 권고 공개일이다. 취약점 공개와 실제 침해를 구분했다. `CVE 미확인`은 인용한 자료에서 해당 사건에 붙은 CVE를 확인하지 못했다는 뜻이다.

| 날짜 | 무엇 | 영향 | 교훈 | 출처 |
| --- | --- | --- | --- | --- |
| 2025-03-21 공개 | Next.js 미들웨어 인가 우회, CVE-2025-29927 | 인가를 미들웨어에만 맡긴 앱에서 검사를 우회할 수 있었다. | 민감한 읽기·쓰기의 인가는 처리기와 도메인 API에서 집행한다. | [Next.js 공식 권고](https://github.com/vercel/next.js/security/advisories/GHSA-f82v-jwr5-mffw) |
| 2025-08-12~17 관측 | Salesloft Drift 연동의 OAuth 자격 증명 탈취, CVE 미확인 | 피해 조직에서 탈취한 연동 자격 증명으로 고객지원 문의와 연락처 정보가 유출됐다. 운영자 MFA가 우회됐다는 사례는 아니다. | 앱 연동도 별도 주체로 취급해 최소 범위만 주고 발급·철회·사용을 추적한다. | [피해 조직의 사고 보고](https://blog.cloudflare.com/response-to-salesloft-drift-incident/) |
| 2025-09-14 신고 접수, 09-22·23 공지 | npm Shai-Hulud 웜, CVE 미확인 | 탈취한 관리 계정으로 배포한 패키지의 설치 스크립트가 개발·빌드 환경의 비밀을 훔치고 확산했다. CISA는 500개 넘는 패키지 침해를 보고했다. | 설치 작업을 배포 자격 증명과 격리한다. 잠금 파일에 이미 들어간 악성 버전도 조사한다. | [GitHub/npm](https://github.blog/security/supply-chain-security/our-plan-for-a-more-secure-npm-supply-chain/), [CISA](https://www.cisa.gov/news-events/alerts/2025/09/23/widespread-supply-chain-compromise-impacting-npm-ecosystem) |
| 2025-12-03 공개, 12-05 KEV 등재 | React2Shell, CVE-2025-55182 | React Server Components(RSC)의 Server Function 요청 역직렬화 결함으로 로그인 없이 서버에서 코드 실행이 가능했다. 앱에 자체 Server Function이 없어도 RSC 지원 구성은 영향받을 수 있었다. CISA가 실제 악용을 확인한 목록에 올렸다. | 화면에서 호출하지 않는다고 서버 진입점이 사라지지는 않는다. RSC가 필요 없다면 사용하지 않는다. | [React 공식 권고](https://react.dev/blog/2025/12/03/critical-security-vulnerability-in-react-server-components), [CISA](https://www.cisa.gov/news-events/alerts/2025/12/05/cisa-adds-one-known-exploited-vulnerability-catalog) |
| 2025-12-11 공개, 2026-01-26 추가 | RSC 후속 권고: CVE-2025-55183, CVE-2025-55184, CVE-2025-67779, CVE-2026-23864 | 일부 Server Function 소스가 노출되거나 서비스 거부가 가능했다. React2Shell과는 별도의 결함이다. | 최초 패치로 알림을 닫지 말고 후속 권고와 실제 배포 버전까지 추적한다. | [React 후속 권고](https://react.dev/blog/2025/12/11/denial-of-service-and-source-code-exposure-in-react-server-components) |
| 2026-03-31 발생·공개 | Axios의 악성 npm 배포, GHSA-fw8c-xr5c-95f9, CVE 미확인 | 침해된 관리자 계정으로 배포한 두 버전이 악성 의존성을 끌어와 설치 환경에서 원격 제어 악성코드를 실행했다. | 유명 패키지도 예외 없이 잠금 파일 변경을 검토하고 설치 작업에 운영 비밀을 주지 않는다. | [Axios 사고 보고](https://github.com/axios/axios/issues/10636), [GitHub Advisory](https://github.com/advisories/GHSA-fw8c-xr5c-95f9) |
| 2026-05-06·07 공개 | Next.js App Router 미들웨어·프록시 우회 및 불완전한 수정, CVE-2026-44575·CVE-2026-45109 | 일부 프리페치 경로 변형에서 미들웨어·프록시에 의존한 인가를 건너뛸 수 있었다. 첫 수정에 후속 패치가 필요했다. | 화면, 데이터 응답, 프리페치가 같은 권한 규칙을 따르는지 확인한다. | [Next.js 최초 권고](https://github.com/vercel/next.js/security/advisories/GHSA-267c-6grr-h53f), [후속 권고](https://github.com/vercel/next.js/security/advisories/GHSA-26hh-7cqf-hhc6) |
| 2026-07-21 공개 | Next.js App Router 미들웨어·프록시 우회, CVE-2026-64642 | Turbopack과 단일 로케일을 사용하는 영향 구성에서 인가를 미들웨어·프록시에만 둔 경로가 우회될 수 있었다. | 프레임워크 라우팅과 업무 권한 검사를 분리한다. | [Next.js 공식 권고](https://github.com/vercel/next.js/security/advisories/GHSA-6gpp-xcg3-4w24) |
| 2026-10-08 발생·공개 | npm `tensorlake` 악성 배포, GHSA-8g63-53c9-65j2, CVE 미확인 | 공식 배포 절차를 거쳐 출처 증명이 유효한 악성 버전이 배포됐다. 설치 스크립트가 접근 가능한 자격 증명을 수집했다. | 출처 증명은 확인하되 코드 검토와 빌드 격리를 함께 적용한다. | [유지보수자 권고](https://github.com/tensorlakeai/tensorlake/security/advisories/GHSA-8g63-53c9-65j2) |
| 2026-10-28 공개 | 자체 호스팅 Appsmith 세션 격리 우회, GHSA-724j-456c-hj36, CVE 미확인 | 공개 앱을 만들 수 있는 작성자가 로그인한 사용자가 그 앱을 열면 그 사용자의 권한으로 비공개 리소스에 접근할 수 있었다. 인스턴스 관리자도 영향 범위에 포함된다. 실제 악용 여부는 미확인이다. | 앱 작성·게시 권한과 운영 권한을 분리하고 플랫폼 커넥터에 넓은 권한을 주지 않는다. | [Appsmith 공식 권고](https://github.com/appsmithorg/appsmith/security/advisories/GHSA-724j-456c-hj36) |
| 2026-11-02 우회 취약점 공개, 11-10 두 건 KEV 등재 | Barracuda ESG의 인증 우회 및 명령 주입, CVE-2026-116606·CVE-2026-111560 | 인증 전 설정 복원 경로의 결함과 관리자 권한이 필요한 명령 주입을 결합하면 장비의 최고 권한에 이를 수 있었다. 관리 인터페이스 위험 사례이며 특정 피해 조직의 공개 설정은 미확인이다. | 관리 화면뿐 아니라 관리 API와 접근 장비도 공개 노출을 막고 긴급 패치 대상으로 관리한다. | [공개 권고](https://github.com/mandiant/Vulnerability-Disclosures/blob/master/2026/MNDT-2026-0042.md), [CISA KEV 원본](https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json) |
| 2026-11-09 배포, 11-11 권고 등록 | crates.io `lwd-uri-util` 악성 배포, RUSTSEC-2026-0483, CVE 미확인 | 해당 크레이트를 쓰는 프로젝트의 빌드에서 SSH 키와 클라우드 자격 증명 등을 빼낼 수 있었다. RustSec은 실제 사용 증거를 확인하지 못했다고 밝혔다. | npm을 빼도 빌드 공급망은 남는다. Cargo 의존성과 빌드 스크립트에도 같은 기준을 적용한다. | [RustSec](https://rustsec.org/advisories/RUSTSEC-2026-0483.html) |

## 후보 플랫폼 비교

공급망 크기는 이 팀이 실제로 써야 할 빌드 체인, 직접·간접 의존성과 운영 서비스의 상대 비교다. 정확한 패키지 수는 구성 선택 전이라 미확인이다. 아래 권고는 대표 사례이며 CVE 개수로 플랫폼의 안전 순위를 매기지 않았다. 특히 등록된 권고가 검색되지 않는 것은 안전하다는 증거가 아니다.

| 후보 | 공격 표면·공급망·최근 이력 | Rust 팀의 생산성과 운영 부담 |
| --- | --- | --- |
| **axum + Askama 또는 Maud + 선택적 htmx** | 명시적인 HTTP 처리기와 HTML 중심. 주요 표면은 폼, 세션, 권한, 출력 이스케이프와 HTML 조각 교체다. htmx 정적 파일을 직접 제공하면 npm 빌드가 필요 없다. Cargo와 OS, 프록시, IdP는 남는다. 2025~2026년 해당 패키지의 직접 권고는 조회 범위에서 미확인이고 crates.io 침해 사례는 위 표 참조. | 기존 Rust 운영 지식을 그대로 쓴다. 검색·페이지 나누기·폼·승인 큐에는 충분하다. 폼, 표, CSRF, 세션과 권한 검사는 구현해야 하므로 공통 부품을 만든다. |
| **Leptos / Dioxus Fullstack** | Rust로 화면과 서버를 개발할 수 있지만 SSR·브라우저 WASM 구성에서는 빌드 표면이 넓어지고, 서버 함수는 실제 HTTP 진입점이다. 인증·인가와 역직렬화 경계를 검토한다. 2025~2026년 해당 패키지의 직접 권고는 조회 범위에서 미확인이다. | 반응형 화면에 유용하나 단순 CRUD에는 프레임워크, WASM·초기 화면 연결, 배포를 배울 비용이 크다. 관리 도구만을 위해 먼저 도입할 이유는 약하다. |
| **Next.js / React Server Components** | Node 서버, npm 빌드, RSC·Server Actions와 라우팅·캐시가 검토 대상이다. 2025-12-03 React2Shell과 2025~2026년 Next.js 인가 우회는 위 공식 권고 참조. | 컴포넌트 선택이 풍부해 UI 개발은 빠를 수 있다. 이 팀에는 두 런타임, 긴급 패치·재배포, 서버 함수별 권한 검토가 추가된다. 단순 CRUD에는 장점보다 유지 부담이 크다. |
| **React 또는 Vue 정적 SPA + Rust REST API** | RSC 없는 정적 배포는 해당 RSC 권고 대상이 아니다([React, 2025-12-03](https://react.dev/blog/2025/12/03/critical-security-vulnerability-in-react-server-components)). API와 브라우저 XSS·CSRF는 남고 npm은 빌드에 필요하다. [Vite 파일 접근 제한 우회, CVE-2026-118662, 2026-10-29 GitHub 등록](https://github.com/advisories/GHSA-8g5g-mxqh-grgj)처럼 개발 서버도 별도 표면이다. 운영에는 빌드한 정적 파일만 같은 출처로 제공한다. | 복잡한 표, 다단계 화면, 화면 상태를 다루기 좋다. Node 운영 서비스는 없지만 Rust API와 프런트엔드의 빌드·테스트·업데이트를 함께 관리한다. |
| **Retool / Appsmith 등 로우코드** | UI 개발용 의존성이 플랫폼 안으로 이동한다. 편집·게시 권한, 플랫폼 관리자, 커넥터 비밀과 실행 권한이 핵심이다. 자체 호스팅은 플랫폼 패치 책임도 진다. Appsmith 2026-10-28 권고는 위 표 참조. Retool의 2025~2026년 비교 가능한 사고는 확인 범위에서 미확인이다. | 조회 화면과 기본 CRUD는 빠르다. 환불·지급의 승인과 감사는 플랫폼 위젯 밖에서 강제해야 한다. 사용할 경우에도 제한된 읽기 전용 API 범위부터 검토한다. |
| **ASP.NET Core Razor Pages / Blazor Server** | Razor는 전통적인 서버 폼 위주로 구성할 수 있다. Blazor Server는 연결이 지속되는 UI 세션과 서버 상태의 수명도 관리한다. NuGet/.NET 패치가 필요하다. [ASP.NET Core 서비스 거부 CVE-2026-114154, 2026-11-11 GitHub 등록](https://github.com/advisories/GHSA-3893-7fxc-qvff). | Razor의 폼과 인증 통합은 CRUD에 편하다. 팀이 .NET을 이미 운영하지 않으면 추가 도구·런타임 비용이 있고, 기본 폼에는 Blazor의 연결 상태가 불필요하다. |
| **Django** | Python/PyPI와 내장 관리자, 모델 기반 데이터 접근을 관리한다. [관리 화면의 일부 URLField 처리 XSS, CVE-2026-15920, 2026-08-04 공개](https://www.djangoproject.com/weblog/2026/aug/04/security-releases/). | 내장 관리자로 일반 CRUD 구축이 빠르다. 도메인 API가 소유해야 할 지급·환불을 모델 편집으로 바로 연결하지 않으면 일부 장점이 줄어든다. Python 운영도 추가된다. |
| **Rails** | Ruby/RubyGems, 세션과 캐시, 사용 시 프런트 도구가 추가된다. [특정 Active Storage 프록시·캐시 구성의 세션 정보 노출 가능성, CVE-2026-126077, 2026-11-12 GitHub 등록](https://github.com/advisories/GHSA-x6mj-8jx9-qhqv). | 표준 폼과 CRUD를 빠르게 만들 수 있다. 이 팀은 Ruby 운영과 업데이트 체계를 새로 맡아야 한다. |
| **Phoenix LiveView** | Elixir/OTP와 Hex, 지속적인 연결과 서버 UI 상태, 소켓 이벤트별 권한 검토가 필요하다. [Phoenix 채널의 서비스 거부 CVE-2026-56811, 2026-09-03 GitHub 등록](https://github.com/advisories/GHSA-6983-jfq8-485w)은 Phoenix의 권고이며 LiveView 패키지 자체의 직접 CVE는 미확인이다. | 지속적으로 변하는 화면에는 편리하다. 조회와 명령 폼이 대부분이면 BEAM 운영과 연결 수명 관리에 드는 비용을 정당화하기 어렵다. |

## 권장 구조

1. **공개 인터넷에서 앱과 API로 접속할 수 없게 한다.** 관리 단말은 기기 정책을 적용하는 VPN 또는 제로트러스트 접근 프록시를 통해서만 사설망에 들어온다. 공개 진입점이 필요한 배치에서도 인증 프록시만 외부에 두고, 원본은 사설 주소와 방화벽으로 보호한다. 프록시와 원본, 웹 서버와 관리 API 사이에는 서로 인증하는 TLS(mTLS)와 서비스별 신원을 쓴다. 앱은 프록시에서 온 요청이라는 이유만으로 작업을 허용하지 않는다.
2. **운영자용 SSO와 피싱 방지 MFA를 강제한다.** 고객 로그인과 별도의 운영자용 IdP 정책을 쓰고, OIDC 클라이언트는 앱마다 나눈다. Authorization Code + PKCE로 로그인하고 서버에서 발급자·대상·서명·만료·`state`·`nonce`를 확인한다. WebAuthn 패스키나 보안 키를 요구하고 지급·환불에는 작업 직전 재인증을 요구한다. 토큰은 서버에 보관하고 브라우저에는 앱별 `Secure`, `HttpOnly`, `SameSite` 세션 쿠키만 준다. 시작값으로 유휴 15분, 최대 8시간을 제안한다. 권한 회수 시 앱 세션도 폐기할 수 있어야 한다. 연동용 토큰은 범위와 수명을 줄이고 발급·폐기 경로를 마련한다.
3. **웹 화면과 도메인 API 양쪽에서 작업별 인가를 검사한다.** 기본 거부로 시작하고 조회, 공지·점검, 킥·제재, 지급, 고객지원, 계정 변경, 환불, 감사 조회 권한을 나눈다. 웹 서버는 인증된 운영자와 요청 식별자를 API에 검증 가능한 방식으로 전달한다. API는 서비스 신원과 운영자 권한을 모두 검사한다. 화면에서 버튼을 숨기는 동작에는 권한을 맡기지 않는다.
4. **두 도구와 데이터 경계를 분리한다.** 게임 운영은 공지·점검, 킥·제재, 지급, 플레이어 조회, 상태 조회를 사설 게임 관리 API에 요청한다. 게임 웹 앱에는 게임 DB 자격 증명을 주지 않는다. 소유 서비스가 기존 게임 상태 변경 경로로 명령을 처리한다. 퍼블리셔 앱은 플랫폼 계정·로그인 수단·결제·환불·지원 업무를 각 소유 서비스 API에 요청하고 필요한 정보만 본다. 두 앱은 별도 배포, 사설 호스트명, 쿠키, OIDC 클라이언트, 서비스 계정·비밀, 권한 묶음과 네트워크 정책을 쓴다. 고객 로그인 정보와 결제 자격 증명을 게임 운영 쪽에 제공하지 않는다.
5. **지급과 환불은 승인과 중복 방지를 도메인 API에서 집행한다.** 정한 한도를 넘는 아이템·재화 지급은 신청자 외 다른 승인자가 승인하고, 고위험 환불에도 같은 방식을 쓴다. 승인에는 대상·수량 또는 금액·사유·만료를 묶어 내용을 바꾸면 재승인하도록 한다. 지급은 요청별 멱등 키를 소유 서비스 PostgreSQL에 남긴다. 같은 키와 내용이면 기존 결과를 돌려주고 내용이 다르면 거부한다. 지급, 감사 이벤트, 멱등 결과가 따로 성공하지 않도록 같은 트랜잭션으로 기록한다. 외부 결제 호출은 결제 측이 지원하는 중복 방지와 내부 내구성 있는 처리 기록을 함께 사용한다. 지급·환불의 원장과 멱등 키를 Valkey에 두지 않는다.
6. **운영자 행동을 의무적으로 감사한다.** 조회·다운로드·변경·승인·거부를 포함해 누가, 언제(UTC), 어떤 대상에 무엇을 했는지, 사유·결과·요청 ID를 남긴다. 변경은 소유 서비스에서 실제 변경 전후를 기록한다. 조회·거부처럼 변경이 없는 경우 전후 값은 해당 없음으로 남긴다. 비밀번호·토큰·결제정보 원문은 기록하지 않고 개인정보는 필요한 수준으로 가린다. 감사 기록은 운영자가 수정·삭제할 수 없는 별도 권한으로 보호한다. 위험 변경은 감사 이벤트를 내구성 있게 남길 수 없으면 실행하지 않는다.
7. **브라우저 경계를 좁힌다.** 상태 변경은 `GET`으로 처리하지 않고 서버에서 CSRF 토큰과 요청 출처를 확인한다. 기본 HTML 이스케이프를 유지하고 고객지원 본문·플레이어 입력은 텍스트로 렌더링한다. 같은 출처의 스크립트만 허용하는 CSP를 기본으로 두고 `frame-ancestors 'none'`, `object-src 'none'`, `form-action 'self'`를 적용한다. 민감 응답에는 `Cache-Control: no-store`를 쓰고 htmx 로컬 히스토리 저장도 끈다. htmx는 외부 CDN 없이 검토한 파일을 제공하고 동적 스크립트 평가를 사용하지 않는다.
8. **잠금 파일과 업데이트 절차를 운영한다.** `Cargo.lock`을 커밋하고 잠금 버전으로 빌드한다. `cargo-audit`로 RustSec 권고를 확인하고 `cargo-deny`로 허용 출처·금지 의존성 규칙 등을 검사한다. htmx는 고정 버전과 해시를 기록한다. SPA를 택하면 npm 잠금 파일도 검토하고 설치 스크립트는 필요한 것만 허용한다. 깨끗한 권고 검사 결과만으로 새 패키지의 안전을 가정하지 않는다. 의존성 설치·빌드에는 운영 비밀을 주지 않고, 패키지 변경과 긴급 패치를 담당할 사람을 정한다.

## 권장안과 대안

**주 권장안: 두 도구 모두 axum + Askama.** 기본 HTML 폼과 서버 처리로 시작해 검색 결과 갱신, 페이지 나누기, 상태 표시처럼 필요한 곳만 htmx로 갱신한다. 두 도구가 폼과 표의 공통 코드는 공유할 수 있지만 배포와 권한은 공유하지 않는다. 이 팀이 익숙한 Rust 배포 흐름을 쓰면서 npm 빌드와 추가 웹 런타임을 피할 수 있다. 환불·지급에 시간을 써야 할 곳은 프런트엔드 프레임워크보다 승인, 감사, 도메인 규칙이다.

**대안: Vue 정적 SPA + 같은 출처의 Rust REST API.** 사용성 시험에서 복잡한 다단계 업무나 한 화면의 상호작용이 병목으로 확인되면 그 도구에 적용한다. 예를 들어 고객지원 화면에서 계정과 결제 이력을 대조하며 여러 단계를 오가야 할 때다. 운영 Node 서버 없이 Vue의 컴포넌트를 쓸 수 있고, 승인·세션·감사의 집행 위치는 Rust에 유지한다. 대신 npm 공급망과 프런트엔드 테스트를 담당해야 한다.

## 남은 결정

- 운영자 IdP와 사설 접근 방식, 관리 단말 요건, 패스키 분실 시 복구 절차, 세션 만료·철회 목표 시간을 정한다.
- 업무·환경별 역할표와 지급·환불의 건당 및 누적 한도, 제2 승인자, 감사 로그 보존 기간과 민감정보 가림 기준을 정한다.
- 출시 전 두 앱에서 비공개 원본 직접 접속 차단, 다른 앱 권한으로 접근 거부, 세션 철회, CSRF 거부, 승인 후 신청 내용 변경 거부, 지급 재요청 시 한 번만 반영, 감사 누락 시 변경 거부를 확인한다. 실제 배포 형태의 검증 결과는 미확인이다.
- 사용성 시험에서 기본 검색·조회·승인 흐름이 충분한지 확인한 뒤 SPA가 필요한지 정한다. 결제 사업자의 멱등 키 및 재조회 지원 범위는 미확인이다.

## 출처

- 사고 및 프레임워크 권고의 날짜와 원문 링크는 위 표에 붙였다. Rust 패키지의 직접 권고는 [RustSec](https://rustsec.org/advisories/)와 [GitHub Advisory Database](https://github.com/advisories) 범위에서 확인했다. 그 밖의 미공개 사고와 모든 간접 의존성의 이력은 미확인이다.
- 프레임워크 문서: [Askama](https://docs.rs/askama/latest/askama/), [htmx 보안과 히스토리](https://htmx.org/docs/#security), [Leptos 서버 함수](https://book.leptos.dev/server/25_server_functions.html), [Dioxus Fullstack](https://dioxuslabs.com/learn/0.7/essentials/fullstack/).
- 인증과 브라우저 보안: [OpenID Connect Core](https://openid.net/specs/openid-connect-core-1_0.html), [OAuth 보안 권고 RFC 9700](https://datatracker.ietf.org/doc/rfc9700/), [NIST SP 800-63B-4](https://pages.nist.gov/800-63-4/sp800-63b.html), [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html), [OWASP CSP](https://cheatsheetseries.owasp.org/cheatsheets/Content_Security_Policy_Cheat_Sheet.html).
- 공급망 검사 도구: [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit), [cargo-deny](https://embarkstudios.github.io/cargo-deny/).
