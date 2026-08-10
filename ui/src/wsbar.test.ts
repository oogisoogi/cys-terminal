// wsbar.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 사이드바 폭·글자 배율 클램프가 경계·비정상 입력에서 결정론인지 검증한다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  clampWsbarWidth,
  clampWsbarFont,
  showsRowAge,
  WSBAR_W_MIN,
  WSBAR_W_MAX,
  WSBAR_W_DEFAULT,
  WSBAR_FONT_MIN,
  WSBAR_FONT_MAX,
  WSU_FONT_PX,
} from "./wsbar";

describe("clampWsbarWidth — 사이드바 폭", () => {
  it("범위 내 값은 정수 반올림 통과", () => {
    expect(clampWsbarWidth(300.4)).toBe(300);
    expect(clampWsbarWidth(216)).toBe(WSBAR_W_DEFAULT);
  });
  it("하한 클램프", () => {
    expect(clampWsbarWidth(80)).toBe(WSBAR_W_MIN);
  });
  it("상한 클램프", () => {
    expect(clampWsbarWidth(9999)).toBe(WSBAR_W_MAX);
  });
  it("비정상(NaN·Infinity)은 기본폭", () => {
    expect(clampWsbarWidth(NaN)).toBe(WSBAR_W_DEFAULT);
    expect(clampWsbarWidth(Infinity)).toBe(WSBAR_W_DEFAULT);
  });
});

describe("clampWsbarFont — 글자 배율", () => {
  it("범위 내 값은 소수 2자리 통과", () => {
    expect(clampWsbarFont(1.25)).toBe(1.25);
  });
  it("하한·상한 클램프", () => {
    expect(clampWsbarFont(0.3)).toBe(WSBAR_FONT_MIN);
    expect(clampWsbarFont(5)).toBe(WSBAR_FONT_MAX);
  });
  it("비정상(NaN·0·음수)은 1.0", () => {
    expect(clampWsbarFont(NaN)).toBe(1);
    expect(clampWsbarFont(0)).toBe(1);
    expect(clampWsbarFont(-2)).toBe(1);
  });
  it("부동소수 잔여 자릿수 절사(0.1 step 누적 안전)", () => {
    expect(clampWsbarFont(1.1 + 0.1 + 0.1)).toBe(1.3);
  });
});

// ── 사용량 패널 글자 크기의 단일 출처 (오너 지시 2026-08-10 — 20px 고정)
describe("WSU_FONT_PX — CSS와 산식이 같은 수를 쓰는가", () => {
  // ★이 테스트가 곧 「단일 출처」의 집행이다. 상수와 CSS는 서로를 못 읽으므로(TS↔CSS),
  //   묶어 두는 방법은 **선언을 읽어 대조하는 것**뿐이다. 한쪽만 고치면 여기서 적색이 난다.
  //   ⚠파일을 읽는 검사라 「대상에 닿았는가」를 먼저 단언한다 — 선택자를 못 찾으면 0건이
  //   통과로 읽히기 때문이다(부재를 세는 측정은 자기 실패를 조용히 성공으로 표현한다).
  it("style.css의 #ws-usage font-size 선언이 WSU_FONT_PX와 일치", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const block = css.match(/#ws-usage\s*\{([\s\S]*?)\}/);
    expect(block).not.toBeNull(); // 선택자 자체가 사라지면(이름 변경 등) 대조가 무의미해진다
    const decl = block![1].match(/(?:^|;|\*\/)\s*font-size:\s*([^;]+);/);
    expect(decl).not.toBeNull(); // 선언이 없으면 「어긋남 0」이 아니라 「재지 못했다」이다
    expect(decl![1].trim()).toBe(`${WSU_FONT_PX}px`);
  });

  it("배율 변수에 매여 있지 않다 — 두 배율 어디에도 비연동(오너 지시의 본체)", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const block = css.match(/#ws-usage\s*\{([\s\S]*?)\}/)![1];
    // 주석에는 옛 산식을 사유로 적어 두므로 주석을 걷어내고 선언만 본다.
    const decls = block.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(decls).not.toContain("--ui-chrome-scale");
    expect(decls).not.toContain("--wsbar-font");
  });
});

