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
  CTX_NAME_ORDER,
  filterDisplayRates,
  hasMultipleSockets,
  mergeCtxRows,
  mergeRates,
  namedCtxRows,
  paneCtxRows,
  RATE_LABEL_ORDER,
  renderSignature,
  scopedRates,
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
    expect(renderSignature(rates, rows, false, false)).toBe(renderSignature(rates, rows, false, false));
  });

  test("★연속 폴링(now 1000 → 1003)에서 서명이 같아야 한다 — 나이가 들어가면 스킵이 무력이다 (codex 2R)", () => {
    const mk = (now: number) => {
      const surfaces = [sf(1, { usage: mkUsage({ ctx_pct: 10, updated_at: 980 }) })];
      return renderSignature(aggregateRates(surfaces, now), paneCtxRows(surfaces, now), false, false);
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
      return renderSignature(aggregateRates(surfaces, now), paneCtxRows(surfaces, now), false, false);
    };
    expect(mk(1000 + USAGE_STALE_SECS)).not.toBe(mk(1000 + USAGE_STALE_SECS + 1));
  });

  test("화면에 보이는 값이 바뀌면 서명도 바뀐다 — 값·미관측", () => {
    const base = paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 10 }) })], NOW);
    const sig = renderSignature([], base, false, false);
    expect(renderSignature([], paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 11 }) })], NOW), false, false)).not.toBe(sig);
    expect(renderSignature([], paneCtxRows([sf(1, { usage: null })], NOW), false, false)).not.toBe(sig);
  });

  // ★티켓⑥ 회귀: 자체 집계 줄이 사라졌으니 서명의 칸도 3개다(rates#ctx#footer).
  //   빈 칸이 남아 있으면(`…##0`) 「아직 무언가 들어갈 자리」로 읽혀 다음 사람이 되살린다 —
  //   삭제는 값만이 아니라 **자리까지** 없애야 끝난다.
  test("★삭제 후 서명 칸은 정확히 3개 — 유령 칸(자체 집계 자리)이 남지 않는다", () => {
    const rows = paneCtxRows([sf(1, { usage: fresh({ ctx_pct: 10 }) })], NOW);
    const rates = aggregateRates([sf(1, { usage: fresh({ rate: [{ label: "5h", used_pct: 10, resets_at: null }] }) })], NOW);
    const sig = renderSignature(rates, rows, false, false);
    expect(sig.split("#")).toHaveLength(3);
    expect(sig.endsWith("#1")).toBe(true); // 마지막 칸 = 푸터 존재 여부
    expect(sig).not.toContain("##"); // 비어 있는 중간 칸 = 삭제가 덜 된 흔적
  });

  test("★부서가 달라 태그가 갈리면 서명도 갈린다 — 실물 경로 형태로 검증", () => {
    const a = paneCtxRows([sf(1, { socket: DEPT_A, usage: fresh({ ctx_pct: 10 }) })], NOW);
    const b = paneCtxRows([sf(1, { socket: DEPT_B, usage: fresh({ ctx_pct: 10 }) })], NOW);
    expect(renderSignature([], a, false, true)).not.toBe(renderSignature([], b, false, true));
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

// ── 모델 스코프 주간 게이지 + codex 비표시 (티켓⑤ · 오너 승인 2026-08-07)
//
// 픽스처는 데몬 local_json이 실제로 내보내는 모양 그대로다(accounts.rs ScopedGauge 직렬화):
// scoped[]는 계정 안에 들어 있고 **자기 updated_at**을 들고 있다.
describe("scopedRates — 「7d·Fable」 실게이지", () => {
  const ACCT_WITH_SCOPED: AccountLike = {
    ...ACCT_CLAUDE,
    scoped: [
      { model: "Fable", used_pct: 6, resets_at: 1786654800, updated_at: 1786062874, source: "oauth" },
    ],
  };

  test("★계정 소속으로 「7d·Fable」 행이 나온다 — 게이지에 필요한 값이 전부 실려야 한다", () => {
    const rows = scopedRates([ACCT_WITH_SCOPED], ANOW);
    expect(rows).toHaveLength(1);
    expect(rows[0].label).toBe("7d·Fable");
    expect(rows[0].usedPct).toBe(6);
    expect(rows[0].agent).toBe("claude");
    // 계정 신원 — 이 값이 비면 범위 머리표가 갈리지 않아 「누구의 한도인지」를 잃는다.
    expect(rows[0].accountId).toBe("66e877cb-727f-4fb1-8ec9-8f2e8fa68f18");
    expect(rows[0].accountLabel).toBe("oogisoogi@gmail.com");
    // 리셋 시각은 게이지가 준 값과 짝을 유지한다(다른 창 것을 끌어오지 않는다).
    expect(rows[0].resetsAt).toBe(1786654800);
  });

  test("★★나이는 게이지 자신의 시각으로 잰다 — 계정 시각을 물려 쓰면 낡은 게이지가 방금 관측으로 둔갑한다", () => {
    // 계정(rate 슬롯)은 방금 갱신됐지만 게이지는 10분 전 관측인 상황 —
    // statusline이 rate만 갱신하고 지나갈 때 실제로 생기는 모양이다.
    const drifted: AccountLike = {
      ...ACCT_CLAUDE,
      updated_at: ANOW, // 계정 rate는 지금
      scoped: [{ model: "Fable", used_pct: 6, resets_at: null, updated_at: ANOW - 600, source: "oauth" }],
    };
    const rows = scopedRates([drifted], ANOW);
    expect(rows[0].ageSecs).toBe(600);
    expect(rows[0].stale).toBe(true);
    // 대조군: 같은 계정의 rate 행은 신선하다 — 두 축이 따로 늙는 것이 정상이다.
    expect(accountRates([drifted], ANOW)[0].stale).toBe(false);
  });

  test("stale 문턱은 rate 행과 같은 값을 쓴다 — 한 표에서 판정이 갈리면 같은 색이 두 뜻을 갖는다", () => {
    const at = (age: number): AccountLike => ({
      ...ACCT_CLAUDE,
      scoped: [{ model: "Fable", used_pct: 6, resets_at: null, updated_at: ANOW - age, source: "oauth" }],
    });
    expect(scopedRates([at(USAGE_STALE_SECS)], ANOW)[0].stale).toBe(false); // 경계는 아직 아니다
    expect(scopedRates([at(USAGE_STALE_SECS + 1)], ANOW)[0].stale).toBe(true);
  });

  test("이름 없는 모델·관측 시각 없는 게이지는 그리지 않는다 — 없는 사실을 만들지 않는다", () => {
    const bad: AccountLike = {
      ...ACCT_CLAUDE,
      scoped: [
        { model: "", used_pct: 5, resets_at: null, updated_at: ANOW, source: "oauth" },
        { model: "Fable", used_pct: 5, resets_at: null, updated_at: 0, source: "oauth" },
        { model: "Fable", used_pct: NaN as unknown as number, resets_at: null, updated_at: ANOW, source: "oauth" },
      ],
    };
    expect(scopedRates([bad], ANOW)).toEqual([]);
    // scoped 자체가 없는 계정(실물의 codex·agy)도 조용히 0행이다.
    expect(scopedRates([ACCT_CODEX, ACCT_UNOBSERVED], ANOW)).toEqual([]);
  });

  test("정렬 — 5h · 7d 다음에 온다(미등재 라벨이 뒤로 가는 규칙 그대로)", () => {
    const rows = mergeRates(
      [...accountRates([ACCT_WITH_SCOPED], ANOW), ...scopedRates([ACCT_WITH_SCOPED], ANOW)],
      aggregateRates([], ANOW),
    );
    expect(rows.map((r) => r.label)).toEqual(["5h", "7d", "7d·Fable"]);
  });

  // ★티켓⑥ 회귀(삭제 후 렌더): 「자체 집계」 줄이 없어져도 **이 게이지는 그대로 산다.**
  //   초판 테스트는 「두 줄이 함께 산다」를 지켰는데, 삭제 국면에서 진짜 위험은 그 반대다 —
  //   같은 「Fable」이라는 낱말을 지우다가 살려야 할 실게이지까지 함께 지우는 것.
  //   ⇒ 남는 쪽을 단언한다.
  test("★자체 집계 줄 삭제 후에도 「7d·Fable」 실게이지는 남는다 — 지울 것과 남길 것을 낱말로 가르지 않는다", () => {
    const rates = mergeRates(
      [...accountRates([ACCT_WITH_SCOPED], ANOW), ...scopedRates([ACCT_WITH_SCOPED], ANOW)],
      aggregateRates([], ANOW),
    );
    expect(rates.map((r) => r.label)).toContain("7d·Fable");
    const sig = renderSignature(rates, [], false, false);
    expect(sig).toContain("7d·Fable|6"); // 한도 대비 게이지(서버 진실)만 남는다
    // 자체 집계(관측 절대치 400 tok · 비중 40%)는 서명 어디에도 없다.
    expect(sig).not.toContain("400|40");
  });
});

describe("filterDisplayRates — codex 행 비표시 (오너 판정 2026-08-07)", () => {
  test("★계정 유래 codex 행이 화면에서 사라진다 — 수집은 그대로 두고 표시만 끊는다", () => {
    const merged = mergeRates(accountRates([ACCT_CLAUDE, ACCT_CODEX], ANOW), aggregateRates([], ANOW));
    expect(merged.some((r) => r.agent === "codex")).toBe(true); // 병합 단계엔 아직 있다(수집 무영향)
    const shown = filterDisplayRates(merged);
    expect(shown.some((r) => r.agent === "codex")).toBe(false);
    expect(shown.map((r) => `${r.agent}/${r.label}`)).toEqual(["claude/5h", "claude/7d"]);
  });

  test("★★surface 관측 유래 codex 행도 막힌다 — 계정 행만 지우면 덮개가 사라져 밑에 있던 것이 드러난다", () => {
    // 계정 저장소에 codex가 없으면 mergeRates의 중복 방지가 걸리지 않아 surface 행이 그대로 올라온다.
    const surface = aggregateRates(
      [sf(1, { usage: mkUsage({ agent: "codex", updated_at: ANOW, rate: [{ label: "7d", used_pct: 52, resets_at: null }] }) })],
      ANOW,
    );
    const merged = mergeRates(accountRates([ACCT_CLAUDE], ANOW), surface);
    expect(merged.some((r) => r.agent === "codex")).toBe(true);
    expect(filterDisplayRates(merged).some((r) => r.agent === "codex")).toBe(false);
  });

  test("다른 에이전트는 건드리지 않는다 — 필터가 넓어지면 살아 있는 원천까지 지운다", () => {
    const surface = aggregateRates(
      [sf(1, { usage: mkUsage({ agent: "gemini", updated_at: ANOW, rate: [{ label: "5h", used_pct: 7, resets_at: null }] }) })],
      ANOW,
    );
    const shown = filterDisplayRates(mergeRates(accountRates([ACCT_CLAUDE], ANOW), surface));
    expect(shown.map((r) => r.agent)).toEqual(["claude", "claude", "gemini"]);
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

  // ★티켓⑥-b(오너 2026-08-07): 서열은 master → cso → 그 밖의 이름 → 번호 페인이다.
  //   초판은 「이름 먼저 + 사전순」이라 c < m 때문에 cso가 master 위로 왔다 — 화면의 서열이
  //   조직의 서열과 거꾸로였다. 아래 기대값의 첫 두 자리가 그 수리의 전부다.
  test("★페인 CTX 서열 = master → cso → 번호 오름차순 (오너 지정 순서)", () => {
    const panes = paneCtxRows(
      [sf(7, { usage: fresh({ ctx_pct: 30 }) }), sf(3, { usage: fresh({ ctx_pct: 20 }) })],
      NOW,
    );
    const rows = mergeCtxRows(namedCtxRows([NR({ name: "master" }), NR({ name: "cso", ctx_pct: 7 })], NOW), panes);
    expect(rows.map((r) => r.name || r.surfaceId)).toEqual(["master", "cso", 3, 7]);
  });

  test("★입력 순서가 어떻든 결과 서열은 같다 — 정렬이지 도착 순서가 아니다", () => {
    const panes = paneCtxRows([sf(3, { usage: fresh({ ctx_pct: 20 }) })], NOW);
    const named = namedCtxRows([NR({ name: "cso", ctx_pct: 7 }), NR({ name: "master" })], NOW);
    expect(mergeCtxRows(named, panes).map((r) => r.name || r.surfaceId)).toEqual(["master", "cso", 3]);
    // 병합 인자를 뒤집어도(번호 행이 먼저 들어와도) 같다.
    expect(mergeCtxRows([], [...panes, ...named]).map((r) => r.name || r.surfaceId)).toEqual(["master", "cso", 3]);
  });

  test("★등재되지 않은 이름 보고자는 master·cso 뒤 · 번호 페인 앞 — 사이 자리가 정해져 있다", () => {
    const panes = paneCtxRows([sf(2, { usage: fresh({ ctx_pct: 5 }) })], NOW);
    const named = namedCtxRows(
      [NR({ name: "worker-a" }), NR({ name: "cso" }), NR({ name: "alpha" }), NR({ name: "master" })],
      NOW,
    );
    // 미등재끼리는 사전순(alpha < worker-a) — 서열이 없으면 값에 안 흔들리는 순서를 쓴다.
    expect(mergeCtxRows(named, panes).map((r) => r.name || r.surfaceId)).toEqual([
      "master",
      "cso",
      "alpha",
      "worker-a",
      2,
    ]);
  });

  test("대소문자가 달라도 서열은 유지된다 — env 재정의로 「Master」가 올 수 있다", () => {
    const rows = mergeCtxRows(namedCtxRows([NR({ name: "CSO" }), NR({ name: "Master" })], NOW), []);
    expect(rows.map((r) => r.name)).toEqual(["Master", "CSO"]);
    expect(CTX_NAME_ORDER).toEqual(["master", "cso"]);
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
    expect(renderSignature([], a, false, false)).not.toBe(renderSignature([], b, false, false));
  });

  test("빈 함대 + named 만 있어도 표가 선다 — 티켓④의 목적 그 자체", () => {
    const rows = mergeCtxRows(namedCtxRows([NR()], NOW), paneCtxRows([], NOW));
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe("master");
  });
});

// 「fableFromAnalytics — 응답 필드 위치를 아는 유일한 자리」 describe가 여기 있었다.
// 티켓⑥에서 그 함수(와 fableObserved·compactTokens)를 지우면서 함께 삭제했다 —
// ★사라진 코드의 테스트를 남겨 두면 그것이 「아직 있는 기능」의 증거처럼 보인다.
// 삭제 자체의 회귀 그물은 위 renderSignature 절(칸 3개·유령 칸 없음)과
// scopedRates 절(실게이지 생존)이 대신 진다.

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
