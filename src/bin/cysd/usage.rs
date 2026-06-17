//! T5 사용량 관측 수집기 — 에이전트 CLI의 로컬 산출물을 무간섭(passive) 관측해
//! context 사용량·rate limit 잔량을 결정론 산출한다. `cys set-status` 자기보고(LLM 추론)의
//! 관측 보강 — 절대지침 "결정론 환원"의 사용량 축.
//!
//! 데이터 소스 (실측 검증 2026-06-13):
//! - claude: `~/.claude*/projects/<munged-cwd>/<session>.jsonl` — assistant 라인의
//!   `message.usage`. 현재 컨텍스트 = input + cache_read + cache_creation (output 제외 —
//!   공식 statusline 문서의 used_percentage 공식과 동일). `isSidechain:true`(서브에이전트)
//!   라인은 메인 컨텍스트가 아니므로 제외. rate limit은 로컬 파일에 없음(Phase 2 statusline).
//! - codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` — `token_count` 이벤트의
//!   `info.last_token_usage`(컨텍스트)·`model_context_window`·`rate_limits`(primary 5h /
//!   secondary 7d, used_percent·resets_at).
//! - gemini(agy): 토큰·쿼터를 평문 로컬 파일에 남기지 않음 — Phase 2(로컬 RPC) 대상, 여기선 스킵.
//!
//! pane↔세션 매핑 우선순위:
//! ① `usage.register` RPC (SessionStart hook이 transcript_path를 등록 — 같은 cwd 동시
//!    세션 다수와 무관한 결정론 1:1)
//! ② codex: 에이전트 프로세스의 열린 fd(lsof)에서 rollout 경로 직독
//! ③ 휴리스틱 폴백: 에이전트 프로세스 cwd 기준 디렉터리에서 pane 생성 이후 mtime 최신 파일
//!    (동시 세션 경합 시 오귀속 가능 — usage.source로 구분 노출)

use crate::state::{now_epoch, Daemon, Surface};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 최초 attach 시 파일 끝에서 거슬러 읽는 창 (최신 usage 라인은 이 안에 있다)
const FIRST_ATTACH_TAIL: u64 = 256 * 1024;
/// 틱당 최대 읽기 — 초과분은 따라잡기를 포기하고 마지막 창으로 점프 (데몬 정체 방지)
const MAX_READ_PER_TICK: u64 = 4 * 1024 * 1024;
/// 미완성 라인 carry 상한 — 초과 시 폐기 (개행 없는 거대 라인의 메모리 무한 성장 차단)
const MAX_CARRY: usize = 8 * 1024 * 1024;
/// 휴리스틱(비등록) 매핑의 재발견 주기 초 — 새 세션 파일(/clear 등) 전환 추적
const REDISCOVER_SECS: f64 = 30.0;
/// statusline 보고(usage.report) 신선도 창 초 — claude는 이 안에 statusline 보고가 있으면
/// 트랜스크립트 tail이 ctx를 덮어써 rate limit을 유실시키지 않게 수집을 건너뛴다(우선순위 병합).
const STATUSLINE_FRESH_SECS: f64 = 60.0;

/// rate limit 윈도우 1개 (codex primary/secondary; Phase 2에서 claude 5h/7d 합류)
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RateWindow {
    pub label: String, // "5h" | "7d" | "Nm" | "?"
    pub used_pct: f64,
    pub resets_at: Option<f64>, // unix epoch 초
}

/// 관측 사용량 스냅샷 — Surface.observed_usage에 저장, surface.list/org.status로 노출
#[derive(Clone, Debug, serde::Serialize)]
pub struct ObservedUsage {
    pub agent: String,
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub ctx_pct: Option<u8>,
    pub rate: Vec<RateWindow>,
    pub source: String, // "transcript" | "transcript:heuristic" | "rollout" | "rollout:heuristic"
    pub session_file: String,
    pub updated_at: f64,
}

/// surface별 tail 진행 상태 (수집기 태스크 로컬 — 데몬 상태 오염 없음)
struct TailState {
    path: PathBuf,
    offset: u64,
    carry: String,
    /// 휴리스틱 매핑 여부 — true면 REDISCOVER_SECS마다 재발견 (등록 매핑은 고정)
    heuristic: bool,
    last_discovery: f64,
}

fn poll_secs() -> u64 {
    cys::env_compat("CYS_USAGE_POLL_SECS")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(2)
}

pub fn spawn_usage_collector(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut tails: HashMap<u64, TailState> = HashMap::new();
        let mut attempts: HashMap<u64, f64> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(poll_secs())).await;
            // 패닉 격리 — watchdog과 동일: 한 틱의 패닉이 수집기를 영구 침묵시키지 않게
            let tick = std::panic::AssertUnwindSafe(|| {
                collect_tick(&daemon, &mut tails, &mut attempts)
            });
            if std::panic::catch_unwind(tick).is_err() {
                daemon.bus.publish(
                    "usage.tick_panic",
                    "usage",
                    None,
                    json!({"note": "usage collector tick panicked; continuing next tick"}),
                );
            }
        }
    });
}

