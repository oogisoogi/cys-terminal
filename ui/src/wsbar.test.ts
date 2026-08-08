// wsbar.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 사이드바 폭·글자 배율 클램프가 경계·비정상 입력에서 결정론인지 검증한다.
import { describe, it, expect } from "bun:test";
import {
  clampWsbarWidth,
  clampWsbarFont,
  showsRowAge,
  WSBAR_W_MIN,
  WSBAR_W_MAX,
  WSBAR_W_DEFAULT,
  WSBAR_FONT_MIN,
  WSBAR_FONT_MAX,
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

// ── 페인 CTX 행별 나이 칸의 자리 판정 (오너 발의 2026-08-08)
describe("showsRowAge — 나이 칸을 낼 자리가 있는가", () => {
  const S = 1.25; // --ui-chrome-scale 기본(메뉴 크기 125%)

  it("기본 폭(216)에서는 낸다 — 오너가 실제로 보는 화면", () => {
    expect(showsRowAge(WSBAR_W_DEFAULT, S)).toBe(true);
  });

  // ★이 케이스가 이 함수의 존재 이유다. 헤드리스 실측에서 176px는 트랙바가 0px가 되고
  //   행이 4.1px 넘쳐 잘렸다 — 사용자가 드래그로 도달할 수 있는 폭이므로 가정이 아니다.
  it("★최소 폭(WSBAR_W_MIN=176)에서는 내주지 않는다 — 트랙바가 0이 되고 행이 넘친다", () => {
    expect(showsRowAge(WSBAR_W_MIN, S)).toBe(false);
  });

  it("최대 폭에서는 당연히 낸다", () => {
    expect(showsRowAge(WSBAR_W_MAX, S)).toBe(true);
  });

  it("폭에 대해 단조다 — 넓힐수록 참, 좁힐수록 거짓(뒤집히는 구간이 없다)", () => {
    let seenTrue = false;
    for (let w = WSBAR_W_MIN; w <= WSBAR_W_MAX; w++) {
      const v = showsRowAge(w, S);
      if (v) seenTrue = true;
      else expect(seenTrue).toBe(false); // 참이었다가 다시 거짓이 되면 실패
    }
    expect(seenTrue).toBe(true);
  });

  it("메뉴 배율을 키우면 같은 폭이라도 내주지 않는다 — 칸 폭이 em이라 글자를 따라 커진다", () => {
    expect(showsRowAge(WSBAR_W_DEFAULT, 1.25)).toBe(true);
    expect(showsRowAge(WSBAR_W_DEFAULT, 2.5)).toBe(false); // 메뉴 크기 250%
    expect(showsRowAge(WSBAR_W_DEFAULT, 0.8)).toBe(true); // 80%면 여유가 더 생긴다
  });

  it("비정상 입력은 거짓 — 자리를 모르면 새 칸을 내지 않는다(fail-safe)", () => {
    expect(showsRowAge(NaN, S)).toBe(false);
    expect(showsRowAge(WSBAR_W_DEFAULT, NaN)).toBe(false);
    expect(showsRowAge(WSBAR_W_DEFAULT, 0)).toBe(false);
    expect(showsRowAge(WSBAR_W_DEFAULT, -1)).toBe(false);
  });
});
