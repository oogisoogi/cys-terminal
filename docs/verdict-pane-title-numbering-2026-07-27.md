# 판정문 — cys 페인 제목 생성 로직 규명 (번호 반영 가능성)

- 작성: worker@surface:297 (cwd `/Users/oogisoogi/cys-terminal-src`)
- 일시: 2026-07-27
- 범위: 규명·판정만. 구현·수정·빌드·설치 0건. `cys` 상태 변경 명령 0건.
- 실측 기준: 소스 `cys-terminal-src` @ `7e19cc0` (버전 0.13.20) + 설치본 `/Applications/cys.app` (`cys --version` = 0.13.20)

---

## 0. 한 줄 판정

**가능하다. 그리고 바이너리 재빌드 없이도 가능하다.**

제목은 Rust CLI 바이너리(`cys`)가 `surface.create` 요청에 실어 보내는 문자열이며,
번호는 그 시점에 아직 없다. 그러나 **제목을 사후에 바꾸는 프리미티브 `surface.rename` RPC가
데몬에 실재하고, 지금 돌고 있는 설치본에서 응답하는 것을 실측으로 확인했다.**
따라서 「번호를 제목에 넣는다」는 재빌드 없이 즉시 구현 가능하며, 항구적 해법으로는
데몬 쪽 3줄 수정이 더 깨끗하다.

---

## 1. 제목은 어디서 만들어지는가

### 1-1. 조합 지점 = Rust **CLI** 바이너리 (pack 아님·데몬 아님)

`src/bin/cys.rs:5307-5316`

```rust
/// 절대지침(앵커1-b): 탭(타이틀) = 워크플로우 폴더명 — "{role}-{agent} · {폴더}".
fn workflow_title(role: &str, agent: &str, cwd: &Option<String>) -> String {
    cwd.as_deref()
        .map(|s| s.trim_end_matches(['/', '\\']))
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
        .filter(|f| !(f.len() == 2 && f.ends_with(':') && f.as_bytes()[0].is_ascii_alphabetic()))
        .map(|folder| format!("{role}-{agent} · {folder}"))
        .unwrap_or_else(|| format!("{role}-{agent}"))
}
```

호출 지점은 단 한 곳 — `src/bin/cys.rs:5372`, `surface.create` 요청의 `title` 파라미터:

```rust
let r = request(
    "surface.create",
    json!({"cwd": cwd, "title": workflow_title(role, agent, &cwd), "role": role, ...}),
)?;
let sid = r["surface_id"].as_u64().ok_or("create returned no id")?;   // ← 5382. 번호는 '응답'에서 온다
```

이 함수는 `run_launch_agent_opts()` 안에 있고(`cys.rs:5319`), `cys launch-agent`·`cys boot`
(`cys.rs:4393`)·`cys restore`의 fresh 재기동(`cys.rs:7185`)이 전부 이 한 경로를 공유한다.
불변식이 소스에 박제되어 있다 — `cys.rs:10507`: 「탭 타이틀 = "{role}-{agent} · {워크플로우 폴더명}"」.

### 1-2. 패턴 확정 — 추정이 아니라 실측 (confidence: High)

master의 추정 `<role>-<agent> · <cwd basename>` 은 **맞다.** 소스(위) + 라이브 2건 동시 확인:

```
$ cys list
surface:285  role=worker-3  pid=52569  exited=false  worker-3-claude · channels          /Users/oogisoogi/.claude/channels
surface:297  role=worker    pid=88966  exited=false  worker-claude · cys-terminal-src    /Users/oogisoogi/cys-terminal-src
```

`worker-3`+`claude`+`channels`, `worker`+`claude`+`cys-terminal-src` — 조합 규칙과 정확히 일치.

### 1-3. 제목의 다른 출처 3가지 (전수)

| # | 출처 | 근거 | 결과 |
|---|------|------|------|
| ① | `cys launch-agent`/`boot`/`restore` | `cys.rs:5372` | `{role}-{agent} · {폴더}` ← 우리 노드 전부 |
| ② | 데몬 기본값(제목 미지정 시) | `src/bin/cysd/state.rs:2167` `title.unwrap_or_else(\|\| format!("surface {id}"))` | `surface 297` — **이미 번호를 쓰고 있다** |
| ③ | 사용자 지정 | `cys new-surface --title`(`cys.rs:44`), GUI 인라인 rename(`ui/src/main.ts:1993`) | 임의 문자열 |

