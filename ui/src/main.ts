// cys UI — xterm.js panes over the cysd socket (thin client).
// 세션 영속은 구조로 해결: 세션(PTY)은 데몬 소유, UI는 attach만 한다.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

declare global {
  interface Window {
    __TAURI__: {
      core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
      event: {
        listen: (
          name: string,
          handler: (e: { payload: unknown }) => void,
        ) => Promise<() => void>;
      };
    };
  }
}
const invoke = (cmd: string, args?: Record<string, unknown>) => window.__TAURI__.core.invoke(cmd, args);
const listen = (name: string, handler: (e: { payload: unknown }) => void) =>
  window.__TAURI__.event.listen(name, handler);

// ---------- layout model (v2: multiple workspaces, splits with ratio) ----------

type Node =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: Node; b: Node }
  | { type: "pane"; sid: number };

interface Workspace {
  id: number;
  name: string;
  tree: Node | null;
}

const LAYOUT_KEY = "cys-layout-v2";

interface PaneRuntime {
  sid: number;
  el: HTMLElement;
  termHost: HTMLElement;
  titleEl: HTMLElement;
  usageEl: HTMLElement;
  term: Terminal;
  fit: FitAddon;
  unlisten: (() => void)[];
  observer: ResizeObserver;
}

// ---------- T5 사용량 관측 배지 (pane 헤더) ----------

interface RateWindow {
  label: string;
  used_pct: number;
  resets_at: number | null;
}
interface ObservedUsage {
  agent: string;
  ctx_tokens: number | null;
  ctx_window: number | null;
  ctx_pct: number | null;
  rate: RateWindow[];
  source: string;
  session_file: string;
  updated_at: number;
}

const sevClass = (pct: number, warn: number, crit: number): string =>
  pct >= crit ? "crit" : pct >= warn ? "warn" : "";

// 컨텍스트는 60%(/clear 사이클 임계)·80%, rate limit은 70%·90%에서 단계 상승
function renderUsage(el: HTMLElement, u: ObservedUsage | null | undefined) {
  el.replaceChildren();
  if (!u) {
    el.title = "";
    return;
  }
  const parts: { text: string; cls: string }[] = [];
  if (u.ctx_pct !== null && u.ctx_pct !== undefined)
    parts.push({ text: `CTX ${u.ctx_pct}%`, cls: sevClass(u.ctx_pct, 60, 80) });
  for (const w of u.rate ?? [])
    parts.push({ text: `${w.label} ${Math.round(w.used_pct)}%`, cls: sevClass(w.used_pct, 70, 90) });
  if (!parts.length) {
    el.title = "";
    return;
  }
  parts.forEach((p, i) => {
    const s = document.createElement("span");
    s.textContent = (i ? "·" : "") + p.text;
    if (p.cls) s.className = p.cls;
    el.appendChild(s);
  });
  const tip: string[] = [`${u.agent} 사용량 (관측: ${u.source})`];
  if (u.ctx_tokens != null && u.ctx_window != null)
    tip.push(`context ${u.ctx_tokens.toLocaleString()} / ${u.ctx_window.toLocaleString()} tokens`);
  for (const w of u.rate ?? []) {
    const reset = w.resets_at ? ` — reset ${new Date(w.resets_at * 1000).toLocaleString()}` : "";
    tip.push(`rate ${w.label}: ${w.used_pct}%${reset}`);
  }
  const age = Math.max(0, Math.round(Date.now() / 1000 - u.updated_at));
  if (age > 120) tip.push(`⚠ ${Math.round(age / 60)}분 전 관측 (stale)`);
  el.title = tip.join("\n");
  el.classList.toggle("stale", age > 120);
}

// ---------- T6 Control Center (전용 풀 패널 — 네이티브 실시간 모니터링) ----------
let ccOpen = false;
let ccTimer: number | null = null;
let ccClockTimer: number | null = null;
let ccUptimeBase = 0;
let ccUptimeFetchedAt = 0;

