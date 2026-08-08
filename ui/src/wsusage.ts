// 사이드바 사용량 패널의 순수 계산 — rate 집계 + 페인별 CTX 목록 + 신선도 판정.
//
// 오너 요청 2026-08-07: "주간 토큰 사용량과 페인별 CTX를 한 곳에 모아 보고 싶다"(페인 푸터 대신
// 사이드바 빈 공간). main.ts의 renderSidebarUsage는 여기 결과를 DOM으로 옮기기만 한다 —
// 집계·정렬·신선도 판정은 DOM 없이 회귀 테스트할 수 있어야 하므로 전부 이 모듈에 둔다
// (ccscale.ts·wsbar.ts와 같은 분리 관례).

// 관측치가 낡았다고 보는 경계. renderUsage(pane 배지)와 **같은 값을 써야 한다** —
// 두 표면이 다른 문턱을 쓰면 같은 페인이 한쪽에선 stale, 한쪽에선 정상으로 보인다.
export const USAGE_STALE_SECS = 120;

export interface RateWindowLike {
  label: string;
  used_pct: number;
  resets_at: number | null;
}
export interface UsageLike {
  agent: string;
  ctx_pct: number | null;
  rate: RateWindowLike[];
  source: string;
  updated_at: number;
}
export interface SurfaceLike {
  surface_id: number;
  // 소속 데몬 소켓. "" = 기본(base) 데몬. 부서 데몬은 각자 다른 소켓이고 **계정도 다를 수 있다**.
  socket: string;
  exited: boolean;
  // UI에 pane이 붙어 있는가. CTX 목록은 화면의 페인 번호와 대조하는 표라 이 범위로 좁힌다.
  adopted: boolean;
  usage?: UsageLike | null;
}

// 표시 순서 — 여기 없는 라벨은 이 뒤에 데이터 등장 순으로 붙는다.
//
// ★모델 스코프 게이지(「7d·Fable」)는 **여기에 넣지 않는다**(티켓⑤ 2026-08-07). 초판 주석은
// 「fable5 확장점 = 이 배열에 라벨 한 줄 추가」였는데, 실제 API를 열어 보니 라벨의 모델 이름이
// 응답(`scope.model.display_name`)에서 온다 — 상수로 박으면 스코프가 다른 모델로 옮겨 간 날
// 남의 게이지에 옛 이름이 붙는다. 미등재 라벨은 labelRank가 5h·7d 뒤로 보내므로 순서도 맞다.
// 없는 값을 미리 0%로 그려 두지 않는 이유는 그대로다: 빈 게이지는 「안 썼다」로 읽히지
// 「데이터가 없다」로 읽히지 않는다.
export const RATE_LABEL_ORDER = ["5h", "7d"];

// 사용량 절에서 **표시하지 않는** 에이전트 (오너 판정 2026-08-07 · master#05bec9).
//
// ★왜 지우는가: codex rate의 원천은 rollout 파일 관측치뿐이라 **codex가 도는 동안에만** 갱신된다.
// 오너 실측에서 화면값(52%)이 ChatGPT 설정의 실물(55%)과 어긋났고, 실시간 조회 API가 없어
// 좁힐 방법이 없다. ⇒ 낡은 게이지는 없느니만 못하다(사용자는 그것을 현재값으로 읽는다).
// ★수집은 끊지 않는다 — note_rate·스냅샷·소진 경보는 그대로 돈다(내부 운영용). 여기서 자르는 것은
//   **화면에 내보내는 마지막 한 걸음**뿐이다. 실시간 원천이 생기면 이 집합에서 빼면 된다.
export const RATE_HIDDEN_AGENTS = new Set(["codex"]);

export interface RateRow {
  socket: string;
  agent: string;
  // ★계정 식별자 — 계정 저장소(usage.accounts) 유래 행에만 있다. "" = surface 관측 유래(계정 미상).
  // rate limit이 걸리는 진짜 경계는 소켓도 에이전트도 아니고 **계정**이므로, 경계 혼합 금지
  // 규율(codex 1R [High])의 키를 여기까지 넓힌다. 같은 claude라도 계정이 둘이면 두 줄이다.
  accountId: string;
  // 사람이 읽는 계정 이름(이메일 등). 범위 머리표에 쓴다 — 없으면 에이전트 이름만 나온다.
  accountLabel: string;
  label: string;
  usedPct: number;
  // ★usedPct를 준 **바로 그 관측**의 리셋 시각. 다른 관측에서 가져오면 %와 시각의 짝이 깨진다.
  resetsAt: number | null;
  ageSecs: number;
  // ★관측 시각 원본. 나이는 그릴 때마다 다시 계산해야 하므로(재생성 없이 갱신) 원본을 들고 다닌다.
  updatedAt: number;
  stale: boolean;
}