fn collect_tick(
    daemon: &Arc<Daemon>,
    tails: &mut HashMap<u64, TailState>,
    attempts: &mut HashMap<u64, f64>,
) {
    let surfaces: Vec<Arc<Surface>> = daemon.surfaces.lock().unwrap().values().cloned().collect();
    let live_ids: HashSet<u64> = surfaces
        .iter()
        .filter(|s| !s.exited.load(Ordering::Relaxed))
        .map(|s| s.id)
        .collect();
    tails.retain(|sid, _| live_ids.contains(sid));
    attempts.retain(|sid, _| live_ids.contains(sid));
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, bin)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        match agent.as_str() {
            "claude" => collect_for(daemon, &s, "claude", &bin, tails, attempts),
            "codex" => collect_for(daemon, &s, "codex", &bin, tails, attempts),
            // gemini(agy)·grok: 로컬 평문 산출물에 토큰 미기록 — Phase 2 (로컬 RPC) 대상
            _ => {}
        }
    }
}

/// 단일 surface 수집: 세션 파일 결정 → 증분 read → 파싱 → 스냅샷 갱신 → 이벤트 발행
fn collect_for(
    daemon: &Arc<Daemon>,
    s: &Arc<Surface>,
    agent: &str,
    bin: &str,
    tails: &mut HashMap<u64, TailState>,
    attempts: &mut HashMap<u64, f64>,
) {
    let registered = s.registered_transcript.lock().unwrap().clone();
    let now = now_epoch();

    // T5 Phase 2-A 우선순위 병합 — claude는 statusline 보고(rate limit + 서버 진실 ctx)가
    // 신선하면 트랜스크립트 tail이 ctx만 덮어써 rate를 유실시키지 않도록 수집을 건너뛴다.
    // statusline이 끊기면(age > STATUSLINE_FRESH_SECS) 트랜스크립트로 graceful 폴백.
    if agent == "claude" {
        if let Some(prev) = s.observed_usage.lock().unwrap().as_ref() {
            if prev.source == "statusline" && now - prev.updated_at < STATUSLINE_FRESH_SECS {
                return;
            }
        }
    }

    // ── 세션 파일 결정 (등록 > lsof > 휴리스틱) ──
    let desired: Option<(PathBuf, bool)> = if let Some(reg) = registered {
        Some((PathBuf::from(reg), false))
    } else {
        let need_discovery = match tails.get(&s.id) {
            None => true,
            Some(t) => {
                !t.path.exists() || (t.heuristic && now - t.last_discovery > REDISCOVER_SECS)
            }
        };
        let existing = || {
            tails
                .get(&s.id)
                .filter(|t| t.path.exists())
                .map(|t| (t.path.clone(), t.heuristic))
        };
        if need_discovery {
            // 발견 백오프: 실패가 반복돼도 전수 프로세스 refresh·lsof는 REDISCOVER_SECS에
            // 1회만 (자원 거버넌스 — 트랜스크립트가 아직 없는 pane이 틱마다 비용 유발 금지)
            let recently = attempts
                .get(&s.id)
                .map(|t| now - *t < REDISCOVER_SECS)
                .unwrap_or(false);
            if recently {
                existing()
            } else {
                attempts.insert(s.id, now);
                discover_session_file(s, agent, bin)
                    .map(|p| (p, true))
                    .or_else(existing)
            }
        } else {
            existing()
        }
    };
    let Some((path, heuristic)) = desired else {
        // 미발견 — 다음 재발견 시도까지 빈 상태 유지 (배지 없음이 정직한 표현)
        return;
    };

    // tail 상태 초기화/전환: 경로가 바뀌었으면 파일 끝 창에서 새로 시작
    let need_reset = tails.get(&s.id).map(|t| t.path != path).unwrap_or(true);
    if need_reset {
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tails.insert(
            s.id,
            TailState {
                path: path.clone(),
                offset: len.saturating_sub(FIRST_ATTACH_TAIL),
                carry: String::new(),
                heuristic,
                last_discovery: now,
            },
        );
        // 새 세션 파일 = 새 세션 — 에지 게이트 재무장. 직전 세션이 임계 위에서 끝났어도
        // 새 세션이 곧장 임계 이상으로 시작하면(거대 지침 재주입) 발화해야 한다.
        s.ctx_threshold_armed.store(true, Ordering::Relaxed);
    } else if let Some(t) = tails.get_mut(&s.id) {
        t.heuristic = heuristic;
        if heuristic {
            t.last_discovery = now;
        }
    }
    let Some(state) = tails.get_mut(&s.id) else {
        return;
    };

    // ── 증분 read + 파싱 (마지막 유효 관측이 승리) ──
    let lines = read_new_lines(state);
    if lines.is_empty() {
        return;
    }
    let prev = s.observed_usage.lock().unwrap().clone();
    let mut next: Option<ObservedUsage> = None;
    for line in &lines {
        match agent {
            "claude" => {
                if let Some((ctx_tokens, model)) = parse_claude_line(line) {
                    let window = claude_ctx_window(&model);
                    next = Some(ObservedUsage {
                        agent: agent.into(),
                        ctx_tokens: Some(ctx_tokens),
                        ctx_window: Some(window),
                        ctx_pct: pct(ctx_tokens, window),
                        rate: next
                            .as_ref()
                            .map(|n| n.rate.clone())
                            .or_else(|| prev.as_ref().map(|p| p.rate.clone()))
                            .unwrap_or_default(),
                        source: source_label("transcript", state.heuristic),
                        session_file: state.path.to_string_lossy().into_owned(),
                        updated_at: now,
                    });
                }
            }
            "codex" => {
                if let Some(obs) = parse_codex_line(line) {
                    // 필드별 병합: token_count 이벤트에 info/rate_limits가 따로 올 수 있다
                    let base = next.as_ref().or(prev.as_ref());
                    let ctx_tokens = obs.ctx_tokens.or(base.and_then(|b| b.ctx_tokens));
                    let ctx_window = obs.ctx_window.or(base.and_then(|b| b.ctx_window));
                    let rate = obs
                        .rate
                        .or_else(|| base.map(|b| b.rate.clone()))
                        .unwrap_or_default();
                    next = Some(ObservedUsage {
                        agent: agent.into(),
                        ctx_tokens,
                        ctx_window,
                        ctx_pct: ctx_tokens
                            .zip(ctx_window)
                            .and_then(|(t, w)| pct(t, w)),
                        rate,
                        source: source_label("rollout", state.heuristic),
                        session_file: state.path.to_string_lossy().into_owned(),
                        updated_at: now,
                    });
                }
            }
            _ => {}
        }
    }
    let Some(new) = next else {
        return;
    };

    // ── 스냅샷 갱신 + 이벤트 (정수 % 변화시에만 — 이벤트 폭주 차단) ──
    let changed = prev
        .as_ref()
        .map(|p| p.ctx_pct != new.ctx_pct || p.rate != new.rate)
        .unwrap_or(true);
    *s.observed_usage.lock().unwrap() = Some(new.clone());
    if changed {
        daemon.bus.publish(
            "usage.updated",
            "usage",
            Some(s.id),
            json!({
                "surface_ref": cys::surface_ref(s.id),
                "role": s.role.lock().unwrap().clone(),
                "agent": new.agent, "ctx_pct": new.ctx_pct, "ctx_tokens": new.ctx_tokens,
                "ctx_window": new.ctx_window, "rate": new.rate, "source": new.source,
            }),
        );
    }
    // 결정론 컨텍스트 임계 — 자기보고(status.set)와 **공유 에지 게이트**(ctx_threshold_armed)
    // 로 발화한다. 분리된 에지 상태를 쓰면 같은 교차에 두 경로가 각각 발화해 master/CSO가
    // cycle-agent를 이중 집행한다. payload source:"observed"로 자기보고 발화와 구분.
    if let Some(p) = new.ctx_pct {
        crate::handlers::maybe_fire_context_threshold(daemon, s, p, "observed", Some(&new.agent));
    }
}

