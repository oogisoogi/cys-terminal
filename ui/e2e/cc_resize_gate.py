#!/usr/bin/env python3
"""CC 창 비례 스케일 회귀 게이트 (수동 실행 — CI 미배선).

검증 항목 (2026-07-12 오너 승인 '비례 스케일' 스펙):
  1. 대시보드 탭(Live)의 본문·섹션이 창 크기에 비례해 커지고 작아진다
     (#cc-body zoom == --panel-zoom == ccscale.ts 산식).
  2. 오피스 탭은 zoom 1 고정(3D fit 카메라가 창 연동의 진실 — 이중 스케일 금지),
     iframe은 창을 100% 추종한다.
  3. body.cc-office 클래스가 탭 이탈 시 잔류하지 않는다.

사전 조건:
  sh ui/build.sh                                  # ui/dist 생성
  pip install playwright && playwright install chromium webkit

실행:  python3 ui/e2e/cc_resize_gate.py           # exit 0 = PASS
"""
import http.server
import json
import socket
import sys
import threading
from pathlib import Path

DIST = Path(__file__).resolve().parent.parent / "dist"

# ── ccscale.ts 산식 미러 (상수 변경 시 여기도 갱신 — 단위테스트는 ccscale.test.ts가 담당,
#    이 게이트는 "빌드 산출물이 그 산식대로 화면에 반영되는가"를 본다) ──
BASE_W, BASE_H, AUTO_MIN, AUTO_MAX, ZOOM_CAP = 820, 760, 0.7, 2.2, 2.5

def expected_zoom(w: int, h: int, manual: float = 1.0) -> float:
    s = min(w / BASE_W, h / BASE_H)
    s = min(AUTO_MAX, max(AUTO_MIN, s))
    return min(ZOOM_CAP, manual * s)

SHIM = """
window.__TAURI__ = {
  core: { invoke: (c,a) => new Promise(()=>{}) },
  event: { listen: (n,h) => Promise.resolve(()=>{}) },
};
"""

MEASURE = """
(() => {
  const cs = (sel, prop) => { const el = document.querySelector(sel); return el ? getComputedStyle(el)[prop] : null; };
  const r = (sel) => { const el = document.querySelector(sel); if (!el) return null;
    const b = el.getBoundingClientRect(); return {w: b.width, h: b.height}; };
  return {
    bodyClass: document.body.className,
    zoomVar: getComputedStyle(document.documentElement).getPropertyValue('--panel-zoom').trim(),
    bodyZoom: cs('#cc-body', 'zoom'),
    section: r('.cc-section'),
    officeFrame: r('#cc-office-frame'),
  };
})()
"""

SIZES = [(1400, 1000), (2200, 1300), (700, 600)]
failures: list[str] = []

def check(cond: bool, msg: str) -> None:
    tag = "PASS" if cond else "FAIL"
    print(f"  [{tag}] {msg}")
    if not cond:
        failures.append(msg)

def run_engine(pw, engine: str, port: int) -> None:
    print(f"── {engine}")
    browser = getattr(pw, engine).launch()
    page = browser.new_page(viewport={"width": 1400, "height": 1000})
    page.add_init_script(SHIM)
    page.goto(f"http://127.0.0.1:{port}/index.html")
    page.wait_for_timeout(400)
    page.click("#btn-cc")
    page.wait_for_timeout(300)

    # 오피스(기본 탭): zoom 1 고정 + iframe 창 추종
    for w, h in [(1400, 1000), (900, 700)]:
        page.set_viewport_size({"width": w, "height": h})
        page.wait_for_timeout(300)
        m = page.evaluate(MEASURE)
        check(float(m["bodyZoom"]) == 1, f"office {w}x{h}: bodyZoom==1 (got {m['bodyZoom']})")
        check(m["officeFrame"] and abs(m["officeFrame"]["w"] - (w - 40)) < 3,
              f"office {w}x{h}: iframe 창 추종 (got {m['officeFrame']})")

    # Live 탭: 비례 스케일 + 클래스 잔류 없음
    page.click('#cc-tabs .cc-tab[data-view="live"]')
    page.wait_for_timeout(200)
    m = page.evaluate(MEASURE)
    check("cc-office" not in m["bodyClass"], f"live: cc-office 클래스 잔류 없음 (got {m['bodyClass']!r})")
    for w, h in SIZES:
        page.set_viewport_size({"width": w, "height": h})
        page.wait_for_timeout(300)
        m = page.evaluate(MEASURE)
        exp = expected_zoom(w, h)
        got = float(m["zoomVar"])
        check(abs(got - exp) < 0.005, f"live {w}x{h}: --panel-zoom≈{exp:.3f} (got {got})")
        check(abs(float(m["bodyZoom"]) - exp) < 0.005, f"live {w}x{h}: #cc-body zoom 적용 (got {m['bodyZoom']})")
        # 섹션 시각 폭 = min(본문 780-패딩 40, 가용/zoom) × zoom — 비례 추종의 실측 증거
        avail = w / exp - 40
        exp_w = min(740, avail) * exp
        check(m["section"] and abs(m["section"]["w"] - exp_w) < 6,
              f"live {w}x{h}: 섹션 폭≈{exp_w:.0f}px (got {m['section'] and round(m['section']['w'])})")
    browser.close()

def main() -> int:
    if not (DIST / "index.html").exists():
        print(f"FAIL: {DIST}/index.html 없음 — 먼저 `sh ui/build.sh` 실행")
        return 2
    from playwright.sync_api import sync_playwright

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(DIST), **kw)
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    try:
        with sync_playwright() as pw:
            for engine in ("chromium", "webkit"):
                try:
                    run_engine(pw, engine, port)
                except Exception as e:  # webkit 미설치 등 — 엔진 단위로 건너뛰되 chromium 0개 실행은 실패
                    print(f"  [SKIP] {engine}: {str(e)[:120]}")
                    if engine == "chromium":
                        failures.append(f"chromium 실행 불가: {e}")
    finally:
        httpd.shutdown()
    print(f"\n{'PASS' if not failures else 'FAIL'} — 실패 {len(failures)}건")
    for f in failures:
        print(f"  ✗ {f}")
    return 1 if failures else 0

if __name__ == "__main__":
    sys.exit(main())
