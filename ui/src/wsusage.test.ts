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
  ageText,
  ageAt,
  aggregateRates,
  compactTokens,
  fableObserved,
  hasMultipleSockets,
  paneCtxRows,
  RATE_LABEL_ORDER,
  renderSignature,
  sevClassFor,
  shortSocketTag,
  sourceGrade,
  USAGE_STALE_SECS,
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