// ★키 구분자는 JSON 인코딩으로 만든다. 초판은 NUL 제어문자를 소스에 직접 넣었는데,
// 그 때문에 git이 이 파일을 **바이너리로 판정**해 패치에서 통째로 빠졌다
// (codex [Blocker]의 숨은 2차 원인). JSON.stringify는 제어문자 없이 충돌 없는 키를 만든다.
const scopeKey = (socket: string, agent: string) => JSON.stringify([socket, agent]);

// rate 집계 — ★계정 경계를 넘어 섞지 않는다.
//
// 결함 이력(codex 이종검증 2026-08-07 [High]): 초판은 label만 키로 최댓값을 취해서
// Claude 5h=80%와 codex 5h=5%가 하나의 「5h 80%」로 합성됐다. 합산도 아니고 계정별 표시도 아닌,
// **출처를 알 수 없는 값**이었다. rate limit은 (데몬 소켓 × 에이전트)마다 다른 계정에 걸리므로
// 그 조합을 키로 삼아 따로 집계한다.
//
// 신선도 규율(같은 검증 [High]): updated_at을 안 보면 낡은 95%가 최신 10%를 영원히 이긴다.
// ⇒ **신선한 관측이 하나라도 있으면 신선한 것들 중에서만** 최댓값을 고른다. 전부 낡았을 때만
// 낡은 값을 쓰고 stale로 표시한다(데이터를 버리지 않되 거짓 최신으로 보이지도 않게).
export function aggregateRates(surfaces: SurfaceLike[], nowSecs: number): RateRow[] {
  // key → { fresh: 후보, stale: 후보 }
  const best = new Map<string, { fresh: RateRow | null; stale: RateRow | null }>();
  for (const s of surfaces) {
    if (s.exited || !s.usage) continue; // 종료 페인의 마지막 관측은 계정 현황이 아니다
    const u = s.usage;
    const age = Math.max(0, Math.round(nowSecs - u.updated_at));
    const isStale = age > USAGE_STALE_SECS;
    const agent = u.agent || "?";
    for (const w of u.rate ?? []) {
      const used = Number(w.used_pct);
      if (!Number.isFinite(used)) continue;
      const k = JSON.stringify([scopeKey(s.socket, agent), w.label]);
      const cand: RateRow = {
        socket: s.socket,
        agent,
        // surface 관측은 어느 계정에 걸린 한도인지 스스로 모른다 — 계정 귀속은 데몬(accounts.rs
        // resolve)이 세션 파일 경로로 판정한다. 여기서 추측해 채우면 없는 신원을 지어내는 것이다.
        accountId: "",
        accountLabel: "",
        label: w.label,
        usedPct: used,
        resetsAt: w.resets_at ?? null, // ★같은 관측에서 함께 가져온다(짝 유지)
        ageSecs: age,
        updatedAt: u.updated_at,
        stale: isStale,
      };
      const slot = best.get(k) ?? { fresh: null, stale: null };
      const side = isStale ? "stale" : "fresh";
      const cur = slot[side];
      // 동률이면 더 신선한 쪽을 남긴다 — 같은 수치면 최근 관측이 더 믿을 만하다.
      if (!cur || used > cur.usedPct || (used === cur.usedPct && age < cur.ageSecs)) slot[side] = cand;
      best.set(k, slot);
    }
  }
  const rows: RateRow[] = [];
  for (const slot of best.values()) rows.push((slot.fresh ?? slot.stale) as RateRow);
  return sortRates(rows);
}

// 정렬: 소켓 → 에이전트 → 계정 → 라벨 순서(RATE_LABEL_ORDER 우선, 나머지는 사전순).
// ★계정을 정렬 키에 넣는 이유: 같은 에이전트에 계정이 둘이면 두 계정의 5h·7d가 번갈아 나와
// 머리표가 매 줄 바뀐다. 계정으로 묶어야 「계정 하나 = 연속된 블록」이 된다.
const labelRank = (l: string) => {
  const i = RATE_LABEL_ORDER.indexOf(l);
  return i < 0 ? RATE_LABEL_ORDER.length : i;
};
function sortRates(rows: RateRow[]): RateRow[] {
  rows.sort(
    (a, b) =>
      a.socket.localeCompare(b.socket) ||
      a.agent.localeCompare(b.agent) ||
      a.accountId.localeCompare(b.accountId) ||
      labelRank(a.label) - labelRank(b.label) ||
      a.label.localeCompare(b.label),
  );
  return rows;
}

