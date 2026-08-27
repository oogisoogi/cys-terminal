# 구현 기록 — 페인 제목 번호화 A안(무재빌드 pack 스위퍼)

- 작성: worker@surface:297 · 2026-07-27
- 설계 근거: `docs/verdict-pane-title-numbering-2026-07-27.md`(같은 워커의 규명 판정문)
- 산출물: `~/.cys/pack/bin/javis_panetitle.py` (**신규 파일 1개. 기존 파일 수정 0건**)
- 빌드·설치·재빌드 0건 · role/배선/ACL/큐 무접촉 · pack 외 파일 수정 0건 · 스케줄 잡 등록 0건

---

## 1. 적용 전/후 — 실제 값

```
# 적용 전 (cys list)
surface:285  role=worker-3  worker-3-claude · channels          /Users/oogisoogi/.claude/channels
surface:297  role=worker    worker-claude · cys-terminal-src    /Users/oogisoogi/cys-terminal-src

# 적용 후 (cys list)
surface:285  role=worker-3  285 · channels                      /Users/oogisoogi/.claude/channels
surface:297  role=worker    297 · cys-terminal-src              /Users/oogisoogi/cys-terminal-src
```

멱등 확인 — 같은 명령 재실행:

```
surface:285  worker-3  285 · channels            [무동작 — 이미 번호로 시작 — 멱등(호출 0)]
surface:297  worker    297 · cys-terminal-src    [무동작 — 이미 번호로 시작 — 멱등(호출 0)]
― 대상 2 · 변경 0 · 무동작 2 · 실제적용 0
```

`rename` RPC 호출 자체가 0건이다(제목 비교 후 호출 전에 빠진다).

## 2. 자기 페인(297) 먼저 적용한 이유 — master 지시와 다른 순서를 택했다

master 지시는 「자기 제목 변경이 위험하면 남의 것부터」였다. **위험하지 않다고 판정**했고,
그 판정이 틀렸을 경우의 피해를 **남이 아니라 내가 받도록** 순서를 뒤집었다.

- 판정 근거: `surface.rename`은 `*surface.title.lock().unwrap() = title` 한 줄이 전부다
  (`cysd/handlers.rs:1764`). 이벤트 발행·PTY 쓰기·role 갱신이 없다.
- 실측: 297 적용 직후 같은 세션에서 `cys list`가 정상 동작했고 `role=worker` 그대로다.
  그 다음에 285를 적용했다.

## 3. 규칙 구현 (master 확정분 그대로)

| 규칙 | 구현 | 근거 |
|---|---|---|
| 제목 = 「번호 · 특성」 | `expected_title()` → `"%d · %s"` | master 확정 1 |
| agent 표기 제거 | 새 제목에 agent를 넣지 않는다 | master 확정 2 |
| 특성 = cwd basename, 범용명은 제외(denylist) | `GENERIC_DIRS` 59개 + 홈 폴더명 동적 추가 | master 확정 3 |
| 폴백 = 번호 · worker+서수 | `role_fallback()` = `re.sub(r"-(\d)", r"\1", role)` — **role 값은 읽기만** | master 확정 4 |

denylist 보강분(master 최소 목록 9개에 더한 것): 홈/시스템 표준 폴더(library·movies·music·
pictures·public·users·home·root·var·opt·usr·etc·lib·share·private·volumes·system) +
어느 저장소에나 있어 노드를 식별하지 못하는 이름(work·workspace(s)·projects·project·repos·repo·
code·dev·temp·test(s)·build·dist·target·out·node_modules·docs·doc·scripts·script·config·data·
new·untitled·folder). 소문자 **완전 일치**만 걸린다 — `src`는 걸리지만 `cys-terminal-src`는 안 걸린다.

**서수 날조는 하지 않았다.** `worker-3`→`worker3`이지만 서수 없는 `worker`는 `worker`로 둔다.
없는 번호를 지어내면 규칙이 아니라 추측이 된다.

## 4. 무동작(SKIP) 조건 4가지 — 무엇을 일부러 안 건드렸나

| 조건 | 이유 |
|---|---|
| 이미 번호로 시작하는 제목 | 멱등. **사용자가 손으로 지은 이름도 번호만 있으면 존중**한다 |
| `role` 없는 surface | UI가 자동제목(`/^surface \d+$/`)을 **현재 경로로 실시간 치환**한다(`ui/src/main.ts:1772`). 제목을 박으면 그 실시간 추적이 죽는다 — 규칙을 지키려다 기능을 없애는 거래는 하지 않았다 |
| `exited` surface | 죽은 페인 |
| `surface_id` 미상 | 방어 |

## 5. 양방향 강제발화 테스트 — 값으로 보고

