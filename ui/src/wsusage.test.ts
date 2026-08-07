import { describe, expect, test } from "bun:test";

// ★픽스처는 **실제 소켓 경로 형태**여야 한다(codex 2R 지적).
// 실물: 기본 데몬 `~/.local/state/cys/cys.sock` · 부서 `~/.local/state/cys-dept-<name>/cys.sock`
// (정본 = Rust src/lib.rs dept_socket_path / is_dept_socket).
// 초판 픽스처는 `/tmp/dept-a.sock`이었는데, 그 형태에서는 basename 로직이 우연히 통과해
// **초록불이 거짓이었다** — 실물 형태에서는 두 부서가 전부 `cys`로 뭉개진다.
const DEPT_A = "/Users/x/.local/state/cys-dept-a/cys.sock";
const DEPT_B = "/Users/x/.local/state/cys-dept-b/cys.sock";
const MAIN_SOCK = "/Users/x/.local/state/cys/cys.sock";
import {
  accountRates,
  ageText,
  ageAt,
  aggregateRates,
  compactTokens,
  fableFromAnalytics,
  fableObserved,
  hasMultipleSockets,
  mergeCtxRows,
  mergeRates,
  namedCtxRows,
  paneCtxRows,
  RATE_LABEL_ORDER,
  renderSignature,
  sevClassFor,
  shortSocketTag,
  sourceGrade,
  USAGE_STALE_SECS,
  type AccountLike,
  type NamedReporterLike,
  type SurfaceLike,
} from "./wsusage";

const mkUsage = (o: Record<string, unknown> = {}) => ({
  agent: "claude",
  ctx_pct: null,
  rate: [],
  source: "statusline",
  updated_at: 1000,
  ...o,
}) as never;

// 기본 surface — socket "" (기본 데몬), 화면에 붙어 있음(adopted).
const sf = (id: number, o: Partial<SurfaceLike> = {}): SurfaceLike => ({
  surface_id: id,
  socket: "",
  exited: false,
  adopted: true,
  usage: null,
  ...o,
});

const NOW = 10_000;
const fresh = (extra: Record<string, unknown> = {}) => mkUsage({ updated_at: NOW, ...extra });
const old = (extra: Record<string, unknown> = {}) => mkUsage({ updated_at: NOW - 600, ...extra });

