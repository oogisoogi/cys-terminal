#!/usr/bin/env python3
"""오피스 3D 직원 디테일 12종 E2E 게이트 (수동 실행 — CI 미배선).

무엇을 검증하나:
  office3d.html 이 계약 v1.1(docs/DESIGN-office-detail-v11.md)의 디테일 12종을
  실제로 렌더·반응하는지. WebGL 화면은 직접 단언 불가하므로 office3d.html 의
  `?debug=1` 훅(window.__officeDebug)이 노출하는 상태 카운터/스냅샷을 단언한다.

구성:
  (a) mock 서버 — office3d.html·/vendor 정적 서빙 + /world(합성: 2부서×4노드,
      presence 전종·ctx 15/65/95·activity 0.1/0.9·rate·progress·run·blocked·
      kanban 6태스크·review 2건·board 히트+비용) + /stream SSE 각본 프레임
      (progress·runcard failed/succeeded·blocked/unblocked·verdict·doc from→to /
       from null·patch_top kanban/review/board/blocked·patch ctx→critical).
  (b) Playwright(chromium)로 ?debug=1 로드 → 스냅샷·카운터 단언 → 스크린샷.

사전 조건:  pip install playwright && playwright install chromium
실행:      python3 ui/e2e/office_detail_gate.py     # exit 0 = PASS
"""
import http.server
import json
import socket
import sys
import threading
import time
from pathlib import Path

WEB_DIR = Path(__file__).resolve().parents[2] / "cysjavis-pack" / "web"
SHOT = Path(__file__).resolve().parent / "office_detail_snapshot.png"
NOW = int(time.time())

# ── 합성 월드 (계약 §2 스키마) ────────────────────────────────────────────
def rate(label, used, resets=None):
    return {"label": label, "used_pct": used, "resets_at": resets or (NOW + 3600)}

def heat_row(base):
    return [round(min(1.0, base + (h % 6) * 0.12), 2) for h in range(24)]

NODES_D1 = [
    {"key": "surface:11", "role": "master", "presence": "active", "presence_conf": 0.9,
     "ctx": {"pct": 65}, "activity": 0.9, "agent": "claude",
     "task": "메타버스 오피스 디테일 12종 프론트 구현 및 E2E 게이트 하네스 작성 — 계약 v1.1 준수",
     "rate": [rate("5h", 80)], "flags": [], "idle_secs": 0,
     "progress": {"task": "office-detail", "stage": "구현", "pct": 42, "detail": "링"},
     "run": {"queued": 2, "active": {"task": "build", "started": NOW}, "done_today": 5, "failed_today": 1}},
    {"key": "surface:12", "role": "worker", "presence": "waiting", "presence_conf": 0.8,
     "ctx": {"pct": 15}, "activity": 0.1, "agent": "claude", "task": "백엔드 브리지 대기",
     "rate": [], "flags": [], "idle_secs": 0,
     "progress": {"task": "bridge", "stage": "대기", "pct": 10},
     "run": {"queued": 5, "active": None, "done_today": 1, "failed_today": 0}},
    {"key": "surface:13", "role": "reviewer-codex", "presence": "drowsy", "presence_conf": 0.7,
     "ctx": {"pct": 95}, "activity": 0.0, "agent": "codex", "task": "리뷰 대기 중 졸음",
     "rate": [rate("5h", 95)], "flags": ["ctx_critical"], "idle_secs": 180, "progress": None, "run": None},
    {"key": "surface:14", "role": "reviewer-agy", "presence": "sleeping", "presence_conf": 0.6,
     "ctx": None, "activity": 0.0, "agent": "agy", "task": "수면", "rate": [],
     "flags": [], "idle_secs": 600, "progress": None, "run": None},
]
NODES_D2 = [
    {"key": "surface:21", "role": "cso", "presence": "quiescing", "presence_conf": 0.9,
     "ctx": {"pct": 95}, "activity": 0.5, "agent": "claude", "task": "컨텍스트 정리",
     "rate": [], "flags": ["ctx_critical"], "idle_secs": 0, "progress": None, "run": None},
    {"key": "surface:22", "role": "worker", "presence": "active", "presence_conf": 0.9,
     "ctx": {"pct": 40}, "activity": 0.7, "agent": "claude", "task": "테스트 작성",
     "rate": [], "flags": [], "idle_secs": 0, "progress": None,
     "run": {"queued": 1, "active": {"task": "pytest", "started": NOW}, "done_today": 3, "failed_today": 0}},
    {"key": "surface:23", "role": "worker", "presence": "dead", "presence_conf": 0.5,
     "ctx": None, "activity": 0.0, "agent": None, "task": "", "rate": [],
     "flags": [], "idle_secs": 0, "progress": None, "run": None},
    {"key": "surface:24", "role": "reviewer-gemini", "presence": "waiting", "presence_conf": 0.8,
     "ctx": {"pct": 72}, "activity": 0.3, "agent": "gemini", "task": "리뷰 준비",
     "rate": [rate("weekly", 50)], "flags": [], "idle_secs": 0, "progress": None, "run": None},
]