// ── 계정 저장소 유래 rate (오너 육안 판정 2026-08-07 09:27 수리)
//
// ★결함 계보: 초판은 사이드바 「사용량」을 **살아 있는 surface의 usage만** 집계해 만들었다.
// 그래서 페인이 0이면 rates가 0이 되어 절 전체가 사라졌다 — 오너 육안에서 「계정 사용량이
// 안 보인다」로 드러난 결함이다. 그러나 5h·7d는 **페인의 속성이 아니라 계정의 속성**이다.
// 페인이 없어도 계정의 한도는 그대로 소진돼 있다. ⇒ 원천을 계정 저장소로 옮긴다.
//
// ★왜 계정 저장소가 상위 집합인가(실측 근거): 페인 관측은 데몬이 usage.rs에서
// `accounts::note_rate`로 계정에 귀속시킨다(usage.rs:328 codex rollout · usage.rs:1169 gemini ·
// handlers.rs:3400 statusline). 즉 계정 저장소는 같은 관측을 받아 **페인이 죽어도 들고 있는** 곳이다.
// 그러므로 계정 유래 행은 surface 유래 행을 대체할 수 있다(정보 손실 없음).
// 모델 스코프 주간 게이지 — 데몬 OAuth 프로브 유래(accounts.rs ScopedGauge).
// ★계정의 updated_at과 **별개의 자기 시각**을 들고 온다: 이 게이지의 생산자는 프로브 하나뿐이고
//   statusline이 rate를 갱신해도 이 값은 그때 관측된 게 아니다. 계정 시각을 물려 쓰면 낡은
//   게이지가 매번 「방금 관측」으로 둔갑한다.
export interface ScopedGaugeLike {
  model: string; // API가 준 표시 이름("Fable") — 우리가 짓지 않는다
  used_pct: number;
  resets_at: number | null;
  updated_at: number;
  source: string;
}

export interface AccountLike {
  provider: string;
  account_id: string;
  label: string;
  rate: RateWindowLike[];
  // 관측이 한 번도 없으면 null이다(계정은 등록됐으나 아직 못 봤다).
  updated_at: number | null;
  scoped?: ScopedGaugeLike[] | null;
}

export function accountRates(accounts: AccountLike[] | null | undefined, nowSecs: number): RateRow[] {
  const rows: RateRow[] = [];
  for (const a of accounts ?? []) {
    if (!a) continue;
    const updatedAt = Number(a.updated_at);
    // ★관측 시각이 없으면 그리지 않는다. 계정은 등록됐지만 아직 아무것도 못 본 상태이고,
    //   이때 rate는 빈 배열이다. 나이를 0으로 채워 「방금 관측」처럼 보이게 만들면 거짓 신선이다.
    if (!a.updated_at || !Number.isFinite(updatedAt) || updatedAt <= 0) continue;
    const age = Math.max(0, Math.round(nowSecs - updatedAt));
    const agent = a.provider || "?";
    for (const w of a.rate ?? []) {
      const used = Number(w?.used_pct);
      if (!Number.isFinite(used)) continue;
      rows.push({
        // 계정 저장소는 부서 데몬까지 병합한 뷰라 특정 소켓에 속하지 않는다.
        // ★""를 쓰는 것은 「기본 데몬」이라는 뜻이 아니라 「소켓 축이 이 행에 없다」는 뜻이다 —
        //   계정 경계가 소켓 경계보다 넓기 때문이다(같은 계정을 여러 데몬이 함께 쓴다).
        socket: "",
        agent,
        accountId: a.account_id || "",
        accountLabel: a.label || "",
        label: w.label,
        usedPct: used,
        resetsAt: w.resets_at ?? null, // ★같은 관측에서 함께 가져온다(짝 유지)
        ageSecs: age,
        updatedAt,
        stale: age > USAGE_STALE_SECS,
      });
    }
  }
  return sortRates(rows);
}

