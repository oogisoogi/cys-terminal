# cys-terminal

**AI 에이전트 함대를 지휘하는 오케스트레이션 터미널.** macOS · Windows 크로스플랫폼.

터미널 멀티플렉서 + 로컬 데몬 + 관제 대시보드가 한 몸입니다. Claude Code·Codex 같은
CLI 에이전트 여러 개를 역할(마스터·워커·리뷰어)로 나눠 동시에 굴리고, 서로 소켓으로
대화시키고, 비용·컨텍스트·하드웨어를 실시간 관제합니다.

> 이 프로젝트의 코드는 대부분 **사람의 지휘 아래 AI 에이전트들이 작성**했습니다 —
> 커밋 로그의 `Co-Authored-By` 체인이 그 과정의 기록입니다. 이 저장소 자체가
> "AI 함대 오케스트레이션이 실제로 동작한다"는 실증입니다.

*Read this in [English](README.en.md).*

## 왜 만들었나

기존 터미널·멀티플렉서는 "사람이 명령을 치는 곳"입니다. AI 에이전트를 여러 개 띄우면
곧바로 한계가 옵니다 — pane끼리 서로 말을 걸 수 없고, 에이전트가 남긴 고아 서버가 쌓여
시스템이 마비되고, 누가 얼마나 쓰는지 보이지 않습니다. cys-terminal은 그 문제들을
1급 기능으로 해결하기 위해 처음부터 새로 작성한 독자 구현입니다.

## 설계 원칙 (ABSOLUTE)

1. **양방향 소켓통신** — 단방향 send + capture 폴링을 쓰지 않는다.
   같은 소켓에 물린 모든 pane은 surface ID만 알면 서로에게 능동 push하는 **동등 노드**다.
   `cys send --surface surface:31 "..."` + `send-key Return` → 대상 pane의 **PTY stdin에 직접 주입** → 새 user turn 도착.
   서버→클라이언트 방향은 `cys events` 푸시 스트림(시퀀스 번호·재접속 이어받기).
2. **자원 거버넌스 1급 기능** — 고아 서버 누적 → load 폭주 → 401·hang을 원천 차단하는 완화책 내장.
3. **코어/UI 분리** — 데몬(cysd)은 UI와 무관하게 동작. UI가 hang이어도 소켓 제어 채널은 항상 살아있다(OOB 회생).

## 설치