★②가 중요하다. **데몬은 제목을 만들 때 이미 번호를 알고 있다**(아래 2절).

②의 기본 제목은 GUI에서 화면에 그대로 뜨지 않는다 — `ui/src/main.ts:1772-1774`가
`/^surface \d+$/`를 "자동 제목"으로 보고 **현재 경로로 치환**한다:

```ts
const isAutoTitle = (t) => !t || /^surface \d+$/.test(t);
const paneTitle = (title, liveCwd) => isAutoTitle(title) ? liveCwd || "…" : title;
```

⇒ 지금 박사님 화면에 번호가 안 보이는 이유는 두 갈래다: 역할 노드는 ①이 번호 없는 제목을 박고,
무역할 셸은 ②의 번호 있는 기본 제목이 UI에서 경로로 치환된다.

### 1-4. ★셸이 제목을 덮어쓰지 않는다 (안정성 확인)

일반 터미널은 OSC 0/2(`ESC]0;...`)로 프로그램이 탭 제목을 바꾼다. cysd는 **그것을 소비하지 않는다** —
OSC 스캐너는 **9/99/777 데스크톱 알림만** 추출한다(`state.rs:2718-2760` `parse_osc_notification`,
`state.rs:2363-2392` 소비 지점). 즉 한 번 설정한 제목은 셸·TUI가 덮지 않는다. (confidence: High)

---

## 2. surface 번호를 제목에 넣을 수 있는가 — 번호는 언제 배정되는가

**가능하다.** 다만 "누가 만드는가"에 따라 시점이 갈린다.

### 2-1. CLI 관점 — 제목 생성 시점에 번호는 **아직 없다**

`cys.rs:5372`에서 제목을 만들어 보내고, `cys.rs:5382`에서 응답의 `surface_id`를 처음 받는다.
⇒ CLI가 제목에 번호를 넣으려면 **생성 후 rename** 이 필요하다. (선(先)조합 불가)

### 2-2. 데몬 관점 — 제목 확정 시점에 번호는 **이미 있다**

`src/bin/cysd/state.rs`, `create_surface_with_env()` 한 함수 안에서 순서가 이렇다:

- `state.rs:2028` `let id = self.next_id.fetch_add(1, Ordering::SeqCst);` ← 번호 배정
- `state.rs:2167` `title: Mutex::new(title.unwrap_or_else(|| format!("surface {id}")))` ← 제목 확정

**139줄 차이로 번호가 먼저다.** 그리고 이 함수는 `role`도 인자로 받는다(`state.rs:2022`).
⇒ 데몬 안에서는 `번호 + role + 넘어온 제목`을 전부 손에 쥔 채 제목을 합성할 수 있다.

### 2-3. ★사후 변경 프리미티브는 실재한다 — master 전제의 반증

`src/bin/cysd/handlers.rs:1750-1766`

```rust
"surface.rename" => {
    let Some(sid) = resolve_surface_id(&params) else { ... "missing surface_id" };
    let Some(title) = param_str(&params, "title") else { ... "missing title" };
    let Some(surface) = daemon.get_surface(sid) else { ... "not found" };
    *surface.title.lock().unwrap() = title.clone();
    Reply::Single(ok_response(&id, json!({"surface_id": sid, "title": title})))
}
```

**지금 돌고 있는 설치본 데몬에서 실측 확인했다**(무변경 probe — 파라미터를 비워 첫 가드에서
반환시켰다. mutation 은 3관문 뒤라 도달 불가):

```
surface.rename                  -> {"error":{"code":"invalid_params","message":"missing surface_id"}}
surface.resize                  -> {"error":{"code":"invalid_params","message":"missing surface_id"}}
surface.definitely_not_a_method -> {"error":{"code":"method_not_found","message":"unknown method: ..."}}
```

미지 메서드는 `method_not_found`로 갈린다. `surface.rename`은 **디스패치된다** = 존재한다. (confidence: High)

소비자는 GUI뿐이다 — `src-tauri/src/main.rs:2067` (`rename_surface` 커맨드) ←
`ui/src/main.ts:1994` (제목 더블클릭 인라인 편집). **CLI 서브커맨드는 없다.**

