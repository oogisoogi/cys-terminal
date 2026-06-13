# cys — the CYSJavis Terminal (코어 데몬 + CLI + CYSCYSJavis Pack)

> 진정한 AI를 위한 터미널. 외부 터미널 체계(외부 프로젝트) 전수조사에서 프로토콜 설계 사상만 참고해
> **처음부터 새로 작성**한 독자 구현 (GPL 코드 미사용). 맥·윈도우 크로스플랫폼.

## 설계 원칙 (ABSOLUTE)

1. **양방향 소켓통신** — 단방향 send + capture 폴링을 쓰지 않는다.
   같은 소켓에 물린 모든 pane은 surface ID만 알면 서로에게 능동 push하는 **동등 노드**다.
   `cys send --surface surface:31 "..."` + `send-key Return` → 대상 pane의 **PTY stdin에 직접 주입** → 새 user turn 도착.
   서버→클라이언트 방향은 `cys events` 푸시 스트림(시퀀스 번호·재접속 이어받기).
2. **자원 거버넌스 1급 기능** — 외부 터미널 체계 치명 단점(고아 서버 누적 → load 폭주 → 401·hang)의 근본 완화책 내장.
3. **코어/UI 분리** — 데몬(cysd)은 UI와 무관하게 동작. UI가 hang이어도 소켓 제어 채널은 항상 살아있다(OOB 회생).

## 구조

```
cysd  헤드리스 코어 데몬: NDJSON 소켓 서버(UDS / win named pipe), PTY(portable-pty:
         macOS openpty·Windows ConPTY), vt100 화면 재구성, 이벤트 버스, watchdog, 프로세스 원장
cys   CLI: pane 안의 AI가 쓰는 동등 노드 클라이언트
```

모든 pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET` 자동 주입 —
pane 안의 AI는 `cys identify`로 자기 주소를 즉시 안다.

## 설치 (배포본)

받는 사람은 **데몬을 따로 설치할 필요가 없다** — 앱/CLI가 자동 기동하고 pack도 자동 설치된다.
설치파일·상시 가동·제거는 **`docs/INSTALL.md`**, 릴리스 빌드 절차는 **`docs/RELEASE.md`**.

- macOS: `cys-0.2.0-macos-arm64.dmg` 드래그 설치 → 앱 더블클릭이면 끝.
- Windows: `cys-0.2.0-windows-{x64,arm64}.msi` (CLI+데몬 중심).
- 24/365 상시 가동(선택): `cys daemon install` (launchd KeepAlive / 작업 스케줄러).

## 빠른 시작 (소스 빌드)

```bash
cargo build --release
./target/release/cysd &                      # 데몬 (중복 기동 자동 거부; 배포본은 자동 기동)