// 모델 스코프 게이지 → rate 행 (티켓⑤ · 오너 승인 2026-08-07).
//
// ★이 값은 서버가 준 **한도 대비 소진율**이다(OAuth usage API 원천) — 그래서 게이지로 그린다.
// 초판에는 이것과 나란히 「자체 집계」 줄(우리가 트랜스크립트를 세어 만든 관측 절대치)이 있었고
// 주석은 「두 줄은 보완 관계라 함께 남긴다」였다. **오너 판정으로 그 줄은 티켓⑥에서 삭제됐다**
// (2026-08-07 육안: 「불확실한 것이니 삭제」 — 한도를 모르는 절대치는 옆의 게이지와 같은 눈금으로
// 읽혀 오독을 만든다). 남는 것은 이 게이지 하나이므로 여기서 「다른 종류의 수」를 구별할 필요가 없다.
//
// 라벨 `7d·<모델>`: 기간이 주간이라 7d와 같은 축이고, 뒤에 모델 이름을 붙여 「전체 주간」과 구별한다.
// ★모델 이름은 응답이 준 것을 그대로 쓴다(RATE_LABEL_ORDER 주석 참조 — 상수 금지).
export function scopedRates(accounts: AccountLike[] | null | undefined, nowSecs: number): RateRow[] {
  const rows: RateRow[] = [];
  for (const a of accounts ?? []) {
    if (!a) continue;
    for (const g of a.scoped ?? []) {
      if (!g || typeof g.model !== "string" || !g.model) continue; // 이름 없는 게이지는 만들지 않는다
      const used = Number(g.used_pct);
      const updatedAt = Number(g.updated_at);
      if (!Number.isFinite(used)) continue;
      // accountRates와 **같은 규율**: 관측 시각이 없으면 그리지 않는다(나이 0 = 거짓 신선).
      if (!g.updated_at || !Number.isFinite(updatedAt) || updatedAt <= 0) continue;
      const age = Math.max(0, Math.round(nowSecs - updatedAt));
      rows.push({
        socket: "",
        agent: a.provider || "?",
        accountId: a.account_id || "",
        accountLabel: a.label || "",
        label: `7d·${g.model}`,
        usedPct: used,
        resetsAt: g.resets_at ?? null,
        ageSecs: age,
        updatedAt,
        stale: age > USAGE_STALE_SECS,
      });
    }
  }
  return sortRates(rows);
}

// 화면에 내보내기 직전 걸러내기 — 실시간 원천이 없는 에이전트의 행을 뺀다(RATE_HIDDEN_AGENTS 주석).
//
// ★**병합이 끝난 뒤에** 걸어야 한다. 계정 유래 행만 지우면 같은 codex의 surface 관측 행이
// mergeRates의 중복 방지를 빠져나와 그대로 올라온다(덮개가 사라지면 밑에 있던 것이 드러난다).
// 그래서 이 함수의 입력은 언제나 「최종 목록」이다.
export function filterDisplayRates(rows: RateRow[]): RateRow[] {
  return rows.filter((r) => !RATE_HIDDEN_AGENTS.has(r.agent));
}

// 계정 유래 + surface 유래 병합 — **계정이 이긴다.**
//
// ★중복 금지가 이 함수의 존재 이유다. 두 원천은 같은 관측을 담고 있으므로 그냥 이어 붙이면
// 「claude 5h」가 두 줄 나온다. 사용자는 그것을 두 계정으로 읽는다 — 없는 계정을 만들어 보이는 것이다.
// ⇒ (에이전트 × 창 라벨)이 계정 쪽에 이미 있으면 surface 행은 버린다.
//
// ★키에 소켓을 넣지 않는 이유: 계정 행에는 소켓 축이 없다(위 주석). 소켓까지 키로 삼으면
// 계정 행과 surface 행이 영원히 안 만나 중복이 그대로 남는다.
export function mergeRates(accountRows: RateRow[], surfaceRows: RateRow[]): RateRow[] {
  const covered = new Set(accountRows.map((r) => JSON.stringify([r.agent, r.label])));
  const merged = accountRows.slice();
  for (const r of surfaceRows) {
    if (covered.has(JSON.stringify([r.agent, r.label]))) continue;
    merged.push(r);
  }
  return sortRates(merged);
}

export interface CtxRow {
  surfaceId: number;
  // 이름 있는 보고자면 그 이름(master·cso). "" = 번호 페인.
  // ★번호와 이름을 한 필드에 섞지 않는 이유: 정렬이 다르다(이름 먼저, 그 다음 번호 오름차순)
  //   ─ 하나로 합치면 "master"와 372를 같은 축으로 비교해야 하고 그 비교는 뜻이 없다.
  name: string;
  socket: string;
  // null = 아직 관측치가 없다. ★행을 지우지 않고 null로 남기는 이유는 아래 paneCtxRows 주석 참조.
  ctxPct: number | null;
  ageSecs: number;
  updatedAt: number;
  stale: boolean;
  source: string;
}