describe("aggregateRates — 계정 경계", () => {
  test("★같은 5h라도 에이전트가 다르면 섞지 않는다 (codex [High]: claude 80%와 codex 5%가 하나로 합성됐다)", () => {
    const rows = aggregateRates(
      [
        sf(1, { usage: fresh({ agent: "claude", rate: [{ label: "5h", used_pct: 80, resets_at: null }] }) }),
        sf(2, { usage: fresh({ agent: "codex", rate: [{ label: "5h", used_pct: 5, resets_at: null }] }) }),
      ],
      NOW,
    );
    expect(rows).toHaveLength(2);
    const byAgent = Object.fromEntries(rows.map((r) => [r.agent, r.usedPct]));
    expect(byAgent).toEqual({ claude: 80, codex: 5 });
  });

  test("★소켓(부서 데몬)이 다르면 같은 에이전트라도 섞지 않는다 — 계정이 다를 수 있다", () => {
    const rows = aggregateRates(
      [
        sf(1, { socket: "", usage: fresh({ rate: [{ label: "7d", used_pct: 60, resets_at: null }] }) }),
        sf(1, { socket: DEPT_A, usage: fresh({ rate: [{ label: "7d", used_pct: 9, resets_at: null }] }) }),
      ],
      NOW,
    );
    expect(rows).toHaveLength(2);
    expect(rows.map((r) => r.usedPct).sort((a, b) => a - b)).toEqual([9, 60]);
  });

  test("같은 계정 안에서는 최댓값으로 접는다(합산하면 100%를 넘는 거짓이 나온다)", () => {
    const rows = aggregateRates(
      [
        sf(1, { usage: fresh({ rate: [{ label: "5h", used_pct: 30, resets_at: 500 }] }) }),
        sf(2, { usage: fresh({ rate: [{ label: "5h", used_pct: 41, resets_at: 400 }] }) }),
      ],
      NOW,
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].usedPct).toBe(41);
  });

  test("★%와 reset은 같은 관측에서 나온다 — 짝이 깨지면 안 된다 (codex [High])", () => {
    // 41%를 준 관측의 reset은 400이다. 다른 관측의 더 이른 reset(100)을 끌어오면 안 된다.
    const rows = aggregateRates(
      [
        sf(1, { usage: fresh({ rate: [{ label: "5h", used_pct: 30, resets_at: 100 }] }) }),
        sf(2, { usage: fresh({ rate: [{ label: "5h", used_pct: 41, resets_at: 400 }] }) }),
      ],
      NOW,
    );
    expect(rows[0].usedPct).toBe(41);
    expect(rows[0].resetsAt).toBe(400);
  });

  test("★낡은 95%가 신선한 10%를 이기지 못한다 (codex [High]: updated_at 미검사)", () => {
    const rows = aggregateRates(
      [
        sf(1, { usage: old({ rate: [{ label: "5h", used_pct: 95, resets_at: null }] }) }),
        sf(2, { usage: fresh({ rate: [{ label: "5h", used_pct: 10, resets_at: null }] }) }),
      ],
      NOW,
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].usedPct).toBe(10);
    expect(rows[0].stale).toBe(false);
  });

  test("전부 낡았으면 버리지 않고 쓰되 stale로 표시한다(데이터 손실도 거짓 신선도 아님)", () => {
    const rows = aggregateRates([sf(1, { usage: old({ rate: [{ label: "5h", used_pct: 95, resets_at: null }] }) })], NOW);
    expect(rows[0].usedPct).toBe(95);
    expect(rows[0].stale).toBe(true);
    expect(rows[0].ageSecs).toBe(600);
  });

  test("표시 순서는 5h → 7d, 미지 라벨은 뒤 (fable5 확장점)", () => {
    const rows = aggregateRates(
      [
        sf(1, {
          usage: fresh({
            rate: [
              { label: "7d", used_pct: 12, resets_at: null },
              { label: "fable5", used_pct: 5, resets_at: null },
              { label: "5h", used_pct: 41, resets_at: null },
            ],
          }),
        }),
      ],
      NOW,
    );
    expect(rows.map((r) => r.label)).toEqual(["5h", "7d", "fable5"]);
    expect(RATE_LABEL_ORDER).toEqual(["5h", "7d"]);
  });

  test("종료 페인·NaN·빈 입력은 조용히 버린다", () => {
    expect(aggregateRates([], NOW)).toEqual([]);
    const rows = aggregateRates(
      [
        sf(1, { exited: true, usage: fresh({ rate: [{ label: "5h", used_pct: 99, resets_at: null }] }) }),
        sf(2, { usage: fresh({ rate: [{ label: "5h", used_pct: NaN, resets_at: null }] }) }),
        sf(3, { usage: null }),
      ],
      NOW,
    );
    expect(rows).toEqual([]);
  });
});

describe("paneCtxRows", () => {
  test("화면에 붙은 pane만 낸다 — 미입양·headless surface는 대조할 번호가 없다 (codex [Medium])", () => {
    const rows = paneCtxRows(
      [
        sf(1, { adopted: true, usage: fresh({ ctx_pct: 12 }) }),
        sf(2, { adopted: false, usage: fresh({ ctx_pct: 88 }) }),
      ],
      NOW,
    );
    expect(rows.map((r) => r.surfaceId)).toEqual([1]);
  });

  test("★관측이 없는 pane은 지우지 않고 null로 남긴다 — 「미관측」과 「그런 pane 없음」은 다르다", () => {
    const rows = paneCtxRows(
      [sf(7, { usage: null }), sf(8, { usage: fresh({ ctx_pct: null }) }), sf(9, { usage: fresh({ ctx_pct: 0 }) })],
      NOW,
    );
    expect(rows.map((r) => [r.surfaceId, r.ctxPct])).toEqual([
      [7, null],
      [8, null],
      [9, 0], // ★0은 정상 값이다 — null과 다르다
    ]);
  });

  test("소켓 → 페인 번호 오름차순(사용률 순 아님)", () => {
    const rows = paneCtxRows(
      [
        sf(370, { usage: fresh({ ctx_pct: 12 }) }),
        sf(2, { usage: fresh({ ctx_pct: 88 }) }),
        sf(15, { usage: fresh({ ctx_pct: 40 }) }),
      ],
      NOW,
    );
    expect(rows.map((r) => r.surfaceId)).toEqual([2, 15, 370]);
  });

  test("stale 문턱은 renderUsage와 같은 120초 — 경계 양쪽을 못박는다", () => {
    const rows = paneCtxRows(
      [
        sf(1, { usage: mkUsage({ ctx_pct: 10, updated_at: NOW - USAGE_STALE_SECS }) }),
        sf(2, { usage: mkUsage({ ctx_pct: 10, updated_at: NOW - USAGE_STALE_SECS - 1 }) }),
      ],
      NOW,
    );
    expect(rows[0].stale).toBe(false);
    expect(rows[1].stale).toBe(true);
    expect(USAGE_STALE_SECS).toBe(120);
  });

  test("미래 타임스탬프에도 음수 나이를 만들지 않는다", () => {
    const rows = paneCtxRows([sf(1, { usage: mkUsage({ ctx_pct: 10, updated_at: NOW + 500 }) })], NOW);
    expect(rows[0].ageSecs).toBe(0);
  });
});