fn source_label(base: &str, heuristic: bool) -> String {
    if heuristic {
        format!("{base}:heuristic")
    } else {
        base.into()
    }
}

// ───────────────────────── 세션 파일 발견 ─────────────────────────

/// 에이전트별 세션 파일 발견 (등록 부재 시) — claude: 프로필 스캔 / codex: lsof → 휴리스틱
fn discover_session_file(s: &Arc<Surface>, agent: &str, bin: &str) -> Option<PathBuf> {
    let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(bin);
    let (agent_pid, agent_cwd) = find_agent_descendant(s.pid, bin_base);
    let cwd = agent_cwd.unwrap_or_else(|| s.cwd.clone());
    match agent {
        "claude" => discover_claude_transcript(&cwd, s.created_at),
        "codex" => agent_pid
            .and_then(discover_codex_rollout_lsof)
            .or_else(|| discover_codex_rollout(&cwd, s.created_at)),
        _ => None,
    }
}

/// surface 자식 트리에서 에이전트 프로세스의 (pid, cwd)를 찾는다 — 발견 시점에만 호출
/// (전수 프로세스 refresh 비용이 있어 매 틱 호출 금지).
fn find_agent_descendant(surface_pid: u32, bin_base: &str) -> (Option<u32>, Option<String>) {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let pid = crate::governance::collect_descendants(&sys, surface_pid)
        .into_iter()
        .find(|(_, cmdline)| crate::governance::cmdline_matches_agent(cmdline, bin_base))
        .map(|(p, _)| p);
    let cwd = pid.and_then(|p| {
        sys.process(sysinfo::Pid::from_u32(p))
            .and_then(|pr| pr.cwd())
            .map(|c| c.display().to_string())
    });
    (pid, cwd)
}

