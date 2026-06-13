//! Method dispatch: NDJSON request → handler → single response or stream upgrade.

use crate::governance;
use crate::state::{Daemon, FeedItem, HealthRule, LedgerEntry, DEFAULT_COLS, DEFAULT_ROWS};
use cys::{err_response, ok_response, parse_surface_ref, surface_ref, Request};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// What the connection loop should do after a request.
pub enum Reply {
    Single(Value),
    /// Upgrade connection to an event stream (push channel).
    EventStream {
        ack: Value,
        after_seq: Option<u64>,
        names: Vec<String>,
        categories: Vec<String>,
    },
    /// Upgrade connection to a raw PTY output stream.
    Attach {
        ack: Value,
        surface_id: u64,
    },
    /// Block the connection until the feed item is resolved (or timeout).
    FeedWait {
        id: Value,
        request_id: String,
        rx: tokio::sync::oneshot::Receiver<String>,
        timeout_secs: u64,
    },
    /// T3-14: block until a scrollback line matches the pattern (or timeout).
    WaitFor {
        id: Value,
        surface_id: u64,
        pattern: regex::Regex,
        timeout_secs: u64,
        since_line: u64,
    },
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn param_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

/// 현실적 PTY 치수 상한 — u16 절단 통과(0·65536+)와 vt100 grid 거대 할당(메모리 DoS)을 차단.
const MAX_ROWS: u64 = 1000;
const MAX_COLS: u64 = 4000;

/// health_rules 하드 캡: 룰 전부가 run_health_rules의 `for line × for rule` 핫패스에서
/// 매 완성 라인마다 정규식 평가되므로(O(rules×lines)), 룰 벡터 무한 성장은 메모리 누수일
/// 뿐 아니라 모든 surface 출력 처리의 CPU 비용 증폭이다. caller_cache(4096)·feed_items(5000)
/// 처럼 유한하게 묶는다. 내장 룰 5개 + 운영 룰 여유를 넉넉히 두되 폭주는 차단.
const MAX_HEALTH_RULES: usize = 256;

/// rows/cols 파라미터: 제공되면 범위 검증, 미제공이면 fallback.
fn param_dim(params: &Value, key: &str, fallback: u16, max: u64) -> Result<u16, String> {
    match param_u64(params, key) {
        None => Ok(fallback),
        Some(v) if (1..=max).contains(&v) => Ok(v as u16),
        Some(v) => Err(format!("{key} out of range (1..={max}): {v}")),
    }
}

/// feed.push 자동 request_id의 프로세스 내 유일성 보장 카운터
static FEED_REQ_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// PTY 쓰기 채널 send 결과 → RPC 응답 (성공 시 None)
fn try_write(
    surface: &crate::state::Surface,
    req: crate::state::WriteReq,
    id: &Value,
) -> Option<Value> {
    match surface.write_tx.try_send(req) {
        Ok(()) => None,
        Err(std::sync::mpsc::TrySendError::Full(_)) => Some(err_response(
            id,
            "write_stalled",
            "surface input channel full (pane not consuming input)",
        )),
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
            Some(err_response(id, "write_failed", "surface writer closed"))
        }
    }
}

/// Resolve target surface from params: "surface_id" accepts 31, "31", "surface:31".
fn resolve_surface_id(params: &Value) -> Option<u64> {
    match params.get("surface_id") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => parse_surface_ref(s),
        _ => None,
    }
}

/// 단순 글롭 매칭: '*'만 와일드카드, 나머지는 리터럴 (역할 패턴용 — reviewer-*)
pub fn glob_match(pattern: &str, value: &str) -> bool {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        if ch == '*' {
            re.push_str(".*");
        } else {
            re.push_str(&regex::escape(&ch.to_string()));
        }
    }
    re.push('$');
    regex::Regex::new(&re)
        .map(|r| r.is_match(value))
        .unwrap_or(false)
}

/// T1-3 발신자 소속 surface 해석: peer pid의 조상 체인에서 surface 루트 pid를 찾는다.
/// (cys CLI 프로세스는 pane 셸의 자손이므로 조상 추적으로 소속 pane이 확정된다)
fn resolve_caller_surface(daemon: &Daemon, caller_pid: u32) -> Option<u64> {
    {
        let cache = daemon.caller_cache.lock().unwrap();
        if let Some((sid, ts, cached_start)) = cache.get(&caller_pid) {
            if crate::state::now_epoch() - ts < 60.0 {
                // pid 재사용 차단: 캐시된 start_time이 있으면 현재 peer pid의 start_time과
                // 대조한다. 단명 CLI가 죽고 OS가 같은 pid를 다른 pane 프로세스에 재할당하면
                // incarnation(start_time)이 달라지므로 캐시를 무효화하고 조상 추적을 재실행한다.
                // start_time이 None(합성 주입)이거나 대상 프로세스를 못 찾으면 캐시를 신뢰한다.
                match cached_start {
                    Some(cs) => {
                        if peer_start_time(caller_pid).is_none_or(|now| now == *cs) {
                            return *sid;
                        }
                        // start_time 불일치 → pid 재사용 → 아래로 떨어져 재해석
                    }
                    None => return *sid,
                }
            }
        }
    }
    let pid_to_sid: std::collections::HashMap<u32, u64> = daemon
        .surfaces
        .lock()
        .unwrap()
        .values()
        .map(|s| (s.pid, s.id))
        .collect();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let caller_start = sys
        .process(sysinfo::Pid::from_u32(caller_pid))
        .map(|p| p.start_time());
    let mut cur = caller_pid;
    let mut found = None;
    for _ in 0..32 {
        if let Some(sid) = pid_to_sid.get(&cur) {
            found = Some(*sid);
            break;
        }
        match sys
            .process(sysinfo::Pid::from_u32(cur))
            .and_then(|p| p.parent())
        {
            Some(parent) if parent.as_u32() != cur && parent.as_u32() > 1 => {
                cur = parent.as_u32();
            }
            _ => break,
        }
    }
    // 무한 성장 차단: cys CLI는 매 호출이 단명 프로세스라 동일 pid가 사실상 재등장하지
    // 않는다 → 캐시 히트 경로의 60초 TTL 검사가 영영 발동하지 않아 stale 항목이 데몬 수명
    // 동안 단조 누적된다(노드 간 push가 빈번한 멀티에이전트 운영에서 가속). 매 캐시-미스
    // 삽입 때(이미 락을 쥔 임계영역) 만료(now-ts≥60s) 항목을 일괄 회수하고, 60초 창 내
    // 폭주 대비 하드 캡까지 적용해 캐시를 유한하게 유지한다.
    const CALLER_CACHE_CAP: usize = 4096;
    let now = crate::state::now_epoch();
    let mut cache = daemon.caller_cache.lock().unwrap();
    cache.retain(|_, (_, ts, _)| now - *ts < 60.0);
    cache.insert(caller_pid, (found, now, caller_start));
    if cache.len() > CALLER_CACHE_CAP {
        // 만료 회수 후에도 캡 초과(60초 내 대량 유입) — 가장 오래된 항목부터 캡까지 솎아낸다.
        let mut by_age: Vec<(u32, f64)> = cache.iter().map(|(p, (_, ts, _))| (*p, *ts)).collect();
        by_age.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (pid, _) in by_age.into_iter().take(cache.len() - CALLER_CACHE_CAP) {
            cache.remove(&pid);
        }
    }
    found
}

/// 단일 pid의 현재 start_time(초)만 조회 — 캐시 히트 시 pid 재사용 식별용 경량 lookup.
fn peer_start_time(pid: u32) -> Option<u64> {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p).map(|proc| proc.start_time())
}

/// T1-3 송신 ACL: ~/.cys/pack/acl.json 의 role→role 정책 평가 + from 신원 검증.
/// 파일 부재 = 전부 허용 (하위 호환). 반환: 검증된 발신 surface id (해석 불가 시 None).
fn check_send_acl(
    daemon: &Daemon,
    caller_pid: Option<u32>,
    target: &crate::state::Surface,
) -> Result<Option<u64>, String> {
    let from_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
    let acl_path = cys::pack::pack_dir().join("acl.json");
    let Ok(content) = std::fs::read_to_string(&acl_path) else {
        return Ok(from_sid); // 정책 파일 없음 — 허용 (from 검증만 수행)
    };
    let Ok(acl) = serde_json::from_str::<Value>(&content) else {
        return Ok(from_sid); // 파손된 정책으로 전 노드 통신이 죽지 않게 — 허용 + 무시
    };
    let from_role = from_sid
        .and_then(|sid| daemon.get_surface(sid))
        .and_then(|s| s.role.lock().unwrap().clone())
        .unwrap_or_else(|| {
            if from_sid.is_some() {
                "(pane)".into()
            } else {
                "external".into()
            }
        });
    let to_role = target
        .role
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| "(pane)".into());
    let mut decision: Option<bool> = None;
    if let Some(rules) = acl["rules"].as_array() {
        for rule in rules {
            let f = rule["from"].as_str().unwrap_or("*");
            let t = rule["to"].as_str().unwrap_or("*");
            if glob_match(f, &from_role) && glob_match(t, &to_role) {
                decision = rule["allow"].as_bool();
                break; // 첫 매칭 승리
            }
        }
    }
    let allowed = decision.unwrap_or_else(|| acl["default"].as_str() != Some("deny"));
    if !allowed {
        daemon.bus.publish(
            "acl.denied",
            "system",
            Some(target.id),
            json!({"from_role": from_role, "to_role": to_role,
                   "from_surface": from_sid, "caller_pid": caller_pid}),
        );
        return Err(format!(
            "acl denied: {from_role} → {to_role} (pack/acl.json)"
        ));
    }
    Ok(from_sid)
}

