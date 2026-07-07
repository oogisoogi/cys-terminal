# cys-terminal

**AI 에이전트 함대를 지휘하는 오케스트레이션 터미널.** macOS · Windows 크로스플랫폼.

터미널 멀티플렉서 + 로컬 데몬 + 관제 대시보드 + 멀티에이전트 운영체계(CYSJavis 팩)가
한 몸입니다. Claude Code·Codex 같은 CLI 에이전트 여러 개를 역할(마스터·워커·CSO·리뷰어)로
나눠 동시에 굴리고, 서로 소켓으로 대화시키고, 비용·컨텍스트·하드웨어를 실시간 관제합니다.

> 이 프로젝트의 코드는 대부분 **사람의 지휘 아래 AI 에이전트들이 작성**했습니다 —
> 커밋 로그의 `Co-Authored-By` 체인이 그 과정의 기록입니다. 이 저장소 자체가
> "AI 함대 오케스트레이션이 실제로 동작한다"는 실증입니다.

*Read this in [English](README.en.md).*

## 문서

| 문서 | 내용 |
|---|---|
| **[Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md)** | 설계 철학 10명제·시스템 아키텍처·보안 모델·불변식 |
| **[User Manual](USER-MANUAL.md)** | 설치부터 함대 운용, CLI·환경변수·프로토콜 전체 레퍼런스까지 |
| [INSTALL.md](docs/INSTALL.md) · [INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md) | 설치 상세 |
| [SECURITY.md](SECURITY.md) · [CONTRIBUTING.md](CONTRIBUTING.md) · [NOTICE.md](NOTICE.md) | 보안 신고 · 기여 · 서드파티 귀속 |

## 왜 만들었나

기존 터미널·멀티플렉서는 "사람이 명령을 치는 곳"입니다. AI 에이전트를 여러 개 띄우면
곧바로 한계가 옵니다 — pane끼리 서로 말을 걸 수 없고, 에이전트가 남긴 고아 서버가 쌓여
시스템이 마비되고, 누가 얼마나 쓰는지 보이지 않습니다. cys-terminal은 그 문제들을
1급 기능으로 해결하기 위해 처음부터 새로 작성한 독자 구현입니다.

그리고 네 번째 문제 — **에이전트들을 어떻게 조직으로 묶을 것인가** — 를 내장 팩
(CYSJavis: 역할별 절대지침 + 결정론 운영 도구)으로 해결합니다.

## 설계 원칙 (ABSOLUTE)

1. **양방향 소켓통신** — 단방향 send + capture 폴링을 쓰지 않는다.
   같은 소켓에 물린 모든 pane은 surface ID만 알면 서로에게 능동 push하는 **동등 노드**다.
   `cys send --surface surface:31 "..."` + `send-key Return` → 대상 pane의 **PTY stdin에 직접 주입** → 새 user turn 도착.
   서버→클라이언트 방향은 `cys events` 푸시 스트림(시퀀스 번호·재접속 이어받기).
2. **자원 거버넌스 1급 기능** — 고아 서버 누적 → load 폭주 → 401·hang을 원천 차단하는 완화책 내장.
3. **코어/UI 분리** — 데몬(cysd)은 UI와 무관하게 동작. UI가 hang이어도 소켓 제어 채널은 항상 살아있다(OOB 회생).
4. **fail-closed 서명** — 앱은 Tauri updater 서명, 팩은 minisign(공개키 바이너리 핀).
   검증에 실패하면 설치·전개 자체가 거부된다.
5. **지침과 기계의 한 몸** — 역할별 절대지침·운영 도구·스킬(CYSJavis 팩)이 터미널과 함께
   빌드·서명·배포되고, 노드 기동 시 자동 주입된다.

## 설치