---

## 3. 수정 지점과 비용

### 3-0. pack인가 바이너리인가

**pack이 아니다.** 제목 조합은 Rust 소스 2곳(CLI `cys.rs:5307`, 데몬 `state.rs:2167`)에만 있다.
pack 스크립트는 surface 제목을 만들지도 읽지도 않는다 — `~/.cys/pack/bin/*.py`의 `title`
등장은 전부 ADR 제목·feed 승인 제목이다(`javis_adr.py` 등). pack 스크립트들은 데몬과
`subprocess(["cys", ...])`로 대화한다(`javis_formation.py:235,310`).

단, master 질문에 답하자면 — **pack에 새 파일을 만들 경우 등급은 `custom`이다**(실측):

```
$ cys pack-ownership bin/javis_panetitle.py
bin/javis_panetitle.py: custom — 비출하 자작 파일 — 업데이트·치유·정리 전부 불가침(생존 보증 대상)
$ cys pack-ownership bin/javis_formation.py
bin/javis_formation.py: system — vendor 소유 — 수정본은 다음 설치 스윕에 치유(수정 전 .user 보존). 자작은 새 파일로
```

⇒ **신규 자작 파일은 업데이트에도 산다.** 기존 vendor 스크립트를 고치면 치유로 되돌아간다.
(`cys pack-ownership`은 「결정론 조회 전용(쓰기 0)」로 소스에 명시 — `cys.rs:2249`)

### 3-1. 세 가지 실행안

| 안 | 지점 | 규모 | 재빌드 | 커버리지 | 가역성 |
|----|------|------|--------|----------|--------|
| **A. pack 스위퍼(무재빌드)** | 신규 `pack/bin/javis_panetitle.py` + `cys schedule` 주기 잡 | 새 파일 ~60줄, 기존 파일 수정 0 | **불요** | 100%(생성 경로 무관·복원 후 재교정까지) | 잡 1개 삭제로 원복 |
| **B. CLI 사후 rename** | `cys.rs:5382` 직후 `surface.rename` 1회 | ~5줄 | 필요 | `launch-agent`/`boot`/`restore` 경로만 | 재빌드 필요 |
| **C. 데몬 합성(항구)** | `state.rs:2167` 제목 합성에 `id` 접두 | ~3줄 | 필요 | 모든 생성 경로(GUI 새 페인 포함) | 재빌드 필요 |

**A안 상세** — 무재빌드가 가능한 이유는 §2-3이다. `surface.list`로 현재 제목을 읽고, 번호가
없는 제목만 `surface.rename`으로 교정한다. RPC 프레이밍 선례가 이미 pack 안에 있다
(`~/.cys/pack/bin/javis_queue_drill.py:107-121` — newline-delimited JSON over AF_UNIX,
「cys.rs request()와 동일 프레이밍」이라고 주석에 명시). 상시 서버 프로세스를 만들지 않고
`cys schedule` 주기 잡으로 돌리면 워커 지침 1조(서버 최소화)와도 충돌하지 않는다.

**C안 상세** — `state.rs:2167` 한 줄을 「번호 + 전달받은 제목(또는 role 파생)」으로 바꾸면
`launch-agent`·GUI 새 페인·복원까지 **한 지점에서 전부** 번호가 붙는다. 사용자가 GUI에서
직접 지은 이름은 `surface.rename`이 나중에 덮으므로 존중된다. UI의 `isAutoTitle` 정규식
(`/^surface \d+$/`)과 충돌하지 않게 형식을 정해야 한다 — 예 `297 · channels`는 정규식에
안 걸리므로 그대로 표시된다(안전). 반대로 `surface 297 channels` 같은 형태는 피해야 한다.

### 3-2. 재빌드 외 부작용

- **role 무접촉 확인.** `surface.rename`은 `surface.title` 한 필드만 쓴다(`handlers.rs:1764`).
  `role`·roles 매핑·큐·ACL 어디에도 손대지 않는다. C안(`state.rs:2167`)도 title 필드 한정.
  ⇒ 금지 조건(role 변경 금지) 위반 없음. (confidence: High)