```
$ python3 ~/.cys/pack/bin/javis_panetitle.py self-test
변경해야 할 것: 11/11 발화
건드리면 안 되는 것: 7/7 무동작 유지
판정: PASS   (rc=0)
```

- **반드시 바꿔야 할 것 11종**: 실측 285·297 / 홈폴더 cwd → 폴백 / 루트 cwd → 폴백 /
  Documents → 폴백 / 데몬 기본 자동제목(`surface 44`) / 숫자 없는 하이픈 role 보존(reviewer-codex) /
  **남의 번호가 붙은 제목**(`285 · research`인데 sid=60 → 교정) / **프로덕션 기본 denylist 직격 3종**
- **반드시 건드리면 안 되는 것 7종**: 이미 규칙 준수 / 사용자 수기 이름(`297 박사님 지시 대기창`) /
  role=None / role="" / exited / 번호 뒤 공백형(`10 cso 임시`) / 번호만 있는 제목(`11`)

### 5-1. 테스트가 가짜가 아님을 검산했다 (뮤테이션 4종)

| 뮤테이션 | 결과 | 뜻 |
|---|---|---|
| 원본 | PASS (11/11, 7/7) | — |
| M1 `decide`가 항상 SKIP | **FAIL** (0/11 발화) | 아무것도 안 바꾸는 구현이 통과하지 못한다 |
| M2 `decide`가 항상 RENAME | **FAIL** (0/7 유지) | 다 바꾸는 구현도 통과하지 못한다 |
| M3 멱등 가드 제거 | **FAIL** (3/7 유지) | 멱등 조항이 실제로 검사된다 |
| M4 denylist 비우기 | **FAIL** (9/11 발화) | 폴백 규칙이 실제로 검사된다 |

★M4는 **처음엔 PASS했다.** 테스트가 고정 fixture 집합을 주입해 프로덕션이 실제로 쓰는
`GENERIC_DIRS`를 한 번도 안 때렸기 때문이다 — 뮤테이션 검산이 없었으면 못 봤을 구멍이다.
`MUST_CHANGE_PROD_DEFAULT` 3종(generic 미주입 = `cmd_plan`과 동일 경로)을 추가해 막았다.

## 6. 커버리지 — 「spawn-worker.sh 기동 직후 1회 호출」로 충분한가

**충분하지 않다.** surface를 새로 만드는 경로가 그것 말고도 5개다(전부 근거 있음):

| 경로 | 근거 | 1회 호출로 덮이나 |
|---|---|---|
| `~/.claude/channels/spawn-worker.sh` 워커 기동 | master 브리프 | ○ (호출 지점) |
| `javis_boot_node.py:529` `cys launch-agent` | pack 스크립트 | ✗ |
| `cys boot` 4종 부트 | `cys.rs:4393` | ✗ |
| `cys restore` fresh 재기동 | `cys.rs:7185` → `5372`에서 제목 **재계산** | ✗ (번호가 다시 사라진다) |
| `javis_phoenix.py:1384` fresh-fallback | 독약세션 부활 | ✗ |
| master 수기 `cys launch-agent` | — | ✗ |
| GUI 새 페인 | `src-tauri/src/main.rs:618` | 해당 없음(role 없음 = 의도적 제외, §4) |
| `node-recover`·좌석 내 재연결 | 기존 surface 재사용(`cys.rs:7161`) | 구멍 아님 — 제목이 보존된다 |

★**빠지는 것이 실제로 문제가 되는가 — 된다.** 빠지는 경로 대부분이 **복구 경로**
(restore·phoenix·boot)인데, 페인 번호를 대조해야 할 필요가 가장 큰 순간이 바로 그때다.

### 그래서 무엇을 했나 — **스케줄 잡을 등록하지 않았다**

`cys schedule list` 실측: `panetitle` 이름의 잡 **0건**(등록 0건 확인). 오늘 방침(자동 장치를
늘리지 않는다)을 지켰다. 대신 `apply`를 **어디서 불러도 안전한 형태**로 만들어 두었다:

- 멱등(바꿀 게 없으면 RPC 0건) · 실행 비용 = RPC 2회 · 상시 프로세스 0
- `--surface N`으로 1건만 처리 가능(기동 직후 1회 호출용)
- 인자 없이 `apply`면 전수 스윕(복구 직후 한 번 부르면 전 함대가 교정된다)

**master 결정 사항**: 이 호출을 ⑴master가 필요할 때 손으로 부를지, ⑵master 소유 파일
(`spawn-worker.sh` 등)에 1줄 넣을지, ⑶그래도 주기 잡이 필요한지. **⑵는 pack 외 파일이라
내가 손대지 않았다**(제약 4). 권고는 ⑵ + 복구 루틴 끝에 전수 `apply` 1줄이며, 주기 잡은
지금 필요 없다고 본다 — 잡을 늘리지 않고도 덮인다.