def world():
    return {
        "v": 1, "seq": 1, "daemon": {"version": "0.12.34", "paused": False},
        "departments": [
            {"id": "dept-1", "floor": 1, "nodes": NODES_D1},
            {"id": "dept-2", "floor": 2, "nodes": NODES_D2},
        ],
        "lobby": {"unassigned": []}, "server_room": [], "todo": {},
        "blocked": [{"task": "task-A", "blocked_by": ["surface:11"], "key": "surface:12", "ts": NOW}],
        "kanban": {"ts": NOW, "tasks": [
            {"id": "t1", "title": "게이지 구현", "status": "done", "owner": "master", "blocked_by": []},
            {"id": "t2", "title": "칸반 벽면", "status": "doing", "owner": "master", "blocked_by": []},
            {"id": "t3", "title": "브리지 스캔", "status": "todo", "owner": "worker", "blocked_by": []},
            {"id": "t4", "title": "리뷰 라운드", "status": "blocked", "owner": "worker", "blocked_by": ["t2"]},
            {"id": "t5", "title": "전광판", "status": "todo", "owner": "cso", "blocked_by": []},
            {"id": "t6", "title": "배달 모션", "status": "done", "owner": "worker", "blocked_by": []},
        ]},
        "review": {"ts": NOW, "items": [
            {"reviewer": "codex", "verdict": "REVISE", "target": "office3d", "ts": NOW - 60},
            {"reviewer": "agy", "verdict": "ACCEPT", "target": "office3d", "ts": NOW},
        ]},
        "board": {"heat": {"surface:11": heat_row(0.3), "surface:12": heat_row(0.1),
                           "surface:21": heat_row(0.5)},
                  "cost_today": {"usd": 12.34, "tokens": 456000}},
    }

# ── /stream 각본 (초기 world 후 순차 방출) ──────────────────────────────────
def script():
    return [
        {"t": "world", "world": world()},
        {"t": "fx", "kind": "progress", "key": "surface:11", "task": "office-detail", "stage": "검증", "pct": 60},
        {"t": "fx", "kind": "runcard", "key": "surface:11", "phase": "failed", "task": "build", "summary": "타입 에러"},
        {"t": "fx", "kind": "runcard", "key": "surface:22", "phase": "succeeded", "task": "pytest"},
        {"t": "fx", "kind": "blocked", "task": "task-A", "blocked_by": ["surface:11"], "key": "surface:12"},
        {"t": "fx", "kind": "unblocked", "task": "task-A", "key": "surface:12"},
        {"t": "fx", "kind": "verdict", "reviewer": "codex", "verdict": "BLOCK", "target": "office3d"},
        {"t": "fx", "kind": "doc", "from": "surface:11", "to": "surface:12", "bytes": 320},   # 같은 층 → 보행 배달
        {"t": "fx", "kind": "doc", "from": None, "to": "surface:13", "bytes": 128},           # from null → 아크
        {"t": "patch_top", "field": "kanban", "value": world()["kanban"]},
        {"t": "patch_top", "field": "review", "value": {"ts": NOW, "items": [
            {"reviewer": "agy", "verdict": "ACCEPT", "target": "office3d", "ts": NOW + 1}]}},
        {"t": "patch_top", "field": "blocked",
         "value": [{"task": "task-A", "blocked_by": ["surface:11"], "key": "surface:12", "ts": NOW}]},
        {"t": "patch_top", "field": "board", "value": {
            "heat": {"surface:11": heat_row(0.6), "surface:22": heat_row(0.2)},
            "cost_today": {"usd": 20.01, "tokens": 512000}}},
        {"t": "patch", "key": "surface:12", "node": dict(NODES_D1[1], ctx={"pct": 95})},   # ctx→critical
    ]

STOP = threading.Event()

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):  # 조용히
        pass

    def _json(self, obj):
        body = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        self._json({"ok": True})

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/stream":
            return self._stream()
        if path == "/world":
            return self._json(world())
        if path == "/history":
            return self._json({"events": []})
        if path == "/peek":
            return self._json({"ok": True, "lines": []})
        # 정적 파일
        rel = "office3d.html" if path in ("/", "") else path.lstrip("/")
        fp = (WEB_DIR / rel).resolve()
        if not str(fp).startswith(str(WEB_DIR)) or not fp.is_file():
            self.send_response(404); self.end_headers(); return
        data = fp.read_bytes()
        ctype = ("text/html" if fp.suffix == ".html"
                 else "application/javascript" if fp.suffix in (".js", ".mjs")
                 else "application/octet-stream")
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _stream(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "keep-alive")
        self.end_headers()
        try:
            for i, frame in enumerate(script()):
                if STOP.is_set():
                    return
                self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
                self.wfile.flush()
                time.sleep(0.4 if i == 0 else 0.25)
            # 연결 유지(EventSource 재접속으로 각본 중복 방지)
            while not STOP.is_set():
                self.wfile.write(b":hb\n\n")
                self.wfile.flush()
                time.sleep(0.5)
        except (BrokenPipeError, ConnectionResetError, OSError):
            return