/// T3-13 타이핑 가드 창 (초). 0 = 비활성.
fn typing_guard_secs() -> u64 {
    std::env::var("CYS_TYPING_GUARD_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// 컨텍스트 임계(%) — 절대지침의 60% 사이클을 결정론으로 발화하는 기준.
/// CYS_CONTEXT_THRESHOLD_PCT로 조정 가능. 1~100 범위 밖·파싱 불가는 기본 60으로 폴백.
/// (usage.rs 관측 수집기도 같은 임계로 발화 — 자기보고/관측이 다른 임계를 쓰면 안 된다.)
pub(crate) fn context_threshold_pct() -> u8 {
    threshold_from(std::env::var("CYS_CONTEXT_THRESHOLD_PCT").ok())
}

/// 순수 함수 — env 파싱 규칙의 회귀 핀 (테스트에서 env 전역 오염 없이 검증).
fn threshold_from(raw: Option<String>) -> u8 {
    raw.and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|v| (1..=100).contains(v))
        .unwrap_or(60)
}

pub fn dispatch(daemon: &Arc<Daemon>, req: Request, caller_pid: Option<u32>) -> Reply {
    let id = req.id.clone();
    let params = req.params;
    match req.method.as_str() {
        "system.ping" => Reply::Single(ok_response(&id, json!("pong"))),

        "system.identify" => {
            let caller = params.get("caller").cloned().unwrap_or(Value::Null);
            Reply::Single(ok_response(
                &id,
                json!({
                    "socket_path": daemon.socket_path.to_string_lossy(),
                    "daemon_pid": std::process::id(),
                    "version": env!("CARGO_PKG_VERSION"),
                    "started_at": daemon.started_at,
                    "latest_seq": daemon.bus.latest_seq(),
                    "surface_count": daemon.surfaces.lock().unwrap().len(),
                    "caller": caller,
                }),
            ))
        }

        "surface.create" => {
            let rows = match param_dim(&params, "rows", DEFAULT_ROWS, MAX_ROWS) {
                Ok(v) => v,
                Err(e) => return Reply::Single(err_response(&id, "invalid_params", &e)),
            };
            let cols = match param_dim(&params, "cols", DEFAULT_COLS, MAX_COLS) {
                Ok(v) => v,
                Err(e) => return Reply::Single(err_response(&id, "invalid_params", &e)),
            };
            // 특권 역할 탈취 차단(claim_role과 대칭): create_surface(state.rs)는 요청 role을
            // roles에 무조건 insert("최신 surface 승리")하므로, RPC로 role="master"|"cso"를
            // 지정하면 살아있는 보유자가 있어도 roles 매핑·deadman 감시·--to <role> 라우팅을
            // 통째로 하이재킹할 수 있다. claim_role(handlers.rs)이 막는 바로 그 공격이 create
            // 경로로 우회되므로 동일 게이트를 RPC 입구에 둔다 — 살아있는 보유자가 있으면 거부.
            // PTY를 띄우기 전(create_surface 호출 전)에 차단해 좀비 셸도 남기지 않는다.
            if let Some(role) = param_str(&params, "role") {
                if matches!(role.as_str(), "master" | "cso") {
                    let held_by_live = {
                        // 락 순서 규약: surfaces → roles (close_surface·claim_role과 동일).
                        // 두 락을 동시 보유하므로 순서가 어긋나면 close/claim과 AB-BA 데드락이 난다.
                        let surfaces = daemon.surfaces.lock().unwrap();
                        let roles = daemon.roles.lock().unwrap();
                        roles.get(&role).is_some_and(|&holder| {
                            surfaces
                                .get(&holder)
                                .map(|h| !h.exited.load(Ordering::Relaxed))
                                .unwrap_or(false)
                        })
                    };
                    if held_by_live {
                        daemon.bus.publish(
                            "role.claim_denied",
                            "system",
                            None,
                            json!({"role": role, "reason": "privileged role held by live surface",
                                   "path": "surface.create", "caller_pid": caller_pid}),
                        );
                        return Reply::Single(err_response(
                            &id,
                            "claim_denied",
                            &format!(
                                "surface.create denied: privileged role '{role}' is held by a live surface"
                            ),
                        ));
                    }
                }
            }
            match daemon.create_surface(
                param_str(&params, "cwd"),
                param_str(&params, "cmd"),
                param_str(&params, "title"),
                param_str(&params, "role"),
                rows,
                cols,
            ) {
                Ok(s) => Reply::Single(ok_response(
                    &id,
                    json!({"surface_id": s.id, "surface_ref": surface_ref(s.id), "pid": s.pid}),
                )),
                Err(e) => Reply::Single(err_response(&id, "spawn_failed", &e)),
            }
        }

        "surface.list" => {
            // 살아있는 셸 pid의 현재 작업 디렉토리 — UI pane 제목용 (cd 따라 변함)
            // sysinfo 블로킹 syscall 동안 surfaces 락을 쥐지 않는다 (전 연산 일시정지 방지)
            let pids: Vec<sysinfo::Pid> = daemon
                .surfaces
                .lock()
                .unwrap()
                .values()
                .filter(|s| !s.exited.load(Ordering::Relaxed))
                .map(|s| sysinfo::Pid::from_u32(s.pid))
                .collect();
            let mut sys = sysinfo::System::new();
            // 기본 refresh_processes는 cwd를 갱신하지 않는다 — cwd만 명시 조회 (cd 추적 = Always)
            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&pids),
                false,
                sysinfo::ProcessRefreshKind::nothing().with_cwd(sysinfo::UpdateKind::Always),
            );
            let surfaces = daemon.surfaces.lock().unwrap();
            let mut list: Vec<Value> = surfaces
                .values()
                .map(|s| {
                    let live_cwd = sys
                        .process(sysinfo::Pid::from_u32(s.pid))
                        .and_then(|p| p.cwd())
                        .map(|p| p.display().to_string());
                    // agent 이름과 agent_alive(presence)를 단일 락 1회로 함께 읽어 torn read 제거.
                    let (agent, agent_alive) = {
                        let meta = s.agent_meta.lock().unwrap();
                        (
                            meta.as_ref().map(|(name, _)| name.clone()),
                            meta.as_ref().map(|_| {
                                s.agent_seen.load(Ordering::Relaxed)
                                    && !s.agent_exit_notified.load(Ordering::Relaxed)
                            }),
                        )
                    };
                    json!({
                        "surface_id": s.id,
                        "surface_ref": surface_ref(s.id),
                        "title": s.title.lock().unwrap().clone(),
                        "role": s.role.lock().unwrap().clone(),
                        "cmd": s.cmd,
                        "cwd": s.cwd,
                        "live_cwd": live_cwd,
                        "pid": s.pid,
                        "exited": s.exited.load(Ordering::Relaxed),
                        "created_at": s.created_at,
                        "agent": agent,
                        "agent_alive": agent_alive,
                        "usage": s.observed_usage.lock().unwrap().clone()
                            .and_then(|u| serde_json::to_value(u).ok()),
                    })
                })
                .collect();
            list.sort_by_key(|v| v["surface_id"].as_u64().unwrap_or(0));
            Reply::Single(ok_response(&id, json!({"surfaces": list})))
        }

        // ★양방향 소켓의 핵심: 다른 pane의 PTY stdin에 텍스트를 직접 주입한다.
        "surface.send_text" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(text) = param_str(&params, "text") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing text"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            if surface.exited.load(Ordering::Relaxed) {
                return Reply::Single(err_response(
                    &id,
                    "process_exited",
                    "surface process has exited",
                ));
            }
            // T3-13: 사람(UI) 키 입력 신호 — 타이핑 가드 시각만 기록하고 즉시 통과
            let human = params
                .get("human")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // T1-3 송신 ACL + from 신원 검증 — 항상 커널 peer pid로 평가한다.
            // `human`은 클라이언트 자기신고라 신뢰할 수 없으므로(어떤 pane이든 위조
            // 가능) ACL 우회 신호로 쓰지 않는다. 타이핑 가드 시각 기록은 ACL 통과 후로
            // 미룬다 — 거부된 발신자가 임의 surface의 last_human_input을 갱신해 타이핑
            // 가드를 오염·교착시키지 못하게 한다.
            let verified_from = match check_send_acl(daemon, caller_pid, &surface) {
                Ok(v) => v,
                Err(e) => return Reply::Single(err_response(&id, "acl_denied", &e)),
            };
            if human {
                *surface.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
            }
            // followup 모드: 대상이 조용해질 때 배달자(watchdog 틱)가 순서대로 주입
            if params
                .get("queued")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let depth = {
                    let mut q = surface.pending_queue.lock().unwrap();
                    if q.len() >= 100 {
                        return Reply::Single(err_response(
                            &id,
                            "queue_full",
                            "pending queue cap (100) reached",
                        ));
                    }
                    q.push_back(text.clone());
                    q.len()
                };
                daemon.bus.publish(
                    "queue.enqueued",
                    "queue",
                    Some(sid),
                    json!({"bytes": text.len(), "depth": depth,
                           "from": params.get("from").cloned().unwrap_or(Value::Null)}),
                );
                return Reply::Single(ok_response(
                    &id,
                    json!({"surface_id": sid, "queued": true, "depth": depth}),
                ));
            }
            // T3-13 타이핑 가드: 사람이 방금(기본 3초) 입력 중인 pane에 원격 직접 주입 금지.
            // 무음 큐잉 대신 명시 에러 — 후속 send-key Return이 사람의 미완성 입력을
            // 실행해버리는 최악 경로를 차단한다 (--queued는 quiet 대기 배달이라 허용).
            if !human {
                let guard = typing_guard_secs();
                if guard > 0 {
                    let typing = surface
                        .last_human_input
                        .lock()
                        .unwrap()
                        .map(|t| t.elapsed().as_secs() < guard)
                        .unwrap_or(false);
                    if typing {
                        return Reply::Single(err_response(
                            &id,
                            "typing_guard",
                            "human is typing in this pane; retry later or use --queued",
                        ));
                    }
                }
            }
            let bytes = text.as_bytes().to_vec();
            if let Some(err) = try_write(&surface, crate::state::WriteReq::Data(bytes), &id) {
                return Reply::Single(err);
            }
            if !human {
                // T4-17 에코 제외 창 갱신 — 주입 직후 에코 라인이 헬스룰을 오발시키지 않게
                *surface.last_injected.lock().unwrap() = Some(std::time::Instant::now());
            }
            // quiet=true: interactive keystrokes (UI) — skip event publish to avoid spam.
            let quiet = params
                .get("quiet")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !quiet {
                // T1-3: 발신자 신원이 해석되면 클라이언트 자기신고(from)를 덮어쓴다
                let (from, from_verified) = match verified_from {
                    Some(v) => (json!(v), true),
                    None => (params.get("from").cloned().unwrap_or(Value::Null), false),
                };
                daemon.bus.publish(
                    "surface.input_injected",
                    "surface",
                    Some(sid),
                    json!({"bytes": text.len(), "from": from, "from_verified": from_verified}),
                );
            }
            Reply::Single(ok_response(&id, json!({"surface_id": sid, "sent": true})))
        }

        "surface.send_key" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(key) = param_str(&params, "key") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing key"));
            };
            let Some(bytes) = cys::key_to_bytes(&key) else {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    &format!("unknown key: {key}"),
                ));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            if surface.exited.load(Ordering::Relaxed) {
                return Reply::Single(err_response(
                    &id,
                    "process_exited",
                    "surface process has exited",
                ));
            }
            // T1-3 ACL + T3-13 타이핑 가드 — send_key는 전부 프로그램 경로 (UI는 send_text human)
            if let Err(e) = check_send_acl(daemon, caller_pid, &surface) {
                return Reply::Single(err_response(&id, "acl_denied", &e));
            }
            // queued Return: 대상이 조용해질 때 배달자가 CR을 주입한다(빈 텍스트 Inject =
            // bracketed-paste 빈 본문 + CR). 타이핑 가드 에러가 "use --queued"를 안내하는데
            // send-key만 그 경로가 없던 CLI 비대칭이 노드 보고 채널을 막았다(2026-06-12 실측
            // — codex가 "unexpected argument '--queued'"에 부딪혀 Return 배달 불가).
            // Return/Enter 한정: 다른 키는 텍스트 큐(String)에 실을 수 없다.
            if params
                .get("queued")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if !matches!(key.as_str(), "Return" | "Enter") {
                    return Reply::Single(err_response(
                        &id,
                        "invalid_params",
                        "--queued supports only Return/Enter (other keys cannot ride the text queue)",
                    ));
                }
                let depth = {
                    let mut q = surface.pending_queue.lock().unwrap();
                    if q.len() >= 100 {
                        return Reply::Single(err_response(
                            &id,
                            "queue_full",
                            "pending queue cap (100) reached",
                        ));
                    }
                    q.push_back(String::new());
                    q.len()
                };
                daemon.bus.publish(
                    "queue.enqueued",
                    "queue",
                    Some(sid),
                    json!({"bytes": 0, "depth": depth, "key": "Return",
                           "from": params.get("from").cloned().unwrap_or(Value::Null)}),
                );
                return Reply::Single(ok_response(
                    &id,
                    json!({"surface_id": sid, "key": key, "queued": true, "depth": depth}),
                ));
            }
            {
                let guard = typing_guard_secs();
                if guard > 0 {
                    let typing = surface
                        .last_human_input
                        .lock()
                        .unwrap()
                        .map(|t| t.elapsed().as_secs() < guard)
                        .unwrap_or(false);
                    if typing {
                        return Reply::Single(err_response(
                            &id,
                            "typing_guard",
                            "human is typing in this pane; retry later or use --queued",
                        ));
                    }
                }
            }
            if let Some(err) = try_write(&surface, crate::state::WriteReq::Data(bytes), &id) {
                return Reply::Single(err);
            }
            Reply::Single(ok_response(
                &id,
                json!({"surface_id": sid, "key": key, "sent": true}),
            ))
        }

        "surface.read_text" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            // T3-14 델타 읽기: 단조 라인 커서 이후의 새 라인만 반환 (토큰 절약 모니터링)
            if let Some(since) = param_u64(&params, "since_line") {
                let max_lines = param_u64(&params, "max_lines").unwrap_or(2000).min(10_000) as usize;
                // ★레이스 차단: scrollback 락을 먼저 잡고 그 안에서 line_count를 읽는다.
                // writer(state.rs)가 push(N)과 fetch_add(N)을 같은 락 아래에서 수행하므로,
                // 락 보유 중 읽으면 (sb.len, total)이 항상 일관 — oldest/skip 오프셋 어긋남 차단.
                let sb = surface.scrollback.lock().unwrap();
                let total = surface.line_count.load(Ordering::Relaxed);
                let oldest = total.saturating_sub(sb.len() as u64); // sb[0]의 라인 번호
                let truncated = since < oldest; // 요청 구간 일부가 FIFO에서 퇴출됨
                let start = since.max(oldest);
                let skip = (start - oldest) as usize;
                let lines: Vec<String> = sb.iter().skip(skip).take(max_lines).cloned().collect();
                let next_cursor = start + lines.len() as u64;
                return Reply::Single(ok_response(
                    &id,
                    json!({"surface_id": sid, "surface_ref": surface_ref(sid),
                           "text": lines.join("\n"), "line_count": lines.len(),
                           "since": start, "next_cursor": next_cursor,
                           "latest_cursor": total, "truncated": truncated}),
                ));
            }
            let text = if let Some(lines) = param_u64(&params, "lines") {
                // Tail of the stripped scrollback line buffer.
                let sb = surface.scrollback.lock().unwrap();
                let n = sb.len();
                let start = n.saturating_sub(lines as usize);
                sb.iter()
                    .skip(start)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                // Accurate visible screen, reconstructed by the vt100 grid.
                surface.parser.lock().unwrap().screen().contents()
            };
            Reply::Single(ok_response(
                &id,
                json!({"surface_id": sid, "surface_ref": surface_ref(sid), "text": text,
                       "latest_cursor": surface.line_count.load(Ordering::Relaxed)}),
            ))
        }

        "surface.resize" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            // 미제공 시 현재 크기 유지 (surface 조회 후 fallback 계산)
            let (cur_rows, cur_cols) = {
                let parser = surface.parser.lock().unwrap();
                parser.screen().size()
            };
            let rows = match param_dim(&params, "rows", cur_rows, MAX_ROWS) {
                Ok(v) => v,
                Err(e) => return Reply::Single(err_response(&id, "invalid_params", &e)),
            };
            let cols = match param_dim(&params, "cols", cur_cols, MAX_COLS) {
                Ok(v) => v,
                Err(e) => return Reply::Single(err_response(&id, "invalid_params", &e)),
            };
            let res = surface
                .master
                .lock()
                .unwrap()
                .resize(portable_pty::PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            match res {
                Ok(()) => {
                    surface.parser.lock().unwrap().set_size(rows, cols);
                    Reply::Single(ok_response(
                        &id,
                        json!({"surface_id": sid, "rows": rows, "cols": cols}),
                    ))
                }
                Err(e) => Reply::Single(err_response(&id, "resize_failed", &e.to_string())),
            }
        }

        "surface.rename" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(title) = param_str(&params, "title") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing title"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            *surface.title.lock().unwrap() = title.clone();
            Reply::Single(ok_response(&id, json!({"surface_id": sid, "title": title})))
        }

        "surface.close" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            // 신원·소유 게이트: close_surface는 대상 surface의 자식 프로세스 트리 전체를 kill하고
            // 셸을 죽이며 roles 매핑·인플라이트 큐까지 정리하는 변경계 RPC 중 파괴력이 가장 크다.
            // 가드가 없으면 워커 pane이 임의 surface_id로 master/타 노드 pane을 강제 종료해 send
            // 경로의 ACL 거버넌스(reviewer-*→worker* deny 등)를 우회할 수 있다(claim_role·set_meta·
            // status.set과 동일한 '임의 surface 무인증 쓰기/파괴' 부류). 발신 pane은 커널 peer pid로만
            // 확정한다(client 자기신고 surface_id 불신). 발신이 surface로 해석되면 자기 surface
            // (cs == sid)만 닫을 수 있다. 익명 발신(caller_pid None = 데몬 내부 node-recover·오케스트
            // 레이터 경로)은 통과 — pane은 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다.
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            if let Some(cs) = caller_sid {
                if cs != sid {
                    daemon.bus.publish(
                        "surface.close_denied",
                        "surface",
                        Some(sid),
                        json!({"requested_surface": sid,
                               "caller_surface": cs, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "close_denied",
                        &format!(
                            "surface.close denied: caller (surface {cs}) may only close its own surface, not surface {sid}"
                        ),
                    ));
                }
            }
            match governance::close_surface(daemon, sid) {
                Ok(()) => {
                    Reply::Single(ok_response(&id, json!({"surface_id": sid, "closed": true})))
                }
                Err(e) => Reply::Single(err_response(&id, "not_found", &e)),
            }
        }

        // 사후 역할 등록: 이미 떠 있는 세션이 자기 surface를 역할 주소로 등록 ("너는 마스터이다" 경로)
        "system.claim_role" => {
            let Some(role) = param_str(&params, "role") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing role"));
            };
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            // 신원·소유 검증: 역할 등록은 자기 surface에 대해서만 허용한다. 대상 surface_id는
            // 클라이언트 자기신고라(어떤 pane이든 위조 가능) 신뢰하지 않고, 항상 커널 peer
            // pid로 발신 pane을 확정해 대조한다 (send ACL과 동일한 신원 모델). 이 게이트가
            // 없으면 워커 pane이 ① 자기 소유가 아닌 임의 surface에 역할을 박거나 ② 'master'
            // 같은 특권 역할을 자기 surface로 재지정해 roles 매핑·거버넌스 감시 대상을 탈취할
            // 수 있다. 발신 신원 해석 실패(외부/추적 불가)도 거부 — 익명 claim 금지.
            // resolve_caller_surface는 내부에서 surfaces 락을 잡으므로 아래 임계영역 진입 전에 호출.
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            match caller_sid {
                Some(cs) if cs == sid => {}
                _ => {
                    daemon.bus.publish(
                        "role.claim_denied",
                        "system",
                        Some(sid),
                        json!({"role": role, "requested_surface": sid,
                               "caller_surface": caller_sid, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "claim_denied",
                        &format!(
                            "claim_role denied: caller (surface {caller_sid:?}) may only claim its own surface, not {sid}"
                        ),
                    ));
                }
            }
            // 멤버십 확인 + 역할 전이를 surfaces 락 아래 한 임계영역에서 수행 —
            // 동시 close/claim과의 경합으로 dangling 역할 주소가 남는 것을 차단.
            // 락 순서 규약: surfaces → roles → surface.role (close_surface와 동일)
            {
                let surfaces = daemon.surfaces.lock().unwrap();
                let Some(surface) = surfaces.get(&sid) else {
                    return Reply::Single(err_response(
                        &id,
                        "not_found",
                        &format!("surface {sid} not found"),
                    ));
                };
                let mut roles = daemon.roles.lock().unwrap();
                // 특권 역할 탈취 차단: master·cso는 조직의 단일 장애점·감시 기준점이라,
                // 이미 살아있는 다른 surface가 점유 중이면 재지정을 거부한다. 자기 surface가
                // 이미 보유 중인 경우(idempotent re-claim)와 직전 보유자가 죽은(없거나 exited)
                // 경우의 정당한 승계는 허용 — governance의 live 판정과 동일 기준.
                if matches!(role.as_str(), "master" | "cso") {
                    if let Some(&holder) = roles.get(&role) {
                        let holder_live = holder != sid
                            && surfaces
                                .get(&holder)
                                .map(|h| !h.exited.load(Ordering::Relaxed))
                                .unwrap_or(false);
                        if holder_live {
                            daemon.bus.publish(
                                "role.claim_denied",
                                "system",
                                Some(sid),
                                json!({"role": role, "requested_surface": sid,
                                       "current_holder": holder, "reason": "privileged role held by live surface"}),
                            );
                            return Reply::Single(err_response(
                                &id,
                                "claim_denied",
                                &format!(
                                    "claim_role denied: privileged role '{role}' is held by live surface {holder}"
                                ),
                            ));
                        }
                    }
                }
                let mut srole = surface.role.lock().unwrap();
                if let Some(old) = srole.clone() {
                    if roles.get(&old) == Some(&sid) {
                        roles.remove(&old);
                    }
                }
                roles.insert(role.clone(), sid);
                *srole = Some(role.clone());
            }
            daemon.bus.publish(
                "role.claimed",
                "system",
                Some(sid),
                json!({"role": role, "surface_ref": surface_ref(sid)}),
            );
            crate::governance::persist_topology(daemon);
            Reply::Single(ok_response(&id, json!({"role": role, "surface_id": sid})))
        }

        // 역할 주소 해석: --to <role> 의 서버측 구현
        "system.resolve_role" => {
            let Some(role) = param_str(&params, "role") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing role"));
            };
            // 생존성 게이트: roles 매핑은 surface가 자력 종료(셸 EOF)하면 close_surface를
            // 거치지 않아 dead_sid가 그대로 잔존한다(state.rs:619는 exited만 세우고 roles를
            // 비우지 않음). 검증 없이 반환하면 --to <role> 주소가 이미 죽은 surface를 정상으로
            // 해석해 발신자가 '역할 생존'으로 오인한다. fire_push(schedule.rs)·check_master_deadman과
            // 동일하게 부재(미존재/exited)면 not_found로 강등 — 비대칭 보정.
            let resolved = {
                let roles = daemon.roles.lock().unwrap();
                roles.get(&role).copied()
            };
            let live = resolved.filter(|&sid| {
                daemon
                    .get_surface(sid)
                    .map(|s| !s.exited.load(Ordering::Relaxed))
                    .unwrap_or(false)
            });
            match live {
                Some(sid) => Reply::Single(ok_response(
                    &id,
                    json!({"role": role, "surface_id": sid, "surface_ref": surface_ref(sid)}),
                )),
                None => Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("no surface registered for role '{role}'"),
                )),
            }
        }

        "surface.attach" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            if daemon.get_surface(sid).is_none() {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            }
            Reply::Attach {
                ack: ok_response(&id, json!({"attached": sid})),
                surface_id: sid,
            }
        }

        "events.stream" => {
            let after_seq = param_u64(&params, "after_seq");
            let names = params
                .get("names")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let categories = params
                .get("categories")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Reply::EventStream {
                ack: json!({
                    "type": "ack", "ok": true,
                    "latest_seq": daemon.bus.latest_seq(),
                }),
                after_seq,
                names,
                categories,
            }
        }

        // 프로세스 원장 (완화책 ③) — scoped 실행 등록/해제/조회/강제 종료
        "ledger.register" => {
            let Some(pid) = param_u64(&params, "pid") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing pid"));
            };
            // pid_t(i32) 유효범위 강제 — 절단된 pid가 원장에 저장돼 kill 경로로 재유입되는 것을 차단
            if pid == 0 || pid > i32::MAX as u64 {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    &format!("pid out of valid range (1..=2147483647): {pid}"),
                ));
            }
            let entry = LedgerEntry {
                pid: pid as u32,
                pgid: param_u64(&params, "pgid")
                    .filter(|p| *p > 0 && *p <= i32::MAX as u64)
                    .map(|p| p as i32)
                    .unwrap_or(0),
                cmd: param_str(&params, "cmd").unwrap_or_default(),
                surface_id: params.get("surface_id").and_then(|v| match v {
                    Value::Number(n) => n.as_u64(),
                    Value::String(s) => parse_surface_ref(s),
                    _ => None,
                }),
                scoped: params
                    .get("scoped")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                registered_at: crate::state::now_epoch(),
            };
            daemon.bus.publish(
                "ledger.registered",
                "ledger",
                entry.surface_id,
                json!({"pid": entry.pid, "cmd": entry.cmd, "scoped": entry.scoped}),
            );
            daemon.ledger.lock().unwrap().insert(pid as u32, entry);
            Reply::Single(ok_response(&id, json!({"registered": pid})))
        }

        "ledger.deregister" => {
            let Some(pid) = param_u64(&params, "pid") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing pid"));
            };
            let removed = daemon
                .ledger
                .lock()
                .unwrap()
                .remove(&(pid as u32))
                .is_some();
            Reply::Single(ok_response(&id, json!({"deregistered": removed})))
        }

        "ledger.list" => {
            let ledger = daemon.ledger.lock().unwrap();
            let entries: Vec<Value> = ledger
                .values()
                .map(|e| {
                    json!({
                        "pid": e.pid, "pgid": e.pgid, "cmd": e.cmd,
                        "surface_id": e.surface_id, "scoped": e.scoped,
                        "registered_at": e.registered_at,
                    })
                })
                .collect();
            Reply::Single(ok_response(&id, json!({"entries": entries})))
        }

        "ledger.kill" => {
            let Some(pid) = param_u64(&params, "pid") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing pid"));
            };
            // pid=0(자기 프로세스 그룹 전체)·u32 래핑값이 SIGKILL 경로에 도달하는 것을 차단
            if pid == 0 || pid > i32::MAX as u64 {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    &format!("pid out of valid range (1..=2147483647): {pid}"),
                ));
            }
            let entry = daemon.ledger.lock().unwrap().remove(&(pid as u32));
            match entry {
                Some(e) => {
                    governance::kill_group_or_pid(e.pid, e.pgid);
                    daemon.bus.publish(
                        "ledger.killed",
                        "ledger",
                        e.surface_id,
                        json!({"pid": e.pid, "reason": "explicit kill"}),
                    );
                    Reply::Single(ok_response(&id, json!({"killed": pid})))
                }
                None => {
                    governance::kill_pid(pid as u32);
                    Reply::Single(ok_response(
                        &id,
                        json!({"killed": pid, "note": "not in ledger; killed pid directly"}),
                    ))
                }
            }
        }

        // 헬스 룰 (완화책 ①) — 런타임 추가/조회
        "health.add_rule" => {
            let (Some(name), Some(pattern)) =
                (param_str(&params, "name"), param_str(&params, "pattern"))
            else {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    "missing name or pattern",
                ));
            };
            // T4-17 조치 바인딩 (opt-in): action="pause-queue"만 허용 — 비파괴 조치 한정
            let action = match param_str(&params, "action") {
                None => None,
                Some(a) if a == "pause-queue" => Some(a),
                Some(a) => {
                    return Reply::Single(err_response(
                        &id,
                        "invalid_params",
                        &format!("unknown action '{a}' (allowed: pause-queue)"),
                    ))
                }
            };
            match regex::Regex::new(&pattern) {
                Ok(regex) => {
                    let new_rule = HealthRule {
                        name: name.clone(),
                        regex,
                        action,
                        threshold: param_u64(&params, "threshold").unwrap_or(3).clamp(1, 100)
                            as u32,
                        pause_secs: param_u64(&params, "pause_secs").unwrap_or(300).min(3600),
                    };
                    let mut rules = daemon.health_rules.lock().unwrap();
                    // upsert: 같은 name이 이미 있으면 갱신(중복 누적 차단 — 재등록 스크립트가
                    // 룰 벡터를 단조 성장시키지 못하게 한다). 없으면 캡 검사 후 추가.
                    if let Some(slot) = rules.iter_mut().find(|r| r.name == name) {
                        *slot = new_rule;
                    } else if rules.len() >= MAX_HEALTH_RULES {
                        return Reply::Single(err_response(
                            &id,
                            "limit_reached",
                            &format!("health rule cap ({MAX_HEALTH_RULES}) reached"),
                        ));
                    } else {
                        rules.push(new_rule);
                    }
                    Reply::Single(ok_response(&id, json!({"added": name})))
                }
                Err(e) => Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    &format!("bad regex: {e}"),
                )),
            }
        }

        "health.list_rules" => {
            let rules = daemon.health_rules.lock().unwrap();
            let list: Vec<Value> = rules
                .iter()
                .map(|r| {
                    json!({"name": r.name, "pattern": r.regex.as_str(),
                           "action": r.action, "threshold": r.threshold,
                           "pause_secs": r.pause_secs})
                })
                .collect();
            Reply::Single(ok_response(&id, json!({"rules": list})))
        }

        // ─── 승인 Feed: 워커 승인 요청 집중 처리 (외부 터미널 체계 Feed 사상 계승) ───
        "feed.push" => {
            let kind = param_str(&params, "kind").unwrap_or_else(|| "notification".into());
            let title = param_str(&params, "title").unwrap_or_else(|| "(untitled)".into());
            let body = param_str(&params, "body").unwrap_or_default();
            let surface_id = params.get("surface_id").and_then(|v| match v {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => parse_surface_ref(s),
                _ => None,
            });
            // pid + epoch초 + 프로세스 내 카운터 — 동일 초 동시 요청 충돌과
            // 재시작·pid 재사용 교차 충돌을 모두 차단
            let request_id = param_str(&params, "request_id").unwrap_or_else(|| {
                format!(
                    "req-{}-{}-{}",
                    std::process::id(),
                    crate::state::now_epoch() as u64,
                    FEED_REQ_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                )
            });
            let wait = params
                .get("wait")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // 클라이언트 임의값으로 waiter·태스크가 장기 상주하지 않게 1시간 상한
            let timeout_secs = param_u64(&params, "timeout_secs").unwrap_or(120).min(3600);

            let item = FeedItem {
                request_id: request_id.clone(),
                kind: kind.clone(),
                title: title.clone(),
                body: body.clone(),
                surface_id,
                status: "pending".into(),
                decision: None,
                created_at: crate::state::now_epoch(),
                resolved_at: None,
            };
            // waiter 등록을 항목 공개와 같은 임계영역에서 수행 — 항목이 다른 커넥션에
            // 보이는 순간 waiter가 이미 존재해, 빠른 feed.reply의 결정이 유실되지 않는다.
            // (락 순서: feed_items → feed_waiters. feed.reply는 한 번에 하나만 잡으므로 안전)
            let rx = {
                let mut items = daemon.feed_items.lock().unwrap();
                if items.iter().any(|i| i.request_id == request_id) {
                    return Reply::Single(err_response(
                        &id,
                        "invalid_params",
                        "duplicate request_id",
                    ));
                }
                items.push(item.clone());
                // 메모리 무한 누적 차단: 한도 초과 시 가장 오래된 종결 항목부터 퇴출
                if items.len() > 5000 {
                    if let Some(pos) = items.iter().position(|i| i.status != "pending") {
                        items.remove(pos);
                    }
                }
                if wait {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    daemon
                        .feed_waiters
                        .lock()
                        .unwrap()
                        .insert(request_id.clone(), tx);
                    Some(rx)
                } else {
                    None
                }
            };
            daemon.persist_feed_item(&item);
            daemon.bus.publish(
                "feed.item.created",
                "feed",
                surface_id,
                json!({"request_id": request_id, "kind": kind, "title": title,
                       "body": body, "wait": wait}),
            );
            match rx {
                None => Reply::Single(ok_response(
                    &id,
                    json!({"request_id": request_id, "status": "pending"}),
                )),
                Some(rx) => Reply::FeedWait {
                    id,
                    request_id,
                    rx,
                    timeout_secs,
                },
            }
        }

        "feed.reply" => {
            let Some(request_id) = param_str(&params, "request_id") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing request_id"));
            };
            let Some(decision) = param_str(&params, "decision") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing decision"));
            };
            let mut resolved_snapshot = None;
            {
                let mut items = daemon.feed_items.lock().unwrap();
                if let Some(item) = items.iter_mut().find(|i| i.request_id == request_id) {
                    if item.status != "pending" {
                        return Reply::Single(err_response(
                            &id,
                            "invalid_params",
                            "item already resolved",
                        ));
                    }
                    item.status = "resolved".into();
                    item.decision = Some(decision.clone());
                    item.resolved_at = Some(crate::state::now_epoch());
                    resolved_snapshot = Some(item.clone());
                }
            }
            let Some(snapshot) = resolved_snapshot else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("no feed item {request_id}"),
                ));
            };
            daemon.persist_feed_item(&snapshot);
            // Unblock the waiting pusher, if any.
            if let Some(tx) = daemon.feed_waiters.lock().unwrap().remove(&request_id) {
                let _ = tx.send(decision.clone());
            }
            daemon.bus.publish(
                "feed.item.resolved",
                "feed",
                None,
                json!({"request_id": request_id, "decision": decision}),
            );
            Reply::Single(ok_response(
                &id,
                json!({"request_id": request_id, "decision": decision}),
            ))
        }

        "feed.list" => {
            let status_filter = param_str(&params, "status");
            let items = daemon.feed_items.lock().unwrap();
            let list: Vec<Value> = items
                .iter()
                .filter(|i| {
                    status_filter
                        .as_deref()
                        .map(|s| i.status == s)
                        .unwrap_or(true)
                })
                .map(|i| {
                    json!({
                        "request_id": i.request_id, "kind": i.kind, "title": i.title,
                        "body": i.body, "surface_id": i.surface_id, "status": i.status,
                        "decision": i.decision, "created_at": i.created_at,
                        "resolved_at": i.resolved_at,
                    })
                })
                .collect();
            Reply::Single(ok_response(&id, json!({"items": list})))
        }

        // ─── 세션 기억 검색 (자가개선 루프의 recall) ───
        "recall.search" => {
            let Some(query) = param_str(&params, "query") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing query"));
            };
            let role = param_str(&params, "role");
            let surface_id = params.get("surface_id").and_then(|v| match v {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => parse_surface_ref(s),
                _ => None,
            });
            let days = params.get("days").and_then(|v| v.as_f64());
            let limit = param_u64(&params, "limit").unwrap_or(20);
            match crate::recall::search(daemon, &query, role, surface_id, days, limit) {
                Ok(result) => Reply::Single(ok_response(&id, result)),
                Err(e) => Reply::Single(err_response(&id, "search_failed", &e)),
            }
        }

        // ─── Heartbeat 스케줄 ───
        "schedule.status" => Reply::Single(ok_response(&id, crate::schedule::status(daemon))),

        "schedule.run_now" => {
            let Some(job_id) = param_str(&params, "job_id") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing job_id"));
            };
            match crate::schedule::run_now(daemon, &job_id) {
                Ok(()) => Reply::Single(ok_response(&id, json!({"fired": job_id}))),
                Err(e) => Reply::Single(err_response(&id, "not_found", &e)),
            }
        }

        // ─── T1-1 에이전트 자기보고 ───
        "status.set" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            // 신원·소유 게이트: agent_status는 자기보고(신뢰등급 '참고')지만, org.status 보드를 통해
            // master/CSO의 거버넌스 판단(60% /clear·blocked/done·deadman 보조)에 입력된다. 가드가
            // 없으면 워커 pane이 임의 surface_id로 타 노드의 'done'·낮은 context_pct를 위조해 자율주행
            // 의사결정을 오도할 수 있다(claim_role·set_meta·send ACL과 동일한 '임의 surface 무인증
            // 쓰기' 부류). 발신 pane은 커널 peer pid로만 확정한다(client 자기신고 surface_id 불신).
            // 발신이 surface로 해석되면 자기 surface(cs == sid)에만 자기 상태를 쓸 수 있다 — 상태는
            // 순수 자기보고라 타인 대리 보고 정당 경로가 없다. 익명 발신(caller_pid None = 데몬 내부)은
            // 통과(pane은 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다).
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            if let Some(cs) = caller_sid {
                if cs != sid {
                    daemon.bus.publish(
                        "status.set_denied",
                        "system",
                        Some(sid),
                        json!({"requested_surface": sid,
                               "caller_surface": cs, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "status_denied",
                        &format!(
                            "status.set denied: caller (surface {cs}) may only report its own status, not surface {sid}"
                        ),
                    ));
                }
            }
            let state = param_str(&params, "state").unwrap_or_else(|| "working".into());
            const STATES: [&str; 4] = ["working", "waiting", "blocked", "done"];
            if !STATES.contains(&state.as_str()) {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    &format!("state must be one of {STATES:?}"),
                ));
            }
            let context_pct = param_u64(&params, "context").map(|v| v.min(100) as u8);
            let task = param_str(&params, "task").map(|t| t.chars().take(500).collect::<String>());
            let role = surface.role.lock().unwrap().clone();
            let status = crate::state::AgentStatus {
                state: state.clone(),
                context_pct,
                task: task.clone(),
                updated_at: crate::state::now_epoch(),
            };
            let changed = {
                let mut cur = surface.agent_status.lock().unwrap();
                let changed = cur
                    .as_ref()
                    .map(|c| c.state != state || c.context_pct != context_pct)
                    .unwrap_or(true);
                *cur = Some(status);
                changed
            };
            if changed {
                daemon.bus.publish(
                    "status.changed",
                    "status",
                    Some(sid),
                    json!({"role": role, "state": state, "context_pct": context_pct, "task": task}),
                );
            }
            // ─── 결정론 컨텍스트 임계 (절대지침: 60% 도달 시 저장→clear→복원 사이클) ───
            // "무거워진 것 같다"는 LLM 재량 판단을 트리거에서 배제한다 — 자기보고 pct와 임계의
            // 수치 비교만이 발화 조건이다. 에지 트리거: 미만→이상 교차 시 1회 발행, 임계 위
            // 체류 중 재발행 없음, 내려갔다 다시 넘으면 재발행. 에지 상태는 Surface의
            // ctx_threshold_armed — 관측 경로(usage.rs)와 **공유**해 같은 교차의 이중 발화
            // (cycle-agent 이중 집행)를 차단한다. master/CSO는 이 이벤트(watchdog)를 받아
            // cycle-agent를 집행한다.
            if let Some(pct) = context_pct {
                let threshold = context_threshold_pct();
                if pct < threshold {
                    surface.ctx_threshold_armed.store(true, Ordering::Relaxed);
                } else if surface.ctx_threshold_armed.swap(false, Ordering::Relaxed) {
                    daemon.bus.publish(
                        "context.threshold",
                        "watchdog",
                        Some(sid),
                        json!({"role": role, "context_pct": pct, "threshold": threshold,
                               "surface_ref": cys::surface_ref(sid), "source": "self-report",
                               "action": "cycle-agent(저장→검증→clear→복원) 집행 대상 — MASTER_DIRECTIVE §컨텍스트 사이클"}),
                    );
                }
            }
            Reply::Single(ok_response(&id, json!({"surface_id": sid, "state": state})))
        }

        // ─── T5 사용량 관측: 세션 트랜스크립트 경로 등록 (SessionStart hook의 결정론 매핑) ───
        "usage.register" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            // 소유 게이트 — status.set과 동형: 발신 pane은 자기 surface에만 등록할 수 있다.
            // 없으면 워커가 타 pane에 가짜 트랜스크립트를 등록해 master/CSO가 보는 컨텍스트
            // 수치를 위조(60% 사이클 오발·억제)할 수 있다.
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            if let Some(cs) = caller_sid {
                if cs != sid {
                    daemon.bus.publish(
                        "usage.register_denied",
                        "usage",
                        Some(sid),
                        json!({"requested_surface": sid,
                               "caller_surface": cs, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "usage_denied",
                        &format!(
                            "usage.register denied: caller (surface {cs}) may only register its own transcript, not surface {sid}"
                        ),
                    ));
                }
            }
            let Some(path) = param_str(&params, "transcript") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing transcript"));
            };
            let pb = std::path::PathBuf::from(&path);
            // 존재는 요구하지 않는다 — SessionStart 시점엔 트랜스크립트 파일이 아직 없을 수
            // 있다(첫 메시지에서 생성). 수집기는 파일이 생길 때까지 무해하게 대기한다.
            // `..` 컴포넌트는 거부 — 확장자 검사를 끝 컴포넌트만 보고 통과시키는 트래버설
            // 변형을 차단한다 (수집기는 숫자만 추출하지만 경계 기만 자체를 막는다).
            if !pb.is_absolute()
                || pb
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || pb.extension().and_then(|e| e.to_str()) != Some("jsonl")
            {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    "transcript must be an absolute .jsonl path (no '..')",
                ));
            }
            *surface.registered_transcript.lock().unwrap() = Some(path.clone());
            daemon.bus.publish(
                "usage.session_registered",
                "usage",
                Some(sid),
                json!({"transcript": path, "surface_ref": cys::surface_ref(sid)}),
            );
            Reply::Single(ok_response(&id, json!({"surface_id": sid})))
        }

        // ─── T1-2 통합 관제 보드: read-screen 폴링 없이 1콜로 전 노드 상황 파악 ───
        "org.status" => {
            let now = crate::state::now_epoch();
            // live_cwd(cd 추적): surfaces 락 밖에서 sysinfo 조회 — surface.list와 동일 패턴.
            // 워커가 워크플로우 폴더 밖으로 cd해도 진행% 산출(javis_report)이 실제 _round를 찾게 한다.
            let pids: Vec<sysinfo::Pid> = daemon
                .surfaces
                .lock()
                .unwrap()
                .values()
                .filter(|s| !s.exited.load(Ordering::Relaxed))
                .map(|s| sysinfo::Pid::from_u32(s.pid))
                .collect();
            let mut sys = sysinfo::System::new();
            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&pids),
                false,
                sysinfo::ProcessRefreshKind::nothing().with_cwd(sysinfo::UpdateKind::Always),
            );
            let surfaces = daemon.surfaces.lock().unwrap();
            let mut list: Vec<Value> = surfaces
                .values()
                .map(|s| {
                    let live_cwd = sys
                        .process(sysinfo::Pid::from_u32(s.pid))
                        .and_then(|p| p.cwd())
                        .map(|p| p.display().to_string());
                    let status = s.agent_status.lock().unwrap().clone().map(|st| {
                        json!({"state": st.state, "context_pct": st.context_pct,
                               "task": st.task, "age_secs": (now - st.updated_at).max(0.0) as u64})
                    });
                    let queue_paused = s
                        .queue_paused_until
                        .lock()
                        .unwrap()
                        .map(|t| t > std::time::Instant::now())
                        .unwrap_or(false);
                    // agent 이름과 agent_alive(presence)를 단일 락 1회로 함께 읽어 torn read 제거.
                    let (agent, agent_alive) = {
                        let meta = s.agent_meta.lock().unwrap();
                        (
                            meta.as_ref().map(|(n, _)| n.clone()),
                            meta.as_ref().map(|_| {
                                s.agent_seen.load(Ordering::Relaxed)
                                    && !s.agent_exit_notified.load(Ordering::Relaxed)
                            }),
                        )
                    };
                    json!({
                        "surface_id": s.id,
                        "surface_ref": surface_ref(s.id),
                        "role": s.role.lock().unwrap().clone(),
                        "title": s.title.lock().unwrap().clone(),
                        "cwd": s.cwd.clone(),
                        "live_cwd": live_cwd,
                        "exited": s.exited.load(Ordering::Relaxed),
                        "idle_secs": s.last_output.lock().unwrap().elapsed().as_secs(),
                        "queue_depth": s.pending_queue.lock().unwrap().len(),
                        "queue_paused": queue_paused,
                        "agent": agent,
                        "agent_alive": agent_alive,
                        "status": status,
                        "usage": s.observed_usage.lock().unwrap().clone()
                            .and_then(|u| serde_json::to_value(u).ok()),
                        "line_count": s.line_count.load(Ordering::Relaxed),
                        "created_at": s.created_at,
                    })
                })
                .collect();
            drop(surfaces);
            list.sort_by_key(|v| v["surface_id"].as_u64().unwrap_or(0));
            let (pending, oldest_age) = {
                let items = daemon.feed_items.lock().unwrap();
                let pending: Vec<&FeedItem> =
                    items.iter().filter(|i| i.status == "pending").collect();
                let oldest = pending
                    .iter()
                    .map(|i| (now - i.created_at).max(0.0) as u64)
                    .max();
                (pending.len(), oldest)
            };
            let health_recent: Vec<Value> = daemon
                .recent_health
                .lock()
                .unwrap()
                .iter()
                .rev()
                .take(10)
                .cloned()
                .collect();
            let todo: Value = {
                let tp = daemon.todo_progress.lock().unwrap();
                tp.iter()
                    .map(|(path, (done, total, mtime))| {
                        (
                            path.clone(),
                            json!({"done": done, "total": total,
                                   "age_secs": (now - mtime).max(0.0) as u64}),
                        )
                    })
                    .collect::<serde_json::Map<String, Value>>()
                    .into()
            };
            let pause_info = daemon.pause_info.lock().unwrap().clone();
            Reply::Single(ok_response(
                &id,
                json!({
                    "paused": daemon.paused.load(Ordering::Relaxed),
                    "pause_info": pause_info.map(|(since, reason)|
                        json!({"since": since, "reason": reason})),
                    "daemon": {"version": env!("CARGO_PKG_VERSION"),
                               "started_at": daemon.started_at,
                               "latest_seq": daemon.bus.latest_seq()},
                    "surfaces": list,
                    "feed": {"pending": pending, "oldest_pending_age_secs": oldest_age},
                    "health_recent": health_recent,
                    "todo": todo,
                }),
            ))
        }

        // ─── T2-5 에이전트 메타 등록 (launch-agent가 호출 — 사망 감지·status 보드의 기반) ───
        "surface.set_meta" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            let agent = param_str(&params, "agent").unwrap_or_default();
            let agent_bin = param_str(&params, "agent_bin").unwrap_or_else(|| agent.clone());
            if agent.is_empty() {
                return Reply::Single(err_response(&id, "invalid_params", "missing agent"));
            }
            // 신원·소유 게이트: agent_meta는 사망 감지(governance.rs agent_seen/exit_notified)와
            // 승인 격상 스캔(check_approvals가 agents.json[agent].approval_patterns로 그 surface
            // 화면을 정규식 매칭)의 기반이라, 다른 pane이 임의 surface의 메타를 덮어쓰면 ① 타 노드의
            // 승인 패턴/feed 알림을 임의로 켜거나 ② agent_seen/exit_notified를 리셋해 사망 감지를
            // 교란할 수 있다 (claim_role과 동일한 '임의 surface 무인증 쓰기' 부류). 발신 pane은
            // 커널 peer pid로만 확정한다(client 자기신고 surface_id 불신). 정당 경로는 그대로 통과:
            // ① 자기 메타 갱신(cs == sid) ② 오케스트레이터가 갓 만든 자식 surface 초기화
            //   (대상 agent_meta == None — 아직 미등록) ③ 데몬이 spawn한 node-recover(발신 pane
            //   없음 = caller_sid None — 이미 메타가 있는 surface에 동일 에이전트 재등록).
            // 차단 대상은 오직 '발신 pane이 자기 소유 아닌, 이미 살아있는 타 노드의 메타를 덮어쓰는'
            // 단일 케이스다.
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            if let Some(cs) = caller_sid {
                if cs != sid && surface.agent_meta.lock().unwrap().is_some() {
                    daemon.bus.publish(
                        "meta.set_denied",
                        "system",
                        Some(sid),
                        json!({"agent": agent, "requested_surface": sid,
                               "caller_surface": cs, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "meta_denied",
                        &format!(
                            "set_meta denied: caller (surface {cs}) may not overwrite the live agent meta of another surface {sid}"
                        ),
                    ));
                }
            }
            *surface.agent_meta.lock().unwrap() = Some((agent.clone(), agent_bin));
            surface.agent_seen.store(false, Ordering::Relaxed);
            surface.agent_exit_notified.store(false, Ordering::Relaxed);
            crate::governance::persist_topology(daemon);
            Reply::Single(ok_response(&id, json!({"surface_id": sid, "agent": agent})))
        }

        // ─── T4-15 kill-switch: 큐 배달·스케줄 발화 동결 (직접 send는 통과 = 신경 차단) ───
        "system.pause" => {
            let reason = param_str(&params, "reason").unwrap_or_default();
            daemon.paused.store(true, Ordering::Relaxed);
            *daemon.pause_info.lock().unwrap() = Some((crate::state::now_epoch(), reason.clone()));
            daemon.persist_pause();
            daemon
                .bus
                .publish("autopilot.paused", "system", None, json!({"reason": reason}));
            Reply::Single(ok_response(&id, json!({"paused": true})))
        }

        "system.resume" => {
            daemon.paused.store(false, Ordering::Relaxed);
            *daemon.pause_info.lock().unwrap() = None;
            daemon.persist_pause();
            daemon
                .bus
                .publish("autopilot.resumed", "system", None, json!({}));
            Reply::Single(ok_response(&id, json!({"paused": false})))
        }

        "system.gate_check" => {
            let info = daemon.pause_info.lock().unwrap().clone();
            Reply::Single(ok_response(
                &id,
                json!({"paused": daemon.paused.load(Ordering::Relaxed),
                       "since": info.as_ref().map(|(s, _)| *s),
                       "reason": info.map(|(_, r)| r)}),
            ))
        }

        // ─── T4-15 짝 기능: 미배달 큐 검사·철회 ───
        "queue.list" => {
            let filter_sid = resolve_surface_id(&params);
            let surfaces = daemon.surfaces.lock().unwrap();
            let mut out: Vec<Value> = Vec::new();
            for s in surfaces.values() {
                if let Some(f) = filter_sid {
                    if s.id != f {
                        continue;
                    }
                }
                let q = s.pending_queue.lock().unwrap();
                for (i, text) in q.iter().enumerate() {
                    out.push(json!({
                        "surface_id": s.id, "surface_ref": surface_ref(s.id),
                        "index": i, "bytes": text.len(),
                        "preview": text.chars().take(80).collect::<String>(),
                    }));
                }
            }
            Reply::Single(ok_response(&id, json!({"entries": out})))
        }

        "queue.clear" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            // 신원·소유 게이트: queue.clear는 대상 surface의 pending_queue를 통째로 drain해, 제3자가
            // --queued로 보낸(queued:true 응답까지 받은) 인플라이트 메시지를 조용히 폐기한다. 가드가
            // 없으면 워커 pane이 임의 surface_id로 타 노드에 향하던 큐를 인멸해 send ACL이 막은 대상을
            // 큐 인멸로 방해할 수 있다(status.set·close와 동일한 '임의 surface 무인증 파괴' 부류). 발신
            // pane은 커널 peer pid로만 확정한다(client 자기신고 surface_id 불신). 자기 surface(cs == sid)
            // 만 비울 수 있다. 익명 발신(caller_pid None = 데몬 내부 경로)은 통과 — pane은 peer pid가
            // 항상 자기 surface로 해석되므로 익명을 위조할 수 없다.
            let caller_sid = caller_pid.and_then(|p| resolve_caller_surface(daemon, p));
            if let Some(cs) = caller_sid {
                if cs != sid {
                    daemon.bus.publish(
                        "queue.clear_denied",
                        "queue",
                        Some(sid),
                        json!({"requested_surface": sid,
                               "caller_surface": cs, "caller_pid": caller_pid}),
                    );
                    return Reply::Single(err_response(
                        &id,
                        "clear_denied",
                        &format!(
                            "queue.clear denied: caller (surface {cs}) may only clear its own surface queue, not surface {sid}"
                        ),
                    ));
                }
            }
            let dropped: Vec<String> = surface.pending_queue.lock().unwrap().drain(..).collect();
            if !dropped.is_empty() {
                daemon.bus.publish(
                    "queue.dropped",
                    "queue",
                    Some(sid),
                    json!({"reason": "cleared", "count": dropped.len(),
                           "bytes": dropped.iter().map(|t| t.len()).sum::<usize>()}),
                );
            }
            Reply::Single(ok_response(
                &id,
                json!({"surface_id": sid, "cleared": dropped.len()}),
            ))
        }

        // ─── T2-6 토폴로지: 영속 스냅샷 + 현재 라이브 역할 (cys restore의 데이터 소스) ───
        "system.topology" => {
            let saved = crate::governance::load_topology(daemon);
            let live: Vec<Value> = daemon
                .surfaces
                .lock()
                .unwrap()
                .values()
                .filter(|s| !s.exited.load(Ordering::Relaxed))
                .filter_map(|s| {
                    s.role.lock().unwrap().clone().map(|role| {
                        json!({"role": role, "surface_ref": surface_ref(s.id),
                               "agent": s.agent_meta.lock().unwrap().as_ref().map(|(n, _)| n.clone())})
                    })
                })
                .collect();
            Reply::Single(ok_response(&id, json!({"saved": saved, "live": live})))
        }

        // ─── T3-14 완료 대기: 데몬측 블로킹 regex 감시 (plain-line 마커 규약 전제) ───
        "surface.wait_for" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let Some(surface) = daemon.get_surface(sid) else {
                return Reply::Single(err_response(
                    &id,
                    "not_found",
                    &format!("surface {sid} not found"),
                ));
            };
            let Some(pattern) = param_str(&params, "pattern") else {
                return Reply::Single(err_response(&id, "invalid_params", "missing pattern"));
            };
            let regex = match regex::Regex::new(&pattern) {
                Ok(r) => r,
                Err(e) => {
                    return Reply::Single(err_response(
                        &id,
                        "invalid_params",
                        &format!("bad regex: {e}"),
                    ))
                }
            };
            let since_line = param_u64(&params, "since_line")
                .unwrap_or_else(|| surface.line_count.load(Ordering::Relaxed));
            let timeout_secs = param_u64(&params, "timeout_secs").unwrap_or(120).min(600);
            Reply::WaitFor {
                id,
                surface_id: sid,
                pattern: regex,
                timeout_secs,
                since_line,
            }
        }

        // ─── T4-18 트랜스크립트 해시체인 attest (producer≠evaluator의 기계적 토대) ───
        "attest.pin" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            match crate::recall::attest_pin(daemon, sid) {
                Ok(v) => Reply::Single(ok_response(&id, v)),
                Err(e) => Reply::Single(err_response(&id, "attest_failed", &e)),
            }
        }

        "attest.verify" => {
            let Some(sid) = resolve_surface_id(&params) else {
                return Reply::Single(err_response(&id, "invalid_params", "missing surface_id"));
            };
            let (Some(hash), Some(count)) = (
                param_str(&params, "hash"),
                param_u64(&params, "count"),
            ) else {
                return Reply::Single(err_response(
                    &id,
                    "invalid_params",
                    "missing hash or count",
                ));
            };
            match crate::recall::attest_verify(daemon, sid, &hash, count) {
                Ok(v) => Reply::Single(ok_response(&id, v)),
                Err(e) => Reply::Single(err_response(&id, "attest_failed", &e)),
            }
        }

        other => Reply::Single(err_response(
            &id,
            "method_not_found",
            &format!("unknown method: {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_literal_and_star() {
        // '*'만 와일드카드, 나머지는 리터럴
        assert!(glob_match("reviewer-*", "reviewer-gemini"));
        assert!(glob_match("reviewer-*", "reviewer-"));
        assert!(!glob_match("reviewer-*", "worker-gemini"));
        // '*' 단독은 전체 매치 (빈 문자열 포함)
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        // 와일드카드 없는 패턴은 정확 일치만
        assert!(glob_match("master", "master"));
        assert!(!glob_match("master", "master2"));
        // 앵커링: 부분 일치는 거부 (^...$)
        assert!(!glob_match("rev", "reviewer"));
    }

    #[test]
    fn glob_regex_special_chars_are_literal() {
        // 정규식 메타문자는 escape되어 리터럴로 매칭돼야 한다
        assert!(glob_match("a.b", "a.b"));
        assert!(!glob_match("a.b", "axb")); // '.'이 임의문자로 새지 않음
        assert!(glob_match("role+1", "role+1"));
        assert!(glob_match("a(b)", "a(b)"));
        assert!(glob_match("x[1]", "x[1]"));
        // '*' 와 리터럴 메타문자 혼합
        assert!(glob_match("a.*-*", "a.b-c"));
        assert!(!glob_match("a.*-*", "axb-c"));
    }

    #[test]
    fn glob_multistar_matches_cli_semantics() {
        // cys.rs의 재귀 cli_glob_match와 동일 의미를 regex판이 보장해야 한다
        // (두 독립 구현이 역할 매칭에서 갈리면 ACL이 비대칭 동작 — 일관성 불변식).
        assert!(glob_match("*-*", "worker-2"));
        assert!(glob_match("w*r*2", "worker-2"));
        assert!(glob_match("**", "abc"));
        assert!(glob_match("a**c", "abbbc"));
        assert!(glob_match("a*z", "az")); // '*' 빈 매치
        assert!(!glob_match("a*c", "abd"));
        assert!(!glob_match("*x", "abc"));
        // value 내부 '*'는 리터럴 (패턴의 '*'만 와일드카드)
        assert!(glob_match("a*", "a*literal"));
        assert!(!glob_match("abc", "a*c"));
    }

    /// cys.rs `cli_glob_match`와 1:1 동일한 재귀 명세 (독립 오라클).
    /// regex 기반 glob_match가 이 명세에서 한 글자라도 갈리면 두 바이너리의 ACL이
    /// 비대칭 동작한다 — 그 분기점을 코퍼스 전수로 잡는다.
    fn glob_oracle(pattern: &str, value: &str) -> bool {
        fn inner(p: &[char], v: &[char]) -> bool {
            match p.first() {
                None => v.is_empty(),
                Some('*') => (0..=v.len()).any(|i| inner(&p[1..], &v[i..])),
                Some(c) => v.first() == Some(c) && inner(&p[1..], &v[1..]),
            }
        }
        inner(
            &pattern.chars().collect::<Vec<_>>(),
            &value.chars().collect::<Vec<_>>(),
        )
    }

    #[test]
    fn glob_match_agrees_with_recursive_oracle_over_corpus() {
        // 패턴·값 전수 곱집합에서 regex판(glob_match)과 재귀 명세(glob_oracle)가
        // 완전히 일치해야 한다. 불일치 1건이라도 = ACL 비대칭의 증거 → 즉시 빨간불.
        // 메타문자(.+?[](){}^$\)를 일부러 섞어 regex escape 누락도 함께 검출한다.
        let patterns = [
            "", "*", "**", "a", "a*", "*a", "*a*", "a*b", "a**b", "a*b*c", "reviewer-*", "*-*",
            "w*r*2", "abc", "a.b", "a+b", "a?b", "[x]", "a*z", "**a**",
        ];
        let values = [
            "", "a", "ab", "abc", "a*literal", "reviewer-gemini", "reviewer-", "reviewer",
            "worker-2", "a.b", "axb", "a+b", "a?b", "[x]", "az", "abz", "abcz", "x", "-", "a-b-c",
        ];
        for p in patterns {
            for v in values {
                assert_eq!(
                    glob_match(p, v),
                    glob_oracle(p, v),
                    "glob 비대칭: pattern={p:?} value={v:?} (regex={} oracle={})",
                    glob_match(p, v),
                    glob_oracle(p, v),
                );
            }
        }
    }

    #[test]
    fn param_dim_range_validation() {
        // 미제공 → fallback
        let p = json!({});
        assert_eq!(param_dim(&p, "rows", 35, MAX_ROWS), Ok(35));
        // 경계 내 정상값
        let p = json!({"rows": 80});
        assert_eq!(param_dim(&p, "rows", 35, MAX_ROWS), Ok(80));
        // 하한 경계 1 허용
        let p = json!({"rows": 1});
        assert_eq!(param_dim(&p, "rows", 35, MAX_ROWS), Ok(1));
        // 0은 범위 밖 → 에러 (u16 절단으로 0 grid 통과 차단)
        let p = json!({"rows": 0});
        assert!(param_dim(&p, "rows", 35, MAX_ROWS).is_err());
        // 상한 경계 정확히 max 허용
        let p = json!({"cols": MAX_COLS});
        assert_eq!(param_dim(&p, "cols", 120, MAX_COLS), Ok(MAX_COLS as u16));
        // max 초과 → 에러 (vt100 거대 할당 DoS 차단)
        let p = json!({"cols": MAX_COLS + 1});
        assert!(param_dim(&p, "cols", 120, MAX_COLS).is_err());
        // u16 초과 거대값 (65536) → 에러 (silent wrap 금지)
        let p = json!({"rows": 65536});
        assert!(param_dim(&p, "rows", 35, MAX_ROWS).is_err());
    }

    #[test]
    fn param_dim_accepts_numeric_string() {
        // param_u64는 숫자 문자열도 수용
        let p = json!({"rows": "80"});
        assert_eq!(param_dim(&p, "rows", 35, MAX_ROWS), Ok(80));
    }

    #[test]
    fn param_dim_unparseable_falls_back_to_default() {
        // 음수·소수·비숫자 문자열은 param_u64가 None → param_dim이 안전한 fallback을 쓴다
        // (에러가 아니라 기본값으로 surface가 생성됨 — 의도된 안전 경로, 회귀 시 빨간불).
        assert_eq!(param_dim(&json!({"rows": -5}), "rows", 35, MAX_ROWS), Ok(35));
        assert_eq!(param_dim(&json!({"rows": "-5"}), "rows", 35, MAX_ROWS), Ok(35));
        assert_eq!(param_dim(&json!({"rows": 3.5}), "rows", 35, MAX_ROWS), Ok(35));
        assert_eq!(param_dim(&json!({"rows": "abc"}), "rows", 35, MAX_ROWS), Ok(35));
        assert_eq!(param_dim(&json!({"rows": null}), "rows", 35, MAX_ROWS), Ok(35));
        // 단, 파싱 가능한 범위 밖 값은 fallback이 아니라 명시적 에러여야 한다 (DoS 게이트)
        assert!(param_dim(&json!({"rows": "0"}), "rows", 35, MAX_ROWS).is_err());
        assert!(param_dim(&json!({"rows": "99999"}), "rows", 35, MAX_ROWS).is_err());
    }

    #[test]
    fn resolve_surface_id_variants() {
        // 숫자
        assert_eq!(resolve_surface_id(&json!({"surface_id": 31})), Some(31));
        // 문자열 숫자
        assert_eq!(resolve_surface_id(&json!({"surface_id": "31"})), Some(31));
        // surface:N 형식 문자열
        assert_eq!(
            resolve_surface_id(&json!({"surface_id": "surface:31"})),
            Some(31)
        );
        // 키 부재
        assert_eq!(resolve_surface_id(&json!({})), None);
        // 잘못된 문자열
        assert_eq!(resolve_surface_id(&json!({"surface_id": "x"})), None);
        // 음수 숫자 (as_u64 None)
        assert_eq!(resolve_surface_id(&json!({"surface_id": -5})), None);
        // 소수 (as_u64 None)
        assert_eq!(resolve_surface_id(&json!({"surface_id": 3.5})), None);
        // null·bool 등 비숫자/비문자 → None
        assert_eq!(resolve_surface_id(&json!({"surface_id": null})), None);
        assert_eq!(resolve_surface_id(&json!({"surface_id": true})), None);
    }

    #[test]
    fn glob_match_dot_does_not_cross_newline() {
        // regex '.'은 기본 \n 미매치 + ^…$는 문자열(라인 아님) 앵커.
        // 역할명에 개행이 없다는 전제를 박제 — value에 \n이 끼면 '*'도 매치 실패.
        assert!(!glob_match("*", "role\nwith-newline"));
        assert!(!glob_match("a*", "a\nb"));
        // 개행 없는 동일 길이 입력은 정상 매치 (대조군)
        assert!(glob_match("*", "role-no-newline"));
        // 빈 패턴은 빈 값만 (^$)
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    #[test]
    fn param_u64_string_edge_parsing() {
        // 공백 포함 문자열은 파싱 실패 → None → fallback
        assert_eq!(param_dim(&json!({"rows": " 80"}), "rows", 35, MAX_ROWS), Ok(35));
        assert_eq!(param_dim(&json!({"rows": "80 "}), "rows", 35, MAX_ROWS), Ok(35));
        // '+80'은 u64 parse가 수용(범위 내) — 의도된 관용 (silent 거부 아님을 박제)
        assert_eq!(param_dim(&json!({"rows": "+80"}), "rows", 35, MAX_ROWS), Ok(80));
        // 16진·접두는 10진 parse 실패 → fallback
        assert_eq!(param_dim(&json!({"rows": "0x50"}), "rows", 35, MAX_ROWS), Ok(35));
        // 숫자형 우선(as_u64) — 문자열 경로와 동일 결과
        assert_eq!(param_dim(&json!({"rows": 80}), "rows", 35, MAX_ROWS), Ok(80));
    }

    /// 회귀(룰 벡터 무한 성장 + 핫패스 O(rules×lines) 증폭):
    /// health.add_rule이 같은 name을 무조건 push만 하면 ── 재시작 후 룰 재등록 같은
    /// 반복 호출에서 health_rules가 단조 성장하고, 그 전부가 run_health_rules의
    /// `for line × for rule`에서 매 라인 정규식 평가된다. caller_cache(4096)·feed_items(5000)·
    /// recent_health(50) 등 다른 상태엔 모두 캡이 있는데 이 벡터만 무제한이었다.
    /// ① 같은 name 반복 등록은 upsert(중복 누적 0) ② 고유 name 폭주도 하드 캡으로 유한.
    #[test]
    fn health_add_rule_upserts_by_name_and_caps_total() {
        let dir = std::env::temp_dir().join(format!(
            "cys-healthrule-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let base = daemon.health_rules.lock().unwrap().len();

        // 같은 name으로 수천 회 재등록 — 벡터가 1개만 늘고(upsert) 단조 성장하지 않아야 한다.
        for i in 0..5000 {
            let req = Request {
                id: json!(i),
                method: "health.add_rule".into(),
                params: json!({ "name": "redeploy_rule", "pattern": format!("p{}", i % 7) }),
            };
            let Reply::Single(resp) = dispatch(&daemon, req, None) else {
                panic!("expected single reply");
            };
            assert_eq!(resp["ok"], json!(true), "add_rule 실패: {resp}");
        }
        assert_eq!(
            daemon.health_rules.lock().unwrap().len(),
            base + 1,
            "같은 name 반복 등록이 upsert가 아니라 누적됐다 (룰 벡터 무한 성장)"
        );
        // 마지막 등록의 패턴이 유효한지(최신값으로 갱신됐는지) 확인
        assert!(
            daemon
                .health_rules
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.name == "redeploy_rule"),
            "upsert 후 룰이 사라졌다"
        );

        // 고유 name 폭주 — 하드 캡을 넘지 않아야 한다 (핫패스 비용 상한).
        for i in 0..5000 {
            let req = Request {
                id: json!(i),
                method: "health.add_rule".into(),
                params: json!({ "name": format!("uniq_{i}"), "pattern": "x" }),
            };
            let _ = dispatch(&daemon, req, None);
        }
        let len = daemon.health_rules.lock().unwrap().len();
        assert!(
            len <= MAX_HEALTH_RULES,
            "고유 name 폭주가 캡({MAX_HEALTH_RULES})을 넘었다: {len}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // CYS_PACK_DIR는 프로세스 전역 env라 set/사용 윈도를 직렬화해야 cargo 병렬 러너에서
    // 다른 ACL 테스트와 충돌하지 않는다 (pack.rs PACK_ENV_LOCK과 동일 패턴).
    static ACL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 격리된 임시 디렉터리에 acl.json을 깔고 그 안에 소켓 경로를 둔 Daemon을 만든다.
    /// 반환된 _guard가 살아있는 동안 CYS_PACK_DIR가 이 디렉터리를 가리킨다.
    fn daemon_with_acl(tag: &str, acl_json: &str) -> (Arc<Daemon>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cys-acl-{}-{}-{}",
            tag,
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("acl.json"), acl_json).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        (daemon, dir)
    }

    /// T1-3 회귀: send_text의 `human:true`는 ACL을 우회하지 못한다.
    /// 발견(신원 위조·ACL 우회): reviewer pane이 {"human":true}를 끼워 reviewer-*→worker*
    /// deny 규칙을 뚫고 워커 stdin에 직접 주입할 수 있었다. human은 클라이언트 자기신고라
    /// 커널 peer pid 기반 ACL을 우회하는 신호로 쓰여선 안 된다 — 이 분기점을 박제한다.
    #[test]
    fn send_text_human_flag_does_not_bypass_acl() {
        let _g = ACL_ENV_LOCK.lock().unwrap();
        let acl = r#"{
            "default": "allow",
            "rules": [
                { "from": "reviewer-*", "to": "worker*", "allow": false }
            ]
        }"#;
        let (daemon, dir) = daemon_with_acl("human-bypass", acl);

        // 대상: worker 역할 surface (reviewer가 주입하려는 stdin)
        let worker = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("worker-1".into()), 24, 80)
            .expect("create worker surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(worker.id, worker.clone());

        // 발신: reviewer 역할 surface. caller_cache에 synthetic pid→reviewer sid를 심어
        // 프로세스 트리 워크 없이 발신자 신원이 reviewer로 해석되게 한다 (커널 경로 대역).
        let reviewer = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("reviewer-gemini".into()), 24, 80)
            .expect("create reviewer surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(reviewer.id, reviewer.clone());
        let reviewer_pid = 999_001_u32;
        daemon
            .caller_cache
            .lock()
            .unwrap()
            .insert(reviewer_pid, (Some(reviewer.id), crate::state::now_epoch(), None));

        // reviewer가 human:true로 worker stdin 주입 시도 → ACL deny가 떠야 한다.
        let req = Request {
            id: json!(1),
            method: "surface.send_text".into(),
            params: json!({
                "surface_id": worker.id,
                "text": "rm -rf /\n",
                "human": true
            }),
        };
        let reply = dispatch(&daemon, req, Some(reviewer_pid));
        let Reply::Single(resp) = reply else {
            panic!("expected single reply");
        };
        assert_eq!(
            resp["ok"], json!(false),
            "human:true가 reviewer→worker ACL을 우회했다 (응답: {resp})"
        );
        assert_eq!(
            resp["error"]["code"], json!("acl_denied"),
            "ACL deny가 아닌 다른 경로로 통과/거부됐다 (응답: {resp})"
        );

        // 대조군: 동일 reviewer가 human 없이 보내도 같은 deny (비대칭이 아님을 박제)
        let req2 = Request {
            id: json!(2),
            method: "surface.send_text".into(),
            params: json!({ "surface_id": worker.id, "text": "x\n" }),
        };
        let Reply::Single(resp2) = dispatch(&daemon, req2, Some(reviewer_pid)) else {
            panic!("expected single reply");
        };
        assert_eq!(resp2["error"]["code"], json!("acl_denied"));

        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 회귀(ACL 거부 발신의 부작용 누수 → 타이핑 가드 오염·교착):
    /// 발견 — send_text의 `human:true`가 ACL 검증 *이전*에 대상 surface의 last_human_input을
    /// 무조건 갱신했다. send 대상에는 소유 검증이 없어(누구나 살아있는 surface 지정 가능)
    /// ACL이 거부(Err)하더라도 갱신이 이미 일어난 뒤였다. 결과: reviewer-*→worker* deny된
    /// 노드가 worker를 향해 human:true를 반복하면, 텍스트 배달은 거부되지만 worker의
    /// last_human_input이 계속 갱신되어 타이핑 가드 창(기본 3초)이 영구 갱신 → master 등
    /// 정당한 발신자의 비-human send_text·send_key가 'human is typing'으로 직접 주입 차단.
    /// 수정: last_human_input 기록을 check_send_acl 통과 *이후*로 옮긴다. 이 분기점을 박제한다.
    #[test]
    fn send_text_denied_human_flag_does_not_touch_typing_guard() {
        let _g = ACL_ENV_LOCK.lock().unwrap();
        let acl = r#"{
            "default": "allow",
            "rules": [
                { "from": "reviewer-*", "to": "worker*", "allow": false }
            ]
        }"#;
        let (daemon, dir) = daemon_with_acl("denied-guard", acl);

        // 대상: worker pane (타이핑 가드가 오염될 피해자)
        let worker = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("worker-1".into()), 24, 80)
            .expect("create worker surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(worker.id, worker.clone());
        // 사전 조건: worker는 아무도 타이핑하지 않은 상태 (가드 비활성)
        assert!(
            worker.last_human_input.lock().unwrap().is_none(),
            "사전조건 위반: worker last_human_input이 처음부터 Some"
        );

        // 발신: ACL로 차단된 reviewer pane
        let reviewer = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("reviewer-gemini".into()), 24, 80)
            .expect("create reviewer surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(reviewer.id, reviewer.clone());
        let reviewer_pid = 999_003_u32;
        daemon
            .caller_cache
            .lock()
            .unwrap()
            .insert(reviewer_pid, (Some(reviewer.id), crate::state::now_epoch(), None));

        // reviewer가 human:true로 worker stdin 주입 시도 → ACL deny가 떠야 한다.
        let req = Request {
            id: json!(1),
            method: "surface.send_text".into(),
            params: json!({ "surface_id": worker.id, "text": "x\n", "human": true }),
        };
        let Reply::Single(resp) = dispatch(&daemon, req, Some(reviewer_pid)) else {
            panic!("expected single reply");
        };
        assert_eq!(
            resp["error"]["code"], json!("acl_denied"),
            "전제: 차단된 발신은 acl_denied여야 한다 (응답: {resp})"
        );

        // 핵심 불변식: 거부된 발신은 피해 surface의 타이핑 가드 상태를 건드리지 못한다.
        assert!(
            worker.last_human_input.lock().unwrap().is_none(),
            "ACL 거부된 human:true 발신이 worker의 last_human_input을 갱신했다 (타이핑 가드 오염)"
        );

        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 대조: ACL이 허용하는 발신(reviewer→master)은 human 유무와 무관하게 통과한다.
    /// 수정이 정상 경로를 막지 않았음을 박제 (UI=external·허용 발신 회귀 방지).
    #[test]
    fn send_text_allowed_path_still_passes() {
        let _g = ACL_ENV_LOCK.lock().unwrap();
        let acl = r#"{
            "default": "allow",
            "rules": [
                { "from": "reviewer-*", "to": "worker*", "allow": false },
                { "from": "reviewer-*", "to": "master", "allow": true }
            ]
        }"#;
        let (daemon, dir) = daemon_with_acl("allow-path", acl);

        let master = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("create master surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(master.id, master.clone());
        let reviewer = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("reviewer-codex".into()), 24, 80)
            .expect("create reviewer surface");
        daemon
            .surfaces
            .lock()
            .unwrap()
            .insert(reviewer.id, reviewer.clone());
        let reviewer_pid = 999_002_u32;
        daemon
            .caller_cache
            .lock()
            .unwrap()
            .insert(reviewer_pid, (Some(reviewer.id), crate::state::now_epoch(), None));

        let req = Request {
            id: json!(1),
            method: "surface.send_text".into(),
            params: json!({ "surface_id": master.id, "text": "hi\n", "human": true }),
        };
        let Reply::Single(resp) = dispatch(&daemon, req, Some(reviewer_pid)) else {
            panic!("expected single reply");
        };
        assert_eq!(
            resp["ok"], json!(true),
            "허용된 reviewer→master 발신이 막혔다 (응답: {resp})"
        );

        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 락 없는 임시 데몬 + 발신 pane 신원 주입 헬퍼 (claim_role 신원 검증 테스트용).
    /// caller_cache에 synthetic pid→sid를 심어 프로세스 트리 워크 없이 발신자를 확정한다.
    fn claim_daemon() -> Arc<Daemon> {
        let dir = std::env::temp_dir().join(format!(
            "cys-claim-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        Daemon::new(dir.join("cysd.sock"))
    }

    fn make_surface(daemon: &Arc<Daemon>, role: Option<&str>) -> u64 {
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, role.map(|r| r.into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.id
    }

    fn bind_caller(daemon: &Arc<Daemon>, pid: u32, sid: u64) {
        daemon
            .caller_cache
            .lock()
            .unwrap()
            .insert(pid, (Some(sid), crate::state::now_epoch(), None));
    }

    fn claim(daemon: &Arc<Daemon>, role: &str, surface_id: u64, caller_pid: Option<u32>) -> Value {
        let req = Request {
            id: json!(1),
            method: "system.claim_role".into(),
            params: json!({ "role": role, "surface_id": surface_id }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(pid 재사용 → 신원 오인): resolve_caller_surface의 60초 caller_cache는 pid만으로
    /// 히트를 반환해, 단명 CLI가 죽고 OS가 같은 pid를 다른 pane 프로세스에 재할당하면 60초 창
    /// 안에서 이전 pane의 surface(=이전 role)로 오인됐다 (ACL from_role이 이 결과로 결정됨).
    /// 수정: 캐시에 peer start_time을 함께 저장하고, 히트 시 현재 pid의 start_time과 대조해
    /// incarnation이 다르면(=pid 재사용) 캐시를 무효화하고 재해석한다. 이 게이트를 박제한다.
    #[test]
    fn caller_cache_rejects_reused_pid_by_start_time() {
        let daemon = claim_daemon();
        let stale = make_surface(&daemon, Some("master")); // pid를 물려준 옛 incarnation의 pane

        // 현재 살아있는 실제 pid: 데몬 자기 프로세스. 그 진짜 start_time을 구한다.
        let live_pid = std::process::id();
        let real_start = peer_start_time(live_pid).expect("self process must be visible");

        // ── 시나리오 1: incarnation 불일치 ──
        // 옛 CLI가 stale pane으로 해석돼 캐시됐고 그 뒤 pid가 재사용됐다고 가정.
        // 캐시된 start_time을 일부러 어긋나게(현재≠캐시) 심는다. 재사용 식별자가 작동하면
        // 캐시 히트를 신뢰하지 않고 재해석해야 한다 → stale surface를 반환하면 안 된다.
        daemon.caller_cache.lock().unwrap().insert(
            live_pid,
            (Some(stale), crate::state::now_epoch(), Some(real_start ^ 0xFFFF)),
        );
        let resolved = resolve_caller_surface(&daemon, live_pid);
        assert_ne!(
            resolved,
            Some(stale),
            "pid 재사용(start_time 불일치)인데 이전 pane surface로 오인했다 (resolved={resolved:?})"
        );

        // ── 시나리오 2: 동일 incarnation은 정상 캐시 히트 (수정이 캐시를 무력화하지 않았음) ──
        // 같은 start_time이면 같은 프로세스이므로 캐시된 surface를 그대로 반환해야 한다.
        let same = make_surface(&daemon, Some("worker-1"));
        daemon.caller_cache.lock().unwrap().insert(
            live_pid,
            (Some(same), crate::state::now_epoch(), Some(real_start)),
        );
        assert_eq!(
            resolve_caller_surface(&daemon, live_pid),
            Some(same),
            "동일 incarnation(start_time 일치)인데 캐시 히트가 무효화됐다 — 성능 회귀"
        );

        // ── 시나리오 3: 합성/레거시 항목(start_time=None)은 무조건 신뢰 (테스트·주입 경로 보존) ──
        let synth = make_surface(&daemon, Some("reviewer-gemini"));
        daemon.caller_cache.lock().unwrap().insert(
            live_pid,
            (Some(synth), crate::state::now_epoch(), None),
        );
        assert_eq!(
            resolve_caller_surface(&daemon, live_pid),
            Some(synth),
            "start_time=None 합성 항목이 신뢰되지 않았다 — 주입 경로 회귀"
        );
    }

    /// 발견(caller_cache 무한 성장): resolve_caller_surface는 캐시-미스마다 caller_pid→항목을
    /// insert만 하고 어디서도 stale을 회수하지 않았다. 60초 TTL은 '같은 pid를 다시 조회할 때'만
    /// 검사되는데 cys CLI는 매 호출이 새 단명 프로세스라 동일 pid가 사실상 재등장하지 않아 TTL
    /// 가지치기가 영영 발동하지 않았다 → 데몬 수명 동안 HashMap이 단조 누적(send/send_key의
    /// ACL 검증 경로라 멀티에이전트 push에서 가속). 수정: 삽입 시 만료 항목 일괄 회수 + 하드 캡.
    /// 이 게이트(만료 항목이 회수돼 캐시가 유한하게 유지됨)를 박제한다.
    #[test]
    fn caller_cache_evicts_expired_entries_on_insert() {
        let daemon = claim_daemon();

        // 단명 CLI 호출 N건이 누적된 상태 모사: 전부 60초보다 오래된(만료) ts로 직접 심는다.
        // 각 pid는 사실상 유일 → 캐시 히트 TTL 검사가 영영 닿지 않는 stale 항목들이다.
        let stale_ts = crate::state::now_epoch() - 120.0; // 만료(>60s)
        {
            let mut cache = daemon.caller_cache.lock().unwrap();
            for pid in 1_000u32..6_000u32 {
                cache.insert(pid, (None, stale_ts, None));
            }
        }
        let before = daemon.caller_cache.lock().unwrap().len();
        assert_eq!(before, 5_000, "사전 조건: stale 항목 5000건이 적재돼야 한다");

        // 새 caller 해석(캐시 미스 → 삽입 경로) 1회 — 데몬 자기 pid를 발신자로 쓴다.
        // 수정 전: insert만 → 5001건 잔존. 수정 후: 만료 일괄 회수 → 갓 삽입한 항목만 남는다.
        let fresh_pid = std::process::id();
        let _ = resolve_caller_surface(&daemon, fresh_pid);

        let after = daemon.caller_cache.lock().unwrap().len();
        assert!(
            after <= 2,
            "만료(now-ts≥60s) 항목이 삽입 시 회수되지 않았다 — caller_cache 무한 성장 \
             (before={before}, after={after})"
        );
        // 갓 해석한 fresh_pid 항목은 살아있어야 한다(정상 캐싱 동작 불변).
        assert!(
            daemon.caller_cache.lock().unwrap().contains_key(&fresh_pid),
            "방금 해석한 fresh 항목까지 회수됐다 — 회수 로직이 과도하다"
        );
    }

    /// 하드 캡(60초 창 내 폭주): 만료 회수만으로는 60초 안에 대량 유입되는 fresh 항목을 못 막는다.
    /// 캡(CALLER_CACHE_CAP)을 초과하면 가장 오래된 항목부터 솎여 캐시가 상한 아래로 유지돼야 한다.
    ///
    /// 합성 pid는 실존 불가 고역(10M+)을 쓴다 — OS pid 상한(macOS 99999·Linux ≤4194304) 밖이라
    /// 테스트 프로세스 pid와 절대 충돌하지 않는다. 저역(1000..7000)을 쓰면 cargo test 프로세스
    /// pid가 그 범위에 들 때 resolve가 합성 항목에 캐시-히트해 조기 반환 → 삽입·캡 경로에 진입
    /// 못 해 환경 의존으로 실패했다(발견된 테스트 비결정성 — 박제).
    #[test]
    fn caller_cache_enforces_hard_cap_within_ttl_window() {
        let daemon = claim_daemon();

        // 전부 '신선한'(만료 아님) ts로 캡(4096)을 크게 초과해 적재 → 만료 회수로는 안 줄어든다.
        let fresh_ts = crate::state::now_epoch();
        {
            let mut cache = daemon.caller_cache.lock().unwrap();
            for pid in 10_000_000u32..10_006_000u32 {
                cache.insert(pid, (None, fresh_ts, None));
            }
        }
        assert_eq!(
            daemon.caller_cache.lock().unwrap().len(),
            6_000,
            "사전 조건: 신선한 항목 6000건이 적재돼야 한다(>캡 4096)"
        );

        // 삽입 경로 1회 진입 → 캡 집행 발동. (자기 pid는 10M 미만이라 캐시-미스 보장)
        let _ = resolve_caller_surface(&daemon, std::process::id());

        let after = daemon.caller_cache.lock().unwrap().len();
        assert!(
            after <= 4_096,
            "하드 캡(4096)을 넘어 신선한 항목이 무한 누적됐다 (after={after})"
        );
    }

    /// 발견(신원·소유 검증 부재): claim_role이 caller_pid를 전혀 쓰지 않아, 워커 pane이
    /// 자기 소유가 아닌 임의 surface에 역할을 박을 수 있었다 (handlers.rs:654 무조건 insert).
    /// 발신 pane은 자기 surface에만 역할을 등록할 수 있어야 한다 — 이 게이트를 박제한다.
    #[test]
    fn claim_role_rejects_foreign_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, None);
        let attacker_pid = 990_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        // 공격: attacker pane이 자기 소유가 아닌 victim surface에 'worker' 역할 등록 시도.
        let resp = claim(&daemon, "worker", victim, Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 surface에 대한 claim이 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("claim_denied"));
        // victim surface의 role이 오염되지 않았는지 확인 (insert가 일어나지 않아야 함).
        assert!(
            daemon.surfaces.lock().unwrap()[&victim].role.lock().unwrap().is_none(),
            "거부됐는데 victim role이 등록됐다"
        );
        assert!(
            daemon.roles.lock().unwrap().get("worker").is_none(),
            "거부됐는데 roles 매핑이 생성됐다"
        );
    }

    /// 발견(특권 역할 탈취): claim_role이 roles.insert(role, sid)를 무조건 수행해, 워커 pane이
    /// 'master'를 자기 surface로 재지정→roles["master"] 매핑·deadman 감시·--to master 라우팅을
    /// 통째로 하이재킹할 수 있었다. 살아있는 master가 점유 중이면 다른 surface의 claim을 거부.
    #[test]
    fn claim_role_rejects_master_takeover_by_live_holder() {
        let daemon = claim_daemon();
        // 정당한 master를 먼저 세운다 (자기 surface에 자기 claim — 허용 경로).
        let master = make_surface(&daemon, None);
        let master_pid = 990_201_u32;
        bind_caller(&daemon, master_pid, master);
        let ok = claim(&daemon, "master", master, Some(master_pid));
        assert_eq!(ok["ok"], json!(true), "정당한 첫 master claim이 막혔다 (응답: {ok})");
        assert_eq!(daemon.roles.lock().unwrap().get("master").copied(), Some(master));

        // 공격: worker pane이 자기 surface에 'master'를 claim해 매핑 탈취 시도.
        let attacker = make_surface(&daemon, Some("worker-1"));
        let attacker_pid = 990_202_u32;
        bind_caller(&daemon, attacker_pid, attacker);
        let resp = claim(&daemon, "master", attacker, Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "살아있는 master가 있는데 워커의 master 탈취가 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("claim_denied"));
        // master 매핑이 여전히 원래 surface를 가리켜야 한다 (탈취 미발생).
        assert_eq!(
            daemon.roles.lock().unwrap().get("master").copied(),
            Some(master),
            "master 매핑이 공격자로 넘어갔다"
        );
    }

    fn create_surface_rpc(daemon: &Arc<Daemon>, role: Option<&str>, caller_pid: Option<u32>) -> Value {
        let params = match role {
            Some(r) => json!({ "cmd": "sleep 30", "role": r }),
            None => json!({ "cmd": "sleep 30" }),
        };
        let req = Request {
            id: json!(1),
            method: "surface.create".into(),
            params,
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(특권 역할 탈취 — create 경로 우회): create_surface(state.rs)가 요청 role을 roles에
    /// 무조건 insert("최신 surface 승리")해, 임의 pane이 surface.create {"role":"master"}로
    /// 살아있는 master가 있어도 roles["master"]·deadman 감시·--to master 라우팅을 통째로
    /// 하이재킹할 수 있었다. claim_role이 막는 바로 그 공격의 create 경로 자매 케이스 — 박제.
    #[test]
    fn surface_create_rejects_master_takeover_by_live_holder() {
        let daemon = claim_daemon();
        // 정당한 master를 먼저 세운다 (create_surface 직접 — 정상 부트 경로).
        let master = make_surface(&daemon, Some("master"));
        assert_eq!(daemon.roles.lock().unwrap().get("master").copied(), Some(master));

        // 공격: 임의 pane이 surface.create로 'master'를 지정해 매핑 탈취 시도.
        let attacker_pid = 991_201_u32;
        let resp = create_surface_rpc(&daemon, Some("master"), Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "살아있는 master가 있는데 create 경로 master 탈취가 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("claim_denied"));
        // master 매핑이 여전히 원래 surface를 가리켜야 한다 (탈취 미발생).
        assert_eq!(
            daemon.roles.lock().unwrap().get("master").copied(),
            Some(master),
            "master 매핑이 create 경로로 공격자에게 넘어갔다"
        );

        // cso도 동일하게 보호되는지 — 살아있는 cso 점유 후 탈취 거부.
        let cso = make_surface(&daemon, Some("cso"));
        assert_eq!(daemon.roles.lock().unwrap().get("cso").copied(), Some(cso));
        let resp2 = create_surface_rpc(&daemon, Some("cso"), Some(991_202_u32));
        assert_eq!(resp2["ok"], json!(false), "create 경로 cso 탈취가 통과했다 (응답: {resp2})");
        assert_eq!(
            daemon.roles.lock().unwrap().get("cso").copied(),
            Some(cso),
            "cso 매핑이 create 경로로 넘어갔다"
        );
    }

    /// 대조군(수정이 정상 경로를 막지 않음을 박제): ① master 미등록 시 create로 첫 등록 허용
    /// ② 비특권 역할(worker)은 create로 항상 재등록 허용 ③ role 없는 일반 surface는 항상 허용.
    #[test]
    fn surface_create_allows_legitimate_roles() {
        let daemon = claim_daemon();

        // ① master 미등록 상태에서 create로 첫 master 등록 — 허용.
        let r1 = create_surface_rpc(&daemon, Some("master"), Some(991_301_u32));
        assert_eq!(r1["ok"], json!(true), "정당한 첫 master create가 막혔다 (응답: {r1})");
        assert!(daemon.roles.lock().unwrap().get("master").is_some());

        // ② 비특권 역할은 보호 대상이 아니므로 살아있는 보유자가 있어도 create 재등록 허용.
        let _w = make_surface(&daemon, Some("worker-1"));
        let r2 = create_surface_rpc(&daemon, Some("worker-1"), Some(991_302_u32));
        assert_eq!(r2["ok"], json!(true), "비특권 worker create가 막혔다 (응답: {r2})");

        // ③ role 미지정 일반 surface는 게이트 무관 — 항상 허용.
        let r3 = create_surface_rpc(&daemon, None, Some(991_303_u32));
        assert_eq!(r3["ok"], json!(true), "role 없는 일반 surface create가 막혔다 (응답: {r3})");
    }

    /// 대조군: 정당한 자기-claim은 통과해야 한다 (수정이 정상 경로를 막지 않음을 박제).
    /// ① 비특권 역할 자기 등록 ② master 미등록 시 첫 claim — 둘 다 허용.
    #[test]
    fn claim_role_allows_self_claim() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, None);
        let own_pid = 990_301_u32;
        bind_caller(&daemon, own_pid, own);

        // ① 비특권 역할 자기 등록
        let r1 = claim(&daemon, "worker-7", own, Some(own_pid));
        assert_eq!(r1["ok"], json!(true), "정당한 자기 비특권 claim이 막혔다 (응답: {r1})");
        assert_eq!(daemon.roles.lock().unwrap().get("worker-7").copied(), Some(own));

        // ② master 미등록 상태에서 별도 surface가 master를 첫 claim
        let m = make_surface(&daemon, None);
        let m_pid = 990_302_u32;
        bind_caller(&daemon, m_pid, m);
        let r2 = claim(&daemon, "master", m, Some(m_pid));
        assert_eq!(r2["ok"], json!(true), "정당한 첫 master claim이 막혔다 (응답: {r2})");
        assert_eq!(daemon.roles.lock().unwrap().get("master").copied(), Some(m));

        // ③ 동일 master가 자기 master를 재-claim (idempotent) — 거부되면 안 됨.
        let r3 = claim(&daemon, "master", m, Some(m_pid));
        assert_eq!(r3["ok"], json!(true), "idempotent master 재claim이 막혔다 (응답: {r3})");
    }

    fn resolve_role(daemon: &Arc<Daemon>, role: &str) -> Value {
        let req = Request {
            id: json!(1),
            method: "system.resolve_role".into(),
            params: json!({ "role": role }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, None) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(roles dangling — 자력 종료 surface): roles 매핑은 surface가 셸 EOF로 자력 종료하면
    /// close_surface를 거치지 않아(state.rs는 exited만 세움) dead_sid가 그대로 남는다.
    /// resolve_role이 생존성을 검증하지 않으면 --to <role> 주소가 죽은 surface를 정상 반환해
    /// 발신자가 '역할 생존'으로 오인한다. fire_push·check_master_deadman과 동일한 부재 보정을 박제.
    #[test]
    fn resolve_role_rejects_dead_surface() {
        let daemon = claim_daemon();
        let sid = make_surface(&daemon, Some("worker"));

        // 사전: 살아있는 surface는 정상 해석된다.
        let live = resolve_role(&daemon, "worker");
        assert_eq!(live["ok"], json!(true), "살아있는 역할 해석이 막혔다 (응답: {live})");
        assert_eq!(live["result"]["surface_id"].as_u64(), Some(sid));

        // 자력 종료 시뮬레이션: close_surface를 거치지 않고 exited만 세운다
        // (state.rs:619 자력 종료 경로와 동일 — roles 매핑은 그대로 잔존).
        daemon.surfaces.lock().unwrap()[&sid]
            .exited
            .store(true, Ordering::Relaxed);
        assert_eq!(
            daemon.roles.lock().unwrap().get("worker").copied(),
            Some(sid),
            "사전 조건: roles 매핑이 dead_sid를 가리켜야 한다"
        );

        // 검증: 죽은 surface는 부재로 강등돼야 한다 (dangling 주소 반환 금지).
        let dead = resolve_role(&daemon, "worker");
        assert_eq!(
            dead["ok"], json!(false),
            "죽은 surface가 살아있는 역할로 해석됐다 (응답: {dead})"
        );
        assert_eq!(dead["error"]["code"], json!("not_found"));
    }

    /// 익명/추적 불가 발신(caller_pid=None)은 신원 확정 불가 → claim 거부.
    #[test]
    fn claim_role_rejects_anonymous_caller() {
        let daemon = claim_daemon();
        let s = make_surface(&daemon, None);
        let resp = claim(&daemon, "master", s, None);
        assert_eq!(
            resp["ok"], json!(false),
            "신원 미확정 익명 claim이 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("claim_denied"));
    }

    fn set_meta(
        daemon: &Arc<Daemon>,
        surface_id: u64,
        agent: &str,
        caller_pid: Option<u32>,
    ) -> Value {
        let req = Request {
            id: json!(1),
            method: "surface.set_meta".into(),
            params: json!({ "surface_id": surface_id, "agent": agent }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(신원·소유 검증 부재): surface.set_meta가 caller_pid를 전혀 쓰지 않아, 워커 pane이
    /// 자기 소유가 아닌 살아있는 타 노드의 agent_meta를 덮어쓸 수 있었다. agent 문자열은
    /// check_approvals(governance.rs)에서 approval_patterns 키로 쓰여 그 surface 화면을 매칭하고,
    /// set_meta는 agent_seen/agent_exit_notified를 리셋해 사망 감지 상태머신을 교란한다.
    /// 발신 pane은 자기 소유 surface(또는 아직 미등록 자식)에만 메타를 쓸 수 있어야 한다 — 박제.
    #[test]
    fn set_meta_rejects_foreign_live_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, Some("reviewer-gemini"));
        let attacker_pid = 991_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        // 피해 노드가 이미 정당한 메타를 보유한 상태 (살아있는 타 노드).
        *daemon.surfaces.lock().unwrap()[&victim].agent_meta.lock().unwrap() =
            Some(("gemini".into(), "gemini".into()));

        // 공격: attacker pane이 victim의 메타를 'claude'로 덮어써 패턴 매칭/사망 감지를 교란.
        let resp = set_meta(&daemon, victim, "claude", Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 노드의 live 메타 덮어쓰기가 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("meta_denied"));
        // victim 메타가 오염되지 않았는지 확인 (원래 agent 유지).
        assert_eq!(
            daemon.surfaces.lock().unwrap()[&victim]
                .agent_meta.lock().unwrap().as_ref().map(|(n, _)| n.clone()),
            Some("gemini".into()),
            "거부됐는데 victim agent_meta가 덮어써졌다"
        );
    }

    /// 대조군 ①: 자기 surface 메타 갱신은 통과 (cs == sid). 정상 경로 박제.
    #[test]
    fn set_meta_allows_self_update() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));
        let own_pid = 991_201_u32;
        bind_caller(&daemon, own_pid, own);
        // 이미 메타가 있어도 자기 자신은 갱신 가능해야 한다.
        *daemon.surfaces.lock().unwrap()[&own].agent_meta.lock().unwrap() =
            Some(("claude".into(), "claude".into()));

        let resp = set_meta(&daemon, own, "claude", Some(own_pid));
        assert_eq!(resp["ok"], json!(true), "자기 메타 갱신이 막혔다 (응답: {resp})");
    }

    /// 대조군 ②: 오케스트레이터가 갓 만든 자식 surface(메타 미등록) 초기화는 통과.
    /// launch-agent 흐름 — 발신 pane은 master이고 대상 자식은 agent_meta == None.
    #[test]
    fn set_meta_allows_fresh_child_init() {
        let daemon = claim_daemon();
        let master = make_surface(&daemon, Some("master"));
        let master_pid = 991_301_u32;
        bind_caller(&daemon, master_pid, master);
        // 갓 create된 자식 — 아직 agent_meta 없음.
        let child = make_surface(&daemon, Some("worker-2"));
        assert!(daemon.surfaces.lock().unwrap()[&child].agent_meta.lock().unwrap().is_none());

        let resp = set_meta(&daemon, child, "claude", Some(master_pid));
        assert_eq!(resp["ok"], json!(true), "자식 초기화 set_meta가 막혔다 (응답: {resp})");
        assert_eq!(
            daemon.surfaces.lock().unwrap()[&child]
                .agent_meta.lock().unwrap().as_ref().map(|(n, _)| n.clone()),
            Some("claude".into()),
        );
    }

    /// 대조군 ③: 데몬 spawn node-recover(발신 pane 없음 = caller_pid None)는 이미 메타가 있는
    /// surface에 동일 에이전트를 재등록한다 — 익명이지만 정당 경로이므로 통과해야 한다.
    /// (pane은 커널 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다 = 안전.)
    #[test]
    fn set_meta_allows_anonymous_recovery_on_existing_meta() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-3"));
        *daemon.surfaces.lock().unwrap()[&node].agent_meta.lock().unwrap() =
            Some(("claude".into(), "claude".into()));

        let resp = set_meta(&daemon, node, "claude", None);
        assert_eq!(
            resp["ok"], json!(true),
            "데몬 내부 복구(익명) 재등록이 막혔다 (응답: {resp})"
        );
    }

    fn status_set(
        daemon: &Arc<Daemon>,
        surface_id: u64,
        state: &str,
        context: u64,
        task: &str,
        caller_pid: Option<u32>,
    ) -> Value {
        let req = Request {
            id: json!(1),
            method: "status.set".into(),
            params: json!({ "surface_id": surface_id, "state": state,
                            "context": context, "task": task }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(신원·소유 검증 부재): status.set이 caller_pid를 전혀 쓰지 않아, 워커 pane이 자기 소유가
    /// 아닌 타 노드의 자기보고 상태(state/context_pct/task)를 위조할 수 있었다. agent_status의 유일
    /// 소비처는 org.status 보드(master/CSO의 '60% /clear'·blocked/done·deadman 보조 판단의 거버넌스
    /// 입력)라, 타 노드의 'done'·낮은 context_pct 위조는 자율주행 의사결정을 오도한다.
    /// 발신 pane은 자기 surface(cs == sid)에만 자기 상태를 보고할 수 있어야 한다 — 박제.
    #[test]
    fn status_set_rejects_foreign_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, Some("worker-2"));
        let attacker_pid = 992_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        // 피해 노드가 정당하게 자기보고한 현재 상태 (실제로는 작업 중·컨텍스트 높음).
        *daemon.surfaces.lock().unwrap()[&victim].agent_status.lock().unwrap() =
            Some(crate::state::AgentStatus {
                state: "working".into(),
                context_pct: Some(85),
                task: Some("진짜 작업".into()),
                updated_at: crate::state::now_epoch(),
            });

        // 공격: attacker pane이 victim을 'done'·context 10으로 위조해 거버넌스 판단을 오도.
        let resp = status_set(&daemon, victim, "done", 10, "위조", Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 노드의 자기보고 상태 위조가 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("status_denied"));
        // victim 상태가 오염되지 않았는지 확인 (원래 자기보고 유지).
        let st = daemon.surfaces.lock().unwrap()[&victim]
            .agent_status.lock().unwrap().clone()
            .expect("victim status present");
        assert_eq!(st.state, "working", "거부됐는데 victim state가 위조됐다");
        assert_eq!(st.context_pct, Some(85), "거부됐는데 victim context_pct가 위조됐다");
    }

    /// 대조군 ①: 자기 surface 상태 보고는 통과 (cs == sid). 정상 자기보고 경로 박제.
    #[test]
    fn status_set_allows_self_report() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));
        let own_pid = 992_201_u32;
        bind_caller(&daemon, own_pid, own);

        let resp = status_set(&daemon, own, "blocked", 60, "내 작업", Some(own_pid));
        assert_eq!(resp["ok"], json!(true), "자기 상태 보고가 막혔다 (응답: {resp})");
        let st = daemon.surfaces.lock().unwrap()[&own]
            .agent_status.lock().unwrap().clone()
            .expect("status present");
        assert_eq!(st.state, "blocked");
        assert_eq!(st.context_pct, Some(60));
    }

    /// 대조군 ②: 익명 발신(caller_pid None = 데몬 내부 경로)은 통과해야 한다.
    /// (pane은 커널 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다 = 안전.)
    #[test]
    fn status_set_allows_anonymous() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-3"));

        let resp = status_set(&daemon, node, "done", 20, "복구", None);
        assert_eq!(
            resp["ok"], json!(true),
            "익명(데몬 내부) 상태 보고가 막혔다 (응답: {resp})"
        );
    }

    fn usage_register(
        daemon: &Arc<Daemon>,
        surface_id: u64,
        transcript: &str,
        caller_pid: Option<u32>,
    ) -> Value {
        let req = Request {
            id: json!(1),
            method: "usage.register".into(),
            params: json!({ "surface_id": surface_id, "transcript": transcript }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// T5 소유 게이트 — status.set과 동형: 발신 pane이 타 surface에 트랜스크립트를 등록하면
    /// 수집기가 가짜 세션 파일을 관측해 master/CSO가 보는 컨텍스트 수치가 위조된다(60%
    /// 사이클 오발·억제). 자기 surface 외 등록은 거부돼야 한다 — 박제.
    #[test]
    fn usage_register_rejects_foreign_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, Some("worker-2"));
        let attacker_pid = 993_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        let resp = usage_register(&daemon, victim, "/tmp/fake.jsonl", Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 surface 트랜스크립트 등록이 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("usage_denied"));
        assert!(
            daemon.surfaces.lock().unwrap()[&victim]
                .registered_transcript.lock().unwrap().is_none(),
            "거부됐는데 victim 등록이 오염됐다"
        );
    }

    /// 대조군: 자기 surface 등록(SessionStart hook 경로)은 통과하고 경로가 저장된다.
    /// 파일 존재는 요구하지 않는다 — SessionStart 시점엔 트랜스크립트가 아직 없을 수 있다.
    #[test]
    fn usage_register_allows_self_and_stores_path() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));
        let own_pid = 993_201_u32;
        bind_caller(&daemon, own_pid, own);

        let resp = usage_register(
            &daemon,
            own,
            "/Users/x/.claude/projects/-p/abc.jsonl",
            Some(own_pid),
        );
        assert_eq!(resp["ok"], json!(true), "자기 등록이 막혔다 (응답: {resp})");
        assert_eq!(
            daemon.surfaces.lock().unwrap()[&own]
                .registered_transcript.lock().unwrap().as_deref(),
            Some("/Users/x/.claude/projects/-p/abc.jsonl")
        );
    }

    /// 경로 위생: 상대경로·비 .jsonl은 거부 — 수집기가 임의 파일을 tail하는 입력을 차단.
    #[test]
    fn usage_register_validates_path_shape() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));

        for bad in [
            "relative/x.jsonl",
            "/tmp/evil.txt",
            "",
            "/tmp/x.jsonl/../../etc/passwd",
            "/tmp/../etc/secret.jsonl",
        ] {
            let resp = usage_register(&daemon, own, bad, None);
            assert_eq!(
                resp["ok"], json!(false),
                "잘못된 경로가 통과했다: {bad:?} (응답: {resp})"
            );
        }
    }

    fn threshold_events(daemon: &Arc<Daemon>, sid: u64) -> Vec<Value> {
        daemon
            .bus
            .replay_after(0)
            .into_iter()
            .filter(|e| {
                e["name"].as_str() == Some("context.threshold")
                    && e["surface_id"].as_u64() == Some(sid)
            })
            .collect()
    }

    /// ★불변식 박제 (절대지침 — 컨텍스트 60% 사이클의 결정론 트리거):
    /// status.set의 context 자기보고가 임계(기본 60)를 '미만→이상'으로 교차하는 순간에만
    /// `context.threshold`(watchdog) 이벤트가 1회 발행된다. 임계 위 체류 중 재발행 없음,
    /// 임계 아래로 내려갔다 다시 넘으면 재발행. LLM 재량이 아니라 수치 비교가 유일 트리거다.
    #[test]
    fn context_threshold_fires_on_crossing_only() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-ctx"));

        // 임계 미만: 발화 없음
        status_set(&daemon, node, "working", 59, "t", None);
        assert_eq!(threshold_events(&daemon, node).len(), 0, "59%에서 발화됐다");

        // 미만→이상 교차: 1회 발화
        status_set(&daemon, node, "working", 65, "t", None);
        let evs = threshold_events(&daemon, node);
        assert_eq!(evs.len(), 1, "60% 교차에서 정확히 1회 발화돼야 한다");
        assert_eq!(evs[0]["category"].as_str(), Some("watchdog"));
        assert_eq!(evs[0]["payload"]["context_pct"].as_u64(), Some(65));
        assert_eq!(evs[0]["payload"]["threshold"].as_u64(), Some(60));

        // 임계 위 체류: 재발행 없음 (스팸 차단)
        status_set(&daemon, node, "working", 70, "t", None);
        assert_eq!(threshold_events(&daemon, node).len(), 1, "체류 중 중복 발화됐다");

        // 아래로 복귀(clear 후 재보고) → 다시 교차: 재발행
        status_set(&daemon, node, "working", 10, "t", None);
        status_set(&daemon, node, "working", 80, "t", None);
        assert_eq!(
            threshold_events(&daemon, node).len(),
            2,
            "사이클 후 재교차에서 재발화돼야 한다"
        );
    }

    /// 첫 보고가 이미 임계 이상이면(무보고→이상) 즉시 발화해야 한다 — 무보고를 '미만'으로 간주.
    #[test]
    fn context_threshold_fires_on_first_report_above() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-ctx2"));
        status_set(&daemon, node, "working", 60, "t", None);
        assert_eq!(
            threshold_events(&daemon, node).len(),
            1,
            "첫 보고 60%(경계값 포함)에서 발화돼야 한다"
        );
    }

    /// 회귀 핀: 임계 env 파싱 규칙 — 1~100만 유효, 그 외 전부 기본 60.
    #[test]
    fn threshold_from_parsing_rules() {
        assert_eq!(threshold_from(None), 60);
        assert_eq!(threshold_from(Some("45".into())), 45);
        assert_eq!(threshold_from(Some(" 80 ".into())), 80);
        assert_eq!(threshold_from(Some("0".into())), 60, "0은 무효(상시발화 방지)");
        assert_eq!(threshold_from(Some("101".into())), 60, "100 초과 무효");
        assert_eq!(threshold_from(Some("abc".into())), 60);
        assert_eq!(threshold_from(Some("-5".into())), 60);
    }

    fn close_surface_rpc(daemon: &Arc<Daemon>, surface_id: u64, caller_pid: Option<u32>) -> Value {
        let req = Request {
            id: json!(1),
            method: "surface.close".into(),
            params: json!({ "surface_id": surface_id }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(신원·소유 검증 부재): surface.close가 caller_pid를 전혀 쓰지 않아, 워커 pane이 자기
    /// 소유가 아닌 임의 surface(master/타 노드)를 강제 종료할 수 있었다. close_surface는 변경계 RPC
    /// 중 파괴력이 가장 커서 자식 프로세스 트리 전체 kill·셸 종료·roles 매핑·인플라이트 큐까지 정리한다.
    /// send 경로는 ACL deny(reviewer-*→worker* 등)로 동일 대상을 막는데 close는 게이트 밖이었다 —
    /// 발신 pane은 자기 surface(cs == sid)만 닫을 수 있어야 한다. 이 게이트를 박제한다.
    #[test]
    fn close_rejects_foreign_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, Some("master"));
        let attacker_pid = 993_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        // 공격: attacker pane이 자기 소유가 아닌 victim(master) surface를 강제 종료 시도.
        let resp = close_surface_rpc(&daemon, victim, Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 surface에 대한 close가 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("close_denied"));
        // victim surface가 여전히 살아 있어야 한다 (kill·맵 제거가 일어나지 않아야 함).
        assert!(
            daemon.surfaces.lock().unwrap().contains_key(&victim),
            "거부됐는데 victim surface가 닫혔다 (맵에서 제거됨)"
        );
        // master 역할 매핑도 보존돼야 한다 (close_surface의 roles 정리 미발생).
        assert_eq!(
            daemon.roles.lock().unwrap().get("master").copied(),
            Some(victim),
            "거부됐는데 victim의 role 매핑이 정리됐다"
        );
    }

    /// 대조군 ①: 자기 surface close는 통과 (cs == sid). 정상 종료 경로 박제.
    #[test]
    fn close_allows_self() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));
        let own_pid = 993_201_u32;
        bind_caller(&daemon, own_pid, own);

        let resp = close_surface_rpc(&daemon, own, Some(own_pid));
        assert_eq!(resp["ok"], json!(true), "자기 surface close가 막혔다 (응답: {resp})");
        assert!(
            !daemon.surfaces.lock().unwrap().contains_key(&own),
            "자기 close가 통과했는데 surface가 맵에 남아 있다"
        );
    }

    /// 대조군 ②: 익명 발신(caller_pid None = 데몬 내부 node-recover·오케스트레이터 경로)은 통과.
    /// (pane은 커널 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다 = 안전.)
    #[test]
    fn close_allows_anonymous() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-3"));

        let resp = close_surface_rpc(&daemon, node, None);
        assert_eq!(
            resp["ok"], json!(true),
            "익명(데몬 내부) close가 막혔다 (응답: {resp})"
        );
        assert!(!daemon.surfaces.lock().unwrap().contains_key(&node));
    }

    fn queue_clear_rpc(daemon: &Arc<Daemon>, surface_id: u64, caller_pid: Option<u32>) -> Value {
        let req = Request {
            id: json!(1),
            method: "queue.clear".into(),
            params: json!({ "surface_id": surface_id }),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("expected single reply");
        };
        resp
    }

    /// 발견(신원·소유 검증 부재): queue.clear가 caller_pid를 전혀 쓰지 않아, 워커 pane이 자기 소유가
    /// 아닌 타 surface의 pending_queue를 통째로 drain할 수 있었다. 큐는 제3자가 --queued로 보낸
    /// (queued:true 응답까지 받은) 인플라이트 메시지를 담으므로, 인멸은 send ACL이 막은 대상을 큐
    /// 인멸로 조용히 방해하는 우회가 된다. 발신 pane은 자기 surface(cs == sid) 큐만 비울 수 있어야
    /// 한다 — 이 게이트를 박제한다.
    #[test]
    fn queue_clear_rejects_foreign_surface() {
        let daemon = claim_daemon();
        let attacker = make_surface(&daemon, Some("worker-1"));
        let victim = make_surface(&daemon, Some("master"));
        let attacker_pid = 994_101_u32;
        bind_caller(&daemon, attacker_pid, attacker);

        // 피해 노드에 제3자가 보낸 인플라이트 큐 메시지 2건.
        {
            let victim_surface = daemon.surfaces.lock().unwrap()[&victim].clone();
            let mut q = victim_surface.pending_queue.lock().unwrap();
            q.push_back("진짜 메시지 1".into());
            q.push_back("진짜 메시지 2".into());
        }

        // 공격: attacker pane이 victim의 큐를 인멸 시도.
        let resp = queue_clear_rpc(&daemon, victim, Some(attacker_pid));
        assert_eq!(
            resp["ok"], json!(false),
            "타 surface 큐 인멸이 통과했다 (응답: {resp})"
        );
        assert_eq!(resp["error"]["code"], json!("clear_denied"));
        // victim 큐가 보존돼야 한다 (drain 미발생).
        assert_eq!(
            daemon.surfaces.lock().unwrap()[&victim].pending_queue.lock().unwrap().len(),
            2,
            "거부됐는데 victim 큐가 인멸됐다"
        );
    }

    /// 대조군 ①: 자기 surface 큐 비우기는 통과 (cs == sid). 정상 철회 경로 박제.
    #[test]
    fn queue_clear_allows_self() {
        let daemon = claim_daemon();
        let own = make_surface(&daemon, Some("worker-1"));
        let own_pid = 994_201_u32;
        bind_caller(&daemon, own_pid, own);
        daemon.surfaces.lock().unwrap()[&own]
            .pending_queue.lock().unwrap()
            .push_back("내 큐".into());

        let resp = queue_clear_rpc(&daemon, own, Some(own_pid));
        assert_eq!(resp["ok"], json!(true), "자기 큐 비우기가 막혔다 (응답: {resp})");
        assert_eq!(resp["result"]["cleared"].as_u64(), Some(1));
        assert!(
            daemon.surfaces.lock().unwrap()[&own].pending_queue.lock().unwrap().is_empty(),
            "자기 clear가 통과했는데 큐가 남아 있다"
        );
    }

    /// 대조군 ②: 익명 발신(caller_pid None = 데몬 내부 경로)은 통과해야 한다.
    /// (pane은 커널 peer pid가 항상 자기 surface로 해석되므로 익명을 위조할 수 없다 = 안전.)
    #[test]
    fn queue_clear_allows_anonymous() {
        let daemon = claim_daemon();
        let node = make_surface(&daemon, Some("worker-3"));
        daemon.surfaces.lock().unwrap()[&node]
            .pending_queue.lock().unwrap()
            .push_back("큐".into());

        let resp = queue_clear_rpc(&daemon, node, None);
        assert_eq!(
            resp["ok"], json!(true),
            "익명(데몬 내부) 큐 비우기가 막혔다 (응답: {resp})"
        );
    }

    /// 발견(torn read): surface.list·org.status가 한 json! 블록 안에서 agent_meta 락을
    /// 'agent'용·'agent_alive'용으로 각각 별도 획득하면, 두 락 사이에 동시 set_meta가 끼어
    /// 이름은 직전 값에서·presence는 새 값 기준으로 읽혀 같은 응답 안 스냅샷이 깨질 수 있다.
    /// 단일 락 1회로 (이름, presence)를 함께 읽으면 두 필드는 항상 동일 presence에서 파생되어
    /// 일관된다 — agent_meta가 Some이면 두 필드 모두 non-null, None이면 두 필드 모두 null. 박제.
    fn surface_entry<'a>(resp: &'a Value, method_key: &str, sid: u64) -> &'a Value {
        resp["result"][method_key]
            .as_array()
            .expect("result array")
            .iter()
            .find(|v| v["surface_id"].as_u64() == Some(sid))
            .expect("surface entry present")
    }

    #[test]
    fn agent_meta_snapshot_is_consistent_across_list_and_status() {
        let daemon = claim_daemon();
        // 메타 등록된 살아있는 surface 1개 + 메타 없는 surface 1개.
        let live = make_surface(&daemon, Some("worker-1"));
        let bare = make_surface(&daemon, Some("worker-2"));
        {
            let surfaces = daemon.surfaces.lock().unwrap();
            *surfaces[&live].agent_meta.lock().unwrap() =
                Some(("gemini".into(), "gemini".into()));
            surfaces[&live].agent_seen.store(true, Ordering::Relaxed);
            surfaces[&live].agent_exit_notified.store(false, Ordering::Relaxed);
        }

        for (method, key) in [("surface.list", "surfaces"), ("org.status", "surfaces")] {
            let req = Request { id: json!(1), method: method.into(), params: json!({}) };
            let Reply::Single(resp) = dispatch(&daemon, req, None) else {
                panic!("expected single reply for {method}");
            };
            assert_eq!(resp["ok"], json!(true), "{method} 실패: {resp}");

            // 메타 보유 surface: agent·agent_alive가 같은 Some presence에서 파생 → 둘 다 non-null.
            let live_e = surface_entry(&resp, key, live);
            assert_eq!(
                live_e["agent"], json!("gemini"),
                "{method}: 등록된 agent 이름이 잘못됐다: {live_e}"
            );
            assert!(
                live_e["agent_alive"].is_boolean(),
                "{method}: agent는 Some인데 agent_alive가 null이다 (torn read): {live_e}"
            );
            assert_eq!(
                live_e["agent_alive"], json!(true),
                "{method}: seen=true·notified=false인데 alive가 true가 아니다: {live_e}"
            );

            // 메타 없는 surface: 두 필드 모두 null이어야 한다 (presence 일관).
            let bare_e = surface_entry(&resp, key, bare);
            assert!(
                bare_e["agent"].is_null() && bare_e["agent_alive"].is_null(),
                "{method}: 메타 없는 surface인데 agent/agent_alive가 null이 아니다: {bare_e}"
            );
        }
    }

    /// 발견(AB-BA 데드락 — 락 순서 역전): surface.create의 master/cso 특권역할 게이트가
    /// `roles → surfaces` 순으로 두 락을 동시 보유했다(handlers.rs). 반면 코드베이스의 락 순서
    /// 규약은 `surfaces → roles`이고 close_surface(governance.rs)·claim_role(handlers.rs)은 모두
    /// surfaces를 먼저 잡는다. 커넥션마다 별도 tokio task(main.rs)라 두 RPC가 다른 워커
    /// 스레드에서 동시 실행될 수 있어, A가 roles를 쥔 채 surfaces를, B가 surfaces를 쥔 채 roles를
    /// 기다리면 std::sync::Mutex(타임아웃 없음)로 양쪽이 영구 정지 → 데몬 전체 hang.
    ///
    /// 이 테스트는 실제 dispatch(surface.create {role:master})와 실제 governance::close_surface를
    /// 배리어로 최대한 겹쳐 다수 반복 실행한다. 락 순서가 역전돼 있으면(버그) 두 스레드가 교착되어
    /// 워치독 시한 내에 끝나지 않고 → 패닉으로 빨간불. 순서가 규약(surfaces→roles)과 일치하면
    /// (수정) 어떤 인터리빙에서도 교착이 불가능해 즉시 완료된다.
    #[test]
    fn surface_create_privileged_gate_keeps_lock_order_no_deadlock() {
        use std::sync::{Arc as StdArc, Barrier};
        use std::time::{Duration, Instant};

        // 워치독: 작업을 자식 스레드로 돌리고, 시한 내 완료 신호가 없으면 교착으로 간주해 패닉.
        // (교착된 두 스레드는 누수되지만 테스트 프로세스는 명확한 실패 메시지로 종료한다.)
        let done = StdArc::new(std::sync::atomic::AtomicBool::new(false));
        let done_w = StdArc::clone(&done);

        let worker = std::thread::spawn(move || {
            let daemon = claim_daemon();
            // 살아있는 master 보유자 — create 게이트가 roles·surfaces 두 락을 모두 잡는 경로를 강제.
            let _master = make_surface(&daemon, Some("master"));

            // 매 반복: 닫을 더미 surface 하나를 미리 만들어두고, A=create(role=master) 게이트와
            // B=close(dummy)를 배리어로 동시에 출발시켜 AB-BA 윈도를 최대화한다.
            for _ in 0..200 {
                let dummy = make_surface(&daemon, Some("worker-x"));
                let barrier = StdArc::new(Barrier::new(2));

                let d_a = StdArc::clone(&daemon);
                let b_a = StdArc::clone(&barrier);
                let t_a = std::thread::spawn(move || {
                    b_a.wait();
                    // 실제 buggy 블록(handlers.rs:308-)을 타는 경로: master 탈취 시도는 거부되지만
                    // 거부 판정 전에 roles·surfaces 두 락을 동시 보유한다.
                    let _ = create_surface_rpc(&d_a, Some("master"), Some(994_401_u32));
                });

                let d_b = StdArc::clone(&daemon);
                let b_b = StdArc::clone(&barrier);
                let t_b = std::thread::spawn(move || {
                    b_b.wait();
                    // 실제 close_surface(governance.rs) 경로: surfaces → roles 순.
                    let _ = crate::governance::close_surface(&d_b, dummy);
                });

                t_a.join().unwrap();
                t_b.join().unwrap();
            }
            done_w.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // 200회 반복은 정상(수정) 코드에서 수백 ms 내 완료된다. 넉넉히 30초를 주고,
        // 그 안에 done 신호가 없으면 교착으로 단정한다.
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            done.load(std::sync::atomic::Ordering::SeqCst),
            "surface.create 특권 게이트와 close_surface가 교착됐다 — 락 순서 역전(roles→surfaces) AB-BA 데드락"
        );
        let _ = worker.join();
    }
}