- **표시 외 소비처 2곳(무해하지만 표기가 바뀐다).** `cys status`/`cys fleet` 표에서 노드가
  현재 업무를 자기보고하지 않았을 때 **제목을 업무 칸 폴백 라벨로** 쓴다
  (`cys.rs:6529-6533`, `cys.rs:6606-6610`). 번호가 붙으면 그 칸 문구가 `297 · channels`로 바뀐다.
  기계 파싱이 아니라 사람 표시용이므로 깨지는 로직은 없다.
- ★**복원 시 유실(A·C안이 자동 치유, B안은 남는 문제).** 제목은 topology에 영속된다
  (`src/bin/cysd/governance.rs:1130` `"title": s.title.lock().unwrap().clone()`).
  그러나 `cys restore`의 fresh 재기동은 저장된 title을 **쓰지 않고** `workflow_title()`을
  다시 계산한다(`cys.rs:7185` → `run_launch_agent_opts` → `cys.rs:5372`). 리포지토리 전체에서
  topology의 `entry["title"]`을 되읽는 코드는 없다(grep 0건). ⇒ 복원 한 번이면 rename 결과가
  사라진다. A안(주기 스위퍼)·C안(생성 지점)은 이 경로도 자동으로 덮는다.
  (같은 restore의 **좌석 내 재연결** 경로(`cys.rs:7161-7171`)는 surface를 새로 만들지 않으므로 제목이 보존된다.)
- **`surface.rename`에는 ACL 게이트가 없다**(`handlers.rs:1750-1766`). 같은 파일의
  `surface.close`(`handlers.rs:1767~`)는 신원·소유 게이트를 명시적으로 두는데 rename은 없고,
  디스패처 차원의 공통 게이트도 없다(`handlers.rs:975-1018`). 이번 목적에는 이점이지만
  **로컬 소켓에 닿는 아무 클라이언트나 임의 surface의 제목을 바꿀 수 있다**는 사실은 기록해 둔다.
  (제목은 배선에 안 쓰이므로 현재 영향도는 낮음 — 위 "소비처 2곳"이 전부.)

---

## 4. 박사님 규칙 3항(`번호 + worker1/2/3`)을 만족시키려면

규칙: ①번호를 제목에 ②「번호 + 역할특성(가급적 한 단어)」 ③특성 부여가 어려우면 「번호 + worker1/2/3」.

지금 가진 재료로 대부분 충족된다:

- **번호** — `surface_id`. `cys list`/`surface.list` 응답에 있고 데몬 내부에도 있다.
- **역할특성 한 단어** — 지금 제목의 `cwd basename`이 정확히 그 역할을 하고 있다
  (`channels`, `cys-terminal-src`, 과거 `eduscan`·`research`). 새로 만들 필요가 없다.
- **worker1/2/3 폴백** — 서수는 **이미 role에 있다**(`worker-3`). role을 **읽기만** 하면
  `worker-3` → `worker3` 으로 파생된다. **role 값 자체는 건드리지 않는다**(금지 조건 준수).

따라서 추가로 필요한 것은 **딱 하나, 결정 사항**이다:

> **「폴더명이 특성으로 쓸 만한가」의 판정 규칙을 누가 정하는가.**
> 예: 홈 디렉터리(`oogisoogi`)·루트·`Documents` 같은 범용 폴더명은 특성이 아니다 → role 폴백.
> 반대로 `channels`·`eduscan`·`research`는 그대로 특성.
> 이건 코드가 아니라 **기준**이라 master/박사님 승인 사항으로 남긴다.

부수적으로 정할 것 2개(둘 다 사소·권고안 병기):

- **구분자** — `297 · channels` 권고. UI의 `isAutoTitle` 정규식(`/^surface \d+$/`)에 안 걸리고,
  기존 제목의 ` · ` 관습과 일관된다.
- **agent 표기 유지 여부** — 현행 제목엔 `-claude`가 붙는다. 규칙2는 「번호 + 역할특성」이므로
  agent를 빼는 것이 규칙에 더 충실하다(`297 · channels`). 진단에 agent가 필요하면
  `297 · channels · claude` 도 가능하나 길어진다. **권고: 뺀다**(`cys list`에 이미 role이 별도 열로 있다).

---

## 5. master가 준 전제 재판정 (지시대로 전부 의심하고 쟀다)