// ── 페인 CTX 행별 나이 칸의 자리 판정 (오너 발의 2026-08-08 · 20px 고정으로 재조준 2026-08-10)
describe("showsRowAge — 나이 칸을 낼 자리가 있는가", () => {
  // 임계 폭 = (10.115 + 2.4)em × 20px + 41px(경계선1 + padding16 + gap24) = 291.3px → 292px부터 참.
  // ★이 수는 상수에서 파생하지 않고 손으로 계산해 박는다 — 산식에서 유도하면 산식이 바뀔 때
  //   기대값도 같이 따라가 가드가 죽는다(같은 실수를 두 번 쓰는 셈이 된다).
  // ★292는 헤드리스 실측과 일치한다(291에서 트랙 47.7px < 하한 48px, 292에서 48.7px).
  const THRESHOLD_PX = 292;

  // ★기본 폭에서의 기대가 뒤집혔다(참→거짓). 규칙이 바뀐 게 아니라 **글자가 커졌다** —
  //   13.75px에서 216px는 자리가 됐지만 20px에서는 안 된다. 나이는 툴팁·푸터에 그대로 남는다.
  it("★기본 폭(216)에서는 내주지 않는다 — 20px 고정으로 행이 커져 자리가 없다", () => {
    expect(showsRowAge(WSBAR_W_DEFAULT)).toBe(false);
  });

  // ★이 케이스가 이 함수의 존재 이유다. 헤드리스 실측에서 176px는 트랙바가 0px가 되고
  //   행이 4.1px 넘쳐 잘렸다 — 사용자가 드래그로 도달할 수 있는 폭이므로 가정이 아니다.
  it("★최소 폭(WSBAR_W_MIN=176)에서는 내주지 않는다 — 트랙바가 0이 되고 행이 넘친다", () => {
    expect(showsRowAge(WSBAR_W_MIN)).toBe(false);
  });

  it("최대 폭에서는 당연히 낸다", () => {
    expect(showsRowAge(WSBAR_W_MAX)).toBe(true);
  });

  it("임계 폭 경계 — 294는 거짓, 295부터 참", () => {
    expect(showsRowAge(THRESHOLD_PX - 1)).toBe(false);
    expect(showsRowAge(THRESHOLD_PX)).toBe(true);
  });

  it("폭에 대해 단조다 — 넓힐수록 참, 좁힐수록 거짓(뒤집히는 구간이 없다)", () => {
    let seenTrue = false;
    for (let w = WSBAR_W_MIN; w <= WSBAR_W_MAX; w++) {
      const v = showsRowAge(w);
      if (v) seenTrue = true;
      else expect(seenTrue).toBe(false); // 참이었다가 다시 거짓이 되면 실패
    }
    expect(seenTrue).toBe(true);
  });

  // ← 옛 「메뉴 배율을 키우면 같은 폭이라도 내주지 않는다」 케이스를 여기로 재조준했다.
  //   지우지 않는 이유: 그 케이스가 지키던 것은 「배율이 판정에 관여한다」가 아니라
  //   **「판정이 무엇에 좌우되는가」** 였다. 답이 「폭과 배율」에서 「폭 하나」로 바뀌었을 뿐이다.
  it("판정은 폭 단독이다 — 메뉴 배율은 인자가 아니다(패널 글자 20px 고정)", () => {
    expect(showsRowAge.length).toBe(1); // 배율 인자를 되살리면 여기서 잡힌다
    // 옛 인자를 넘겨도 무시된다 — 배율 250%에서도 같은 폭이면 같은 판정이다.
    expect((showsRowAge as (w: number, s?: number) => boolean)(WSBAR_W_MAX, 2.5)).toBe(
      showsRowAge(WSBAR_W_MAX),
    );
  });

  it("비정상 입력은 거짓 — 자리를 모르면 새 칸을 내지 않는다(fail-safe)", () => {
    expect(showsRowAge(NaN)).toBe(false);
    expect(showsRowAge(Infinity)).toBe(false);
    expect(showsRowAge(-1)).toBe(false);
  });
});