const CC_ROLE_COLOR: Record<string, string> = {
  master: "#3b82f6", cso: "#8b5cf6", worker: "#00e676",
  "reviewer-gemini": "#ffa726", "reviewer-codex": "#00d4ff",
};
const CC_STATE: Record<string, { cls: string; label: string }> = {
  working: { cls: "working", label: "작업중" }, idle: { cls: "idle", label: "대기" },
  error: { cls: "error", label: "오류" }, offline: { cls: "offline", label: "오프라인" },
};
const ccEsc = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
const ccFmtTokens = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}만` : n.toLocaleString());

function ccUptimeStr(s: number): string {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return [h, m, sec].map((x) => String(x).padStart(2, "0")).join(":");
}
function ccReset(label: string, epoch: number | null): string {
  if (!epoch) return "";
  const d = new Date(epoch * 1000);
  const p = (x: number) => String(x).padStart(2, "0");
  return label === "7d"
    ? `리셋 ${p(d.getMonth() + 1)}/${p(d.getDate())}`
    : `리셋 ${p(d.getHours())}:${p(d.getMinutes())}`;
}
function ccAggRate(fleet: any[]): Record<string, { used: number; reset: number | null }> {
  const agg: Record<string, { used: number; reset: number | null }> = {};
  for (const f of fleet) {
    for (const w of f.usage?.rate ?? []) {
      const cur = agg[w.label] ?? { used: 0, reset: null };
      if (w.used_pct > cur.used) cur.used = w.used_pct;
      if (w.resets_at != null && (cur.reset == null || w.resets_at < cur.reset)) cur.reset = w.resets_at;
      agg[w.label] = cur;
    }
  }
  return agg;
}

function setCcOpen(open: boolean) {
  ccOpen = open;
  document.getElementById("cc-panel")!.hidden = !open;
  if (open) {
    refreshControlCenter();
    tickCc();
    if (ccTimer == null) ccTimer = setInterval(refreshControlCenter, 5000) as unknown as number;
    if (ccClockTimer == null) ccClockTimer = setInterval(tickCc, 1000) as unknown as number;
  } else {
    if (ccTimer != null) { clearInterval(ccTimer); ccTimer = null; }
    if (ccClockTimer != null) { clearInterval(ccClockTimer); ccClockTimer = null; }
  }
}

function tickCc() {
  const p = (x: number) => String(x).padStart(2, "0");
  const clk = document.getElementById("cc-clock");
  if (clk) {
    const n = new Date();
    clk.textContent = `${n.getFullYear()}.${p(n.getMonth() + 1)}.${p(n.getDate())} ${p(n.getHours())}:${p(n.getMinutes())}:${p(n.getSeconds())}`;
  }
  const up = document.getElementById("cc-uptime-val");
  if (up && ccUptimeFetchedAt) {
    up.textContent = ccUptimeStr(ccUptimeBase + Math.floor(Date.now() / 1000 - ccUptimeFetchedAt));
  }
}

async function refreshControlCenter() {
  if (!ccOpen) return;
  let d: any;
  try {
    d = await invoke("control_dashboard");
  } catch {
    return;
  }
  renderControlCenter(d);
}

function renderControlCenter(d: any) {
  const fleet: any[] = d.fleet ?? [];
  const active = fleet.filter((f) => f.state === "working");
  const online = fleet.filter((f) => f.state !== "offline");
  const ratio = online.length ? Math.round((active.length / online.length) * 100) : 0;
  const live = active.length > 0;

  const badge = document.getElementById("cc-livebadge")!;
  badge.textContent = live ? "LIVE" : "IDLE";
  badge.className = "cc-badge " + (live ? "live" : "idle");

  const radar = document.getElementById("cc-radar")!;
  radar.classList.toggle("active", live);
  document.getElementById("cc-radar-val")!.textContent = `${ratio}%`;
  document.getElementById("cc-radar-sub")!.textContent = `${active.length}/${online.length} 활성`;
  (radar as HTMLElement).style.setProperty("--ratio", String(ratio));

  ccUptimeBase = d.uptime_secs ?? 0;
  ccUptimeFetchedAt = Date.now() / 1000;

  const agg = ccAggRate(fleet);
  document.getElementById("cc-kpi")!.innerHTML = ["5h", "7d"]
    .map((lab) => {
      const w = agg[lab];
      const used = w ? Math.round(w.used) : 0;
      const name = lab === "5h" ? "세션 (5h)" : "주간 (7d)";
      return `<div class="cc-card ${sevClass(used, 60, 80)}"><div class="cc-card-val">${used}%</div><div class="cc-card-reset">${w ? ccReset(lab, w.reset) : ""}</div><div class="cc-card-name">${name}</div></div>`;
    })
    .join("");

  document.getElementById("cc-fleet")!.innerHTML = fleet
    .map((f) => {
      const role = f.role ?? "?";
      const color = CC_ROLE_COLOR[role] ?? "#64748b";
      const st = CC_STATE[f.state] ?? CC_STATE.idle;
      const ctx = f.usage?.ctx_pct != null ? `CTX ${f.usage.ctx_pct}%` : "";
      return `<div class="cc-fleet-row" style="--rc:${color}"><span class="cc-fleet-name">${ccEsc(role)}</span><span class="cc-fleet-agent">${ccEsc(f.agent ?? "")}</span><span class="cc-fleet-ctx">${ctx}</span><span class="cc-dot ${st.cls}"></span><span class="cc-fleet-state">${st.label}</span></div>`;
    })
    .join("");

  document.getElementById("cc-token-bars")!.innerHTML = ["5h", "7d"]
    .map((lab) => {
      const w = agg[lab];
      const used = w ? Math.round(w.used) : 0;
      const name = lab === "5h" ? "세션" : "주간";
      return `<div class="cc-tbar"><span class="cc-tbar-lab">${name}</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(used, 60, 80)}" style="width:${Math.min(100, used)}%"></span></span><span class="cc-tbar-pct">${used}%</span><span class="cc-tbar-reset">${w ? ccReset(lab, w.reset) : ""}</span></div>`;
    })
    .join("");

  const c = d.consumption ?? {};
  document.getElementById("cc-token-stats")!.innerHTML = (
    [
      ["오늘 비용", `$${(c.today_cost_usd ?? 0).toFixed(2)}`, "추정"],
      ["최근 1시간", ccFmtTokens(c.last_1h_tokens ?? 0), "토큰"],
      ["오늘 소비", ccFmtTokens(c.today_tokens ?? 0), `입력 ${ccFmtTokens(c.today_input ?? 0)}`],
      ["세션 수", String(c.session_count ?? 0), `메시지 ${c.today_msgs ?? 0}`],
    ] as [string, string, string][]
  )
    .map(([t, v, sub]) => `<div class="cc-stat"><div class="cc-stat-t">${t}</div><div class="cc-stat-v">${v}</div><div class="cc-stat-sub">${sub}</div></div>`)
    .join("");

  // 모델 믹스 — 모델별 토큰 점유율 (claude/codex/agy 어느 모델에 얼마나)
  const mix = (c.model_mix ?? {}) as Record<string, number>;
  const mixRows = Object.entries(mix).sort((a, b) => b[1] - a[1]);
  const mixTotal = mixRows.reduce((s, [, v]) => s + v, 0) || 1;
  document.getElementById("cc-model-mix")!.innerHTML =
    mixRows.length === 0
      ? ""
      : `<div class="cc-mix-h">모델 믹스</div>` +
        mixRows
          .map(([m, v]) => {
            const pct = Math.round((v / mixTotal) * 100);
            const short = (m || "?").replace(/^claude-/, "").replace(/\[1m\]$/, "");
            return `<div class="cc-mix-row"><span class="cc-mix-name">${ccEsc(short || "?")}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-mix-pct">${pct}%</span></div>`;
          })
          .join("");

  const spark: number[] = d.sparkline ?? [];
  const max = Math.max(1, ...spark);
  document.getElementById("cc-spark")!.innerHTML =
    `<div class="cc-spark-label">12h</div><div class="cc-spark-bars">` +
    spark.map((v) => `<span class="cc-spark-bar" style="height:${Math.max(2, Math.round((v / max) * 100))}%" title="${ccFmtTokens(v)}"></span>`).join("") +
    `</div>`;

  const sys = d.system ?? {};
  const cpu = Math.round(sys.cpu_pct ?? 0);
  const memU = sys.mem_used ?? 0;
  const memT = sys.mem_total ?? 1;
  const memPct = Math.round((memU / memT) * 100);
  const gb = (b: number) => (b / 1024 / 1024 / 1024).toFixed(1);
  document.getElementById("cc-sys")!.innerHTML =
    `<div class="cc-tbar"><span class="cc-tbar-lab">CPU</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(cpu, 60, 85)}" style="width:${Math.min(100, cpu)}%"></span></span><span class="cc-tbar-pct">${cpu}%</span></div>` +
    `<div class="cc-tbar"><span class="cc-tbar-lab">MEM</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(memPct, 70, 90)}" style="width:${Math.min(100, memPct)}%"></span></span><span class="cc-tbar-pct">${gb(memU)}/${gb(memT)}G</span></div>`;

  document.getElementById("cc-footer")!.textContent = `cys Control Center · v${d.version ?? ""} · 5초 새로고침`;
}

let fontSize = Number(localStorage.getItem("cys-font-size") || 13);
function applyZoom(delta: number | null) {
  fontSize = delta === null ? 13 : Math.min(32, Math.max(8, fontSize + delta));
  localStorage.setItem("cys-font-size", String(fontSize));
  for (const rt of panes.values()) {
    rt.term.options.fontSize = fontSize;
    fitPane(rt);
  }
}

let workspaces: Workspace[] = [];
let activeWs = 0;
let wsCounter = 1;
let focusedSid: number | null = null;
const panes = new Map<number, PaneRuntime>();
const root = document.getElementById("root")!;

const current = (): Workspace => workspaces[activeWs];

function saveLayout() {
  localStorage.setItem(
    LAYOUT_KEY,
    JSON.stringify({ workspaces, active: activeWs, counter: wsCounter }),
  );
}

function collectSids(node: Node | null, out: number[] = []): number[] {
  if (!node) return out;
  if (node.type === "pane") out.push(node.sid);
  else {
    collectSids(node.a, out);
    collectSids(node.b, out);
  }
  return out;
}

function allLayoutSids(): Set<number> {
  const s = new Set<number>();
  for (const ws of workspaces) collectSids(ws.tree).forEach((x) => s.add(x));
  return s;
}

function replaceNode(node: Node, target: number, make: (old: Node) => Node | null): Node | null {
  if (node.type === "pane") {
    return node.sid === target ? make(node) : node;
  }
  const a = replaceNode(node.a, target, make);
  const b = replaceNode(node.b, target, make);
  if (a && b) return { ...node, a, b };
  return a ?? b; // one side removed → collapse to sibling
}

// ---------- pane lifecycle ----------