describe("소켓 표기 — 실제 경로 형태로만 검증한다", () => {
  test("★부서 둘이 각각 SID 1을 발급해도 구분된다 (초판은 둘 다 「cys」로 뭉개졌다 · codex 2R)", () => {
    const rows = paneCtxRows(
      [sf(1, { socket: DEPT_A, usage: fresh({ ctx_pct: 10 }) }), sf(1, { socket: DEPT_B, usage: fresh({ ctx_pct: 20 }) })],
      NOW,
    );
    expect(rows).toHaveLength(2);
    expect(hasMultipleSockets(rows)).toBe(true);
    // 파일명은 둘 다 cys.sock이다 — 이름은 부모 디렉터리에 있다.
    expect(shortSocketTag(DEPT_A)).toBe("a");
    expect(shortSocketTag(DEPT_B)).toBe("b");
    expect(shortSocketTag(DEPT_A)).not.toBe(shortSocketTag(DEPT_B));
  });

  test("기본 데몬은 무표기 — 빈 문자열이든 실제 메인 소켓 경로든 접두를 안 붙인다", () => {
    expect(shortSocketTag("")).toBe("");
    expect(shortSocketTag(MAIN_SOCK)).toBe("");
  });

  test("윈도우 named pipe 형태도 같은 규칙으로 잡는다", () => {
    expect(shortSocketTag("\\\\.\\pipe\\cys-dept-sales")).toBe("sales");
    expect(shortSocketTag("\\\\.\\pipe\\cys")).toBe("");
  });

  test("★부서가 아닌 것은 전부 무표기 — 없는 경로를 위한 폴백을 두지 않는다", () => {
    // 폴백을 뒀다가 윈도우 파이프에서 "pipe"를 부서 이름처럼 뱉었다.
    // 소켓 경로는 dept_socket_path/기본 슬러그만 만든다(src/lib.rs) — 그 밖은 없다.
    expect(shortSocketTag("/tmp/myrig/cys.sock")).toBe("");
    expect(shortSocketTag("/whatever/x.sock")).toBe("");
  });

  test("소켓이 하나뿐이면 접두 판정이 false", () => {
    expect(hasMultipleSockets([{ socket: MAIN_SOCK }, { socket: MAIN_SOCK }])).toBe(false);
  });
});

