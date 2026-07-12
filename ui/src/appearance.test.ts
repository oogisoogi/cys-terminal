import { describe, expect, test } from "bun:test";
import { composeFontFamily, DEFAULT_FONT_STACK, FONT_CHOICES, ROLE_COLOR, roleDotColor } from "./appearance";

describe("composeFontFamily", () => {
  test("null·공백 = 기본 스택 그대로", () => {
    expect(composeFontFamily(null)).toBe(DEFAULT_FONT_STACK);
    expect(composeFontFamily("")).toBe(DEFAULT_FONT_STACK);
    expect(composeFontFamily("   ")).toBe(DEFAULT_FONT_STACK);
  });

  test("선택 폰트를 기본 스택 앞에 합성 — CJK 폴백 보존", () => {
    const fam = composeFontFamily("JetBrains Mono");
    expect(fam.startsWith("'JetBrains Mono', ")).toBe(true);
    expect(fam.endsWith(DEFAULT_FONT_STACK)).toBe(true);
  });

  test("따옴표 섞인 입력은 소거 후 인용 — CSS 리스트 파손 방지", () => {
    expect(composeFontFamily("'SF Mono'")).toBe(`'SF Mono', ${DEFAULT_FONT_STACK}`);
    expect(composeFontFamily('D2"Coding')).toBe(`'D2Coding', ${DEFAULT_FONT_STACK}`);
  });

  test("FONT_CHOICES 전 선택지가 유효 합성값을 낸다(기본값 포함)", () => {
    for (const c of FONT_CHOICES) {
      const fam = composeFontFamily(c.face);
      expect(fam.endsWith(DEFAULT_FONT_STACK)).toBe(true);
    }
  });
});

describe("roleDotColor", () => {
  test("무역할(일반 셸) = null → 점 숨김", () => {
    expect(roleDotColor(null)).toBeNull();
    expect(roleDotColor(undefined)).toBeNull();
    expect(roleDotColor("")).toBeNull();
  });

  test("정식 역할 5종은 CC 색상표와 일치", () => {
    for (const role of ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"]) {
      expect(roleDotColor(role)).toBe(ROLE_COLOR[role]);
    }
  });

  test("변형 역할은 접두 매칭 — 데몬 역할 변형 계약(overrides.rs·pack.rs)과 정합", () => {
    expect(roleDotColor("worker-2")).toBe(ROLE_COLOR.worker);
    expect(roleDotColor("cso-1")).toBe(ROLE_COLOR.cso);
    expect(roleDotColor("master-2")).toBe(ROLE_COLOR.master);
    expect(roleDotColor("reviewer")).toBe(ROLE_COLOR["reviewer-gemini"]);
  });

  test("미지 역할은 회색 폴백", () => {
    expect(roleDotColor("librarian")).toBe("#64748b");
  });

  test("4대 역할군 색이 서로 구별된다(오너 요구)", () => {
    const set = new Set([roleDotColor("master"), roleDotColor("cso"), roleDotColor("worker"), roleDotColor("reviewer-gemini")]);
    expect(set.size).toBe(4);
  });
});