const b64ToBytes = (b64: string): Uint8Array => {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

// surface도 번호 대신 이름 — 기본 자동 제목("surface N"·빈 문자열)이면 현재 디렉토리 경로 표시.
const isAutoTitle = (t: string | null | undefined) => !t || /^surface \d+$/.test(t);
const paneTitle = (title: string | null | undefined, liveCwd?: string | null) =>
  isAutoTitle(title) ? liveCwd || "…" : (title as string);

// 주기적으로 데몬에 물어 자동 제목 pane의 현재 디렉토리(cd 추적)를 갱신.
// + 외부(CLI launch-agent·cys boot)에서 생성된 역할 노드 surface를 pane으로 자동 입양 —
//   이게 없으면 노드가 데몬 안에서 헤드리스로만 돌고 화면에 보이지 않는다.
let refreshing = false;
let started = false; // start()의 세션 복원이 끝나기 전 인터벌 자동 입양 차단 (이중 생성 방지)
async function refreshPaneTitles() {
  if (!started || refreshing) return; // 겹친 호출의 이중 입양 방지
  refreshing = true;
  try {
    const r = (await invoke("list_surfaces")) as {
      surfaces: {
        surface_id: number;
        title: string;
        role: string | null;
        live_cwd: string | null;
        exited: boolean;
        usage?: ObservedUsage | null;
      }[];
    };
    for (const s of r.surfaces) {
      const rt = panes.get(s.surface_id);
      if (!rt) continue;
      renderUsage(rt.usageEl, s.exited ? null : s.usage); // 종료 pane은 배지 제거 (혼동 방지)
      if (rt.titleEl.isContentEditable) continue; // 이름 편집 중에는 덮어쓰지 않음
      rt.titleEl.textContent = paneTitle(s.title, s.live_cwd) + (s.exited ? " [exited]" : "");
    }
    // 자동 입양: role이 등록된 살아있는 surface 중 UI에 없는 것 → 현재 ws에 pane으로 표출.
    // role 조건이 UI 자체 생성 pane(역할 없음)과의 생성 경합을 차단한다.
    let adopted = false;
    for (const s of r.surfaces) {
      if (s.exited || !s.role || panes.has(s.surface_id)) continue;
      if (allLayoutSids().has(s.surface_id)) continue;
      await makePane(s.surface_id, s.title);
      const ws = current();
      ws.tree = ws.tree
        ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid: s.surface_id } }
        : { type: "pane", sid: s.surface_id };
      adopted = true;
    }
    if (adopted) render();
  } catch {
    /* 데몬 일시 미응답은 다음 틱에 */
  } finally {
    refreshing = false;
  }
  updateFtRoot(); // cd 추적 — 파일 트리 루트도 따라간다
}
setInterval(refreshPaneTitles, 3000);

async function makePane(sid: number, title: string): Promise<PaneRuntime> {
  // 멱등 보장 — 같은 surface에 pane 런타임·리스너가 이중 생성되지 않게
  const existing = panes.get(sid);
  if (existing) return existing;
  const el = document.createElement("div");
  el.className = "pane";
  el.dataset.sid = String(sid); // 드래그 드롭존 탐색용
  const header = document.createElement("div");
  header.className = "pane-title";
  header.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || titleEl.isContentEditable) return;
    if ((e.target as HTMLElement).classList?.contains("pane-close")) return;
    startPaneDrag(e, sid);
  });
  const titleEl = document.createElement("span");
  titleEl.className = "pane-title-text";
  titleEl.textContent = paneTitle(title);
  const usageEl = document.createElement("span");
  usageEl.className = "pane-usage";
  // 배지 위 mousedown이 pane 드래그로 번지지 않게 — tooltip(hover) 확인 중 오발 방지
  usageEl.addEventListener("mousedown", (e) => e.stopPropagation());
  const closeBtn = document.createElement("span");
  closeBtn.className = "pane-close";
  closeBtn.textContent = "×";
  closeBtn.title = "surface 닫기 (셸 종료)";
  closeBtn.addEventListener("click", async () => {
    // WKWebView에서 confirm()은 무동작 — ws 탭과 동일한 2-click 확인 패턴
    if (closeBtn.dataset.arm !== "1") {
      closeBtn.dataset.arm = "1";
      closeBtn.textContent = "정말?";
      setTimeout(() => {
        closeBtn.dataset.arm = "";
        closeBtn.textContent = "×";
      }, 2500);
      return;
    }
    await invoke("close_surface", { surfaceId: sid }).catch(() => {});
    destroyPaneRuntime(sid);
    const ws = current();
    if (ws.tree) ws.tree = replaceNode(ws.tree, sid, () => null);
    if (focusedSid === sid) focusedSid = collectSids(ws.tree)[0] ?? null;
    render();
  });
  header.append(titleEl, usageEl, closeBtn);
  header.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showCtxMenu(e.clientX, e.clientY, [
      {
        label: "이름 변경",
        action: () => {
          titleEl.contentEditable = "true";
          titleEl.focus();
          window.getSelection()?.selectAllChildren(titleEl);
          const onKey = (ke: KeyboardEvent) => {
            if (ke.key === "Enter") {
              ke.preventDefault();
              titleEl.blur();
            }
          };
          const commit = () => {
            titleEl.removeEventListener("keydown", onKey); // rename마다 리스너 누적 방지
            titleEl.contentEditable = "false";
            const name = (titleEl.textContent || "").trim();
            // 빈 이름 = 자동 제목(경로)으로 복귀 — 데몬에 ""를 저장하면 isAutoTitle이 잡는다
            invoke("rename_surface", { surfaceId: sid, title: name })
              .catch(() => {})
              .then(() => refreshPaneTitles());
          };
          titleEl.addEventListener("blur", commit, { once: true });
          titleEl.addEventListener("keydown", onKey);
        },
      },
    ]);
  });
  const termHost = document.createElement("div");
  termHost.className = "term-host";
  el.append(header, termHost);

  const term = new Terminal({
    fontFamily: "Menlo, 'SF Mono', 'Apple SD Gothic Neo', 'Noto Sans KR', Consolas, monospace",
    fontSize,
    theme: { background: "#0d1117", foreground: "#c9d1d9" },
    scrollback: 5000,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(termHost);

  // WKWebView IME(한글 등 CJK) 조합 가드: 조합 중 keydown(keyCode 229/isComposing)을
  // xterm이 일반 키로 처리하면 자모가 분리 입력된다 — 조합 완성분만 onData로 흐르게 차단.
  term.attachCustomKeyEventHandler((e) => {
    if (e.isComposing || e.keyCode === 229) return false;
    return true;
  });

  // 전송 직렬화 체인: 빠른 타자에서 비동기 IPC 호출이 경주하면 도착 순서가 뒤집힌다 —
  // promise 체인으로 같은 pane의 모든 입력을 발사 순서대로 보장한다.
  let sendChain: Promise<unknown> = Promise.resolve();
  const sendRaw = (data: string) => {
    sendChain = sendChain
      .then(() => invoke("send_input", { surfaceId: sid, data }))
      .catch(() => {});
    return sendChain;
  };

  // ── WKWebView 한글 IME 조합 상태 머신 ──────────────────────────────────
  // WKWebView는 composition 이벤트 없이 ①음절 첫 자모를 insertText로 커밋(xterm이 즉시
  // 전송해버림 = 자모 유출) ②조합 진행을 insertReplacementText로 value 치환(xterm 미인지 =
  // 완성 글자 유실)한다. 여기서 자모 유출을 차단하고 음절 확정 시 완성 글자만 보낸다.
  let pendingHangul = "";
  // 자모(31xx·11xx) + 완성형 음절(AC00-D7A3) — ★멀티문자 허용(2026-06-13): 고속 입력에서
  // IME가 여러 음절을 한 insertText로 병합 커밋하는데, 단일 문자만 인정하면 그 묶음이
  // input 핸들러에서 무시되고 onData(고속에서 발화 비결정)도 못 받쳐 통째로 유실된다
  // — "4자 치면 2자" 절반 유실의 주 경로.
  const isHangulText = (t: string) => /^[\u3131-\u318E\u1100-\u11FF\uAC00-\uD7A3]+$/.test(t);
  // IME 계측(사람 단계 재현용): localStorage.cysImeDebug="1" 설정 시 이벤트 시퀀스를
  // /tmp/cys-ime.log에 기록 — 유실 경로를 결정론으로 확정하는 채널. 평시 비용 0.
  const imeDbg = localStorage.getItem("cysImeDebug") === "1";
  const dbg = (line: string) => {
    if (imeDbg) invoke("log_ime", { line: `[s${sid}] ${line}` }).catch(() => {});
  };
  const flushPending = (why: string) => {
    if (pendingHangul) {
      dbg(`FLUSH(${why}) "${pendingHangul}"`);
      sendRaw(pendingHangul);
      pendingHangul = "";
    }
  };

  term.onData((data) => {
    // 한글 음절: 이 WebKit은 조합을 표준 composition inputType(insertFromComposition)으로
    // 확정하고, 그때 xterm이 완성 음절을 onData로 정확히 1회 발화한다 — 차단 없이 그대로
    // PTY로 보낸다. (구 isHangulText 차단은 insertText 기반 상태머신을 전제했으나, 실기기
    // WebKit은 insertCompositionText/insertFromComposition을 보내 그 머신이 작동한 적이
    // 없었고, 차단만 살아 순수 한글 음절을 통째로 유실시켰다 — "너는 마스터다"→"는 다".)
    flushPending("onData"); // (no-op 안전장치: 잔여 pending 있으면 순서 보존 후 전송)
    sendRaw(data);
  });

  {
    const ta = term.textarea;
    if (ta) {
      ta.addEventListener("input", (e) => {
        const ie = e as InputEvent;
        dbg(`input ${ie.inputType} data="${ie.data ?? "∅"}" pending="${pendingHangul}"`);
        if (ie.inputType === "insertText" && ie.data && isHangulText(ie.data)) {
          // 직전 조합 확정 후 새 커밋을 '수정 가능 창'(pending)에 둔다. 병합 커밋
          // (2음절+)은 마지막 음절만 수정 창에 — 앞 음절들은 확정분이므로 즉시 전송
          // (replacement 재조합은 마지막 음절 단위로 온다).
          flushPending("insertText");
          if (ie.data.length > 1) {
            dbg(`SEND(multi-head) "${ie.data.slice(0, -1)}"`);
            sendRaw(ie.data.slice(0, -1));
          }
          pendingHangul = ie.data.slice(-1);
        } else if (ie.inputType === "insertReplacementText" && ie.data) {
          if (pendingHangul) {
            pendingHangul = ie.data; // 조합 갱신 (하→한)
          } else {
            // 이미 전송된 직전 음절의 교정 — PTY 동기화: 백스페이스+재전송
            dbg(`SEND(repl-sync) DEL+"${ie.data}"`);
            sendRaw("\x7f" + ie.data);
          }
        } else if (ie.inputType === "deleteContentBackward" && pendingHangul) {
          // 멀티 pending(병합 커밋 잔여)이면 마지막 글자만 — IME 부분 재조합 대응
          pendingHangul = pendingHangul.slice(0, -1);
          dbg(`del-backward pending="${pendingHangul}"`);
        }
      });
      ta.addEventListener("keydown", (e) => {
        if (imeDbg && e.keyCode !== 229) dbg(`keydown ${e.key}`);
        // 일반 키(Enter·Space·화살표 등, IME 처리중 229 제외) 직전에 조합 확정
        if (e.keyCode !== 229) flushPending("keydown");
        // 조합 중이 아닐 때 textarea 잔여 value 정리 (IME value 누적 방지)
        if (e.keyCode !== 229 && !pendingHangul && ta.value.length > 64) {
          (ta as HTMLTextAreaElement).value = "";
        }
      });
      ta.addEventListener("blur", () => flushPending("blur"));
    }
  }
  el.addEventListener("mousedown", () => setFocus(sid));
  term.textarea?.addEventListener("focus", () => setFocus(sid));

  const un1 = await listen(`surface-output-${sid}`, (e) => {
    term.write(b64ToBytes(e.payload as string));
  });
  const un2 = await listen(`surface-exited-${sid}`, () => {
    term.write("\r\n\x1b[31m[surface exited]\x1b[0m\r\n");
  });

  let resizeTimer: number | undefined;
  const observer = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => fitPane(rt), 60);
  });
  observer.observe(termHost);

  const rt: PaneRuntime = { sid, el, termHost, titleEl, usageEl, term, fit, unlisten: [un1, un2], observer };
  panes.set(sid, rt);
  await invoke("attach_surface", { surfaceId: sid });
  return rt;
}