describe("renderSignature", () => {
  test("같은 값이면 같은 서명 — 무변경 스킵의 근거", () => {
    const rows = paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 10 }) })], NOW);
    const rates = aggregateRates([sf(1, { usage: fresh({ rate: [{ label: "5h", used_pct: 10, resets_at: null }] }) })], NOW);
    expect(renderSignature(rates, rows, null, false, false)).toBe(renderSignature(rates, rows, null, false, false));
  });

  test("★연속 폴링(now 1000 → 1003)에서 서명이 같아야 한다 — 나이가 들어가면 스킵이 무력이다 (codex 2R)", () => {
    const mk = (now: number) => {
      const surfaces = [sf(1, { usage: mkUsage({ ctx_pct: 10, updated_at: 980 }) })];
      return renderSignature(aggregateRates(surfaces, now), paneCtxRows(surfaces, now), null, false, false);
    };
    // 20초 → 23초 (둘 다 60초 미만 = 초 단위 표시 구간). 초판은 여기서 매번 달라졌다.
    expect(mk(1000)).toBe(mk(1003));
    // 여러 틱을 연달아 돌려도 계속 같아야 한다(3초 인터벌 5회분).
    const sigs = [1000, 1003, 1006, 1009, 1012].map(mk);
    expect(new Set(sigs).size).toBe(1);
  });

  test("★stale 경계를 넘는 순간에는 서명이 바뀐다 — 그때는 전체 재생성이 맞다", () => {
    const mk = (now: number) => {
      const surfaces = [sf(1, { usage: mkUsage({ ctx_pct: 10, updated_at: 1000 }) })];
      return renderSignature(aggregateRates(surfaces, now), paneCtxRows(surfaces, now), null, false, false);
    };
    expect(mk(1000 + USAGE_STALE_SECS)).not.toBe(mk(1000 + USAGE_STALE_SECS + 1));
  });

  test("화면에 보이는 값이 바뀌면 서명도 바뀐다 — 값·미관측·Fable", () => {
    const base = paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 10 }) })], NOW);
    const sig = renderSignature([], base, null, false, false);
    expect(renderSignature([], paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 11 }) })], NOW), null, false, false)).not.toBe(sig);
    expect(renderSignature([], paneCtxRows([sf(1, { usage: null })], NOW), null, false, false)).not.toBe(sig);
    expect(renderSignature([], base, { tokens: 1, sharePct: 1 }, false, false)).not.toBe(sig);
  });

  test("★부서가 달라 태그가 갈리면 서명도 갈린다 — 실물 경로 형태로 검증", () => {
    const a = paneCtxRows([sf(1, { socket: DEPT_A, usage: fresh({ ctx_pct: 10 }) })], NOW);
    const b = paneCtxRows([sf(1, { socket: DEPT_B, usage: fresh({ ctx_pct: 10 }) })], NOW);
    expect(renderSignature([], a, null, false, true)).not.toBe(renderSignature([], b, null, false, true));
  });
});

// ── 계정 저장소 유래 rate (오너 육안 판정 2026-08-07 09:27 수리)
//
// ★픽스처는 **실물 usage.accounts 응답 형태**다(라이브 RPC 실측본을 줄여 옮겼다).
// 실물에 있는 필드를 빠뜨린 픽스처는 초록불이 거짓이 된다 —
// 이 파일 맨 위 소켓 픽스처가 정확히 그 이유로 한 번 무너졌다(codex 2R).
const ACCT_CLAUDE: AccountLike = {
  provider: "claude",
  account_id: "66e877cb-727f-4fb1-8ec9-8f2e8fa68f18",
  label: "oogisoogi@gmail.com",
  rate: [
    { label: "5h", used_pct: 53, resets_at: 1786068600 },
    { label: "7d", used_pct: 10, resets_at: 1786654800 },
  ],
  updated_at: 1786062874,
};
const ACCT_CODEX: AccountLike = {
  provider: "codex",
  account_id: "default",
  label: "OpenAI Codex",
  rate: [{ label: "7d", used_pct: 52, resets_at: 1786431165 }],
  updated_at: 1786062874,
};
// 등록만 되고 아직 한 번도 관측되지 않은 계정 — 실물에서 antigravity가 이 모양이다.
const ACCT_UNOBSERVED: AccountLike = {
  provider: "antigravity",
  account_id: "default",
  label: "Antigravity (agy)",
  rate: [],
  updated_at: null,
};
const ANOW = 1786062880; // ACCT_* 관측 6초 뒤

