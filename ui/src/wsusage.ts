// 사이드바 사용량 패널의 순수 계산 — rate 집계 + 페인별 CTX 목록 + 신선도 판정 + Fable 관측치.
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
// ★fable5 확장점: 모델별 주간 게이지가 statusline JSON에 확인되어 파서에 추가되는 날,
// 이 배열에 라벨 한 줄만 넣으면 패널에 나타난다. 없는 값을 미리 0%로 그려 두지 않는 이유:
// 빈 게이지는 「안 썼다」로 읽히지 「데이터가 없다」로 읽히지 않는다.
export const RATE_LABEL_ORDER = ["5h", "7d"];

export interface RateRow {
  socket: string;
  agent: string;
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
  // 정렬: 소켓 → 에이전트 → 라벨 순서(RATE_LABEL_ORDER 우선, 나머지는 사전순).
  const labelRank = (l: string) => {
    const i = RATE_LABEL_ORDER.indexOf(l);
    return i < 0 ? RATE_LABEL_ORDER.length : i;
  };
  rows.sort(
    (a, b) =>
      a.socket.localeCompare(b.socket) ||
      a.agent.localeCompare(b.agent) ||
      labelRank(a.label) - labelRank(b.label) ||
      a.label.localeCompare(b.label),
  );
  return rows;
}

export interface CtxRow {
  surfaceId: number;
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
      rows.push({ surfaceId: s.surface_id, socket: s.socket, ctxPct: null, ageSecs: 0, updatedAt: 0, stale: false, source: "" });
      continue;
    }
    const raw = u.ctx_pct;
    const pct = raw == null || !Number.isFinite(Number(raw)) ? null : Number(raw);
    const age = Math.max(0, Math.round(nowSecs - u.updated_at));
    rows.push({
      surfaceId: s.surface_id,
      socket: s.socket,
      ctxPct: pct,
      ageSecs: pct == null ? 0 : age,
      updatedAt: pct == null ? 0 : u.updated_at,
      stale: pct == null ? false : age > USAGE_STALE_SECS,
      source: pct == null ? "" : (u.source ?? ""),
    });
  }
  rows.sort((a, b) => a.socket.localeCompare(b.socket) || a.surfaceId - b.surfaceId);
  return rows;
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

// ── Fable 관측치(오너 발주 3번의 「fable5」 · master 결정② B안 2026-08-07)
//
// ★이 값은 5h·7d와 **종류가 다르다.** 5h·7d는 Anthropic이 준 「한도 대비 소진율」이고, 이것은
// 우리가 트랜스크립트를 세어 만든 「관측 절대치」다. 한도가 얼마인지 우리는 모른다.
// 그래서 master가 형태를 못박았다: ⑴게이지 바로 그리지 말 것(같은 눈금으로 보이면 안 된다)
// ⑵%가 아니라 절대치 + 비중으로 적고 「자체 집계」를 명시할 것.
// ⇒ 아래 반환값에 pct(한도 대비)가 아예 없다. 형태 규율을 주석이 아니라 **타입으로** 강제한다.
export interface ByModelRow {
  model: string;
  tokens: number;
}
export interface FableObserved {
  tokens: number; // 관측 절대치(input+output+cache 합)
  sharePct: number; // 전체 관측 토큰 대비 비중 — 한도 대비가 아니다
}

const isFableModel = (model: string): boolean => model.toLowerCase().includes("fable");

// by_model 집계 → Fable 관측치. 데이터 자체가 없으면 null(그리지 않는다).
//
// ★「Fable을 안 썼다(0)」와 「집계가 비었다(모름)」를 가른다 — 둘 다 0으로 보이지만 뜻이 정반대다.
// totals가 0이면 아직 아무것도 못 셌다는 뜻이므로 null(표시 없음), totals가 있는데 fable 행이
// 없으면 진짜로 0 쓴 것이므로 0을 표시한다(오너가 지켜보려는 값이라 「썼는지 안 썼는지」가 정보다).
export function fableObserved(byModel: ByModelRow[] | null | undefined, totalTokens: number): FableObserved | null {
  const total = Number(totalTokens);
  if (!Number.isFinite(total) || total <= 0) return null;
  let tokens = 0;
  for (const m of byModel ?? []) {
    if (!m || typeof m.model !== "string" || !isFableModel(m.model)) continue;
    const t = Number(m.tokens);
    if (Number.isFinite(t) && t > 0) tokens += t;
  }
  return { tokens, sharePct: Math.round((tokens / total) * 1000) / 10 };
}

// 큰 토큰 수 → 짧은 표기(1.2M · 340K). 사이드바 폭이 좁아 원수치는 줄을 넘긴다.
// 정확한 원수치는 툴팁에 남긴다 — 줄이는 것과 잃는 것은 다르다.
export function compactTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "?";
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) {
    const k = n / 1000;
    return `${k < 10 ? Math.round(k * 10) / 10 : Math.round(k)}K`;
  }
  const m = n / 1_000_000;
  return `${m < 10 ? Math.round(m * 10) / 10 : Math.round(m)}M`;
}

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
export function renderSignature(
  rates: RateRow[],
  ctxRows: CtxRow[],
  fable: FableObserved | null,
  showScope: boolean,
  showSocket: boolean,
): string {
  const r = rates
    .map(
      (x) =>
        `${showScope ? x.socket + "/" + x.agent : ""}|${x.label}|${Math.round(x.usedPct)}|${x.resetsAt ?? ""}|${
          x.stale ? 1 : 0
        }`,
    )
    .join(";");
  const c = ctxRows
    .map(
      (x) =>
        `${showSocket ? shortSocketTag(x.socket) : ""}|${x.surfaceId}|${
          x.ctxPct == null ? "-" : Math.round(x.ctxPct)
        }|${x.source}|${x.stale ? 1 : 0}`,
    )
    .join(";");
  const f = fable ? `${fable.tokens}|${fable.sharePct}` : "";
  // 푸터의 존재 여부만 넣는다(문구는 나이라서 노드 갱신 대상이다).
  const foot = ctxRows.some((x) => x.ctxPct != null) ? "1" : "0";
  return `${r}#${c}#${f}#${foot}`;
}

// 나이 재계산 — 저장된 updated_at으로 언제든 다시 잰다(재생성 없이 텍스트만 갱신하기 위함).
export function ageAt(updatedAt: number, nowSecs: number): number {
  if (!updatedAt) return 0;
  return Math.max(0, Math.round(nowSecs - updatedAt));
}