| # | master 전제 | 판정 | 근거 |
|---|-------------|------|------|
| 1 | `cys actions` 전수 → 제목 변경 명령 없음 | **부분 참** — CLI 서브커맨드로는 참. 그러나 ①`cys new-surface --title` 이 존재하고(생성 시 지정) ②RPC `surface.rename` 이 존재하며 설치본에서 응답한다 | `cys.rs:44`, live `cys new-surface --help`, `handlers.rs:1750`, probe 실측 |
| 2 | `launch-agent --help` 옵션은 `--role`·`--agent`·`--cwd` 셋뿐 | **참**(+전역 `--socket`) | live `cys launch-agent --help` |
| 3 | 관측 제목 = `<role>-<agent> · <cwd basename>` (추정) | **참·확정** | `cys.rs:5307-5316` + live `cys list` 2건 |
| 4 | `spawn-worker.sh:462` 주석 「cys 사후 set-title 프리미티브 **부재(실측)**」 | **거짓** — 프리미티브는 있다. CLI 노출이 없을 뿐이다 | `handlers.rs:1750` + 설치본 probe |
| 5 | pack `*.py` grep + 바이너리 `strings`로 조합 지점 못 찾음 | **참이고, 못 찾은 이유가 규명됐다** | ①조합 지점이 pack이 아니라 Rust CLI라 pack grep은 원리적으로 못 찾는다 ②`strings`는 짧은 match arm 리터럴을 못 잡는다 — 실측: 설치본 `cysd`에서 `surface.rename`·`surface.resize` **strings 0건**인데 데몬은 둘 다 정상 디스패치한다(rustc가 16바이트 이하 문자열 비교를 즉값으로 최적화해 .rodata에 남지 않는다). ⇒ **`strings` 부재는 부재의 증거가 아니다** |
| 6 | 설치본은 자체 빌드본(adhoc 서명·mtime 일치) ⇒ 바이너리 수정 경로가 열려 있다 | **정황 일치**(confidence: Med) — 버전 동일(소스 `Cargo.toml` 0.13.20 = `cys --version` 0.13.20), 런타임 동작이 소스와 일치(rename 디스패치·제목 패턴). 바이트 동일성은 재빌드 없이 증명 불가라 여기서 확정하지 않는다 | `Cargo.toml:8`, `cys --version`, probe |

---

## 6. 권고 (결정은 master·박사님)

1. **먼저 A안(무재빌드 pack 스위퍼)으로 즉시 만족시킨다.** 이유: 재빌드·재설치 위험 0,
   배선 변경 0(role 무접촉), 커버리지 100%, 잡 하나 지우면 완전 원복. 오늘 규칙을 오늘 만족시킨다.
2. **항구적으로는 C안(데몬 `state.rs:2167`)으로 흡수한다.** 이유: 스위퍼는 「사후 교정」이라
   생성~첫 스윕 사이 짧은 공백이 있고, 잡이 죽으면 조용히 규칙이 깨진다. 생성 지점에서
   붙이면 공백도 감시 대상도 사라진다. 다음 정규 빌드에 태우면 된다.
3. **B안(CLI 사후 rename)은 권하지 않는다.** C안과 재빌드 비용이 같은데 커버리지가 좁고,
   복원 유실 문제(§3-2)를 못 막는다.
4. **선행 결정 1건**: §4의 「폴더명이 특성으로 쓸 만한가」 판정 기준. 이것만 정해지면
   A·C 어느 쪽이든 로직이 확정된다.

---

## 7. 이 조사에서 하지 않은 것 (정직 고지)

- **`surface.rename`을 실제로 실행해 제목이 바뀌는 것까지는 확인하지 않았다.** 브리프의
  「cys 상태를 바꾸는 명령 일체 금지」를 지켜, 파라미터를 비워 첫 가드에서 반환시키는
  무변경 probe로 **디스패치 존재까지만** 확인했다. 실제 rename 동작은 승인 후 검증 대상이다.
- **설치본 바이너리와 이 소스의 바이트 동일성**은 증명하지 않았다(재빌드 금지). 버전·런타임
  동작 일치까지만 확인했다(confidence: Med).
- **`cys pack-plan` 은 실행하지 않았다.** 제목 로직이 pack에 없다고 판정됐으므로 불필요했다.
  대신 읽기 전용으로 명시된 `cys pack-ownership`(`cys.rs:2249` 「결정론 조회 전용(쓰기 0)」)만
  써서 등급을 실측했다.