describe("accountRates — 페인이 없어도 계정 사용량은 있다", () => {
  test("★★surface 0개인데 계정 데이터가 있으면 rate 행이 나온다 (오너 육안 결함의 정본 회귀)", () => {
    // 결함: 초판은 사이드바 사용량을 살아 있는 surface의 usage만으로 만들었다.
    // ⇒ 페인 0 ⇒ rates 0 ⇒ 「사용량」 절이 통째로 사라졌다. 5h·7d는 페인이 아니라 계정의 속성이다.
    const rows = mergeRates(accountRates([ACCT_CLAUDE, ACCT_CODEX], ANOW), aggregateRates([], ANOW));
    expect(rows.length).toBe(3);
    expect(rows.map((r) => `${r.agent}/${r.label}/${r.usedPct}`)).toEqual([
      "claude/5h/53",
      "claude/7d/10",
      "codex/7d/52",
    ]);
    // 계정 신원이 실려 있어야 범위 머리표가 계정별로 갈린다.
    expect(rows[0].accountLabel).toBe("oogisoogi@gmail.com");
    expect(rows[0].accountId).toBe("66e877cb-727f-4fb1-8ec9-8f2e8fa68f18");
  });

  test("★관측된 적 없는 계정(updated_at=null)은 그리지 않는다 — 나이 0은 「방금 봤다」로 읽힌다", () => {
    expect(accountRates([ACCT_UNOBSERVED], ANOW)).toEqual([]);
    // rate 배열이 비어 있지 않은데 updated_at만 없는 경우에도 같다(거짓 신선 금지).
    const forged: AccountLike = { ...ACCT_UNOBSERVED, rate: [{ label: "5h", used_pct: 99, resets_at: null }] };
    expect(accountRates([forged], ANOW)).toEqual([]);
  });

  test("나이·stale은 계정 관측 시각으로 잰다 — 폴링 주기가 아니라", () => {
    const fresh6 = accountRates([ACCT_CLAUDE], ANOW);
    expect(fresh6[0].ageSecs).toBe(6);
    expect(fresh6[0].stale).toBe(false);
    const late = accountRates([ACCT_CLAUDE], ACCT_CLAUDE.updated_at! + USAGE_STALE_SECS + 1);
    expect(late[0].stale).toBe(true);
    // 경계값은 아직 stale이 아니다(aggregateRates와 같은 문턱을 써야 한 화면에서 판정이 갈리지 않는다).
    expect(accountRates([ACCT_CLAUDE], ACCT_CLAUDE.updated_at! + USAGE_STALE_SECS)[0].stale).toBe(false);
  });

  test("used_pct가 숫자가 아니면 그 창만 버린다 — 계정 전체를 버리지 않는다", () => {
    const mixed: AccountLike = {
      ...ACCT_CLAUDE,
      rate: [
        { label: "5h", used_pct: NaN as unknown as number, resets_at: null },
        { label: "7d", used_pct: 10, resets_at: 1786654800 },
      ],
    };
    const rows = accountRates([mixed], ANOW);
    expect(rows.map((r) => r.label)).toEqual(["7d"]);
  });

  test("같은 provider라도 계정이 둘이면 두 블록 — 계정별로 묶여 나온다", () => {
    const second: AccountLike = { ...ACCT_CLAUDE, account_id: "zz-second", label: "work@example.com" };
    const rows = accountRates([second, ACCT_CLAUDE], ANOW);
    // 계정 id 순으로 묶인다 — 한 계정의 5h·7d가 붙어 있어야 머리표가 매 줄 뒤집히지 않는다.
    expect(rows.map((r) => `${r.accountId}/${r.label}`)).toEqual([
      "66e877cb-727f-4fb1-8ec9-8f2e8fa68f18/5h",
      "66e877cb-727f-4fb1-8ec9-8f2e8fa68f18/7d",
      "zz-second/5h",
      "zz-second/7d",
    ]);
  });
});

