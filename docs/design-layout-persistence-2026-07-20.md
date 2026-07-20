# 설계: 레이아웃 커스텀 영구화 + 명명 템플릿 (cys UI)

> 작성 2026-07-20 · cys-dev(node227) · 박사 07-20 확정 과제1 재설계
> 상태: **설계 초안 — codex(surface:228) 적대검토 대기 · 빌드 미착수**
> 방침: 빌드·설치·재시작 미실행(검증만) · 전달=소스 커밋(format-patch)

## 0. 한 줄 요약

박사가 드래그로 만든 페인 배치·비율을 **cys/데몬 재시작에도 복원**되도록 하고(현재는
데몬 재시작 시 붕괴), 저장 위치를 `~/.cys` 사용자 데이터로 옮기며, 화면구성 1/2/3
명명 템플릿(2단계 옵션)을 추가한다. **자동 4열 타일링(4bf8aa3)은 폐기가 아니라
"저장된 레이아웃이 없을 때의 초기 기본값"으로 강등한다.**

---

## 1. 현재 실측 (추정 아님 · 파일·라인)

| 기능 | 상태 | 근거 |
|---|---|---|
| 드래그로 분할 비율 조정 UI | **이미 존재** | `ui/src/main.ts:2055` `attachDividerDrag` — divider mousedown→ratio 계산→`node.ratio` 갱신→`saveLayout()` |
| 드래그로 페인 배치(split/이동) | **부분 존재** | Split→/↓ 버튼(`actionSplit`)·정렬(`actionEqualize`)로 위상 생성. 페인 자체를 드래그로 재배치하는 UI는 **없음**(divider는 크기만) |
| 레이아웃 저장 | **이미 존재** | `main.ts:1384` `saveLayout` — `workspaces`(트리 위상+ratio)를 localStorage `cys-layout-v2`에 직렬화 |
| 레이아웃 복원 | **이미 존재하나 sid 종속** | `main.ts:4095~` 시작 시 저장본 로드 → 살아있는 surface **sid 정확매칭** 대조 |
| persist 물리 위치 | localStorage = `~/Library/WebKit/com.cysjavis.terminal` | identifier 기반 WKWebView 저장소. pack-update와는 무관하게 보존되나 **identifier 종속** |
| 앱↔`~/.cys` 파일 R/W | **검증된 Tauri command 패턴 존재** | `src-tauri/src/main.rs:967` `read_board_catalog`, `:976` `read_profile_audience`(`~/.cys/profile.json`), `:446` `std::fs::write`. `cys::home_dir().join(".cys/...")` 관례 확립 |

**결론: 수동 드래그 크기조정 UI와 저장/복원 골격은 이미 있다. 신규 UI를 처음부터
만들 필요 없음.** 남은 것은 아래 3개 gap이다.

---

## 2. 진짜 Gap (재설계 대상)

### G1 — 데몬 재시작 시 배치·비율 붕괴 (★핵심)
복원(`main.ts:4188-4190`)은 저장 tree의 sid가 **살아있는 surface sid와 정확히 일치**할
때만 그 노드를 살린다. 불일치 sid는 `replaceNode(...,()=>null)`로 트리에서 제거되고,
레이아웃에 없는 surface는 `main.ts:4227-4231`에서 **무조건 `dir:"row"` 단순 일렬**로
append된다(ratio 기본 0.5).

- surface sid는 **데몬 생애 내에서만 안정**하다(데몬은 앱과 별개 상주 프로세스 —
  `cys list`로 surface:213/214/227/228 유지 확인). **앱만 재시작 → sid 유지 → 복원 OK.**
- 그러나 **데몬까지 재시작(`cys boot`·재부팅)하면 sid가 새로 발급** → 저장 tree의
  sid 전부 dead 판정 → **박사가 만든 배치·비율이 통째로 일렬로 붕괴**한다.
- 이것이 박사가 겪는 "재시작에도 복원" 실패의 근본이다.

### G2 — persist가 identifier 종속 localStorage
박사 요구 = `~/.cys` 사용자 데이터에 저장(pack-update 보존 대상). 현재는 WKWebView
localStorage(identifier `com.cysjavis.terminal` 종속). **과제2와 직접 충돌**: dev
오버레이가 identifier를 `.dev`로 바꾸면 dev/prod localStorage가 **다른 경로로 분리** →
박사가 한쪽에서 저장한 레이아웃이 다른 쪽에서 안 보인다. → **`~/.cys` 파일 persist가
이 분리를 원천 해소**한다(두 과제가 연결됨).

### G3 — 명명 템플릿 슬롯 1/2/3 없음 (2단계 옵션)
현재 저장 슬롯은 단일(현재 상태만). 명명된 화면구성 여러 개 저장·전환 불가.

---

## 3. 데이터 모델

### 3-1. 안정 재매핑 키 (G1 해결의 핵심)
트리 리프를 `sid`(휘발성) 대신 **역할 기반 안정 슬롯**으로 저장한다. 복원 시
살아있는 surface를 이 키로 재매핑한다.

```jsonc
// 저장 리프 (기존 {type:"pane", sid} 확장)
{ "type": "pane",
  "slot": { "role": "master", "ord": 0 } }   // role + 같은 role 내 순번
```

- 복원 매핑: 살아있는 surface들을 `(role, 등장순서 ord)`로 인덱싱 → 저장 슬롯과
  대조해 sid 부여. role 불명(일반 셸)은 `role:null, ord:n`로 순서만 매칭.