[Releases](https://github.com/idoforgod/cys-terminal/releases/latest)에서 받으세요.
받는 사람은 **데몬을 따로 설치할 필요가 없습니다** — 앱이 자동 기동하고 팩도 자동 설치됩니다.

- **macOS**: `cys_<버전>_aarch64.dmg` (Apple Silicon) — 드래그 설치 후 앱 실행이면 끝.
- **Windows**: `cys_<버전>_x64-setup.exe` — 데몬·CLI·런타임 동봉(자기완결 설치).
  상세: [docs/INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md)
- 24/365 상시 가동(선택): `cys daemon install` (launchd KeepAlive / 작업 스케줄러).
- 외부 터미널에서 `cys` 명령 쓰기: 앱 Control Center → **"셸에 cys 설치"** 1클릭.

설치·제거 상세는 [docs/INSTALL.md](docs/INSTALL.md), 사용법 전체는
[User Manual](USER-MANUAL.md).

## 빠른 시작

```bash
cys identify                                  # 내 surface 주소 확인
cys launch-agent --role worker --agent claude # 역할 노드 기동(절대지침 자동 주입)
cys send --to worker "상태 보고해줘"            # 역할 주소로 push
cys send-key --to worker Return               # 전송 확정
cys status --json                             # 전 노드 1콜 스냅샷
cys events --reconnect                        # 이벤트 푸시 구독 (폴링 대체)
cys run --scoped -- python -m http.server     # 생명주기 관리되는 스코프드 실행
cys boot                                      # 표준 노드 세트 일괄 기동(설치된 CLI 자동 감지)
```

## 구조

```
cys.app  Tauri 데스크톱 앱: 터미널 UI(xterm.js) + Control Center — 데몬의 thin client
cysd     헤드리스 코어 데몬: NDJSON 소켓 서버(UDS / win named pipe), PTY(portable-pty:
         macOS openpty·Windows ConPTY), vt100 화면 재구성, 이벤트 버스, watchdog,
         프로세스 원장, 사용량/비용 수집기, 영속 분석(SQLite), 스케줄러
cys      CLI: pane 안의 AI가 쓰는 동등 노드 클라이언트 (60+ 서브커맨드 — `cys actions`)
pack     cysjavis-pack/: 절대지침 6·결정론 도구 56·훅 18·스킬 102·스키마 3
         (빌드 시 임베드 · minisign 서명 배포 · 사용자 수정 파일 불가침)
```

모든 pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET` 자동 주입 —
pane 안의 AI는 `cys identify`로 자기 주소를 즉시 안다. PTY는 데몬 소유라서 앱을
재시작·재설치·업데이트해도 세션은 살아 있다(재attach).

## CYSJavis 팩 — 내장 멀티에이전트 운영체계

터미널을 설치하고 AI CLI를 연결하면 **master–worker–CSO–reviewer 멀티에이전트 운영체계**가
바로 구동됩니다. 시스템은 3층입니다:

| 층 | 내용 | 출처 |
|---|---|---|
| 코어 (기계 기능) | 양방향 소켓·승인 Feed·watchdog/원장·이벤트 push·세션 영속 | cys-terminal 코어 |
| CYSJavis 팩 | 역할별 절대지침·결정론 운영 도구·훅·스킬 | `cys init-pack` |
| 개인 층 | soul.md(우선순위·금지선)·장기기억 | **사용자가 사용하며 축적** |

soul.md와 memory/는 **의도적으로 비어 있는 골격**입니다 — "운영 취향과 장기기억은 빌려
쓰는 것이 아니라 사용자 자신이 채워가는 것"이라는 설계 철학입니다. 자율주행(승인된 로드맵
자율 완주)은 오너가 soul.md에 명시적으로 부여할 때만 켜지며, **오너의 어떤 입력이든
즉시 일시정지시키는 kill-switch**가 최우선입니다.

상세: [Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md) §2–4,
운용법: [User Manual](USER-MANUAL.md) §12.

## Control Center (실시간 관제 + 영속 분석)

앱의 전용 풀 패널 — cysd가 단일 RPC로 플릿·사용량·시스템을 제공하고(외부 대시보드 무의존),
영속 분석은 cysd 내장 SQLite(`analytics.db` · open 실패 시 graceful degrade)에 쌓입니다.
철학: **로컬 우선**(데이터가 머신 밖으로 나가지 않음) · 추가 인프라 0 · 에이전트 0ms
지연(hook은 fire-and-forget).

| 탭 (9) | 내용 |
|---|---|
| **Live** | 노드 플릿 · 하드웨어(CPU 코어별·GPU·NPU·MEM 2초 실시간) · 오늘 토큰/비용/모델믹스 · 경보 스트립 |
| **비용·효율** | 영속 집계 — 토큰 4분해 · 모델별 비용(단가미상 표시) · 캐시 절감·재사용율 · 조직단위 비용 |
| **스킬·에이전트** | 스킬/툴/위임 호출 집계 · 실패율(exit_code≠0) · 반복 실패 |
| **세션** | 세션 타임라인 · 활동 리본 · 전사 발췌 드릴다운 · ⭐즐겨찾기 · 🔒PII 가림 |
| **추세·주간** | 주간 WoW% 델타 · 효율 리더 · 스킬 자산(신규/휴면) |
| **학습** | 자기개선(RSI) 라운드 타임라인 · 채택/롤백 · 발견 누적 |
| **스킬 보드** | 큐레이션 스킬 버튼 클릭 = 일회용 워커 실행(HITL 미리보기) |
| **작업** | 모든 부서×노드의 현재 업무(관측 전용) · 자기보고/파생 신뢰 배지 |
| **승인 Feed** | 승인 요청 집중 처리(Allow/Deny) |

그 밖에: ⌘K Command Palette(노드 점프·60% 컨텍스트 순회·승인 처리) · Glance 모드(⌘G,
비기술자용 요약 화면) · 워크스페이스 그룹 · **부서**(독립 데몬으로 프로젝트 격리) ·
RBAC PII 가림(`CYS_CONTROL_REDACT=1`). 상세 설계: docs/CONTROL_CENTER_DESIGN.md

## 자비스 네이티브 기능 (19건)

> 설계 철학: **지침이 오케스트레이터에게 수동으로 시키는 모든 운영 의무 = 터미널의 기능 결함 목록.**
> ①규약→데몬 보증으로 기계화 ②자기보고 우선·화면 파싱은 fallback ③자동화 3단 안전등급(alert→escalate→act, deny-by-default).

| # | 기능 | 명령/이벤트 |
|---|---|---|
| T1-1 | **자기보고**: 에이전트가 상태·컨텍스트%·작업을 직접 신고 | `cys set-status --state working --context 57` → `status.changed` |
| T1-2 | **관제 보드**: 전 노드 1콜 요약 | `cys status [--json]` · `cys fleet`(전 부서) |
| T1-3 | **발신자 신원·ACL**: 커널 peer pid로 from 검증 + role→role 송신 정책 | `acl.json` · 거부 시 `acl.denied` |
| T2-4 | **컨텍스트 사이클 집행기**: 저장 지시→파일 게이트→clear→지침 재주입→재개 | `cys cycle-agent --role worker` |
| T2-5 | **에이전트 사망 즉시 감지** (+옵션 자동 재기동, 인증 오류 시 차단) | `agent.exited/recovered` · `cys node-recover --role X` |
| T2-6 | **조직 복원**: 토폴로지 영속 + 일괄 재기동·재주입 | `cys restore [--include-master]` |
| T2-7 | **디렉티브 드리프트 감지·재주입** | `cys reinject --role X [--check]` |
| T2-8 | **오케스트레이터 dead-man**: 단일 장애점 봉합 | `master.deadman` 이벤트 |
| T3-9 | **todo 워치**: 역할별 TODO 파일 mtime 감시→진행률 집계 | `todo.updated` · `cys todo-path` |
| T3-10 | **원샷 타이머** (+fresh TTL `--close-after`) | `cys schedule add --id x --in 20m --text ... --to role` |
| T3-11 | **역할 글롭 브로드캐스트** | `cys send --to 'reviewer-*' "..."` |
| T3-12 | **feed aging 재알림**: pending 승인 무음 적체 차단 | `feed.item.aging` |
| T3-13 | **입력 안전**: 타이핑 가드 · 원자 권위 전달 | `typing_guard` 거부 |
| T3-14 | **델타 읽기·완료 대기**: 단조 라인 커서 + 데몬측 regex 감시 | `cys read-screen --since N` · `cys watch --until <re>` |
| T4-15 | **kill-switch**: 큐 배달·스케줄 발화 동결 | `cys pause/resume` · `cys gate-check` |
| T4-16 | **승인 격상**: 화면 스캔→이벤트+feed (자동 응답 절대 없음) | `approval.request` |
| T4-17 | **헬스룰 조치 바인딩**(opt-in): queued 배달만 일시정지 | `cys add-health-rule n p --action pause-queue` |
| T4-18 | **트랜스크립트 해시체인 attest**: 변조 증거성(producer≠evaluator) | `cys attest pin/verify` |
| T4-19 | **recall 보존 정책**: 트랜스크립트 무한 성장 차단 | `CYS_RECALL_RETAIN_DAYS` |

## 자원 거버넌스 (3대 완화책)

| 완화책 | 기능 | 명령/이벤트 |
|---|---|---|
| ① 로그인 감지 강화 | 모든 출력 라인에 헬스 룰(기본: Not logged in·401·token expired·rate limit) 매칭 → 30초 디바운스 push | `health.alert` · `cys add-health-rule <name> <regex>` |
| ② 짧은 작업 단위 | idle(기본 300초 무출력) 감지 push → 분할·점검 판단 | `pane.idle` 이벤트 |
| ③ 서버 생명주기 강제 종료 | **scoped 실행**(새 프로세스 그룹+원장, 종료 시 그룹째 정리) · **close-surface**(자식 트리 전멸) · **watchdog**(load/자식 수/중복 명령 감지) | `cys run -- <cmd>` · `cys ps` · `cys kill <pid>` · `watchdog.*` |

## 승인 Feed · 인플라이트 큐

```bash
cys feed push --wait --title "git push 승인" --body "..."   # 결정까지 블록 (exit 0=allow, 2=deny, 3=timeout)
cys feed reply <request_id> allow                            # CLI 또는 UI Allow/Deny 버튼
```

자동 응답은 없습니다(HITL) — 요청 노드의 자기결재도 데몬이 거부합니다. 반복 위험 명령은
`cys approval sign`(master 전용, HMAC signed-prefix)으로 1회 서명해 통과시킵니다.

- 기본 전송(`cys send`)=**steer**: 즉시 stdin 주입 — 실행 중 입력을 조향으로 소화.
- `cys send --queued`=**followup**: 대상이 3초 이상 조용해지면 한 틱에 한 건씩 자동 배달.

## 업데이트 — 이중 채널 + 무중단

| 배지 | 채널 | 방식 |
|---|---|---|
| `!` | 앱(바이너리) | Tauri updater 서명 검증 → 세션 가드 → 설치·재시작 → 팩 반영+노드 자동 복귀 |
| `↻` | 팩(운영체계) | **무중단** — minisign 검증 → 원자 트랜잭션 → 라이브 노드 재주입. 재시작 0, 세션·데몬 생존 |

시작 시 + 6시간마다 조용히 확인. 재설치 후 "디스크는 새 버전·프로세스는 구 데몬" 스큐가
남으면 배지 클릭 교대 또는 유휴 자동 교대(라이브 세션 0일 때 — 무손실)로 해소됩니다.
진단·수리는 `cys doctor [--fix]`.

**커스터마이즈와 공존**: 사용자 수정본은 업데이트가 파괴하지 않습니다 — user-owned 파일은
보존+신버전 `.new` 병치, system 파일은 치유 전 `.user` 보존, `~/.cys/local/` 오버레이
(디렉티브 append·스킬 shadowing·훅 후행)는 업데이트가 존재 자체를 모릅니다.
`cys pack-plan`(사전 미리보기) · `cys pack-merge`(3-way/AI 병합) — 상세는
[User Manual §12.7](USER-MANUAL.md#127-커스터마이징--업데이트와-공존하는-방법).

## 채널 브리지 (Slack·Discord)

함대의 승인 요청·보고를 외부 메신저로 내보내고, 허가된 발신자의 원격 승인을 받습니다 —
발신자 allowlist · 원격 승인 별도 허가 · 즉시 잠금(lockdown) · 모양 기반 redact 내장.
`cys channel status` 참조.

## 프로토콜 · 환경변수

NDJSON(한 줄 = JSON 하나), RPC 60여 개 + `channel.*` 13종, 이벤트 60여 종.
전수 목록과 환경변수 표는 [User Manual §16–17](USER-MANUAL.md)에 있습니다.

## 소스 빌드 (기여 시)

```bash
git clone https://github.com/idoforgod/cys-terminal
cargo build --release
./target/release/cysd &                      # 데몬 (중복 기동 자동 거부)

cd ui && sh build.sh                          # 프런트엔드 번들 (bun)
cargo build -p cys-app                        # dev 실행: ./target/debug/cys-app
bun x @tauri-apps/cli build                   # 배포 번들
```

주의: ui/ 수정 후 앱 재빌드 필요(프런트엔드가 바이너리에 임베드됨). 세션(PTY)은 데몬 소유 —
UI 재시작·앱 재설치에도 세션 유지(재attach).

## 보안 모델

- 네트워크 리스너 없음 — 사용자 소유 Unix 소켓(macOS) / DACL 봉인 named pipe(Windows)만.
- 발신자 신원은 커널 peer pid로 검증(자기신고 불신) · role→role ACL · 능력 게이트는
  deny-by-default(리뷰어는 읽기 전용).
- 업데이트 이중 서명 — 앱은 Tauri updater 서명, 팩은 minisign(공개키 바이너리 핀·replay
  단조성·fail-closed).
- 승인 자동응답 없음(HITL) · 자기결재 차단 · 외부 URL은 하드 허용목록(로컬 설정으로만 확장).
- 발행 전 비밀/PII 게이트: `scripts/secret-scan.sh --all` (fail-closed).

취약점 신고: [SECURITY.md](SECURITY.md) · 상세: [Architecture & Philosophy §6](ARCHITECTURE-AND-PHILOSOPHY.md)

## 알려진 한계

- macOS에서 sysinfo가 cmdline 전체를 못 읽으면 프로세스명으로 중복 그룹핑(과탐 가능).
- `cys run` 중 Ctrl-C로 CLI가 죽으면 그룹 정리가 watchdog 주기(5초)로 넘어감.
- Control Center의 GPU/NPU 실시간은 현재 macOS(Apple Silicon) 전용 — Windows는 CPU/MEM만.
- NPU는 활용률(%) 공개 API가 없어 실측 전력(W)으로 표시(macOS).
- 단일-UID 신뢰 모델 — 승인 서명·자기결재 차단은 같은 계정 내 악성 프로세스에 대한
  암호학적 방어가 아니라 탐지·fail-safe 층입니다.

## 기여 · 라이선스

기여는 [CONTRIBUTING.md](CONTRIBUTING.md), 서드파티 귀속은 [NOTICE.md](NOTICE.md) 참조.
MIT License ([LICENSE](LICENSE)) · 문의: **cysinsight@gmail.com**