describe("mergeRates — 계정이 이긴다(중복 줄 = 가짜 계정)", () => {
  test("★같은 (에이전트,창)이 양쪽에 있으면 한 줄만 — 두 줄이면 사용자가 두 계정으로 읽는다", () => {
    const surface = aggregateRates(
      [sf(1, { usage: mkUsage({ agent: "claude", updated_at: ANOW, rate: [{ label: "5h", used_pct: 41, resets_at: null }] }) })],
      ANOW,
    );
    const rows = mergeRates(accountRates([ACCT_CLAUDE], ANOW), surface);
    const fiveH = rows.filter((r) => r.agent === "claude" && r.label === "5h");
    expect(fiveH).toHaveLength(1);
    // 이긴 쪽은 계정이다 — 41(페인 관측)이 아니라 53(계정 저장소).
    expect(fiveH[0].usedPct).toBe(53);
    expect(fiveH[0].accountId).not.toBe("");
  });

  test("계정이 모르는 창은 surface 관측이 메운다 — 원천 전환이 곧 데이터 손실이면 안 된다", () => {
    const surface = aggregateRates(
      [sf(1, { usage: mkUsage({ agent: "gemini", updated_at: ANOW, rate: [{ label: "5h", used_pct: 7, resets_at: null }] }) })],
      ANOW,
    );
    const rows = mergeRates(accountRates([ACCT_CLAUDE], ANOW), surface);
    const gem = rows.filter((r) => r.agent === "gemini");
    expect(gem).toHaveLength(1);
    expect(gem[0].usedPct).toBe(7);
    expect(gem[0].accountId).toBe(""); // surface 유래 — 계정 신원을 지어내지 않는다
  });

  test("양쪽 다 비면 빈 배열 — 패널이 숨는 조건이 그대로 유지된다", () => {
    expect(mergeRates(accountRates([], ANOW), aggregateRates([], ANOW))).toEqual([]);
  });
});

// ── 이름 있는 보고자(master·cso) — surface 없는 Claude의 CTX (오너 2026-08-07 티켓④)
describe("namedCtxRows / mergeCtxRows — 번호 대신 이름", () => {
  const NR = (o: Partial<NamedReporterLike> = {}): NamedReporterLike => ({
    name: "master",
    ctx_pct: 11,
    source: "statusline",
    updated_at: NOW,
    ...o,
  });

  test("★이름 라벨로 행을 만든다 — 번호를 지어내지 않는다", () => {
    const rows = namedCtxRows([NR()], NOW);
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe("master");
    expect(rows[0].ctxPct).toBe(11);
    // 번호가 없다는 사실이 데이터에 남아 있어야 렌더가 라벨을 고른다.
    expect(rows[0].surfaceId).toBe(0);
  });

  test("★이름 행이 번호 행보다 먼저 온다 (오너 지정 순서) — 그 다음은 번호 오름차순", () => {
    const panes = paneCtxRows(
      [sf(7, { usage: fresh({ ctx_pct: 30 }) }), sf(3, { usage: fresh({ ctx_pct: 20 }) })],
      NOW,
    );
    const rows = mergeCtxRows(namedCtxRows([NR({ name: "master" }), NR({ name: "cso", ctx_pct: 7 })], NOW), panes);
    expect(rows.map((r) => r.name || r.surfaceId)).toEqual(["cso", "master", 3, 7]);
  });

  test("이름 없는 보고자는 행을 만들지 않는다 — 지어낸 라벨 금지", () => {
    expect(namedCtxRows([NR({ name: "" })], NOW)).toEqual([]);
  });

  test("★관측된 적 없는 보고자(updated_at 없음)는 그리지 않는다 — 나이 0은 「방금」으로 읽힌다", () => {
    expect(namedCtxRows([NR({ updated_at: 0 })], NOW)).toEqual([]);
  });

  test("신선도·stale 규율은 번호 행과 같은 문턱을 쓴다 — 한 표에서 판정이 갈리면 안 된다", () => {
    expect(namedCtxRows([NR({ updated_at: NOW - USAGE_STALE_SECS })], NOW)[0].stale).toBe(false);
    expect(namedCtxRows([NR({ updated_at: NOW - USAGE_STALE_SECS - 1 })], NOW)[0].stale).toBe(true);
  });

  test("ctx가 없으면 행은 남기되 「—」로 — 미관측과 부재를 구별한다(번호 행과 같은 규율)", () => {
    const rows = namedCtxRows([NR({ ctx_pct: null })], NOW);
    expect(rows).toHaveLength(1);
    expect(rows[0].ctxPct).toBeNull();
    expect(rows[0].stale).toBe(false);
  });

  test("★서명이 이름을 본다 — 안 보면 master와 cso가 둘 다 surfaceId 0이라 갱신이 멈춘다", () => {
    const a = mergeCtxRows(namedCtxRows([NR({ name: "master", ctx_pct: 11 })], NOW), []);
    const b = mergeCtxRows(namedCtxRows([NR({ name: "cso", ctx_pct: 11 })], NOW), []);
    expect(renderSignature([], a, null, false, false)).not.toBe(renderSignature([], b, null, false, false));
  });

  test("빈 함대 + named 만 있어도 표가 선다 — 티켓④의 목적 그 자체", () => {
    const rows = mergeCtxRows(namedCtxRows([NR()], NOW), paneCtxRows([], NOW));
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe("master");
  });
});