/// claude 휴리스틱: `~/.claude*` 전 프로필의 projects/<munged>/ 에서 pane 생성 이후
/// mtime 최신 .jsonl (심링크 프로필은 canonicalize로 중복 제거)
fn discover_claude_transcript(cwd: &str, created_at: f64) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let comp = claude_project_component(cwd);
    let mut best: Option<(f64, PathBuf)> = None;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for e in std::fs::read_dir(&home).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != ".claude" && !name.starts_with(".claude-") {
            continue;
        }
        let proj = e.path().join("projects").join(&comp);
        let canon = std::fs::canonicalize(&proj).unwrap_or_else(|_| proj.clone());
        if !seen.insert(canon) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&proj) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let mt = mtime_epoch(&p);
            // pane 생성 5초 전까지 허용 (시계 흔들림 여유) — 그 이전 세션은 남의 것
            if mt + 5.0 < created_at {
                continue;
            }
            if best.as_ref().map(|(b, _)| mt > *b).unwrap_or(true) {
                best = Some((mt, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// codex 결정론: 에이전트 프로세스가 열어둔 rollout 파일 fd를 lsof로 직독 (unix 전용 —
/// 실패·미설치 시 None → 휴리스틱 폴백)
fn discover_codex_rollout_lsof(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .find(|p| p.contains("/sessions/") && p.contains("rollout-") && p.ends_with(".jsonl"))
        .map(PathBuf::from)
}

/// codex 휴리스틱: 최근 3개 날짜 디렉터리에서 session_meta.cwd 일치 + pane 생성 이후
/// mtime 최신 rollout
fn discover_codex_rollout(cwd: &str, created_at: f64) -> Option<PathBuf> {
    let base = dirs::home_dir()?.join(".codex").join("sessions");
    let mut day_dirs: Vec<PathBuf> = Vec::new();
    'outer: for y in read_subdirs_desc(&base) {
        for m in read_subdirs_desc(&y) {
            for d in read_subdirs_desc(&m) {
                day_dirs.push(d);
                if day_dirs.len() >= 3 {
                    break 'outer;
                }
            }
        }
    }
    let mut best: Option<(f64, PathBuf)> = None;
    for dir in day_dirs {
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let mt = mtime_epoch(&p);
            if mt + 5.0 < created_at {
                continue;
            }
            if rollout_first_line_cwd(&p).as_deref() != Some(cwd) {
                continue;
            }
            if best.as_ref().map(|(b, _)| mt > *b).unwrap_or(true) {
                best = Some((mt, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn read_subdirs_desc(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

fn rollout_first_line_cwd(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(&line).ok()?;
    v["payload"]["cwd"]
        .as_str()
        .or_else(|| v["cwd"].as_str())
        .map(|s| s.to_string())
}

fn mtime_epoch(p: &Path) -> f64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ───────────────────────── 증분 tail ─────────────────────────

/// offset 이후의 완성 라인들을 읽는다. 절단(truncate)·회전 감지 시 마지막 창으로 재정렬,
/// 틱당 읽기 상한 초과 시 따라잡기를 포기하고 점프 (최신 관측만 필요하므로 안전).
fn read_new_lines(state: &mut TailState) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(&state.path) else {
        return Vec::new();
    };
    let len = meta.len();
    if len < state.offset {
        state.offset = len.saturating_sub(FIRST_ATTACH_TAIL);
        state.carry.clear();
    }
    if len == state.offset {
        return Vec::new();
    }
    if len - state.offset > MAX_READ_PER_TICK {
        state.offset = len.saturating_sub(FIRST_ATTACH_TAIL);
        state.carry.clear();
    }
    let to_read = len - state.offset;
    let Ok(mut f) = std::fs::File::open(&state.path) else {
        return Vec::new();
    };
    if f.seek(SeekFrom::Start(state.offset)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::with_capacity(to_read as usize);
    if f.take(to_read).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    state.offset += buf.len() as u64;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let mut combined = std::mem::take(&mut state.carry);
    combined.push_str(&text);
    let ends_nl = combined.ends_with('\n');
    let mut parts: Vec<&str> = combined.split('\n').collect();
    if ends_nl {
        parts.pop(); // 끝 개행 뒤 빈 조각
    } else if let Some(tail) = parts.pop() {
        if tail.len() <= MAX_CARRY {
            state.carry = tail.to_string();
        }
        // 상한 초과 미완성 라인은 폐기 — 다음 개행부터 재동기화
    }
    parts.iter().map(|s| s.to_string()).collect()
}

// ───────────────────────── 파서 (순수함수 — 테스트 핀) ─────────────────────────

/// claude 트랜스크립트 assistant 라인 → (현재 컨텍스트 토큰, 모델명).
/// 컨텍스트 = input + cache_read + cache_creation (output 제외 — 공식 문서 공식).
/// isSidechain:true(서브에이전트 트래픽)는 메인 컨텍스트가 아니므로 None.
pub fn parse_claude_line(line: &str) -> Option<(u64, String)> {
    // 빠른 필터: 전체 JSON 파싱 전 후보 라인만 통과 (트랜스크립트 대부분은 비대상)
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str() != Some("assistant") {
        return None;
    }
    if v["isSidechain"].as_bool() == Some(true) {
        return None;
    }
    let u = &v["message"]["usage"];
    if !u.is_object() {
        return None;
    }
    let g = |k: &str| u[k].as_u64().unwrap_or(0);
    let ctx = g("input_tokens") + g("cache_read_input_tokens") + g("cache_creation_input_tokens");
    if ctx == 0 {
        return None; // usage 없는 합성/에러 라인
    }
    let model = v["message"]["model"].as_str().unwrap_or("").to_string();
    Some((ctx, model))
}

/// claude 컨텍스트 윈도우 추정: 기본 200k, 1M 모델([1m])은 1M. CYS_CLAUDE_CTX_WINDOW로
/// 강제 가능 (passive 관측에선 서버 진실값이 없다 — Phase 2 statusline이 정밀값 제공).
pub fn claude_ctx_window(model: &str) -> u64 {
    if let Some(v) = cys::env_compat("CYS_CLAUDE_CTX_WINDOW").and_then(|v| v.parse().ok()) {
        return v;
    }
    if model.contains("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// codex token_count 이벤트의 부분 관측 (info / rate_limits가 따로 올 수 있어 Option 병합)
#[derive(Debug, PartialEq)]
pub struct CodexObs {
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub rate: Option<Vec<RateWindow>>,
}

/// codex rollout 라인 → 컨텍스트·rate limit 관측.
/// 컨텍스트 점유 ≈ last_token_usage.total - reasoning (reasoning 토큰은 컨텍스트에 잔존 안 함).
pub fn parse_codex_line(line: &str) -> Option<CodexObs> {
    if !line.contains("token_count") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let p = &v["payload"];
    if p["type"].as_str() != Some("token_count") {
        return None;
    }
    let info = &p["info"];
    let (ctx_tokens, ctx_window) = if info.is_object() {
        let last = if info["last_token_usage"].is_object() {
            &info["last_token_usage"]
        } else {
            &info["total_token_usage"]
        };
        let total = last["total_tokens"].as_u64().unwrap_or(0);
        let reasoning = last["reasoning_output_tokens"].as_u64().unwrap_or(0);
        (
            Some(total.saturating_sub(reasoning)),
            info["model_context_window"].as_u64(),
        )
    } else {
        (None, None)
    };
    let rl = &p["rate_limits"];
    let rate = if rl.is_object() {
        let mut ws = Vec::new();
        for key in ["primary", "secondary"] {
            let w = &rl[key];
            if let Some(used) = w["used_percent"].as_f64() {
                ws.push(RateWindow {
                    label: window_label(w["window_minutes"].as_u64().unwrap_or(0)),
                    used_pct: used,
                    resets_at: w["resets_at"].as_f64(),
                });
            }
        }
        Some(ws)
    } else {
        None
    };
    if ctx_tokens.is_none() && rate.is_none() {
        return None;
    }
    Some(CodexObs {
        ctx_tokens,
        ctx_window,
        rate,
    })
}

/// rate limit 윈도우 분 → 사람이 읽는 라벨 (300→"5h", 10080→"7d")
pub fn window_label(minutes: u64) -> String {
    match minutes {
        0 => "?".into(),
        m if m % (24 * 60) == 0 => format!("{}d", m / (24 * 60)),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// 사용률 % (반올림·100 상한). window 0은 None — 0 나눗셈·무의미 값 차단.
pub fn pct(tokens: u64, window: u64) -> Option<u8> {
    if window == 0 {
        return None;
    }
    Some(((tokens as f64 / window as f64) * 100.0).round().min(100.0) as u8)
}

/// Claude Code projects/ 디렉터리명 munge — 실측: '/'와 특수문자가 '-'로 치환된다.
/// ASCII 영숫자·'-'만 보존하는 보수 구현 (휴리스틱 폴백 전용 — 1순위는 hook 등록).
pub fn claude_project_component(cwd: &str) -> String {
    cwd.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// ───────────────────────── T5 Phase 2-B: agy(Antigravity) 쿼터 ─────────────────────────
// agy는 토큰·쿼터를 평문 로컬 파일에 안 남긴다 — 실행 중 프로세스의 로컬 LS RPC(HTTPS,
// self-signed, 127.0.0.1 무인증)로만 노출된다(2026-06-17 라이브 프로브 실측). 포트는 매
// 실행 변동 → lsof로 발견·probe로 검증·캐시. 파일 tail 수집기와 분리된 저빈도 비동기
// 태스크(async curl — tokio 워커 미블로킹). HTTP 클라이언트 의존성을 더하지 않으려 curl
// 셸아웃을 쓴다(codex의 lsof 셸아웃과 동형). 실패·미설치는 graceful(배지 없음 유지).

const AGY_SVC: &str = "exa.language_server_pb.LanguageServerService";

fn agy_poll_secs() -> u64 {
    cys::env_compat("CYS_AGY_POLL_SECS")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(15)
}

/// RetrieveUserQuotaSummary 응답 → RateWindow 벡터 (Gemini 그룹만 — agy 기본 모델).
/// 실측 스키마: `response.groups[].buckets[]{window("5h"|"weekly"), remainingFraction, resetTime}`.
/// used_pct = (1-remainingFraction)*100, weekly→"7d"(claude/codex 배지와 라벨 통일), ISO8601→epoch.
/// PII(GetUserStatus의 name/email)는 건드리지 않는다 — 쿼터 숫자만.
pub fn parse_agy_quota(v: &Value) -> Vec<RateWindow> {
    let mut out = Vec::new();
    let Some(groups) = v["response"]["groups"].as_array() else {
        return out;
    };
    for g in groups {
        if !g["displayName"].as_str().unwrap_or("").contains("Gemini") {
            continue; // 3p(Claude/GPT) 그룹 제외 — agy 기본은 Gemini
        }
        for b in g["buckets"].as_array().into_iter().flatten() {
            let Some(frac) = b["remainingFraction"].as_f64() else {
                continue;
            };
            let label = match b["window"].as_str().unwrap_or("") {
                "5h" => "5h",
                "weekly" => "7d",
                other => other,
            };
            let resets_at = b["resetTime"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp() as f64);
            out.push(RateWindow {
                label: label.to_string(),
                used_pct: ((1.0 - frac) * 100.0).clamp(0.0, 100.0),
                resets_at,
            });
        }
    }
    out.sort_by_key(|r| u8::from(r.label != "5h")); // 5h 먼저, 7d 다음 (배지 순서 안정)
    out
}

/// agy 프로세스가 LISTEN하는 127.0.0.1/localhost 포트 목록 (lsof — codex 패턴 동형, 와일드카드 제외).
async fn agy_listen_ports(pid: u32) -> Vec<u16> {
    let Ok(out) = tokio::process::Command::new("lsof")
        .args(["-nP", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN", "-Fn"])
        .output()
        .await
    else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some(rest) = line.strip_prefix('n') else {
            continue;
        };
        if !(rest.starts_with("localhost:") || rest.starts_with("127.0.0.1:")) {
            continue; // 로컬 바인드만 — agy LS는 localhost
        }
        if let Some(p) = rest.rsplit(':').next().and_then(|s| s.parse::<u16>().ok()) {
            if !ports.contains(&p) {
                ports.push(p);
            }
        }
    }
    ports.truncate(12); // 폭주 가드 — 후보 과다 시 probe 비용 상한
    ports
}

/// 한 포트로 RetrieveUserQuotaSummary 프로브 (async curl -sk, self-signed 수용·2s 타임아웃).
/// 성공 시 Gemini 쿼터 RateWindow, 아니면 None(잘못된 포트·실패).
async fn agy_quota_probe(port: u16) -> Option<Vec<RateWindow>> {
    let url = format!("https://127.0.0.1:{port}/{AGY_SVC}/RetrieveUserQuotaSummary");
    let fut = tokio::process::Command::new("curl")
        .args([
            "-sk",
            "--max-time",
            "2",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-H",
            "connect-protocol-version: 1",
            "--data",
            "{}",
            &url,
        ])
        .output();
    let out = tokio::time::timeout(Duration::from_secs(3), fut)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let rate = parse_agy_quota(&v);
    if rate.is_empty() {
        None
    } else {
        Some(rate)
    }
}

/// agy 쿼터를 surface.observed_usage(source:"agy-rpc")에 반영 + usage.updated 발행.
/// agy는 context window를 안 주므로 ctx_pct=None(배지는 쿼터만). 임계(context.threshold)는
/// ctx_pct가 없으니 발화 대상 아님.
fn update_agy_usage(daemon: &Arc<Daemon>, s: &Arc<Surface>, rate: Vec<RateWindow>) {
    let new = ObservedUsage {
        agent: "gemini".into(),
        ctx_tokens: None,
        ctx_window: None,
        ctx_pct: None,
        rate,
        source: "agy-rpc".into(),
        session_file: String::new(),
        updated_at: now_epoch(),
    };
    let changed = s
        .observed_usage
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.rate != new.rate || p.source != new.source)
        .unwrap_or(true);
    *s.observed_usage.lock().unwrap() = Some(new.clone());
    if changed {
        daemon.bus.publish(
            "usage.updated",
            "usage",
            Some(s.id),
            json!({
                "surface_ref": cys::surface_ref(s.id),
                "role": s.role.lock().unwrap().clone(),
                "agent": "gemini", "ctx_pct": Value::Null,
                "rate": new.rate, "source": "agy-rpc",
            }),
        );
    }
}

/// 한 agy surface의 쿼터 수집 — 캐시 포트 우선, 실패 시 lsof 재발견·probe. 전부 실패면 graceful.
async fn collect_agy_for(daemon: &Arc<Daemon>, s: &Arc<Surface>, ports: &mut HashMap<u64, u16>) {
    let mut candidates: Vec<u16> = Vec::new();
    if let Some(p) = ports.get(&s.id) {
        candidates.push(*p);
    }
    let (agy_pid, _) = find_agent_descendant(s.pid, "agy");
    if let Some(pid) = agy_pid {
        for p in agy_listen_ports(pid).await {
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for port in candidates {
        if let Some(rate) = agy_quota_probe(port).await {
            ports.insert(s.id, port);
            update_agy_usage(daemon, s, rate);
            return;
        }
    }
    ports.remove(&s.id); // 캐시 무효화 — 다음 틱에 재발견 (배지는 갱신 안 함 = 정직)
}

/// agy(Antigravity) 쿼터 수집기 — 파일 tail과 분리된 저빈도 비동기 태스크.
pub fn spawn_agy_collector(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut ports: HashMap<u64, u16> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(agy_poll_secs())).await;
            let surfaces: Vec<Arc<Surface>> = {
                daemon
                    .surfaces
                    .lock()
                    .unwrap()
                    .values()
                    .filter(|s| !s.exited.load(Ordering::Relaxed))
                    .filter(|s| {
                        s.agent_meta
                            .lock()
                            .unwrap()
                            .as_ref()
                            .map(|(a, _)| a == "gemini")
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            };
            let live: HashSet<u64> = surfaces.iter().map(|s| s.id).collect();
            ports.retain(|sid, _| live.contains(sid));
            for s in surfaces {
                collect_agy_for(&daemon, &s, &mut ports).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── claude 파서: 실측 스키마(2026-06-13, CLI 2.1.176) 핀 ──

    fn claude_line(extra: &str, usage: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":false,"requestId":"req_1","sessionId":"s","timestamp":"t"{extra},"message":{{"model":"claude-fable-5","usage":{usage}}}}}"#
        )
    }

    #[test]
    fn claude_ctx_is_input_plus_both_caches_excluding_output() {
        // 공식 statusline 문서 공식: used = input + cache_creation + cache_read (output 제외).
        // 실측값 2+82077+717=82796 — output_tokens가 합산되면 이 핀이 깨진다.
        let line = claude_line(
            "",
            r#"{"input_tokens":2,"cache_creation_input_tokens":717,"cache_read_input_tokens":82077,"output_tokens":999}"#,
        );
        let (ctx, model) = parse_claude_line(&line).expect("assistant usage 라인 파싱 실패");
        assert_eq!(ctx, 82_796);
        assert_eq!(model, "claude-fable-5");
    }

    #[test]
    fn claude_sidechain_lines_are_excluded() {
        // 서브에이전트(isSidechain:true) 트래픽은 메인 컨텍스트가 아니다 — 섞이면
        // 메인 pane 배지가 서브에이전트 컨텍스트로 오염된다.
        let line = claude_line("", r#"{"input_tokens":50000}"#).replace(
            r#""isSidechain":false"#,
            r#""isSidechain":true"#,
        );
        assert_eq!(parse_claude_line(&line), None);
    }

    #[test]
    fn claude_non_assistant_and_zero_usage_skipped() {
        assert_eq!(
            parse_claude_line(r#"{"type":"user","message":{"usage":{"input_tokens":5}}}"#),
            None,
            "user 라인은 무시"
        );
        let zero = claude_line("", r#"{"input_tokens":0,"output_tokens":3}"#);
        assert_eq!(parse_claude_line(&zero), None, "입력측 0은 합성 라인 — 무시");
        assert_eq!(parse_claude_line("not json"), None);
        assert_eq!(parse_claude_line(""), None);
    }

    #[test]
    fn claude_window_default_and_1m_variant() {
        // ★테스트 격리: 런타임 환경(예: Claude Code 세션)이 CYS_CLAUDE_CTX_WINDOW(또는
        // JAVIS_/AITERM_ 호환 별칭)을 설정하면 env 오버라이드가 모델 기본값을 덮어 이 핀이
        // 거짓 실패한다. 모델 기반 분기만 검증하도록 해당 env를 제거 후 단언하고 복원한다.
        let keys = [
            "CYS_CLAUDE_CTX_WINDOW",
            "JAVIS_CLAUDE_CTX_WINDOW",
            "AITERM_CLAUDE_CTX_WINDOW",
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }
        assert_eq!(claude_ctx_window("claude-fable-5"), 200_000);
        assert_eq!(claude_ctx_window("claude-sonnet-4-6[1m]"), 1_000_000);
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }

    // ── codex 파서: 실측 스키마(2026-06-13, codex-cli 0.139.0) 핀 ──

    const CODEX_FULL: &str = r#"{"timestamp":"2026-06-12T23:38:22.044Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":26788,"cached_input_tokens":2432,"output_tokens":508,"reasoning_output_tokens":352,"total_tokens":27296},"last_token_usage":{"input_tokens":26788,"cached_input_tokens":2432,"output_tokens":508,"reasoning_output_tokens":352,"total_tokens":27296},"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":13.0,"window_minutes":300,"resets_at":1781314865},"secondary":{"used_percent":3.0,"window_minutes":10080,"resets_at":1781781650},"credits":null,"individual_limit":null,"plan_type":"plus","rate_limit_reached_type":null}}}"#;

    #[test]
    fn codex_full_event_yields_ctx_and_both_rate_windows() {
        let obs = parse_codex_line(CODEX_FULL).expect("token_count 파싱 실패");
        // 컨텍스트 = total - reasoning (27296 - 352)
        assert_eq!(obs.ctx_tokens, Some(26_944));
        assert_eq!(obs.ctx_window, Some(258_400));
        let rate = obs.rate.expect("rate_limits 누락");
        assert_eq!(rate.len(), 2);
        assert_eq!(rate[0].label, "5h");
        assert_eq!(rate[0].used_pct, 13.0);
        assert_eq!(rate[0].resets_at, Some(1_781_314_865.0));
        assert_eq!(rate[1].label, "7d");
        assert_eq!(rate[1].used_pct, 3.0);
    }

    #[test]
    fn codex_rate_only_event_keeps_ctx_none() {
        // 일부 모드는 info 없이 rate_limits만 싣는다 (codex #14880) — 부분 관측 허용
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":50.5,"window_minutes":300,"resets_at":1781314865}}}}"#;
        let obs = parse_codex_line(line).expect("rate-only 파싱 실패");
        assert_eq!(obs.ctx_tokens, None);
        assert_eq!(obs.rate.as_ref().map(|r| r.len()), Some(1));
        assert_eq!(obs.rate.unwrap()[0].used_pct, 50.5);
    }

    #[test]
    fn codex_non_token_count_lines_skipped() {
        assert_eq!(
            parse_codex_line(r#"{"type":"session_meta","payload":{"cwd":"/x"}}"#),
            None
        );
        assert_eq!(
            parse_codex_line(r#"{"type":"event_msg","payload":{"type":"agent_message"}}"#),
            None
        );
        // payload.type은 token_count지만 내용이 전무 — None
        assert_eq!(
            parse_codex_line(
                r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#
            ),
            None
        );
    }

    #[test]
    fn window_labels_match_known_codex_windows() {
        assert_eq!(window_label(300), "5h");
        assert_eq!(window_label(10080), "7d");
        assert_eq!(window_label(90), "90m");
        assert_eq!(window_label(0), "?");
        assert_eq!(window_label(1440), "1d");
    }

    #[test]
    fn pct_rounds_and_caps() {
        assert_eq!(pct(82_796, 200_000), Some(41));
        assert_eq!(pct(0, 200_000), Some(0));
        assert_eq!(pct(300_000, 200_000), Some(100), "윈도우 초과는 100 상한");
        assert_eq!(pct(1, 0), None, "윈도우 0 — 0 나눗셈 차단");
    }

    #[test]
    fn munge_matches_observed_directory_names() {
        // 실측: /Users/user/Desktop/CYSjavis/cys-terminal → -Users-user-Desktop-CYSjavis-cys-terminal
        assert_eq!(
            claude_project_component("/Users/user/Desktop/CYSjavis/cys-terminal"),
            "-Users-user-Desktop-CYSjavis-cys-terminal"
        );
        // 비ASCII·특수문자는 각각 '-' (보수 구현 — 휴리스틱 폴백 전용)
        assert_eq!(claude_project_component("/tmp/a.b_c"), "-tmp-a-b-c");
    }

    // ── 증분 tail: 회전·부분라인·따라잡기 한도 ──

    #[test]
    fn read_new_lines_handles_partial_lines_and_truncation() {
        let dir = std::env::temp_dir().join(format!("cys-usage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "line1\nline2\npart").unwrap();
        let mut st = TailState {
            path: path.clone(),
            offset: 0,
            carry: String::new(),
            heuristic: false,
            last_discovery: 0.0,
        };
        let lines = read_new_lines(&mut st);
        assert_eq!(lines, vec!["line1".to_string(), "line2".to_string()]);
        assert_eq!(st.carry, "part", "미완성 라인은 carry로 보류");
        // 이어서 완성 — carry와 합쳐 한 줄로
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, b"ial\n").unwrap();
        drop(f);
        assert_eq!(read_new_lines(&mut st), vec!["partial".to_string()]);
        // 절단(truncate) — offset 재정렬 후 새 내용 읽힘
        std::fs::write(&path, "fresh\n").unwrap();
        assert_eq!(read_new_lines(&mut st), vec!["fresh".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollout_first_line_cwd_reads_session_meta() {
        let dir = std::env::temp_dir().join(format!("cys-usage-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-x.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"t","type":"session_meta","payload":{"id":"u","cwd":"/work/dir","cli_version":"0.139.0"}}
{"type":"event_msg","payload":{"type":"token_count"}}
"#,
        )
        .unwrap();
        assert_eq!(rollout_first_line_cwd(&path).as_deref(), Some("/work/dir"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T5 Phase 2-B: agy RetrieveUserQuotaSummary 파싱 핀 — 2026-06-17 라이브 실측 스키마.
    /// Gemini 그룹만 추출(3p Claude/GPT 제외)·weekly→"7d"·used_pct=(1-remainingFraction)*100·
    /// resetTime ISO8601→epoch. PII(GetUserStatus의 name/email)는 만지지 않는다.
    #[test]
    fn agy_quota_parses_gemini_group_only() {
        let v: Value = serde_json::from_str(
            r#"{"response":{"groups":[
            {"displayName":"Gemini Models","buckets":[
                {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.9484245,"resetTime":"2026-06-19T20:29:38Z"},
                {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.993488,"resetTime":"2026-06-16T21:04:55Z"}]},
            {"displayName":"Claude and GPT models","buckets":[
                {"bucketId":"3p-5h","window":"5h","remainingFraction":1.0,"resetTime":"2026-06-16T21:25:07Z"}]}]}}"#,
        )
        .unwrap();
        let r = parse_agy_quota(&v);
        assert_eq!(r.len(), 2, "Gemini 그룹 2버킷만 — 3p 그룹 제외");
        assert_eq!(r[0].label, "5h", "5h 먼저 정렬");
        assert!((r[0].used_pct - 0.6512).abs() < 0.01, "5h used≈0.65: {}", r[0].used_pct);
        assert_eq!(r[1].label, "7d", "weekly→7d 라벨 통일");
        assert!((r[1].used_pct - 5.1576).abs() < 0.01, "weekly used≈5.16: {}", r[1].used_pct);
        assert!(r[0].resets_at.is_some(), "resetTime ISO8601→epoch 변환");
    }

    #[test]
    fn agy_quota_empty_on_no_groups_or_3p_only() {
        assert!(parse_agy_quota(&json!({})).is_empty());
        assert!(parse_agy_quota(&json!({"response":{"groups":[]}})).is_empty());
        // 3p 그룹만 있으면 빈 벡터 (Gemini 그룹 없음)
        let only3p = json!({"response":{"groups":[
            {"displayName":"Claude and GPT models","buckets":[
                {"bucketId":"3p-5h","window":"5h","remainingFraction":1.0}]}]}});
        assert!(parse_agy_quota(&only3p).is_empty());
    }
}