/// Fit only when actually laid out — a detached/hidden pane must not shrink the PTY.
function fitPane(rt: PaneRuntime) {
  if (rt.termHost.offsetWidth < 60 || rt.termHost.offsetHeight < 40) return;
  rt.fit.fit();
  invoke("resize_surface", { surfaceId: rt.sid, rows: rt.term.rows, cols: rt.term.cols }).catch(() => {});
}

function destroyPaneRuntime(sid: number) {
  const rt = panes.get(sid);
  if (!rt) return;
  rt.observer.disconnect();
  rt.unlisten.forEach((u) => u());
  rt.term.dispose();
  rt.el.remove();
  panes.delete(sid);
}

// ---------- pane drag 이동 (탭을 끌어 자유 배치 — 외부 터미널 체계 동급 기능) ----------

type DropSide = "left" | "right" | "top" | "bottom";

function startPaneDrag(e0: MouseEvent, sid: number) {
  const start = { x: e0.clientX, y: e0.clientY };
  let dragging = false;
  let ghost: HTMLElement | null = null;
  let hint: HTMLElement | null = null;
  let target: { sid: number; side: DropSide } | null = null;

  const move = (e: MouseEvent) => {
    if (!dragging) {
      // 클릭(포커스)과 구분 — 6px 이상 움직여야 드래그 시작
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 6) return;
      dragging = true;
      ghost = document.createElement("div");
      ghost.id = "drag-ghost";
      ghost.textContent = panes.get(sid)?.titleEl.textContent || `surface ${sid}`;
      hint = document.createElement("div");
      hint.id = "drop-hint";
      hint.hidden = true;
      document.body.append(ghost, hint);
      document.body.classList.add("pane-dragging");
    }
    ghost!.style.left = `${e.clientX + 10}px`;
    ghost!.style.top = `${e.clientY + 10}px`;
    const over = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest(
      ".pane",
    ) as HTMLElement | null;
    target = null;
    if (over?.dataset.sid && Number(over.dataset.sid) !== sid) {
      const r = over.getBoundingClientRect();
      // 커서가 치우친 변 = 드롭 방향 (사분면 판정)
      const rx = (e.clientX - r.left) / r.width - 0.5;
      const ry = (e.clientY - r.top) / r.height - 0.5;
      const side: DropSide =
        Math.abs(rx) > Math.abs(ry) ? (rx < 0 ? "left" : "right") : (ry < 0 ? "top" : "bottom");
      target = { sid: Number(over.dataset.sid), side };
      const h = hint!;
      h.hidden = false;
      h.style.left = `${side === "right" ? r.left + r.width / 2 : r.left}px`;
      h.style.top = `${side === "bottom" ? r.top + r.height / 2 : r.top}px`;
      h.style.width = `${side === "left" || side === "right" ? r.width / 2 : r.width}px`;
      h.style.height = `${side === "top" || side === "bottom" ? r.height / 2 : r.height}px`;
    } else if (hint) {
      hint.hidden = true;
    }
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    ghost?.remove();
    hint?.remove();
    document.body.classList.remove("pane-dragging");
    if (dragging && target) movePane(sid, target.sid, target.side);
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
}

/// sid pane을 트리에서 떼어 target pane의 side 쪽에 분할 삽입한다.
function movePane(sid: number, targetSid: number, side: DropSide) {
  const ws = current();
  if (!ws.tree || sid === targetSid) return;
  const sids = collectSids(ws.tree);
  if (!sids.includes(sid) || !sids.includes(targetSid)) return;
  ws.tree = replaceNode(ws.tree, sid, () => null);
  const moved: Node = { type: "pane", sid };
  if (!ws.tree) {
    ws.tree = moved;
  } else {
    const dir = side === "left" || side === "right" ? "row" : "col";
    const before = side === "left" || side === "top";
    ws.tree = replaceNode(ws.tree, targetSid, (old) => ({
      type: "split",
      dir,
      a: before ? moved : old,
      b: before ? old : moved,
    }));
  }
  render();
  setFocus(sid);
}