cys new-surface --title worker1              # → surface:1
cys send --surface surface:1 "echo hello"
cys send-key --surface surface:1 Return
cys read-screen --surface surface:1          # vt100 정확 화면
cys read-screen --surface surface:1 --lines 200   # scrollback 꼬리
cys events --reconnect                        # 푸시 이벤트 스트림 (폴링 금지의 대체)
cys attach surface:1                          # 출력 미러 (read-only)
```

## 자원 거버넌스 (박사님 3대 완화책)

| 완화책 | 기능 | 명령/이벤트 |
|---|---|---|
| ① 로그인 감지 강화 | 모든 출력 라인에 헬스 룰(기본: Not logged in·401·token expired·rate limit) 매칭 → 30초 디바운스 push | `health.alert` 이벤트 · `cys add-health-rule <name> <regex>` |
| ② 짧은 작업 단위 | idle(기본 300초 무출력) 감지 push → master가 분할·점검 판단 | `pane.idle` 이벤트 |
| ③ 서버 생명주기 강제 종료 | **scoped 실행**: 새 프로세스 그룹+원장 등록, 종료 시 그룹 전체 SIGKILL · **close-surface**: pane 자식 트리 전멸 · **watchdog**: loadavg/자식 수/동일 명령 중복(기본 3개+) 감지, `CYS_AUTOKILL_DUP=1`이면 자동 정리 | `cys run -- <cmd>` · `cys ps` · `cys kill <pid>` · `watchdog.*` 이벤트 |

## 프로토콜 (NDJSON, 한 줄 = JSON 하나)

요청 `{"id":1,"method":"surface.send_text","params":{"surface_id":"surface:2","text":"..."}}`
응답 `{"id":1,"ok":true,"result":{...}}` / `{"id":1,"ok":false,"error":{"code","message"}}`

메서드: `system.ping` `system.identify` `system.claim_role/resolve_role` `system.pause/resume/gate_check` `system.topology`
`surface.create/list/send_text/send_key/read_text(+since_line 델타)/resize/rename/close/attach/set_meta/wait_for`
`events.stream` `ledger.register/deregister/list/kill` `health.add_rule/list_rules` `feed.push/reply/list`
`recall.search` `schedule.status/run_now` `status.set` `org.status` `queue.list/clear` `attest.pin/verify`

이벤트: `surface.created/closed/exited/input_injected(from 발신자 커널 검증 태깅)` `health.alert/action`
`watchdog.load_high/proc_count_high/duplicate_procs/duplicates_killed` `pane.idle` `ledger.registered/killed`
`daemon.started/stopping` `queue.enqueued/delivered/dropped` `feed.item.created/resolved/timeout/aging`
`schedule.fired/missed/error/command_done` `status.changed` `agent.exited/recovered/restart_blocked/exit_unrecoverable`
`approval.request` `todo.updated` `master.deadman` `autopilot.paused/resumed` `acl.denied` `role.claimed`

## 환경 변수

`CYS_SOCKET` 소켓 경로 (기본 `~/.local/state/cys/cys.sock`, win `\\.\pipe\cys`) ·
`CYS_SHELL` · `CYS_LOAD_THRESHOLD`(기본 코어수×2) · `CYS_PROC_THRESHOLD`(50) ·
`CYS_DUP_THRESHOLD`(3) · `CYS_AUTOKILL_DUP`(0/1) · `CYS_IDLE_SECONDS`(300) ·
`CYS_TYPING_GUARD_SECS`(3, 0=off) · `CYS_FEED_REMIND_SECS`(300, 0=off) ·
`CYS_MASTER_DEADMAN_SECS`(900, 0=off) · `CYS_AGENT_AUTORESTART`(0/1) ·
`CYS_RECALL_RETAIN_DAYS`(30, 0=무제한) · `CYS_TODO_DIRS`(콜론 구분 추가 감시 루트)

## 자비스 네이티브 기능 (2026-06-12 전면 구현 — 19건)

> 설계 철학: **디렉티브(절대지침)가 master에게 수동으로 시키는 모든 운영 의무 = 터미널의 기능 결함 목록.**
> ①규약→데몬 보증으로 기계화 ②자기보고 우선·화면 파싱은 fallback ③자동화 3단 안전등급(alert→escalate→act, deny-by-default).
> 상세 설계·근거: docs/javis-native-features-proposal-2026-06-12.md

| # | 기능 | 명령/이벤트 |
|---|---|---|
| T1-1 | **자기보고**: 에이전트가 상태·컨텍스트%·작업을 직접 신고 | `cys set-status --state working --context 57 --task "..."` → `status.changed` |
| T1-2 | **관제 보드**: 전 노드 1콜 요약 (read-screen 폴링 대체) | `cys status [--json]` (`org.status` RPC) |
| T1-3 | **발신자 신원·ACL**: 커널 peer pid로 from 검증 + role→role 송신 정책 | `~/.cys/pack/acl.json` · 거부 시 `acl.denied` |
| T2-4 | **컨텍스트 사이클 집행기**: 저장 지시→파일 mtime+해시 게이트→버퍼 정리→clear→지침 재주입→재개 포인터. master는 `--verifier` 필수(self-clear 금지) | `cys cycle-agent --role worker [--verifier master]` |
| T2-5 | **에이전트 사망 즉시 감지**: 셸 생존·에이전트만 죽은 상태를 watchdog이 잡음 (+옵션 자동 재기동 3회, 401 시 차단) | `agent.exited/recovered/…` · `cys node-recover --role X` |
| T2-6 | **조직 복원**: 토폴로지 영속(role→agent→cwd) + 일괄 재기동·재주입·resume(`resume_arg`). 작업 재개는 master 판단 | `cys restore [--include-master]` (`system.topology`) |
| T2-7 | **디렉티브 드리프트 감지·재주입**: 각성 핑(자기-에코 오탐 차단 토큰 분리) 후 무응답 시 재주입 | `cys reinject --role X [--check]` |
| T2-8 | **master dead-man**: master 사망·장기 무출력 감시 (단일 장애점 봉합) | `master.deadman` 이벤트 |
| T3-9 | **todo 워치**: `_round/*_TODO.md` mtime 감시→진행률 집계 (push 규약을 기계 보증으로) | `todo.updated` · `cys status`에 집계 |
| T3-10 | **원샷 타이머**: 상대시간 1회 발화 후 job 자동 삭제 (+fresh TTL `--close-after`) | `cys schedule add --id x --in 20m --text ... --to role` |
| T3-11 | **역할 글롭 브로드캐스트** | `cys send --to 'reviewer-*' "..."` |
| T3-12 | **feed aging 재알림**: pending 승인 무음 적체 차단 | `feed.item.aging` (기본 5분 주기) |
| T3-13 | **입력 안전**: ①타이핑 가드 — 사람이 입력 중인 pane에 원격 직접 주입 거부(`--queued`는 허용) ②`send --clear-first`(어댑터 등록 pane 한정 Ctrl-U 선정리) | `typing_guard` 에러 |
| T3-14 | **델타 읽기·완료 대기**: 단조 라인 커서 + 데몬측 블로킹 regex 감시 (★plain-line 마커 규약 — 주입 텍스트의 에코도 매칭되므로 마커는 본문에 그대로 넣지 말 것) | `cys read-screen --since N` · `cys watch --until <re>` |
| T4-15 | **kill-switch**: 큐 배달·스케줄 발화 동결 + feed wait 타이머 동결 + preflight 게이트. ★한계: '조직 간 신경 차단'이지 실행 중 에이전트의 행동 정지가 아님 | `cys pause/resume` · `cys gate-check`(exit 4=paused) · `cys queue list/clear` |
| T4-16 | **승인 격상**: agents.json `approval_patterns` 화면 스캔→이벤트+feed 항목. ★자동 응답 절대 없음 — 판단은 master | `approval.request` |
| T4-17 | **헬스룰 조치 바인딩**(opt-in): 60초 창 연속 매칭 게이트 통과 시 queued 배달만 일시정지(직접 send 통과 — 복구 명령 보장). 에코 제외(주입 2초 창) 내장 | `cys add-health-rule n p --action pause-queue --threshold 3` |
| T4-18 | **트랜스크립트 해시체인 attest**: 저장 트랜스크립트의 변조 증거성 — 평가자가 pin을 외부 보관, 사후 대조 (producer≠evaluator 기계화). 검증 지평=retention 창(prune prefix는 anchor 봉인) | `cys attest pin` → `count:hash` · `cys attest verify <pin>`(exit 2=변조) |
| T4-19 | **recall 보존 정책**: 트랜스크립트 무한 성장 차단 (기본 30일, 6시간 주기 prune) | `CYS_RECALL_RETAIN_DAYS` |

## E2E 검증 결과 (2026-06-11, macOS)

- A pane → B pane stdin push → B 셸 실행 → read-screen 확인 ✅
- `surface.input_injected` push 수신(발신 surface 자동 태깅) ✅
- 'Not logged in' 출력 → `health.alert` 즉시 push ✅
- scoped run: 자식이 남긴 `sleep 300 &` 그룹째 강제 종료, 잔류 0 ✅
- 동일 명령 3개 → `watchdog.duplicate_procs` 감지 ✅
- close-surface → 자식 트리(3개) 전멸, `descendants_killed:3` ✅

## UI (Tauri 2 + xterm.js)

```bash
cd ui && sh build.sh          # 프런트엔드 번들 (bun)
cargo build -p cys-app     # dev 실행: ./target/debug/cys-app
bun x @tauri-apps/cli build   # 배포: target/release/bundle/macos/cys.app
# 번들에 데몬·CLI 동봉: cp target/release/{cysd,cys} <app>/Contents/MacOS/
```

- **코어/UI 분리**: UI는 소켓 클라이언트일 뿐. 세션(PTY)은 데몬 소유 → UI 재시작·앱 재설치에도 세션 유지(재attach).
- 워크스페이스 탭(＋ 추가·더블클릭 이름변경·× 닫기) · 분할 pane(⌘T·⌘D·⌘⇧D·⌘W) · divider 드래그 리사이즈.
- health/watchdog/feed push 이벤트 → 토스트. 주의: ui/ 수정 후 앱 재빌드 필요(프런트엔드가 바이너리에 임베드됨).

## 승인 Feed (워커 승인 요청 집중 처리)

```bash
cys feed push --wait --title "git push 승인" --body "..."   # 결정까지 블록 (exit 0=allow, 2=deny, 3=timeout)
cys feed list --status pending
cys feed reply <request_id> allow                            # master 또는 UI Allow/Deny 버튼
```

- `feed.push wait=true` → 데몬이 oneshot 채널로 연결을 블록(기본 120초) → `feed.reply`가 풀어줌.
- UI: wait 요청 도착 시 Feed 패널 자동 오픈 + 뱃지 + Allow/Deny 버튼 + 토스트.
- 에이전트 hook 연동 예(Claude Code PreToolUse): hook 스크립트에서 `cys feed push --wait ...` 호출 → exit code로 결정 반영.

## 알려진 한계 / 다음 단계

- Windows: 코어(named pipe·ConPTY·DSR 핸드셰이크)는 Parallels Win11 ARM64에서 실검증 완료
  (docs/WINDOWS_VALIDATION.md). MSI 설치 실검증과 Tauri UI의 Windows 빌드는 잔여.
- macOS에서 sysinfo가 cmdline 전체를 못 읽으면 프로세스명으로 중복 그룹핑(과탐 가능).
- `cys run` 중 Ctrl-C로 CLI가 죽으면 그룹 정리가 watchdog 주기(5초)로 넘어감.
- 스킬 자동수확은 현재 디렉티브 규약(작업 후 수확 의무) — Hermes Curator식 자동 트리거는 후속 연구.
- 적대 벤치마킹(docs/COMPETITIVE_ANALYSIS.md) 흡수 잔여: 메신저 채널 어댑터(P2)·모바일 Node(P4)·
  클라우드 실행 백엔드(P6).

## 인플라이트 큐 (steer / followup)

- 기본 전송(`cys send`)=**steer**: 즉시 stdin 주입 — Claude Code 등은 실행 중 입력을 조향으로 소화.
- `cys send --queued`=**followup**: 데몬 큐에 적재 후 **대상이 3초 이상 조용해지면** 한 틱에 한 건씩
  bracketed paste+Return으로 자동 배달(캡 100건). 이벤트: `queue.enqueued`/`queue.delivered`.

## Heartbeat fresh-session 격리

`cys schedule add ... --fresh --agent claude` — 매 발화마다 **새 surface를 기동해 과업을 주입**한다
(기존 세션의 권한·컨텍스트 상속 차단). 표준 4역할 외 임시 역할은 WORKER 지침으로 폴백 각성.

## 라이선스 · 연락처

MIT License (LICENSE 참조). 문의: **cysinsight@gmail.com** (CYSJavis).