// 페인별 CTX — 오너 발주 3번 「CTX = 페인 번호 + %」.
//
// 범위(codex [Medium] 수리): **UI에 붙은 pane만** 낸다. 이 표의 용도는 화면의 페인 번호와
// 대조하는 것인데, 화면에 없는 surface(미입양·headless)를 「페인 1」로 적으면 대조할 대상이 없다.
// 계정 사용량(aggregateRates)은 반대로 전 surface를 봐야 참이므로 두 범위를 일부러 다르게 둔다.
//
// ★관측이 없는 pane을 조용히 빼지 않는다(같은 검증의 지적): 빼 버리면 사용자가 「미관측」과
// 「그런 pane 없음」을 구별할 수 없다. ctxPct=null로 남겨 화면에 「—」로 표시한다.
// (ctx_pct=0은 정상 값이므로 그대로 보존된다 — 0과 null은 다른 것이다.)
//
// 정렬은 소켓 → 페인 번호 오름차순이다(사용률 순 아님). 순서가 값에 따라 흔들리면 눈으로 못 따라간다.
export function paneCtxRows(surfaces: SurfaceLike[], nowSecs: number): CtxRow[] {
  const rows: CtxRow[] = [];
  for (const s of surfaces) {
    if (s.exited || !s.adopted) continue;
    const u = s.usage;
    if (!u) {
      rows.push({ surfaceId: s.surface_id, name: "", socket: s.socket, ctxPct: null, ageSecs: 0, updatedAt: 0, stale: false, source: "" });
      continue;
    }
    const raw = u.ctx_pct;
    const pct = raw == null || !Number.isFinite(Number(raw)) ? null : Number(raw);
    const age = Math.max(0, Math.round(nowSecs - u.updated_at));
    rows.push({
      surfaceId: s.surface_id,
      name: "",
      socket: s.socket,
      ctxPct: pct,
      ageSecs: pct == null ? 0 : age,
      updatedAt: pct == null ? 0 : u.updated_at,
      stale: pct == null ? false : age > USAGE_STALE_SECS,
      source: pct == null ? "" : (u.source ?? ""),
    });
  }
  return sortCtxRows(rows);
}

// 이름 있는 보고자의 **역할 서열** — 이 배열의 자리가 곧 랭크다(티켓⑥-b · 오너 2026-08-07).
//
// ★왜 사전순으로는 안 되는가: 초판은 「이름 행 먼저 + 이름 사전순」이었는데, 사전순에서는
//   c < m 이라 **cso가 master보다 위로 온다**. 화면의 서열이 조직의 서열과 거꾸로 보였다.
//   순서가 뜻을 갖는 목록에서 정렬 키를 글자에 맡기면, 이름을 바꾸는 순간 뜻이 뒤집힌다.
// ★상수로 박아도 되는 이유(RATE_LABEL_ORDER와 반대인 판단): rate 라벨의 모델 이름은 **응답이**
//   정하므로 상수로 박으면 안 됐다. 이 이름들은 우리가 정한다(named.rs DEFAULT_MAP) — 서열도
//   우리 조직의 사실이므로 코드가 지는 것이 맞다. 미등재 이름은 이 뒤·번호 페인 앞에 온다.
export const CTX_NAME_ORDER = ["master", "cso"];

// 행 → 정렬 랭크. master=0 · cso=1 · 그 밖의 이름 보고자=2 · 번호 페인=3.
// ★대소문자를 무시한다: 이름은 env(CYS_NAMED_REPORTERS)로 재정의될 수 있어 「Master」가 올 수
//   있고, 그때 서열이 조용히 무너지면 원인을 찾기 어렵다(정렬은 틀려도 에러가 나지 않는다).
const ctxNameRank = (row: CtxRow): number => {
  if (!row.name) return CTX_NAME_ORDER.length + 1; // 번호 페인은 언제나 맨 뒤
  const i = CTX_NAME_ORDER.indexOf(row.name.toLowerCase());
  return i < 0 ? CTX_NAME_ORDER.length : i;
};