- 매칭 실패한 저장 슬롯 = 그 페인은 이번에 없음 → 트리에서 접기(기존 replaceNode 재사용).
- 매칭 안 된 살아있는 surface = 고아 → 기존 append 경로(G1의 일렬 붙이기)로 폴백.
- **하위호환**: 옛 저장본(`sid`만 있는 리프)은 그대로 sid 매칭(마이그레이션 불요).

> 대안 검토: sid 대신 title 매칭 = title은 사용자 rename·cwd로 흔들려 불안정 → 기각.
> role+ord가 데몬 재시작에도 가장 안정(역할은 launch-agent가 재부여).

### 3-2. `~/.cys` persist 파일
```
~/.cys/ui-layouts.json     # 신규 · pack tar가 푸는 경로와 무충돌(박사 경고 반영: agents.json/soul 회피)
```
```jsonc
{
  "version": 1,
  "current": { /* 현재 활성 레이아웃 스냅샷(기존 cys-layout-v2 페이로드 + slot 리프) */ },
  "templates": [                                  // G3 · 2단계
    { "id": 1, "name": "개발", "layout": { /* … */ } },
    { "id": 2, "name": "리뷰", "layout": { /* … */ } }
  ]
}
```

- **마이그레이션**: 최초 1회 localStorage `cys-layout-v2` → `ui-layouts.json.current` 승격.
  이후 localStorage는 캐시로만(또는 완전 이전). 손상·부재 시 자동 4열(4bf8aa3)로 폴백.
- **쓰기 시점**: `saveLayout()` 말미에 디바운스(예 500ms)로 파일 flush — 드래그 연속
  이벤트마다 디스크 쓰기 폭주 방지.

---

## 4. Persist 배선 (앱↔파일)

기존 검증 패턴(`read_profile_audience`) 그대로 Tauri command 2개 신설:

```rust
#[tauri::command] fn read_ui_layouts() -> Result<Value,String>            // ~/.cys/ui-layouts.json 읽기(없으면 null)
#[tauri::command] fn write_ui_layouts(data: Value) -> Result<(),String>   // 원자적 쓰기(temp→rename)
```
- `cys::home_dir().join(".cys/ui-layouts.json")` · `std::fs::write`는 이미 쓰는 API.
- 원자적 쓰기(temp+rename)로 flush 중 크래시에도 파일 반쪽 손상 방지.
- `invoke_handler`(main.rs:2075) 목록에 2개 등록.

---

## 5. UI 상호작용

### 5-1. 자동 4열 강등 (즉효·저위험)
`actionEqualize`(정렬 버튼)는 **명시적 사용자 액션으로 유지**(그대로 둠). 변경점은
**초기 부팅 폴백**뿐: 저장 레이아웃이 없을 때만 자동 4열 적용(현재는 append 일렬).

### 5-2. 명명 템플릿 (G3 · 2단계 옵션)
- 최소 UI: 상단바 "화면구성 ▾" 드롭다운 = [현재 저장] · [슬롯1 개발] · [슬롯2 리뷰] · [+ 새 구성].
- 전환 = 그 슬롯 layout으로 `ws.tree` 교체 후 render(기존 정렬과 동일 경로).
- 저장 = 현재 tree를 slot 리프로 직렬화해 templates에 upsert.
- **범위 게이트**: 1단계(G1+G2 복원 견고화)를 먼저 독립 커밋. 2단계(템플릿)는 박사
  승인 후 별도 커밋 — 1단계만으로도 박사 핵심 요구("재시작 복원")가 충족되므로.

---

## 6. 단계·커밋 계획 (전부 소스 커밋 · 빌드 미실행)

| 단계 | 내용 | 위험 | 커밋 |
|---|---|---|---|
| 1a | slot 리프 데이터모델 + 복원 재매핑(순수함수·reorder.ts 스타일 단위테스트) | 중(복원 회귀 위험 — 테스트로 방어) | fix(ui): 데몬 재시작에도 레이아웃 복원 |
| 1b | `~/.cys/ui-layouts.json` persist(Rust command 2 + 마이그레이션) | 중 | feat: 레이아웃 ~/.cys 영구화 |
| 1c | 자동 4열 = 초기 폴백으로 강등 | 저 | (1a에 포함 가능) |
| 2 | 명명 템플릿 슬롯 1/2/3 UI | 저 | feat: 화면구성 명명 템플릿(박사 승인 후) |

---

## 7. 미해결·검토 요청 포인트 (codex에게)

1. **G1 재매핑 키**: role+ord가 데몬 재시작 후 안정적인가? launch-agent 재기동이 역할을
   같은 순서로 재부여하는 보장이 있나, 아니면 순서가 흔들려 페인이 뒤섞일 위험은?
2. **부분 매칭 정책**: 저장 슬롯 5개 중 3개만 살아 돌아올 때 — 나머지 2칸을 접고 3개로
   재배치할지, 빈 셸을 생성해 슬롯 보존할지. 박사 UX 기대치와 정합?
3. **디바운스 flush vs 즉시**: 드래그 종료(mouseup)에서만 파일 쓰면 디바운스 불요.
   연속 쓰기 지점이 실제로 있나(현 saveLayout 호출부 감사 필요)?
4. **localStorage↔파일 이중 진실원**: 마이그레이션 후 localStorage를 버릴지 캐시로 둘지.
   이중화 시 동기화 버그 위험 대비 단일화 이득?
5. **과제2 상호작용**: identifier 분리 시 기존 localStorage 레이아웃이 dev에서 안 보이는데,
   ~/.cys 이전 전 과도기 UX(첫 dev 실행이 빈 화면)를 어떻게 매끄럽게?