function setFocus(sid: number) {
  focusedSid = sid;
  for (const [id, rt] of panes) rt.el.classList.toggle("focused", id === sid);
  panes.get(sid)?.term.focus();
  updateFtRoot(); // 파일 트리가 열려 있으면 선택한 surface의 폴더로 전환
}

// ---------- render ----------

function render() {
  for (const rt of panes.values()) rt.el.remove();
  root.innerHTML = "";
  const tree = current()?.tree;
  if (tree) root.appendChild(renderNode(tree));
  renderWsTabs();
  requestAnimationFrame(() => {
    for (const sid of collectSids(current()?.tree ?? null)) {
      const rt = panes.get(sid);
      if (rt) fitPane(rt);
    }
  });
  saveLayout();
}

function renderNode(node: Node): HTMLElement {
  if (node.type === "pane") {
    const rt = panes.get(node.sid);
    if (rt) return rt.el;
    const placeholder = document.createElement("div");
    placeholder.className = "pane";
    placeholder.textContent = `surface:${node.sid} (없음)`;
    return placeholder;
  }
  const div = document.createElement("div");
  div.className = `split ${node.dir}`;
  const aEl = renderNode(node.a);
  const bEl = renderNode(node.b);
  const divider = document.createElement("div");
  divider.className = "divider";
  const ratio = node.ratio ?? 0.5;
  aEl.style.flex = `${ratio} 1 0%`;
  bEl.style.flex = `${1 - ratio} 1 0%`;
  attachDividerDrag(divider, div, node, aEl, bEl);
  div.append(aEl, divider, bEl);
  return div;
}