## 7. 롤백 1스텝

```
python3 ~/.cys/pack/bin/javis_panetitle.py revert          # 원 제목 복원(전수)
python3 ~/.cys/pack/bin/javis_panetitle.py revert --surface 285
rm ~/.cys/pack/bin/javis_panetitle.py                      # 무력화(장치 자체 제거)
```

원장(append-only): `~/.cys/state/panetitle-ledger.jsonl` — 실제 기록

```json
{"ts": "2026-07-27T21:44:05", "surface_id": 297, "old": "worker-claude · cys-terminal-src", "new": "297 · cys-terminal-src"}
{"ts": "2026-07-27T21:44:10", "surface_id": 285, "old": "worker-3-claude · channels", "new": "285 · channels"}
```

`revert`는 **가장 오래된 old**(=최초 원본)로 되돌린다 — 여러 번 rename 돼도 원점 복귀.

## 8. 업데이트 생존 실증 (「그럴 것이다」 아님)

```
$ cys pack-ownership bin/javis_panetitle.py
bin/javis_panetitle.py: custom — 비출하 자작 파일 — 업데이트·치유·정리 전부 불가침(생존 보증 대상)

$ cys pack-ownership bin/javis_formation.py      # 대조군: vendor 파일
bin/javis_formation.py: system — vendor 소유 — 수정본은 다음 설치 스윕에 치유(수정 전 .user 보존). 자작은 새 파일로
```

`cys pack-ownership`은 소스에 「결정론 조회 전용(쓰기 0)」으로 명시돼 있다(`cys.rs:2249`).

## 9. 리스크 3건 — 구현 후 재확인

| 리스크 | 구현 후 상태 |
|---|---|
| **복원 유실** | **그대로 유효**. `cys restore` fresh 재기동은 topology의 title을 안 읽고 `workflow_title()`을 재계산한다(`cys.rs:7185`→`5372`). 다만 이제 `apply` 한 번으로 복구된다(자동은 아님 — §6) |
| **`surface.rename` ACL 무게이트** | **그대로**. 내 스크립트는 이 사실을 이용할 뿐 바꾸지 않았다. 로컬 소켓에 닿는 아무 클라이언트나 임의 제목을 바꿀 수 있다 |
| **status/fleet 폴백 라벨** | 실측 결과 **현재는 발현하지 않는다** — 두 노드 모두 업무를 자기보고 중이라 `cys status`의 TASK/TITLE 열이 self-report를 쓴다(`cys.rs:6529-6533`). 업무 미보고 노드에서만 새 제목이 그 칸에 뜬다(무해) |

## 10. ★내가 만들었을 법한 결함 — 스스로 사냥한 결과

**나는 이 구간에서 producer이자 evaluator다.** 그래서 「통과」를 세지 않고 **결함을 찾으러** 갔다.
찾은 것 2건(1건은 고쳤고 1건은 남겨 보고한다):

1. **[고침] 테스트가 프로덕션 denylist를 한 번도 안 때렸다.** §5-1 M4. 고정 fixture 주입 때문에
   `GENERIC_DIRS`를 비워도 테스트가 통과했다. `MUST_CHANGE_PROD_DEFAULT` 3종으로 막고
   재검산해 M4가 FAIL로 뒤집히는 것을 확인했다.
2. **[남김·경미] `revert`가 사용자 수기 이름을 덮을 수 있다.** 번호화 이후 사용자가 페인을 손으로
   개명하면, `revert`는 원장의 최초 old(번호화 이전 제목)로 되돌리므로 그 수기 이름이 사라진다.
   롤백 명령의 정의상 의도된 동작이라 고치지 않았고, 여기 적어 둔다.

추가로 점검했으나 결함이 아니었던 것:
- `is_conforming`의 `^297(\s|$)`가 `2970 …`을 오탐하는가 → 안 한다(경계 검사).
- 제목 문자열이 셸을 거치는가 → 아니다. JSON-RPC로만 나간다(§7-0 인용사고류 무관).
- 동시 실행 경합 → 마지막 쓰기 우선, 값이 같아 무해.
- 알려진 한계 1건: `characteristic()`의 basename 처리는 POSIX 기준이라 Windows 경로
  (`C:\x\y`)에서는 폴더 분리가 안 된다. 이 배치는 macOS 전용이라 실害 없음(기록만).

## 10-A. 배선 완료 — SessionStart 훅 등록 (master 집행 · 2026-07-27 21:54)