// 정렬: **역할 서열(master → cso → 그 밖의 이름) 먼저**, 그 다음 소켓 → 페인 번호 오름차순.
// ★이름 행을 위에 두는 이유: master·cso는 상시 존재하는 고정 항목이고 번호 페인은 오가는 것이다.
//   고정 항목이 변동 항목 사이에 끼면 볼 때마다 자리가 달라져 눈이 못 따라간다.
// ★같은 랭크 안에서는 종전 규칙 그대로다(이름 사전순 → 소켓 → 번호 오름차순) — 랭크가 같다는 것은
//   서열이 정해지지 않았다는 뜻이고, 그때는 값에 흔들리지 않는 순서(글자·번호)가 맞다.
function sortCtxRows(rows: CtxRow[]): CtxRow[] {
  rows.sort(
    (a, b) =>
      ctxNameRank(a) - ctxNameRank(b) ||
      a.name.localeCompare(b.name) ||
      a.socket.localeCompare(b.socket) ||
      a.surfaceId - b.surfaceId,
  );
  return rows;
}

// 이름 있는 보고자(master·cso 등 surface 없는 Claude) → CTX 행.
//
// ★이들은 cys surface가 아니라 cmux 페인의 Claude다. 그래서 번호가 없고, 번호를 지어내면
// 화면의 페인 번호와 대조할 대상이 없는 가짜 번호가 된다 ⇒ 라벨을 이름으로 둔다.
// 신선도·stale·출처 등급은 번호 행과 **같은 규율**을 쓴다(오너 지시) — 한 표 안에서 판정 기준이
// 갈리면 같은 색이 두 뜻을 갖는다.
export interface NamedReporterLike {
  name: string;
  ctx_pct: number | null;
  source: string;
  updated_at: number;
}

export function namedCtxRows(named: NamedReporterLike[] | null | undefined, nowSecs: number): CtxRow[] {
  const rows: CtxRow[] = [];
  for (const n of named ?? []) {
    if (!n || !n.name) continue; // 이름 없는 행은 만들지 않는다(지어낸 라벨 금지)
    const updatedAt = Number(n.updated_at);
    if (!n.updated_at || !Number.isFinite(updatedAt) || updatedAt <= 0) continue;
    const raw = n.ctx_pct;
    const pct = raw == null || !Number.isFinite(Number(raw)) ? null : Number(raw);
    // ★관측 시각은 있는데 ctx가 없는 경우 — 번호 행과 같이 「—」로 남긴다(행을 지우지 않는다).
    const age = Math.max(0, Math.round(nowSecs - updatedAt));
    rows.push({
      surfaceId: 0, // 번호 없음 — 이 행의 신원은 name이다
      name: n.name,
      socket: "",
      ctxPct: pct,
      ageSecs: pct == null ? 0 : age,
      updatedAt: pct == null ? 0 : updatedAt,
      stale: pct == null ? false : age > USAGE_STALE_SECS,
      source: pct == null ? "" : (n.source ?? ""),
    });
  }
  return sortCtxRows(rows);
}

// 두 원천을 합쳐 한 표로 — 이름 행이 위, 번호 행이 아래.
export function mergeCtxRows(namedRows: CtxRow[], paneRows: CtxRow[]): CtxRow[] {
  return sortCtxRows([...namedRows, ...paneRows]);
}

// 소켓 → 사람이 읽는 짧은 부서 이름.
//
// ★규칙의 정본은 Rust 쪽 is_dept_socket(src/lib.rs)이다: **경로 컴포넌트 중 `cys-dept-` 접두가
// 있으면 부서 데몬**. 여기서도 같은 규칙을 쓴다(윈도우 파이프 `\\.\pipe\cys-dept-<name>`도 같은 형태).
//
// 결함 이력(codex 2R [신규]): 초판은 경로의 **마지막 조각(파일명)**을 썼는데, 실제 부서 소켓은
// `~/.local/state/cys-dept-<name>/cys.sock`이라 파일명이 전부 `cys.sock`이다.
// ⇒ 부서가 둘이면 둘 다 「cys」가 되어 SID 충돌을 못 막는 것을 **그대로 재현**했다.
// 이름은 파일명이 아니라 **부모 디렉터리**에 있다.
export function shortSocketTag(socket: string): string {
  if (!socket) return ""; // 기본(base) 데몬 — 접두 없이 번호만 보인다
  const comps = socket.split(/[/\\]/).filter(Boolean);
  const dept = comps.find((c) => c.startsWith("cys-dept-"));
  if (dept) return dept.slice("cys-dept-".length);
  // 부서가 아니면 메인 데몬이다 — 접두 없이 번호만 보인다.
  //
  // ⛔여기에 「커스텀 경로 폴백」을 두지 않는다. 한 번 넣었다가 윈도우 파이프
  // `\\.\pipe\cys`에서 부모 조각인 "pipe"를 부서 이름처럼 뱉었다 —
  // ★이 시스템이 만들지 않는 경로를 위해 규칙을 지어내면, 그 지어낸 규칙이 실제 경로를 오독한다.
  // 소켓 경로는 전부 dept_socket_path/기본 슬러그가 만들므로 이 둘 말고는 없다(src/lib.rs).
  return "";
}