[Releases](https://github.com/idoforgod/cys-terminal-releases/releases/latest)에서 받으세요.
받는 사람은 **데몬을 따로 설치할 필요가 없습니다** — 앱이 자동 기동하고 팩도 자동 설치됩니다.

- **macOS**: `cys_<버전>_aarch64.dmg` (Apple Silicon) — 드래그 설치 후 앱 실행이면 끝.
- **Windows**: `cys_<버전>_x64-setup.exe` — 데몬·CLI·런타임 동봉(자기완결 설치).
  상세: [docs/INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md)
- 24/365 상시 가동(선택): `cys daemon install` (launchd KeepAlive / 작업 스케줄러).
- 외부 터미널에서 `cys` 명령 쓰기: 앱 Control Center → **"셸에 cys 설치"** 1클릭.

설치·제거 상세는 [docs/INSTALL.md](docs/INSTALL.md), 릴리스 빌드 절차는 docs/RELEASE.md.

## 빠른 시작

```bash
cys identify                                  # 내 surface 주소 확인
cys launch-agent --role worker --agent claude # 역할 노드 기동(지침 자동 주입)
cys send --to worker "상태 보고해줘"            # 역할 주소로 push
cys send-key --to worker Return               # 전송 확정
cys status --json                             # 전 노드 1콜 스냅샷
cys events --reconnect                        # 이벤트 푸시 구독 (폴링 대체)
cys run --scoped -- python -m http.server     # 생명주기 관리되는 스코프드 실행
```

## 구조

```
cys.app  Tauri 데스크톱 앱: 터미널 UI(xterm.js) + Control Center — 데몬의 thin client
cysd     헤드리스 코어 데몬: NDJSON 소켓 서버(UDS / win named pipe), PTY(portable-pty:
         macOS openpty·Windows ConPTY), vt100 화면 재구성, 이벤트 버스, watchdog,
         프로세스 원장, 사용량/비용 수집기, 영속 분석(SQLite)
cys      CLI: pane 안의 AI가 쓰는 동등 노드 클라이언트
pack     cysjavis-pack/: 스킬·지침·훅·도구 (빌드 시 임베드·minisign 서명 배포)
```

모든 pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET` 자동 주입 —
pane 안의 AI는 `cys identify`로 자기 주소를 즉시 안다.

## Control Center (실시간 관제 + 영속 분석)

cys-app UI의 전용 풀 패널 — cysd가 단일 RPC로 플릿·사용량·시스템을 제공하고(외부 대시보드 무의존),
영속 분석은 cysd 내장 SQLite(`analytics.db` · open 실패 시 graceful degrade)에 쌓인다.
철학: **로컬 우선**(데이터가 머신 밖으로 나가지 않음) · 추가 인프라 0 · 에이전트 0ms 지연(hook은 fire-and-forget).

| 탭 | RPC | 내용 |
|---|---|---|
| **Live** | `control.dashboard` · `control.hw` | 노드 플릿(role·agent·state·관측 사용량) · **하드웨어(CPU 코어별·GPU·NPU·MEM 2초 실시간)** · 오늘 토큰/비용$/모델믹스 · 스파크라인 |
| **비용·효율** | `control.analytics {window}` | 영속 집계 — 토큰 4분해 · 모델별 비용($)·단가미상 표시 · 캐시 절감·재사용율 · 조직단위(tier) 비용 |
| **스킬·에이전트** | `control.skills {window}` | 스킬/에이전트 호출 집계 · 🔥실패율(exit_code≠0) · p50 실행시간 |
| **세션** | `control.sessions` · `session_detail` · `session_star` | 세션 타임라인 · 활동 리본 · **전사 발췌** · ⭐즐겨찾기(노트) |
| **추세·주간** | `control.weekly` | 주간 WoW% 델타 · 효율 리더 · 스킬 자산(신규/휴면) |
| **학습** | `learn.status` | 자기개선(RSI) 라운드 타임라인 · 채택/롤백 · 발견 누적 |
| **스킬 보드** | (카탈로그) | 큐레이션된 스킬을 버튼 클릭 = 일회용 워커로 실행(무계약 차단 게이트·HITL 미리보기) |
| **작업 · 승인 Feed** | `org.status` · `feed.*` | 부서×노드 현재 업무(이벤트 드리븐) · 승인 요청 집중 처리 |
| 경보 배지 | `control.alerts` | 토큰/비용 임계 · 이상감지 · 반복실패 — UI 배지 + 이벤트 |

- **RBAC PII 가림**: `CYS_CONTROL_REDACT=1` → session_id 경로 PII를 가리고 집계는 보존.
- 상세 설계: docs/CONTROL_CENTER_DESIGN.md

## 자비스 네이티브 기능 (19건)

> 설계 철학: **지침이 오케스트레이터에게 수동으로 시키는 모든 운영 의무 = 터미널의 기능 결함 목록.**
> ①규약→데몬 보증으로 기계화 ②자기보고 우선·화면 파싱은 fallback ③자동화 3단 안전등급(alert→escalate→act, deny-by-default).

| # | 기능 | 명령/이벤트 |
|---|---|---|
| T1-1 | **자기보고**: 에이전트가 상태·컨텍스트%·작업을 직접 신고 | `cys set-status --state working --context 57 --task "..."` → `status.changed` |
| T1-2 | **관제 보드**: 전 노드 1콜 요약 | `cys status [--json]` (`org.status` RPC) |
| T1-3 | **발신자 신원·ACL**: 커널 peer pid로 from 검증 + role→role 송신 정책 | `acl.json` · 거부 시 `acl.denied` |
| T2-4 | **컨텍스트 사이클 집행기**: 저장 지시→파일 게이트→clear→지침 재주입→재개 | `cys cycle-agent --role worker [--verifier master]` |
| T2-5 | **에이전트 사망 즉시 감지** (+옵션 자동 재기동, 401 시 차단) | `agent.exited/recovered` · `cys node-recover --role X` |
| T2-6 | **조직 복원**: 토폴로지 영속 + 일괄 재기동·재주입·resume | `cys restore [--include-master]` |
| T2-7 | **디렉티브 드리프트 감지·재주입** | `cys reinject --role X [--check]` |
| T2-8 | **오케스트레이터 dead-man**: 단일 장애점 봉합 | `master.deadman` 이벤트 |
| T3-9 | **todo 워치**: `_round/*_TODO.md` mtime 감시→진행률 집계 | `todo.updated` |
| T3-10 | **원샷 타이머** (+fresh TTL `--close-after`) | `cys schedule add --id x --in 20m --text ... --to role` |
| T3-11 | **역할 글롭 브로드캐스트** | `cys send --to 'reviewer-*' "..."` |
| T3-12 | **feed aging 재알림**: pending 승인 무음 적체 차단 | `feed.item.aging` |
| T3-13 | **입력 안전**: 타이핑 가드 · `send --clear-first` 원자 권위 전달 | `typing_guard` 에러 |
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

## 승인 Feed (승인 요청 집중 처리)

```bash
cys feed push --wait --title "git push 승인" --body "..."   # 결정까지 블록 (exit 0=allow, 2=deny, 3=timeout)
cys feed list --status pending
cys feed reply <request_id> allow                            # 오케스트레이터 또는 UI Allow/Deny 버튼
```

에이전트 hook 연동 예(PreToolUse): hook 스크립트에서 `cys feed push --wait ...` → exit code로 결정 반영.

## 인플라이트 큐 (steer / followup)

- 기본 전송(`cys send`)=**steer**: 즉시 stdin 주입 — 실행 중 입력을 조향으로 소화.
- `cys send --queued`=**followup**: 대상이 3초 이상 조용해지면 한 틱에 한 건씩 자동 배달(캡 100건).

`cys schedule add ... --fresh --agent claude` — 매 발화마다 새 surface를 기동해 과업 주입(권한·컨텍스트 상속 차단).

## 프로토콜 (NDJSON, 한 줄 = JSON 하나)

요청 `{"id":1,"method":"surface.send_text","params":{"surface_id":"surface:2","text":"..."}}`
응답 `{"id":1,"ok":true,"result":{...}}` / `{"id":1,"ok":false,"error":{"code","message"}}`

메서드: `system.ping/identify/claim_role/resolve_role/pause/resume/gate_check/topology`
`surface.create/list/send_text/send_key/read_text/resize/rename/close/attach/set_meta/wait_for`
`events.stream` `ledger.*` `health.*` `feed.*` `recall.search` `schedule.*` `status.set` `org.status`
`queue.*` `attest.*` `control.dashboard/hw/analytics/skills/cost_baseline/weekly/alerts/sessions/session_detail/session_star` `learn.status`

이벤트: `surface.*` `health.*` `watchdog.*` `pane.idle` `ledger.*` `daemon.*` `queue.*` `feed.item.*`
`schedule.*` `status.changed` `agent.*` `approval.request` `todo.updated` `master.deadman`
`autopilot.paused/resumed` `acl.denied` `role.claimed`

## 환경 변수

`CYS_SOCKET` 소켓 경로 (기본 `~/.local/state/cys/cys.sock`, win `\\.\pipe\cys`) ·
`CYS_SHELL` · `CYS_LOAD_THRESHOLD`(기본 코어수×2) · `CYS_PROC_THRESHOLD`(50) ·
`CYS_DUP_THRESHOLD`(3) · `CYS_AUTOKILL_DUP`(0/1) · `CYS_IDLE_SECONDS`(300) ·
`CYS_TYPING_GUARD_SECS`(3, 0=off) · `CYS_FEED_REMIND_SECS`(300, 0=off) ·
`CYS_MASTER_DEADMAN_SECS`(900, 0=off) · `CYS_AGENT_AUTORESTART`(0/1) ·
`CYS_RECALL_RETAIN_DAYS`(30, 0=무제한) · `CYS_TODO_DIRS`(콜론 구분 추가 감시 루트) ·
`CYS_CONTROL_REDACT`(0/1, Control Center 세션 PII 가림) ·
`CYS_URL_ALLOW_HOSTS`(외부 URL 허용 도메인 확장 — 또는 `~/.cys/url-allow-hosts` 파일) ·
`CYS_WORKER_PROFILE_DIR`(워커 프로필 경로 — 또는 `~/.cys/worker-profile-dir` 파일)

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
- 업데이트 이중 서명 — 앱은 Tauri updater 서명, 팩은 minisign(공개키 바이너리 핀).
- 외부 URL 열기는 하드 허용목록(로컬 설정으로만 확장) · 승인 자동응답 없음(HITL).
- 발행 전 비밀/PII 게이트: `scripts/secret-scan.sh --all` (fail-closed).

취약점 신고: [SECURITY.md](SECURITY.md)

## 알려진 한계

- macOS에서 sysinfo가 cmdline 전체를 못 읽으면 프로세스명으로 중복 그룹핑(과탐 가능).
- `cys run` 중 Ctrl-C로 CLI가 죽으면 그룹 정리가 watchdog 주기(5초)로 넘어감.
- Control Center의 GPU/NPU 실시간은 현재 macOS(Apple Silicon) 전용 — Windows는 CPU/MEM만.
- NPU는 활용률(%) 공개 API가 없어 실측 전력(W)으로 표시(macOS).

## 기여 · 라이선스

기여는 [CONTRIBUTING.md](CONTRIBUTING.md), 서드파티 귀속은 [NOTICE.md](NOTICE.md) 참조.
MIT License ([LICENSE](LICENSE)) · 문의: **cysinsight@gmail.com**