function attachDividerDrag(
  divider: HTMLElement,
  container: HTMLElement,
  node: Node & { type: "split" },
  aEl: HTMLElement,
  bEl: HTMLElement,
) {
  divider.addEventListener("mousedown", (down) => {
    down.preventDefault();
    divider.classList.add("dragging");
    const horizontal = node.dir === "row";
    const move = (e: MouseEvent) => {
      const rect = container.getBoundingClientRect();
      const pos = horizontal ? e.clientX - rect.left : e.clientY - rect.top;
      const size = horizontal ? rect.width : rect.height;
      const ratio = Math.min(0.85, Math.max(0.15, pos / size));
      node.ratio = ratio;
      aEl.style.flex = `${ratio} 1 0%`;
      bEl.style.flex = `${1 - ratio} 1 0%`;
    };
    const up = () => {
      divider.classList.remove("dragging");
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      saveLayout();
      for (const sid of collectSids(node)) {
        const rt = panes.get(sid);
        if (rt) fitPane(rt);
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}

// ---------- 정렬: 역할(role) 기반 고정 배치 ----------
// 현재 워크스페이스의 살아있는 surface를 역할별 표준 자리로 재배치한다:
//   · 왼쪽 끝 컬럼  = master(위) / cso(아래), 세로 5:1
//   · 가운데        = worker·미분류 surface를 같은 폭 컬럼으로 균등 분배(좌→우 순서 보존)
//   · 오른쪽 끝 컬럼 = reviewer-gemini(agy, 위) / reviewer-codex(codex, 아래), 세로 1:1
// 트리 위상만 새로 짜고 attachDividerDrag는 건드리지 않으므로 수동 크기 조절은 그대로 보존된다
// (정렬 후에도 divider를 다시 끌 수 있다 — 현재 크기만 표준 배치로 리셋될 뿐이다).
// divider 1px·pane 헤더 등으로 컬럼 폭엔 셀 1칸 이내 잔차가 있을 수 있다.
function evenComb(nodes: Node[], dir: "row" | "col"): Node {
  let acc = nodes[nodes.length - 1];
  for (let i = nodes.length - 2; i >= 0; i--) {
    acc = { type: "split", dir, ratio: 1 / (nodes.length - i), a: nodes[i], b: acc };
  }
  return acc;
}

function firstWithRole(sids: number[], roleOf: Map<number, string | null>, role: string): number | null {
  for (const sid of sids) if (roleOf.get(sid) === role) return sid;
  return null;
}

function roleLayout(sids: number[], roleOf: Map<number, string | null>): Node {
  const master = firstWithRole(sids, roleOf, "master");
  const cso = firstWithRole(sids, roleOf, "cso");
  const agy = firstWithRole(sids, roleOf, "reviewer-gemini"); // 안티그래피티
  const codex = firstWithRole(sids, roleOf, "reviewer-codex");
  const corners = new Set([master, cso, agy, codex].filter((x): x is number => x != null));
  const middle = sids.filter((sid) => !corners.has(sid)); // worker·미분류 전부 가운데
  const pane = (sid: number): Node => ({ type: "pane", sid });

  const columns: Node[] = [];
  // 왼쪽 끝: master(위) / cso(아래) = 5:1 (누락 시 있는 쪽이 컬럼 전체)
  if (master != null && cso != null) columns.push({ type: "split", dir: "col", ratio: 5 / 6, a: pane(master), b: pane(cso) });
  else if (master != null) columns.push(pane(master));
  else if (cso != null) columns.push(pane(cso));
  // 가운데: worker·미분류 균등 컬럼
  for (const sid of middle) columns.push(pane(sid));
  // 오른쪽 끝: agy(위) / codex(아래) = 1:1 (누락 시 있는 쪽이 컬럼 전체)
  if (agy != null && codex != null) columns.push({ type: "split", dir: "col", ratio: 1 / 2, a: pane(agy), b: pane(codex) });
  else if (agy != null) columns.push(pane(agy));
  else if (codex != null) columns.push(pane(codex));

  return evenComb(columns, "row"); // 컬럼들을 같은 폭으로 가로 배치
}

async function actionEqualize() {
  const ws = current();
  if (!ws?.tree) return;
  const live = collectSids(ws.tree).filter((sid) => panes.has(sid)); // 죽은/placeholder 노드 제외
  if (live.length < 2) return; // 0~1개는 정렬할 대상이 없음
  // 역할은 데몬 surface.list에서 조회 (UI 생성 pane은 role=null → 가운데로)
  const roleOf = new Map<number, string | null>();
  try {
    const r = (await invoke("list_surfaces")) as { surfaces: { surface_id: number; role: string | null }[] };
    for (const s of r.surfaces) roleOf.set(s.surface_id, s.role);
  } catch {
    /* 데몬 일시 미응답: role 없이 진행 → 전부 가운데 균등 */
  }
  ws.tree = roleLayout(live, roleOf);
  render(); // 새 트리로 DOM 재구성 + fitPane→resize_surface + saveLayout
}

// ---------- workspace tabs ----------

// ws별 고유색 (id 기반 — 세션 복원에도 같은 ws는 같은 색)
const WS_COLORS = ["#2f81f7", "#3fb950", "#d29922", "#f85149", "#a371f7", "#db61a2", "#39c5cf", "#e3b341"];

function renderWsTabs() {
  const bar = document.getElementById("ws-tabs")!;
  bar.innerHTML = "";
  workspaces.forEach((ws, idx) => {
    const color = WS_COLORS[ws.id % WS_COLORS.length];
    const tab = document.createElement("div");
    tab.className = "ws-tab" + (idx === activeWs ? " active" : "");
    tab.style.borderLeftColor = color; // ws 고유색은 좌측 바 (사이드바 항목 식별)
    const titleRow = document.createElement("div");
    titleRow.className = "ws-title-row";
    const label = document.createElement("span");
    label.className = "ws-name";
    label.textContent = ws.name;
    const close = document.createElement("span");
    close.className = "ws-close";
    close.textContent = "×";
    close.title = "워크스페이스 닫기 (surface 전부 종료)";
    titleRow.append(label, close);
    // 서브라인: pane 수 + 대표 pane 제목 — 외부 터미널 체계의 상태 라인 차용(항목 가독성)
    const sids = collectSids(ws.tree);
    const firstTitle =
      panes.get(sids[0] ?? -1)?.titleEl.textContent ?? "";
    const sub = document.createElement("span");
    sub.className = "ws-sub";
    sub.textContent = `${sids.length} pane${firstTitle ? ` · ${firstTitle}` : ""}`;
    tab.append(titleRow, sub);
    tab.addEventListener("mousedown", (e) => {
      // 우클릭은 전환하지 않음 — render()가 탭 DOM을 재생성하면 컨텍스트 메뉴가 죽은 엘리먼트를 잡는다
      if (e.button !== 0 || e.target === close) return;
      if (idx !== activeWs) {
        activeWs = idx;
        render();
        const first = collectSids(current().tree)[0];
        if (first != null) setFocus(first);
      }
    });
    const startRename = () => {
      // WKWebView에서 prompt()는 무동작 — 인라인 편집
      label.contentEditable = "true";
      label.focus();
      const sel = window.getSelection();
      sel?.selectAllChildren(label);
      const onKey = (ke: KeyboardEvent) => {
        if (ke.key === "Enter") {
          ke.preventDefault();
          label.blur();
        }
      };
      const commit = () => {
        label.removeEventListener("keydown", onKey); // rename마다 리스너 누적 방지
        label.contentEditable = "false";
        const name = (label.textContent || "").trim();
        ws.name = name || UNTITLED; // 이름을 지우면 미정 표시로 복귀
        render();
      };
      label.addEventListener("blur", commit, { once: true });
      label.addEventListener("keydown", onKey);
    };
    label.addEventListener("dblclick", startRename);
    tab.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showCtxMenu(e.clientX, e.clientY, [{ label: "이름 변경", action: startRename }]);
    });
    close.addEventListener("click", async () => {
      // WKWebView에서 confirm()은 무동작 — 2-click 확인 패턴 사용
      if (close.dataset.arm !== "1") {
        close.dataset.arm = "1";
        close.textContent = "정말?";
        setTimeout(() => {
          close.dataset.arm = "";
          close.textContent = "×";
        }, 2500);
        return;
      }
      for (const sid of collectSids(ws.tree)) {
        await invoke("close_surface", { surfaceId: sid }).catch(() => {});
        destroyPaneRuntime(sid);
      }
      workspaces.splice(idx, 1);
      if (workspaces.length === 0) {
        await addWorkspace(); // addWorkspace가 activeWs를 설정
      } else if (idx < activeWs) {
        activeWs -= 1; // 활성보다 앞 탭을 닫으면 인덱스가 한 칸 당겨진다
      } else {
        activeWs = Math.min(activeWs, workspaces.length - 1);
      }
      render();
    });
    bar.appendChild(tab);
  });
}

// ws는 번호가 아니라 이름으로 구분 — 이름이 정해지지 않으면 "non title" 표시.
const UNTITLED = "non title";

// 커스텀 컨텍스트 메뉴 (WKWebView 기본 메뉴 대체) — 싱글톤, 바깥 클릭·Esc로 닫힘.
function showCtxMenu(x: number, y: number, items: { label: string; action: () => void }[]) {
  document.getElementById("ctx-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "ctx-menu";
  const closeMenu = () => {
    menu.remove();
    window.removeEventListener("mousedown", dismiss, true);
    window.removeEventListener("keydown", onKey, true);
  };
  const dismiss = (e?: Event) => {
    if (e instanceof MouseEvent && menu.contains(e.target as globalThis.Node)) return;
    closeMenu();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") dismiss();
  };
  for (const it of items) {
    const row = document.createElement("div");
    row.className = "ctx-item";
    row.textContent = it.label;
    row.addEventListener("mousedown", (e) => {
      e.preventDefault();
      closeMenu();
      it.action();
    });
    menu.appendChild(row);
  }
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  document.body.appendChild(menu);
  // 화면 밖으로 나가면 안쪽으로 보정
  const r = menu.getBoundingClientRect();
  if (r.right > window.innerWidth) menu.style.left = `${window.innerWidth - r.width - 4}px`;
  if (r.bottom > window.innerHeight) menu.style.top = `${window.innerHeight - r.height - 4}px`;
  window.addEventListener("mousedown", dismiss, true);
  window.addEventListener("keydown", onKey, true);
}

async function addWorkspace(): Promise<Workspace> {
  const sid = await newSurface();
  const ws: Workspace = { id: wsCounter++, name: UNTITLED, tree: { type: "pane", sid } };
  workspaces.push(ws);
  activeWs = workspaces.length - 1;
  render();
  setFocus(sid);
  return ws;
}

// ---------- actions ----------

async function newSurface(cwd: string | null = null): Promise<number> {
  const r = (await invoke("create_surface", { cwd, title: null, rows: 35, cols: 120 })) as {
    surface_id: number;
  };
  await makePane(r.surface_id, ""); // 자동 제목 — 곧 refreshPaneTitles가 현재 경로로 채움
  refreshPaneTitles();
  return r.surface_id;
}

// 같은 ws의 첫 surface가 지금 있는 경로 — 새 surface의 시작 경로로 쓴다 (이후 이동은 사용자 몫)
async function firstPaneCwd(): Promise<string | null> {
  const first = collectSids(current().tree)[0];
  if (first == null) return null;
  try {
    const r = (await invoke("list_surfaces")) as {
      surfaces: { surface_id: number; live_cwd: string | null }[];
    };
    return r.surfaces.find((s) => s.surface_id === first)?.live_cwd ?? null;
  } catch {
    return null;
  }
}

async function actionNew() {
  const sid = await newSurface(await firstPaneCwd());
  const ws = current();
  ws.tree = ws.tree
    ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid } }
    : { type: "pane", sid };
  render();
  setFocus(sid);
}

async function actionSplit(dir: "row" | "col") {
  const ws = current();
  // stale focusedSid 검증 — 트리에 없는 대상을 분할하면 replaceNode가 무음 no-op 되어
  // 보이지 않는 고아 surface(살아있는 PTY)가 생긴다
  if (focusedSid == null || !ws.tree || !collectSids(ws.tree).includes(focusedSid)) {
    return actionNew();
  }
  const target = focusedSid;
  const sid = await newSurface(await firstPaneCwd());
  if (!ws.tree || !collectSids(ws.tree).includes(target)) {
    // await 사이에 대상이 닫힌 경우 — 루트에 덧붙여 고아를 만들지 않는다
    ws.tree = ws.tree
      ? { type: "split", dir, a: ws.tree, b: { type: "pane", sid } }
      : { type: "pane", sid };
  } else {
    ws.tree = replaceNode(ws.tree, target, (old) => ({
      type: "split",
      dir,
      a: old,
      b: { type: "pane", sid },
    }));
  }
  render();
  setFocus(sid);
}

async function actionClose() {
  const ws = current();
  if (focusedSid == null || !ws.tree) return;
  const sid = focusedSid;
  await invoke("close_surface", { surfaceId: sid }).catch(() => {});
  destroyPaneRuntime(sid);
  ws.tree = replaceNode(ws.tree, sid, () => null);
  focusedSid = collectSids(ws.tree)[0] ?? null;
  render();
  if (focusedSid != null) setFocus(focusedSid);
}

// 데몬에서 사라진(종료·닫힘·reap) surface의 UI pane을 자동 제거 — 멱등(이미 없으면 무동작).
// 데몬이 close_surface 하지 않은 자력종료라도 즉시 정리해 죽은 pane이 쌓이지 않게 한다.
// 복구는 보존: 60s grace 내 node-recover로 surface가 되살아나면 refreshPaneTitles 폴링이 재입양한다.
function removeDeadPane(sid: number) {
  const inLayout = workspaces.some((w) => w.tree != null && collectSids(w.tree).includes(sid));
  if (!panes.has(sid) && !inLayout) return; // 이미 정리됨
  destroyPaneRuntime(sid);
  for (const ws of workspaces) {
    if (ws.tree != null && collectSids(ws.tree).includes(sid)) {
      ws.tree = replaceNode(ws.tree, sid, () => null);
    }
  }
  if (focusedSid === sid) focusedSid = collectSids(current()?.tree ?? null)[0] ?? null;
  render();
  if (focusedSid != null) setFocus(focusedSid);
}

// ---------- feed panel ----------

interface FeedItem {
  request_id: string;
  kind: string;
  title: string;
  body: string;
  surface_id: number | null;
  status: string;
  decision: string | null;
}

let feedOpen = false;

function setFeedOpen(open: boolean) {
  feedOpen = open;
  document.getElementById("feed-panel")!.hidden = !open;
  if (open) refreshFeed();
}

// ---------- file tree (오른쪽 섹션 — 선택한 surface의 폴더 탐색) ----------

let ftOpen = false;
let ftRoot: string | null = null;
const ftExpanded = new Set<string>(); // 펼쳐진 하위 폴더 경로

function setFtOpen(open: boolean) {
  ftOpen = open;
  document.getElementById("ft-panel")!.hidden = !open;
  if (open) updateFtRoot(); // pane 폭 변화는 ResizeObserver가 자동 보정
}

// 포커스된 surface의 현재 경로를 트리 루트로 — 포커스 이동·cd 모두 추적
async function updateFtRoot() {
  if (!ftOpen || focusedSid == null) return;
  try {
    const r = (await invoke("list_surfaces")) as {
      surfaces: { surface_id: number; live_cwd: string | null }[];
    };
    const cwd = r.surfaces.find((s) => s.surface_id === focusedSid)?.live_cwd ?? null;
    if (cwd && cwd !== ftRoot) {
      ftRoot = cwd;
      ftExpanded.clear();
      renderFileTree();
    }
  } catch {
    /* 다음 틱에 */
  }
}

async function renderFileTree() {
  const body = document.getElementById("ft-body")!;
  const label = document.getElementById("ft-root-label")!;
  if (!ftRoot) {
    body.innerHTML = "";
    label.textContent = "파일";
    return;
  }
  label.textContent = ftRoot.split("/").pop() || ftRoot;
  label.title = ftRoot;
  const frag = await buildDirNodes(ftRoot, 0);
  body.innerHTML = "";
  body.appendChild(frag);
}

async function buildDirNodes(dir: string, depth: number): Promise<DocumentFragment> {
  const frag = document.createDocumentFragment();
  let entries: { name: string; is_dir: boolean }[] = [];
  try {
    entries = (await invoke("list_dir", { path: dir })) as { name: string; is_dir: boolean }[];
  } catch {
    return frag;
  }
  for (const ent of entries) {
    const full = dir === "/" ? `/${ent.name}` : `${dir}/${ent.name}`;
    const row = document.createElement("div");
    row.className = "ft-row" + (ent.is_dir ? " dir" : "");
    // 폴더 화살표만큼 파일을 더 들여 이름 시작선을 맞춘다
    row.style.paddingLeft = `${8 + depth * 14 + (ent.is_dir ? 0 : 14)}px`;
    row.textContent = ent.is_dir ? `${ftExpanded.has(full) ? "▾" : "▸"} ${ent.name}` : ent.name;
    row.title = full;
    row.addEventListener("click", () => {
      if (ent.is_dir) {
        if (ftExpanded.has(full)) ftExpanded.delete(full);
        else ftExpanded.add(full);
        renderFileTree();
      } else {
        invoke("open_path", { path: full }).catch(() => {}); // 시스템 기본 앱으로 열기
      }
    });
    frag.appendChild(row);
    if (ent.is_dir && ftExpanded.has(full)) frag.appendChild(await buildDirNodes(full, depth + 1));
  }
  return frag;
}

async function refreshFeed() {
  const r = (await invoke("feed_list", { status: null }).catch(() => null)) as
    | { items: FeedItem[] }
    | null;
  if (!r) return;
  const items = r.items.slice().reverse();
  const pending = items.filter((i) => i.status === "pending");
  const badge = document.getElementById("feed-badge")!;
  badge.hidden = pending.length === 0;
  badge.textContent = String(pending.length);

  if (!feedOpen) return;
  const box = document.getElementById("feed-items")!;
  box.innerHTML = "";
  if (items.length === 0) {
    box.textContent = "(비어 있음)";
    return;
  }
  for (const item of items.slice(0, 50)) {
    const el = document.createElement("div");
    el.className = `feed-item ${item.status}`;
    const title = document.createElement("div");
    title.className = "fi-title";
    title.textContent = item.title;
    const meta = document.createElement("div");
    meta.className = "fi-meta";
    meta.textContent = `${item.kind} · ${item.request_id}` + (item.surface_id != null ? ` · surface:${item.surface_id}` : "");
    const body = document.createElement("div");
    body.className = "fi-body";
    body.textContent = item.body;
    el.append(title, meta, body);
    if (item.status === "pending") {
      const actions = document.createElement("div");
      actions.className = "fi-actions";
      for (const [label, decision, cls] of [["Allow", "allow", "allow"], ["Deny", "deny", "deny"]] as const) {
        const btn = document.createElement("button");
        btn.className = cls;
        btn.textContent = label;
        btn.addEventListener("click", async () => {
          await invoke("feed_reply", { requestId: item.request_id, decision }).catch(() => {});
          refreshFeed();
        });
        actions.appendChild(btn);
      }
      el.appendChild(actions);
    } else {
      const d = document.createElement("div");
      d.className = "fi-decision";
      d.textContent = item.status === "timeout" ? "⏱ timeout" : `→ ${item.decision}`;
      el.appendChild(d);
    }
    box.appendChild(el);
  }
}

// ---------- 자동 업데이트 ----------

let updateAvailable: { version: string; notes?: string } | null = null;

/// 업데이트 확인. silent=true면 시작 시 백그라운드 체크(결과 없으면 조용히).
async function checkForUpdate(silent: boolean) {
  let res: { version: string; current?: string; notes?: string } | null;
  try {
    res = (await invoke("check_update")) as typeof res;
  } catch (e) {
    if (!silent) toast("health", "업데이트 확인 실패", String(e));
    return;
  }
  const badge = document.getElementById("update-badge")!;
  if (res && res.version) {
    updateAvailable = { version: res.version, notes: res.notes };
    badge.hidden = false;
    badge.textContent = "!";
    if (!silent) promptInstall();
    else toast("feed", "🔄 업데이트 있음", `새 버전 ${res.version} — 상단 Update 버튼`);
  } else {
    updateAvailable = null;
    badge.hidden = true;
    if (!silent) toast("watchdog", "최신 버전", "이미 최신입니다.");
  }
}

/// 설치 확인 + 데몬 핸드오프 정책(세션 없으면 자동·있으면 확인).
async function promptInstall() {
  if (!updateAvailable) {
    await checkForUpdate(false);
    return;
  }
  const v = updateAvailable.version;
  const sessions = (await invoke("live_session_count").catch(() => 0)) as number;
  let force = false;
  if (sessions > 0) {
    // WKWebView는 confirm 지원이 불안정 → 커스텀 모달
    const ok = await confirmModal(
      `새 버전 ${v} 설치`,
      `현재 작업 세션 ${sessions}개가 데몬에 물려 있습니다. 업데이트는 데몬을 재시작하므로 ` +
        `이 세션들이 종료됩니다.\n\n그래도 지금 설치하시겠습니까? (아니오: 세션을 정리한 뒤 다시 시도)`,
    );
    if (!ok) return;
    force = true;
  }
  toast("feed", "⬇ 업데이트 설치", `버전 ${v} 다운로드 중… 완료 후 자동 재시작됩니다.`);
  try {
    await invoke("install_update", { force });
    // 성공 시 app.restart()로 프로세스가 교체되므로 이 줄에 도달하지 않는다.
  } catch (e) {
    const msg = String(e);
    if (msg.includes("live_sessions:")) {
      // 가드에 막힘(force 미적용 경로) — 다시 확인 흐름으로
      await promptInstall();
    } else {
      toast("health", "업데이트 설치 실패", msg);
    }
  }
}

/// 간단한 확인 모달 (WKWebView confirm 회피). resolve(true/false).
function confirmModal(title: string, body: string): Promise<boolean> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    ov.innerHTML =
      `<div class="modal"><h3></h3><p></p>` +
      `<div class="modal-btns"><button class="modal-no">아니오</button>` +
      `<button class="modal-yes">설치</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = title;
    (ov.querySelector("p") as HTMLElement).textContent = body;
    const done = (v: boolean) => {
      ov.remove();
      resolve(v);
    };
    ov.querySelector(".modal-yes")!.addEventListener("click", () => done(true));
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(false));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(false);
    });
    document.body.appendChild(ov);
  });
}

// ---------- toasts (daemon push events) ----------

function toast(category: string, name: string, detail: string) {
  const box = document.getElementById("toasts")!;
  const el = document.createElement("div");
  el.className = `toast ${category}`;
  el.innerHTML = `<span class="toast-name"></span><span class="toast-detail"></span>`;
  (el.querySelector(".toast-name") as HTMLElement).textContent = name;
  (el.querySelector(".toast-detail") as HTMLElement).textContent = detail;
  box.appendChild(el);
  setTimeout(() => el.remove(), 8000);
}

function onDaemonEvent(event: Record<string, unknown>) {
  const name = String(event.name ?? "");
  const category = String(event.category ?? "");
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  const sid = event.surface_id;
  if (category === "health") {
    toast("health", `⚠ ${name}`, `surface:${sid} rule=${payload.rule} — ${String(payload.line ?? "").slice(0, 120)}`);
  } else if (category === "watchdog") {
    const detail =
      name === "watchdog.duplicate_procs"
        ? `중복 서버 ${payload.count}개: ${String(payload.cmdline ?? "").slice(0, 80)}`
        : JSON.stringify(payload).slice(0, 120);
    toast("watchdog", `🐕 ${name}`, detail);
  } else if (category === "feed") {
    if (name === "feed.item.created") {
      toast("feed", "📥 승인 요청", String(payload.title ?? ""));
      if (payload.wait === true) setFeedOpen(true);
    }
    refreshFeed();
  } else if (name === "surface.exited" || name === "surface.closed" || name === "surface.reaped") {
    // 종료 즉시 죽은 pane 자동 제거 (A안) — 데몬 reap을 기다리지 않는다. 멱등.
    removeDeadPane(Number(sid));
  }
}

// ---------- startup / session restore ----------

async function start() {
  const info = document.getElementById("daemon-info")!;
  await new Promise<void>((resolve) => {
    listen("daemon-ready", () => resolve());
    listen("daemon-error", (e) => {
      info.textContent = `daemon error: ${e.payload}`;
    });
    const probe = setInterval(async () => {
      try {
        await invoke("daemon_status");
        clearInterval(probe);
        resolve();
      } catch {
        /* not yet */
      }
    }, 300);
  });

  const status = (await invoke("daemon_status")) as Record<string, unknown>;
  info.textContent = `daemon pid=${status.daemon_pid} sock=${status.socket_path}`;

  await listen("daemon-event", (e) => onDaemonEvent(e.payload as Record<string, unknown>));

  // 시작 시 + 6시간마다 백그라운드 업데이트 확인 (조용히 — 있으면 badge·toast)
  checkForUpdate(true);
  setInterval(() => checkForUpdate(true), 6 * 3600 * 1000);

  // Session restore: surfaces live in the daemon; reconcile saved workspaces with reality.
  const listResp = (await invoke("list_surfaces")) as {
    surfaces: { surface_id: number; title: string; exited: boolean }[];
  };
  const live = listResp.surfaces.filter((s) => !s.exited);
  const liveIds = new Set(live.map((s) => s.surface_id));

  try {
    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    if (saved && Array.isArray(saved.workspaces)) {
      workspaces = saved.workspaces;
      activeWs = saved.active ?? 0;
      wsCounter = saved.counter ?? workspaces.length + 1;
    }
  } catch {
    workspaces = [];
  }
  // Drop dead panes from every workspace.
  // 활성 ws는 인덱스가 아닌 id로 추적 — 죽은 ws filter가 인덱스를 어긋나게 해도 유지된다
  const activeWsId = workspaces[activeWs]?.id;
  for (const ws of workspaces) {
    for (const sid of collectSids(ws.tree)) {
      if (!liveIds.has(sid)) ws.tree = ws.tree ? replaceNode(ws.tree, sid, () => null) : null;
    }
  }
  workspaces = workspaces.filter((ws) => ws.tree !== null);
  // 구버전 자동 번호 이름("ws N")은 미정 표시로 이행
  for (const ws of workspaces) {
    if (/^ws \d+$/.test(ws.name)) ws.name = UNTITLED;
  }
  if (workspaces.length === 0) {
    workspaces = [{ id: wsCounter++, name: UNTITLED, tree: null }];
  }
  const restoredIdx = workspaces.findIndex((ws) => ws.id === activeWsId);
  activeWs = restoredIdx >= 0 ? restoredIdx : Math.min(activeWs, workspaces.length - 1);

  // Create runtimes for everything still referenced; merge orphans into the active workspace.
  const inLayout = allLayoutSids();
  for (const s of live) {
    await makePane(s.surface_id, s.title);
    if (!inLayout.has(s.surface_id)) {
      const ws = current();
      ws.tree = ws.tree
        ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid: s.surface_id } }
        : { type: "pane", sid: s.surface_id };
    }
  }
  if (!current().tree) {
    const sid = await newSurface();
    current().tree = { type: "pane", sid };
  }
  render();
  const first = collectSids(current().tree)[0];
  if (first != null) setFocus(first);
  refreshFeed();
  started = true; // 복원 완료 — 이 시점부터 인터벌 자동 입양 허용
  refreshPaneTitles();
}

// ---------- ui wiring ----------

document.getElementById("btn-new")!.addEventListener("click", actionNew);
document.getElementById("btn-split-h")!.addEventListener("click", () => actionSplit("row"));
document.getElementById("btn-split-v")!.addEventListener("click", () => actionSplit("col"));
document.getElementById("btn-equalize")!.addEventListener("click", actionEqualize);
document.getElementById("btn-close")!.addEventListener("click", actionClose);
document.getElementById("btn-feed")!.addEventListener("click", () => setFeedOpen(!feedOpen));
document.getElementById("btn-feed-close")!.addEventListener("click", () => setFeedOpen(false));
document.getElementById("btn-files")!.addEventListener("click", () => setFtOpen(!ftOpen));
document.getElementById("btn-ft-close")!.addEventListener("click", () => setFtOpen(false));
document.getElementById("btn-cc")!.addEventListener("click", () => setCcOpen(!ccOpen));
document.getElementById("btn-cc-close")!.addEventListener("click", () => setCcOpen(false));
document.getElementById("btn-update")!.addEventListener("click", () => promptInstall());
document.getElementById("btn-ws-new")!.addEventListener("click", () => addWorkspace());

window.addEventListener("keydown", (e) => {
  if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 무시
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  if (e.key === "t") {
    e.preventDefault();
    actionNew();
  } else if (e.key === "d" && !e.shiftKey) {
    e.preventDefault();
    actionSplit("row");
  } else if ((e.key === "D" || e.key === "d") && e.shiftKey) {
    e.preventDefault();
    actionSplit("col");
  } else if (e.key === "w") {
    e.preventDefault();
    actionClose();
  } else if (e.key === "=" || e.key === "+") {
    e.preventDefault();
    applyZoom(+1);
  } else if (e.key === "-") {
    e.preventDefault();
    applyZoom(-1);
  } else if (e.key === "0") {
    e.preventDefault();
    applyZoom(null);
  }
});

start().catch((e) => {
  document.getElementById("daemon-info")!.textContent = `startup failed: ${e}`;
});