SNAP = "window.__officeDebug ? window.__officeDebug.snapshot() : null"

failures: list[str] = []
asserted = 0

def check(cond, msg):
    global asserted
    asserted += 1
    print(f"  [{'PASS' if cond else 'FAIL'}] {msg}")
    if not cond:
        failures.append(msg)

def main() -> int:
    if not (WEB_DIR / "office3d.html").is_file():
        print(f"FAIL: {WEB_DIR}/office3d.html 없음")
        return 2
    from playwright.sync_api import sync_playwright

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    httpd.daemon_threads = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()

    console_errors: list[str] = []
    page_errors: list[str] = []
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=[
                "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--use-gl=angle"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            page.on("console", lambda m: console_errors.append(m.text) if m.type == "error" else None)
            page.on("pageerror", lambda e: page_errors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/office3d.html?debug=1")

            # 월드 구축 대기 (게이지 등장 = addNode 완료 신호)
            page.wait_for_function("(s)=>{const d=eval(s);return d && d.gauges>0}",
                                   arg=SNAP, timeout=8000)
            # 각본 완료 대기 (board patch_top = 마지막 근처)
            page.wait_for_function("(s)=>{const d=eval(s);return d && d.patchTop && d.patchTop.board>0}",
                                   arg=SNAP, timeout=12000)
            page.wait_for_timeout(500)
            # #4 task 전문 — 선택 경로 구동
            page.evaluate("window.__officeDebug.openPanel('surface:11')")
            page.wait_for_timeout(300)

            snap = page.evaluate(SNAP)
            fx = snap["fx"]; pt = snap["patchTop"]
            print("  스냅샷:", json.dumps(snap, ensure_ascii=False))

            check(snap["gauges"] >= 6, f"#1 ctx 게이지 표시 {snap['gauges']}개(≥6, null 노드 제외)")
            check(snap["papers"] >= 5, f"#2 활동 강도 서류 더미 {snap['papers']}장(≥5)")
            check("progress" in fx, "#2 코드 스크롤 활성(활동 노드 존재)")
            check(snap["rateLeds"] >= 3, f"#3 rate LED 표시 {snap['rateLeds']}개(≥3)")
            check(snap["taskTip"] is True, "#4 task 전문 말풍선(선택 시 표시)")
            check(snap["idle"] >= 2, f"#5 방치 시간 스프라이트 {snap['idle']}개(drowsy+sleeping)")
            check(snap["rings"] >= 2 and 60 in [p for p in snap["ringPcts"] if p is not None],
                  f"#6 진행률 링 {snap['rings']}개·fx pct=60 반영 (pcts={snap['ringPcts']})")
            check(snap["ringPulse"] >= 1, f"#6 progress fx 펄스 {snap['ringPulse']}회")
            check(snap["runCards"] >= 8, f"#7 작업 카드 {snap['runCards']}장(큐+활성 ≥8)")
            check(snap["runcardFail"] >= 1 and snap["runcardOk"] >= 1,
                  f"#7 runcard failed={snap['runcardFail']} succeeded={snap['runcardOk']}")
            check(snap["blockedFx"] >= 1 and snap["unblockedFx"] >= 1,
                  f"#8 blocked/unblocked fx = {snap['blockedFx']}/{snap['unblockedFx']}")
            check(snap["blockedArrows"] >= 1, f"#8 의존 점선 {snap['blockedArrows']}개(양측 귀속)")
            check(snap["kanbanCards"] >= 1 and pt.get("kanban", 0) >= 1,
                  f"#9 칸반 카드 {snap['kanbanCards']}장·patch_top kanban={pt.get('kanban',0)}")
            check(snap["verdict"] >= 1 and snap["reviewConvened"] >= 1 and pt.get("review", 0) >= 1,
                  f"#10 리뷰 verdict={snap['verdict']} 집결={snap['reviewConvened']} patch_top review={pt.get('review',0)}")
            check(snap["docFrom"] >= 1 and snap["docNoFrom"] >= 1,
                  f"#11 배달 보행={snap['docFrom']} 아크={snap['docNoFrom']}")
            check(snap["boardTs"] > 0 and pt.get("board", 0) >= 1,
                  f"#12 전광판 갱신 ts={snap['boardTs']}·patch_top board={pt.get('board',0)}")

            check(len(page_errors) == 0, f"콘솔 pageerror 0건 (got {page_errors})")
            check(len(console_errors) == 0, f"콘솔 error 0건 (got {console_errors[:3]})")

            page.screenshot(path=str(SHOT))
            print(f"  스크린샷 저장: {SHOT}")
            browser.close()
    finally:
        STOP.set()
        httpd.shutdown()

    print(f"\n{'PASS' if not failures else 'FAIL'} — 단언 {asserted}종(12기능 각 ≥1) · 실패 {len(failures)}건")
    for f in failures:
        print(f"  ✗ {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