// 여러 소켓이 섞여 있는가 — 접두 표기 여부의 단일 판정.
export function hasMultipleSockets(items: { socket: string }[]): boolean {
  return new Set(items.map((i) => i.socket)).size > 1;
}

// ── Fable 「자체 집계」 줄은 여기 있었다 — 티켓⑥(오너 육안 2026-08-07)에서 **삭제**했다.
//
// 무엇이 있었나: `fableObserved` · `fableFromAnalytics` · `compactTokens` · `ByModelRow` ·
// `FableObserved`. control.analytics의 by_model 집계를 60초마다 읽어 「Fable 1936M tok ·
// 전체의 32% (자체 집계)」 한 줄을 사이드바에 그렸다.
//
// ★왜 지웠나(오너 판정 원문): 「불확실한 것이니 삭제」. 이 수는 **한도 대비가 아니라 관측 절대치**라
// 한도를 모른다. 형태(게이지 금지·「자체 집계」 태그)로 구별하려 했지만, 한도 게이지 바로 아래
// 나란히 놓인 이상 사용자는 같은 눈금으로 읽는다. ⇒ 같은 자리에 종류가 다른 수를 두 개 두면
// **라벨로는 구별이 안 된다**는 것이 이 줄의 교훈이다.
//
// ★대체물은 이미 있다: 「7d·Fable」 실게이지(scopedRates — OAuth usage API가 준 한도 대비 소진율).
// 그러므로 이 삭제는 정보 손실이 아니라 **불확실한 수를 확실한 수로 갈아 끼운 것**이다.
//
// ⚠데몬의 by_model 집계 자체는 건드리지 않았다 — 다른 소비자(renderEfficiency)가 쓴다.
// 끊은 것은 **화면으로 나가는 마지막 한 걸음**뿐이다(RATE_HIDDEN_AGENTS와 같은 규율).

// 임계 단계 — CTX는 60%(/clear 사이클 임계)·80%, rate는 70%·90%. renderUsage와 동일 기준.
export const sevClassFor = (pct: number, warn: number, crit: number): string =>
  pct >= crit ? "crit" : pct >= warn ? "warn" : "";

// 관측 나이 → 사람이 읽는 짧은 표기. ⓓ 요구(신선도 병기)의 표시 단위.
export function ageText(ageSecs: number): string {
  if (!Number.isFinite(ageSecs) || ageSecs < 0) return "?";
  if (ageSecs < 60) return `${ageSecs}초 전`;
  if (ageSecs < 3600) return `${Math.round(ageSecs / 60)}분 전`;
  return `${Math.round(ageSecs / 3600)}시간 전`;
}

// 행 안에 붙는 나이 — 열 폭이 좁아 「전」 접미를 뺀다(푸터는 문장이라 그대로 둔다).
// ★구간 판정(초/분/시간)은 ageText를 그대로 재사용한다 — 여기서 다시 나누면 같은 나이가
//   행과 푸터에서 다른 단위로 찍히는 날이 온다(문턱이 두 곳에 생기면 반드시 갈라진다).
export function ageShort(ageSecs: number): string {
  const t = ageText(ageSecs);
  return t === "?" ? t : t.replace(/ 전$/, "");
}

// 패널 푸터 문구 — 「가장 낡음」이 이 값의 정의다(관측된 행들 중 **가장 오래된** 관측의 나이).
// ★초판 문구는 「갱신 N분 전」이었다. 그 말은 패널 전체가 그때 멈춘 것처럼 읽혀서, 방금 관측된
//   행까지 낡아 보였다(오너 실문의 2026-08-08 「갱신 31분 전이 뭐냐」). 이름을 값의 정의로 바꾼다.
// ★문자열을 main.ts가 아니라 여기 두는 이유: 라벨이 테스트 밖에 있으면 오독을 고친 문구가
//   조용히 되돌아가도 아무 그물에도 안 걸린다.
export function oldestFootText(ageSecs: number): string {
  return `가장 낡음 ${ageText(ageSecs)}`;
}