describe("fableFromAnalytics — 응답 필드 위치를 아는 유일한 자리", () => {
  // ★픽스처는 control.analytics 라이브 응답의 실제 형태다(RPC 실측본).
  // 초판 결함은 by_model·totals를 **최상위에서** 읽은 것이었다 — 실제로는 summary 아래에 있다.
  const REAL = {
    now: 1786062898.73,
    since: 1785458098.73,
    window: "7d",
    summary: {
      by_model: [
        { model: "claude-fable-5", tokens: 1_880_624_582 },
        { model: "claude-opus-5", tokens: 3_787_689_869 },
      ],
      totals: { tokens: 5_838_120_878, msgs: 17626 },
    },
  };

  test("★★summary 홉을 거쳐 읽는다 — 최상위에서 읽으면 줄이 영원히 안 뜬다(오너 육안 결함)", () => {
    const f = fableFromAnalytics(REAL);
    expect(f).not.toBeNull();
    expect(f!.tokens).toBe(1_880_624_582);
    expect(f!.sharePct).toBe(32.2);
  });

  test("summary가 없는 응답이면 null — 없는 것을 0으로 그리지 않는다", () => {
    expect(fableFromAnalytics({ now: 1, since: 0, window: "7d" })).toBeNull();
    expect(fableFromAnalytics(null)).toBeNull();
    expect(fableFromAnalytics(undefined)).toBeNull();
  });

  test("집계가 비었으면(totals 0) null, Fable만 없으면 0 — 「모름」과 「안 씀」은 다르다", () => {
    expect(fableFromAnalytics({ summary: { by_model: [], totals: { tokens: 0 } } })).toBeNull();
    const none = fableFromAnalytics({
      summary: { by_model: [{ model: "claude-opus-5", tokens: 100 }], totals: { tokens: 100 } },
    });
    expect(none).toEqual({ tokens: 0, sharePct: 0 });
  });
});

describe("ageAt", () => {
  test("저장된 updated_at으로 언제든 나이를 다시 잰다(재생성 없이 갱신하기 위함)", () => {
    expect(ageAt(1000, 1037)).toBe(37);
    expect(ageAt(1000, 1000)).toBe(0);
    expect(ageAt(1000, 900)).toBe(0); // 미래 타임스탬프도 음수 금지
    expect(ageAt(0, 5000)).toBe(0); // updatedAt 0 = 미관측 — 나이 개념 없음
  });
});

describe("sevClassFor", () => {
  test("CTX 임계 60/80 — 경계값 포함", () => {
    expect(sevClassFor(59, 60, 80)).toBe("");
    expect(sevClassFor(60, 60, 80)).toBe("warn");
    expect(sevClassFor(80, 60, 80)).toBe("crit");
  });
  test("rate 임계 70/90", () => {
    expect(sevClassFor(69, 70, 90)).toBe("");
    expect(sevClassFor(70, 70, 90)).toBe("warn");
    expect(sevClassFor(90, 70, 90)).toBe("crit");
  });
});

describe("ageText", () => {
  test("초·분·시간 단위로 접는다", () => {
    expect(ageText(0)).toBe("0초 전");
    expect(ageText(59)).toBe("59초 전");
    expect(ageText(60)).toBe("1분 전");
    expect(ageText(3599)).toBe("60분 전");
    expect(ageText(3600)).toBe("1시간 전");
  });
  test("비정상 값은 물음표 — 숫자처럼 보이는 거짓을 만들지 않는다", () => {
    expect(ageText(NaN)).toBe("?");
    expect(ageText(-5)).toBe("?");
  });
});

describe("sourceGrade", () => {
  test("statusline = 서버 진실, tail = 추정 — 두 값을 같은 눈금에 두므로 등급을 구분한다", () => {
    expect(sourceGrade("statusline").mark).toBe("●");
    expect(sourceGrade("transcript").mark).toBe("○");
    expect(sourceGrade("transcript:heuristic").title).toContain("휴리스틱");
    expect(sourceGrade("").mark).toBe("?");
  });
});