복구 경로(restore·boot·phoenix)마다 1줄을 심는 안은 **폐기**했다. 그 경로들은 pack 파일이 아니라
Rust 바이너리 명령이고(`cys.rs:7185`·`4393`), phoenix만 pack이지만 `system` 등급이라 업데이트가
지운다. 대신 **에이전트가 뜨는 모든 경로가 공통으로 지나는 지점**(claude SessionStart)에 걸었다.

- 훅 파일: `~/.cys/pack/hooks/panetitle-onstart.sh` — `cys pack-ownership` = **custom**(불가침)
- 등록: `~/.cys/claude/settings.json` `hooks.SessionStart` **append** — master 집행(백업 후 원자적 교체)

집행 후 워커 직접 실측(2026-07-27 21:54 · `settings.json` mtime 21:54 · 백업 `*.bak-panetitle-215406` 존재):

```
hook 이벤트 키 7종 전부 보존: SessionStart · PreToolUse · Stop · PreCompact · SessionEnd · PostToolUse · PermissionRequest
SessionStart[1]: sh ~/.cys/pack/hooks/session-start.sh        ← 기존, 무접촉
SessionStart[2]: sh ~/.cys/pack/hooks/inject-context.sh       ← 기존, 무접촉
SessionStart[3]: sh ~/.cys/pack/hooks/panetitle-onstart.sh    ← 신규
최상위 키 9종 보존: hooks · env · statusLine · tui · skipDangerousModePermissionPrompt · theme · agentPushNotifEnabled · model · promptSuggestionEnabled
```

업데이트 생존 근거(§8과 별개 — 등록처에 대한 것):
- Rust는 settings.json을 **파일이 없을 때만** 쓴다 — `src/pack.rs:343` `if !settings.exists()`
- preflight의 훅 등록 3경로(session-start·SELFCORR·appbuild)가 전부 `_prune_stale_hook_entries`
  (`javis_preflight.py:369-386`)를 쓰며, 이 함수는 **자기 스크립트 이름을 참조하는 엔트리만** 정리하고
  타 스크립트 엔트리는 보존한다고 코드에 명시돼 있다

**실발화 확인은 다음 워커 기동 때 master가 실측한다**(현 세션들은 훅 등록 전에 떴다). 이 문서의
E2E 검증(§10-B)은 훅 스크립트를 손으로 발화시킨 것이다.

## 10-B. 훅 E2E 양방향 강제발화 (실측)

| 방향 | 절차 | 결과 |
|---|---|---|
| 바꿔야 한다 | `revert --surface 297`로 원 제목 복원 → 훅 발화 | `worker-claude · cys-terminal-src` → `297 · cys-terminal-src` (rc=0 · stdout 0바이트) |
| 건드리면 안 된다 | 이미 준수 상태에서 훅 발화 | 변화 0 · rename RPC 0건 |
| 가드 A | `CYS_SURFACE_ID` 없음 | rc=0 · 무동작 |
| 가드 B | 엔진 파일 부재(`CYS_PACK_DIR` 오지정) | rc=0 · 무동작 |
| 가드 C | 데몬 소켓 불통 | rc=0 · 무동작 |

★**stdout을 0바이트로 막은 것이 핵심 안전조치다** — SessionStart 훅의 stdout은 **에이전트
컨텍스트로 주입된다**(이 세션 시작 시 `session-start.sh` 출력이 그렇게 들어왔다). 여기서 한 줄이라도
새면 **모든 워커의 첫 컨텍스트가 오염된다.** 실패는 전부 rc=0으로 흘린다 — 제목 부여 실패가
에이전트 기동을 막으면 안 된다(OBSERVABILITY 클래스).

## 10-C. 이 안이 「앞 절반만 자동화」를 피하는 구조 (기록)

복구 경로가 다섯이든 열이든 **전부 claude 세션을 새로 띄운다.** 그래서 경로마다 심는 대신
**공통 통과점 하나**에 건다. 경로가 늘어도 배선은 늘지 않고, 업데이트가 지울 자리에 아무것도
두지 않는다(훅 파일=custom · 등록처=팩 밖).

## 11. 하지 않은 것

- 스케줄 잡 등록 0건(실측: `cys schedule list`에 `panetitle` 0건).
- pack 외 파일 수정 0건(`spawn-worker.sh` 무접촉 — master 소유).
- 기존 pack 파일 수정 0건(신규 파일 1개만 추가).
- GUI 화면에 실제로 어떻게 보이는지는 **눈으로 확인하지 못했다**(워커 페인에서 GUI 창을 볼 수 없다).
  데몬 상태(`cys list`)까지가 내 실측 범위다. UI는 3초 폴링으로 제목을 갱신한다
  (`ui/src/main.ts:1915` `setInterval(refreshPaneTitles, 3000)`) — **박사님 화면 확인은 남아 있다.**
