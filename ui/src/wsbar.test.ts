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

// ── 사용량 패널 글자 크기의 단일 출처
//    (오너 지시 2026-08-10 = 기준 20px · 오너 판정 2026-08-11 = 사이드바 배율에 연동)
describe("WSU_FONT_PX — CSS와 산식이 같은 수를 쓰는가", () => {
  // ★이 테스트가 곧 「단일 출처」의 집행이다. 상수와 CSS는 서로를 못 읽으므로(TS↔CSS),
  //   묶어 두는 방법은 **선언을 읽어 대조하는 것**뿐이다. 한쪽만 고치면 여기서 적색이 난다.
  //   ⚠파일을 읽는 검사라 「대상에 닿았는가」를 먼저 단언한다 — 선택자를 못 찾으면 0건이
  //   통과로 읽히기 때문이다(부재를 세는 측정은 자기 실패를 조용히 성공으로 표현한다).
  // ★대조 축이 「값」에서 「수식」으로 옮겨졌다(2026-08-11). 가드를 지우지 않고 다시 겨눈 것이다 —
  //   지키던 것은 「20px이다」가 아니라 **「CSS와 상수가 같은 수를 쓴다」**였고 그것은 그대로다.
  it("style.css의 #ws-usage font-size 선언이 기준값×사이드바 배율 수식이다", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const block = css.match(/#ws-usage\s*\{([\s\S]*?)\}/);
    expect(block).not.toBeNull(); // 선택자 자체가 사라지면(이름 변경 등) 대조가 무의미해진다
    const decl = block![1].match(/(?:^|;|\*\/)\s*font-size:\s*([^;]+);/);
    expect(decl).not.toBeNull(); // 선언이 없으면 「어긋남 0」이 아니라 「재지 못했다」이다
    // 공백만 관용하고 나머지는 그대로 대조한다 — 기준값·배율변수·폴백 셋 중 하나만 바뀌어도 적색.
    // ★폴백 1까지 대조에 넣는 이유: showsRowAge가 배율 미지정을 1.0으로 푸는 것과 **같은 약속**이라
    //   한쪽만 바뀌면 화면과 자리 판정이 어긋난다(그 어긋남은 화면에 조용히 잘림으로만 나타난다).
    expect(decl![1].replace(/\s+/g, " ").trim()).toBe(
      `calc(${WSU_FONT_PX}px * var(--wsbar-font, 1))`,
    );
  });

  it("사이드바 배율에는 매이고 메뉴 배율에는 안 매인다 — 축이 갈린 것이 이 결정의 본체", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const block = css.match(/#ws-usage\s*\{([\s\S]*?)\}/)![1];
    // 주석에는 두 배율의 이름을 사유로 적어 두므로 주석을 걷어내고 선언만 본다.
    const decls = block.replace(/\/\*[\s\S]*?\*\//g, "");
    // c48dbdf가 고친 원 사고(메뉴 배율을 따라가며 목록과 따로 놀던 것)의 재발 금지 — 이 축은 계속 비연동.
    expect(decls).not.toContain("--ui-chrome-scale");
    // 오너 판정 2026-08-11 — 목록(.ws-tab)이 매인 바로 그 축에 함께 매인다.
    expect(decls).toContain("--wsbar-font");
  });

  // ── 사이드바 전면 통합 배율 (오너 지시 2026-08-11 확장 ⑵⑶)
  //    「A−/A＋ 하나로 사이드바가 통째로 커지고 작아진다」를 이름이 아니라 **선언으로** 단언한다.
  //    ⚠이 목록은 손으로 관리한다 — 새 사이드바 요소를 추가하면서 여기 안 적으면 그 요소만
  //      옛 크기로 남는다. 그 누락을 잡는 것이 아래 「사이드바 전역 훑기」 케이스다.
  const SIDEBAR_SELECTORS = [
    "#wsbar-head button", // 상단 버튼 6종(A−·A＋·▶CEO·▶부서장·＋부서·＋)을 한 자리에서
    ".ws-tab .ws-name", // 워크스페이스 제목
    ".ws-tab .ws-sub", // 부제(페인 수·데몬)
    ".ws-group-head", // 그룹 머리글
    "#ws-usage", // 토큰/CTX 사용량 패널
    // ↓ 서열 교정 2026-08-11에 제목 비율(×20/13)로 함께 옮긴 칸들. 여기 안 적으면 「사이드바가
    //   통째로 움직인다」가 이름만 남고, 이 칸들만 조용히 뒤처져도 아무도 모른다.
    ".ws-tab .ws-close", // 닫기 ×(제목 행)
    ".ws-approve-badge", // 승인 대기 배지(제목 행)
    ".ws-group-count", // 그룹 멤버 수
    ".ws-group-add", // 그룹 ＋
  ];
  it.each(SIDEBAR_SELECTORS)("%s 의 글자 크기가 사이드바 배율에 매여 있다", (sel: string) => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const esc = sel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const block = css.match(new RegExp(`(?:^|\\n)${esc}\\s*\\{([\\s\\S]*?)\\}`));
    expect(block).not.toBeNull(); // 선택자가 사라졌으면 대조 대상이 없어진 것이다(0건 통과 금지)
    const decls = block![1].replace(/\/\*[\s\S]*?\*\//g, "");
    expect(decls).toContain("font-size");
    expect(decls).toContain("var(--wsbar-font, 1)");
    expect(decls).not.toContain("--ui-chrome-scale");
  });

  it("★사이드바 전역 훑기 — #wsbar 계열 선언 중 메뉴 배율을 쓰는 것이 하나도 없다", () => {
    // ★이 케이스가 위 손목록의 구멍을 메운다: 목록에 안 적은 요소가 메뉴 배율을 쓰면 여기서 잡힌다.
    //   (반대 방향 — 크기 선언 자체가 없는 새 요소 — 은 CSS만으로는 못 잡으므로 헤드리스 실측이 진다.)
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const offenders: string[] = [];
    for (const m of css.matchAll(/(?:^|\n)([^\n{}]*(?:wsbar|ws-tab|ws-group|ws-usage|wsu-|ws-approve)[^\n{}]*)\{([\s\S]*?)\}/g)) {
      const decls = m[2].replace(/\/\*[\s\S]*?\*\//g, "");
      if (decls.includes("--ui-chrome-scale")) offenders.push(m[1].trim());
    }
    expect(offenders).toEqual([]);
  });

  it("★「워크스페이스」 라벨이 헤더에서 사라졌다 — 그 자리를 버튼 글자에 내줬다(오너 지시 ⑴)", () => {
    const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
    const head = html.match(/<div id="wsbar-head">([\s\S]*?)<\/div>/);
    expect(head).not.toBeNull();
    expect((head![1].match(/<button/g) ?? []).length).toBe(6); // 버튼은 하나도 잃지 않았다
    expect(head![1]).not.toContain("<span"); // 라벨 자리(span)가 통째로 없어야 한다
    // ★「'워크스페이스'라는 글자가 없다」로 재면 안 된다 — ＋ 버튼의 title="새 워크스페이스"에
    //   그 낱말이 남아 있고 그것은 **툴팁이지 라벨이 아니다**(초판이 여기서 거짓 적색을 냈다).
    //   재야 하는 것은 「헤더에 버튼 말고 보이는 것이 없다」이므로 버튼을 걷어낸 나머지를 본다.
    const outsideButtons = head![1].replace(/<button[\s\S]*?<\/button>/g, "").trim();
    expect(outsideButtons).toBe("");
    // 라벨을 지웠으면 그 라벨만 가리키던 CSS 규칙도 함께 사라져야 한다(죽은 선택자 금지).
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    expect(css).not.toContain("#wsbar-head > span");
  });

  // ── 기준 크기의 **서열** (오너 캡처 판정 2026-08-11 · 서열 교정)
  //    ★재는 것이 값에서 **서열**로 바뀌었다. 오너의 요구는 「패널은 16px이다」가 아니라
  //      「제목이 상단 버튼·패널보다 크고, 그 둘은 서로 같다」이다. 값을 박아 두면 나중에 셋을
  //      다 같은 방향으로 옮길 때 요구가 지켜지는데도 적색이 나고(가짜 적색), 반대로 한 칸만
  //      틀어졌을 때 값 대조는 그 칸만 잡고 **관계가 깨진 사실**은 말해 주지 않는다.
  //      그래서 세 선언을 다 읽어 서로 비교한다 — 기대값은 부등호이지 수가 아니다.
  const BASE_PX_OF = (css: string, sel: string): number => {
    const esc = sel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const block = css.match(new RegExp(`(?:^|\\n)${esc}\\s*\\{([\\s\\S]*?)\\}`));
    expect(block).not.toBeNull(); // 선택자를 못 찾으면 「어긋남 0」이 아니라 **재지 못한 것**이다
    const decls = block![1].replace(/\/\*[\s\S]*?\*\//g, "");
    const m = decls.match(/font-size:\s*calc\(\s*([\d.]+)px\s*\*\s*var\(--wsbar-font,\s*1\)\s*\)/);
    expect(m).not.toBeNull(); // 수식 모양이 바뀌면 기준값을 못 읽는다 — 조용히 0으로 떨어지지 않게
    return parseFloat(m![1]);
  };

  it("★서열 — 목록 제목 > 상단 버튼 = 사용량 패널 (오너 요구의 본체)", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const title = BASE_PX_OF(css, ".ws-tab .ws-name");
    const button = BASE_PX_OF(css, "#wsbar-head button");
    const panel = BASE_PX_OF(css, "#ws-usage");
    // ⑴ 상단 메뉴와 토큰/CTX 패널은 **같은** 기준 크기 (오너 요구 1)
    expect(panel).toBe(button);
    // ⑵ 목록 제목은 그 둘보다 **크다** (오너 요구 2 · 캡처가 잡은 역전의 반대 방향)
    expect(title).toBeGreaterThan(button);
  });

  it("보조행은 제목보다 작고 상단 버튼보다는 크다 — 목록이 한 덩어리로 읽히는 폭", () => {
    // ★보조행에 하한이 없으면 「제목만 키우기」로 위 서열을 통과시키면서 목록이 두 크기로 갈릴 수 있다.
    //   지시가 「제목에 비례한 현행 비율 유지」라고 못박은 이유가 이것이라 부등호 둘로 가둔다.
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const title = BASE_PX_OF(css, ".ws-tab .ws-name");
    const sub = BASE_PX_OF(css, ".ws-tab .ws-sub");
    expect(sub).toBeLessThan(title);
    expect(sub).toBeGreaterThan(BASE_PX_OF(css, "#wsbar-head button"));
  });

  it("★목록 안 어느 칸도 사용량 패널보다 작지 않다 — 역전이 자리만 바꿔 되살아나는 것을 막는다", () => {
    // ★오너가 본 결함은 「제목이 패널보다 작다」였지만, 결함의 **부류**는 「목록이 패널보다 작다」이다.
    //   제목만 고치면 그룹 머리글(12px)이 그 자리를 물려받는다 — 그래서 목록 전체를 훑는다.
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const panel = BASE_PX_OF(css, "#ws-usage");
    const listSelectors = [
      ".ws-tab .ws-name",
      ".ws-tab .ws-sub",
      ".ws-tab .ws-close",
      ".ws-approve-badge",
      ".ws-group-head",
      ".ws-group-add",
    ];
    const smaller = listSelectors.filter((s) => BASE_PX_OF(css, s) < panel);
    expect(smaller).toEqual([]);
    // ★.ws-group-count(멤버 수)는 위 목록에서 뺐다 — 그룹 머리글 안의 **부속 수치**라 머리글보다
    //   작은 것이 의도다(보조행이 제목보다 작은 것과 같은 관계). 뺀 것을 숨기지 않고 여기 적고,
    //   대신 그 칸에는 **머리글에 대한 비율**로 하한을 건다(초판 10:12 = .833).
    //   ⚠하한을 「패널의 절반」 같은 헐거운 수로 잡으면 이 칸만 옛 10px으로 되돌려도 통과한다
    //     (10 > 8). 비율로 매어야 되돌림이 적색이 된다 — 자기감사에서 잡아 조인 자리다.
    const count = BASE_PX_OF(css, ".ws-group-count");
    const groupHead = BASE_PX_OF(css, ".ws-group-head");
    expect(count).toBeLessThan(groupHead);
    expect(count / groupHead).toBeGreaterThan(0.6);
  });

  it("목록(.ws-tab)과 같은 배율 변수를 쓴다 — 「함께 커진다」의 기계 단언", () => {
    // ★이 단언이 없으면 패널이 **어떤** 배율에 매였는지가 위 테스트에서 이름으로만 확인된다.
    //   둘이 같은 변수를 쓰는 것이 오너 요구(목록과 패널이 함께 커진다)의 실제 내용이다.
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const tab = css.match(/\.ws-tab \.ws-name\s*\{([\s\S]*?)\}/);
    expect(tab).not.toBeNull(); // 목록 쪽 선택자가 사라졌다면 대조 대상이 없어진 것이다
    expect(tab![1]).toContain("var(--wsbar-font, 1)");
  });
});

// ── 페인 CTX 행별 나이 칸의 자리 판정
//    (오너 발의 2026-08-08 · 20px 고정으로 재조준 2026-08-10 · 사이드바 배율 복귀 2026-08-11)
describe("showsRowAge — 나이 칸을 낼 자리가 있는가", () => {
  // 임계 폭 = (10.115 + 2.4)em × 16px × 배율 + 41px(경계선1 + padding16 + gap24).
  //   배율 1.0 → 241.24px → 242px부터 참 · 배율 0.8 → 201.19px → 202px부터 참
  //   배율 2.2 → 481.53px → 482px부터 참 (**상한 520px 안으로 들어왔다** — 아래 ★ 참조)
  // ★이 수들은 상수에서 파생하지 않고 손으로 계산해 박는다 — 산식에서 유도하면 산식이 바뀔 때
  //   기대값도 같이 따라가 가드가 죽는다(같은 실수를 두 번 쓰는 셈이 된다).
  // ★세 수가 전부 내려온 이유는 하나다: 패널 기준이 20 → 16px(서열 교정 2026-08-11 후속).
  //   행의 고정 칸은 전부 em이라 **글자가 작아지면 행이 좁아지고 자리가 생긴다**. 즉 아래 기대값이
  //   바뀐 것은 규칙이 흔들린 것이 아니라 규칙의 입력이 바뀐 것이다 — 그래서 케이스를 지우지 않고
  //   같은 축(임계 경계·단조성)에 **다시 겨눴다**.
  // ★배율 상한에서의 판정이 뒤집혔다(전량 거짓 → 482px부터 참). 이것은 기대값 조정이 아니라
  //   **화면 사실의 변화**이므로 산식을 믿지 않고 헤드리스로 직접 확인했다 — 세 경계 전부 일치:
  //     241px 트랙 38.19 < 하한 38.40(거짓) · 242px 39.19 ≥ 38.40(참)          [배율 1.0]
  //     201px 트랙 30.56 < 하한 30.72(거짓) · 202px 31.56 ≥ 30.72(참)          [배율 0.8]
  //     481px 트랙 83.98 < 하한 84.48(거짓) · 482px 84.98 ≥ 84.48(참)          [배율 2.2]
  //   ⚠실측 fixture에는 `has-named`를 걸어야 한다 — CTX_ROW_FIXED_EM의 첫 항 4.2em이 바로 그
  //     클래스의 라벨 열 폭이다. 안 걸면 2.6em짜리 **다른 화면**을 재게 되고(초판이 그랬다),
  //     산식이 화면보다 보수적인 것처럼 보이는 거짓 불일치가 난다.
  const THRESHOLD_PX = 242; // 배율 1.0
  const THRESHOLD_PX_MIN_FONT = 202; // 배율 0.8
  const THRESHOLD_PX_MAX_FONT = 482; // 배율 2.2

  // ★기본 폭에서의 기대가 뒤집혔던 자리다(참→거짓·2026-08-10). 규칙이 바뀐 게 아니라
  //   **글자가 커졌다** — 13.75px에서 216px는 자리가 됐지만 20px에서는 안 된다.
  //   나이는 툴팁·푸터에 그대로 남는다.
  it("★기본 폭(216)·배율 1.0에서는 내주지 않는다 — 기준 16px으로도 임계 242px에 26px 모자란다", () => {
    expect(showsRowAge(WSBAR_W_DEFAULT, 1)).toBe(false);
  });

  // ★이 케이스가 이 함수의 존재 이유다. 헤드리스 실측에서 176px는 트랙바가 0px가 되고
  //   행이 4.1px 넘쳐 잘렸다 — 사용자가 드래그로 도달할 수 있는 폭이므로 가정이 아니다.
  it("★최소 폭(WSBAR_W_MIN=176)에서는 내주지 않는다 — 트랙바가 0이 되고 행이 넘친다", () => {
    expect(showsRowAge(WSBAR_W_MIN, 1)).toBe(false);
  });

  it("최대 폭·배율 1.0에서는 당연히 낸다", () => {
    expect(showsRowAge(WSBAR_W_MAX, 1)).toBe(true);
  });

  it("임계 폭 경계(배율 1.0) — 241은 거짓, 242부터 참", () => {
    expect(showsRowAge(THRESHOLD_PX - 1, 1)).toBe(false);
    expect(showsRowAge(THRESHOLD_PX, 1)).toBe(true);
  });

  it("폭에 대해 단조다 — 넓힐수록 참, 좁힐수록 거짓(뒤집히는 구간이 없다)", () => {
    let seenTrue = false;
    for (let w = WSBAR_W_MIN; w <= WSBAR_W_MAX; w++) {
      const v = showsRowAge(w, 1);
      if (v) seenTrue = true;
      else expect(seenTrue).toBe(false); // 참이었다가 다시 거짓이 되면 실패
    }
    expect(seenTrue).toBe(true);
  });

  // ← 옛 「판정은 폭 단독이다」 케이스(2026-08-10)를 여기로 다시 겨눴다. 두 번 다 지키던 것은
  //   **「판정이 무엇에 좌우되는가」**이지 특정 답이 아니다. 답은 「폭과 메뉴 배율」→「폭 하나」→
  //   「폭과 **사이드바** 배율」로 옮겨 왔고, 축이 갈린 것이 오너 판정 2026-08-11의 내용이다.
  it("판정은 폭과 사이드바 배율 둘이다 — 배율이 커지면 같은 폭이라도 내주지 않는다", () => {
    expect(showsRowAge.length).toBe(2); // 인자를 다시 떼면 여기서 잡힌다
    // ★같은 **한 폭**에서 배율만 달라 판정이 갈리는 것이 「배율이 산식에 들어갔다」의 증거다.
    //   전에는 상한 폭(520)에서 이 대비가 났지만, 패널이 16px으로 작아지며 520은 배율 2.2에서도
    //   자리가 남게 됐다 — 그래서 대비가 나는 폭(임계 242)으로 **겨냥을 옮겼다**(축은 그대로).
    expect(showsRowAge(THRESHOLD_PX, 1)).toBe(true);
    expect(showsRowAge(THRESHOLD_PX, WSBAR_FONT_MAX)).toBe(false);
  });

  it("배율 하한(0.8)에서는 더 좁은 폭부터 낸다 — 임계 201은 거짓, 202부터 참", () => {
    expect(showsRowAge(THRESHOLD_PX_MIN_FONT - 1, WSBAR_FONT_MIN)).toBe(false);
    expect(showsRowAge(THRESHOLD_PX_MIN_FONT, WSBAR_FONT_MIN)).toBe(true);
  });

  // ★옛 케이스는 「배율 2.2에서는 어느 폭에서도 안 낸다」였다. 그 답이 참이었던 이유는 임계 592px가
  //   폭 상한 520px **밖**이었기 때문이고, 패널이 16px으로 작아지며 임계가 482px로 들어왔다.
  //   지키던 축(배율 상한에서의 자리 판정)은 그대로 두고 **경계 케이스로 다시 겨눈다** —
  //   케이스를 지우면 배율 상한 구간이 통째로 무검사가 된다.
  it("배율 상한(2.2) 경계 — 481은 거짓, 482부터 참 (기본폭·최소폭에서는 계속 거짓)", () => {
    expect(showsRowAge(THRESHOLD_PX_MAX_FONT - 1, WSBAR_FONT_MAX)).toBe(false);
    expect(showsRowAge(THRESHOLD_PX_MAX_FONT, WSBAR_FONT_MAX)).toBe(true);
    expect(showsRowAge(WSBAR_W_DEFAULT, WSBAR_FONT_MAX)).toBe(false);
    expect(showsRowAge(WSBAR_W_MIN, WSBAR_FONT_MAX)).toBe(false);
  });

  it("배율에 대해 단조다 — 키울수록 거짓 쪽으로만 간다(뒤집히는 구간이 없다)", () => {
    // ★폭 단조성과 같은 이유의 검사다: 사용자가 A＋를 누르는 도중 칸이 사라졌다 나타났다 하면
    //   그것은 규칙이 아니라 고장으로 읽힌다.
    // ★재는 폭을 상한(520)에서 임계(242)로 옮겼다 — 패널이 16px으로 작아진 뒤 상한 폭은 배율
    //   전 구간에서 참이라 **뒤집히는 구간 자체가 없어** 단조성을 잴 수 없다(항상 초록 = 무검사).
    //   폭을 바꾼 것은 기대값 완화가 아니라 **측정이 대상에 닿게 만든 것**이다.
    let seenFalse = false;
    for (let f = WSBAR_FONT_MIN; f <= WSBAR_FONT_MAX + 1e-9; f += 0.1) {
      const v = showsRowAge(THRESHOLD_PX, +f.toFixed(2));
      if (!v) seenFalse = true;
      else expect(seenFalse).toBe(false); // 거짓이었다가 다시 참이 되면 실패
    }
    expect(seenFalse).toBe(true);
  });

  it("폭이 비정상이면 거짓 — 자리를 모르면 새 칸을 내지 않는다(fail-safe)", () => {
    expect(showsRowAge(NaN, 1)).toBe(false);
    expect(showsRowAge(Infinity, 1)).toBe(false);
    expect(showsRowAge(-1, 1)).toBe(false);
  });

  it("★배율이 비정상이면 1.0으로 푼다 — 폭과 달리 fail-safe가 아니다(CSS 폴백과 같은 약속)", () => {
    // 비대칭이 의도다. CSS는 var(--wsbar-font, 1)로 그리므로, 변수가 없을 때 화면은 실제로
    // 20px이다. 여기서 거짓을 내면 화면은 자리가 있는데 판정만 없다고 해 칸이 사라진다.
    for (const bad of [NaN, Infinity, 0, -1]) {
      expect(showsRowAge(WSBAR_W_MAX, bad)).toBe(showsRowAge(WSBAR_W_MAX, 1));
      expect(showsRowAge(WSBAR_W_DEFAULT, bad)).toBe(showsRowAge(WSBAR_W_DEFAULT, 1));
    }
  });
});