// 출처 등급 — ⓓ 요구(출처 등급 병기).
// "statusline" = Claude가 보고한 서버 진실. 그 외(transcript/rollout tail)는 우리가 파일을 읽어
// 추정한 값이다. ★두 값을 같은 눈금으로 나란히 두면 추정치를 실측으로 오독하므로 등급을 표시한다.
export function sourceGrade(source: string): { mark: string; title: string } {
  if (source === "statusline") return { mark: "●", title: "서버 진실 (statusline 보고)" };
  if (!source) return { mark: "?", title: "출처 불명" };
  const heuristic = source.includes("heuristic");
  return {
    mark: "○",
    title: heuristic
      ? `추정 (${source} — 트랜스크립트 tail 휴리스틱)`
      : `추정 (${source} — 트랜스크립트 tail)`,
  };
}

// ── 렌더 서명(codex [Medium] 수리 · 2R 재수리) — 값이 안 변하면 DOM을 다시 만들지 않는다.
//
// ★서명에 **나이를 넣으면 안 된다**(codex 2R [미해소]). 초판은 ageText를 넣었는데 60초 미만
// 구간에서는 「37초 전 → 40초 전」처럼 매 틱 문자열이 달라져 서명이 항상 불일치했다 —
// 스킵 장치가 있으나 마나였다(있는데 안 도는 것이 없는 것보다 나쁘다: 있다고 믿게 만든다).
//
// ⇒ 서명에는 **구조와 값, 그리고 stale 경계 전이만** 넣는다. 나이 텍스트는 DOM을 다시 만들지 않고
// 해당 노드만 갱신한다(main.ts의 나이 갱신 클로저). 경계를 넘는 순간(fresh↔stale)은 stale 플래그가
// 바뀌므로 서명이 달라져 전체 재생성이 정확히 그때만 일어난다.
//
// ★티켓⑥: 인자에서 `fable`을 뺐다(자체 집계 줄 삭제). 서명 문자열도 그 칸이 사라져
//   `<rates>#<ctx>#<footer>` 3칸이 된다 — 빈 칸을 남겨 두지 않는다. 남겨 두면 다음 사람이
//   「무엇이 들어가던 칸인지」를 코드에서 알 수 없고, 서명은 짝을 맞춰 읽는 값이라 유령 칸이
//   생기면 대조가 어긋난다.
export function renderSignature(
  rates: RateRow[],
  ctxRows: CtxRow[],
  showScope: boolean,
  showSocket: boolean,
): string {
  const r = rates
    .map(
      (x) =>
        // ★범위 머리표에 실제로 찍히는 문자열(에이전트 + 계정 라벨)을 그대로 넣는다.
        //   계정 라벨이 바뀌면 화면 글자가 바뀌므로 서명도 바뀌어야 한다 —
        //   식별자(accountId)만 넣으면 라벨 변경이 화면에 반영되지 않는다.
        `${showScope ? x.socket + "/" + x.agent + "/" + x.accountLabel : ""}|${x.label}|${Math.round(x.usedPct)}|${
          x.resetsAt ?? ""
        }|${x.stale ? 1 : 0}`,
    )
    .join(";");
  const c = ctxRows
    .map(
      (x) =>
        // 이름 행은 번호가 없으므로 라벨을 서명에 넣는다 — 안 넣으면 master·cso가 둘 다
        // surfaceId 0으로 같은 서명이 되어 한쪽 값이 바뀌어도 화면이 안 갱신된다.
        `${showSocket ? shortSocketTag(x.socket) : ""}|${x.name || x.surfaceId}|${
          x.ctxPct == null ? "-" : Math.round(x.ctxPct)
        }|${x.source}|${x.stale ? 1 : 0}`,
    )
    .join(";");
  // 푸터의 존재 여부만 넣는다(문구는 나이라서 노드 갱신 대상이다).
  const foot = ctxRows.some((x) => x.ctxPct != null) ? "1" : "0";
  return `${r}#${c}#${foot}`;
}

// 나이 재계산 — 저장된 updated_at으로 언제든 다시 잰다(재생성 없이 텍스트만 갱신하기 위함).
export function ageAt(updatedAt: number, nowSecs: number): number {
  if (!updatedAt) return 0;
  return Math.max(0, Math.round(nowSecs - updatedAt));
}
