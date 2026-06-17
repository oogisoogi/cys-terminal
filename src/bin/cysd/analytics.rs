//! T7 E1-3 영속 분석 저장소 — cysd 내장 SQLite(`analytics.db`). recall.rs(transcripts.db)와
//! 별개 DB. 휘발성 in-memory `Consumption`을 재시작에도 보존한다(부트 시 최근 12h usage_records를
//! ts 순으로 리플레이해 오늘 비용·토큰·모델믹스·스파크라인 재구성). rusqlite(이미 의존).
//! 실패는 graceful — open이 None이면 영속 없이 데몬은 정상 동작(배지·실시간은 in-memory로 유지).
//! 스키마는 설계(docs/CONTROL_CENTER_DESIGN.md §2) 전체를 미리 만든다(events·messages 등은 E1-④/E3에서 사용).

use crate::state::{state_dir, Consumption, Daemon};
use rusqlite::Connection;
use std::path::Path;

const SPARK_SPAN_SECS: f64 = 43_200.0; // 12h — 부트 리플레이 창

/// analytics.db 열고 스키마 보장. 실패 시 None(graceful degrade).
pub fn open(socket_path: &Path) -> Option<Connection> {
    let path = state_dir(socket_path).join("analytics.db");
    let conn = Connection::open(&path).ok()?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS usage_records(
            id INTEGER PRIMARY KEY,
            session_id TEXT, role TEXT, agent TEXT, model TEXT,
            input_tokens INTEGER, output_tokens INTEGER,
            cache_creation INTEGER, cache_read INTEGER,
            cost_usd REAL, ts REAL);
         CREATE INDEX IF NOT EXISTS ix_usage_ts ON usage_records(ts);
         CREATE INDEX IF NOT EXISTS ix_usage_session ON usage_records(session_id);
         CREATE TABLE IF NOT EXISTS sessions(
            session_id TEXT PRIMARY KEY, role TEXT, agent TEXT, cwd TEXT,
            started_at REAL, ended_at REAL, title TEXT, summary TEXT, turn_count INTEGER);
         CREATE TABLE IF NOT EXISTS events(
            id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
            event_type TEXT, tool_name TEXT, is_skill INTEGER, skill_name TEXT, is_slash INTEGER,
            is_agent INTEGER, agent_type TEXT, agent_id TEXT,
            exit_code INTEGER, duration_ms INTEGER, ts REAL);
         CREATE INDEX IF NOT EXISTS ix_ev_skill ON events(is_skill, ts);
         CREATE INDEX IF NOT EXISTS ix_ev_agent ON events(is_agent, ts);
         CREATE TABLE IF NOT EXISTS messages(
            id INTEGER PRIMARY KEY, session_id TEXT, seq INTEGER, role TEXT,
            content TEXT, tool_name TEXT, tool_use_id TEXT, duration_ms INTEGER, ts REAL);
         CREATE TABLE IF NOT EXISTS daily_rollups(
            date TEXT PRIMARY KEY, payload TEXT, computed_at REAL);
         CREATE TABLE IF NOT EXISTS stars(session_id TEXT PRIMARY KEY, note TEXT, starred_at REAL);",
    )
    .ok()?;
    Some(conn)
}

/// usage_record 1건 적재 — 수집기가 새 claude 메시지마다 호출. 실패는 무해히 무시.
#[allow(clippy::too_many_arguments)]
pub fn record_usage(
    conn: &Connection,
    session: &str,
    agent: &str,
    model: &str,
    input: u64,
    output: u64,
    cache_creation: u64,
    cache_read: u64,
    cost: f64,
    ts: f64,
) {
    let _ = conn.execute(
        "INSERT INTO usage_records(session_id, agent, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            session, agent, model, input as i64, output as i64,
            cache_creation as i64, cache_read as i64, cost, ts
        ],
    );
}

/// 툴 호출의 파생 분류(관측 도구 deriveFields 동형) — (is_skill, skill_name, is_agent, agent_type).
/// Skill 툴 → 스킬 호출, Task/Agent 툴 → 에이전트(서브에이전트) 호출. E3 스킬/에이전트 TOP의 키.
pub fn derive_tool(tool_name: &str, tool_input: &serde_json::Value) -> (bool, Option<String>, bool, Option<String>) {
    let is_skill = tool_name == "Skill";
    let skill_name = if is_skill {
        tool_input
            .get("skill")
            .or_else(|| tool_input.get("command"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim_start_matches('/').to_string())
    } else {
        None
    };
    let is_agent = tool_name == "Task" || tool_name == "Agent";
    let agent_type = if is_agent {
        tool_input.get("subagent_type").and_then(|v| v.as_str()).map(String::from)
    } else {
        None
    };
    (is_skill, skill_name, is_agent, agent_type)
}

/// events 테이블에 hook 이벤트 1건 적재 — usage.event RPC가 호출. 실패는 무해히 무시.
#[allow(clippy::too_many_arguments)]
pub fn record_event(
    conn: &Connection,
    session: &str,
    role: &str,
    agent: &str,
    event_type: &str,
    tool_name: &str,
    is_skill: bool,
    skill_name: Option<&str>,
    is_agent: bool,
    agent_type: Option<&str>,
    agent_id: Option<&str>,
    exit_code: Option<i64>,
    ts: f64,
) {
    let _ = conn.execute(
        "INSERT INTO events(session_id, role, agent, event_type, tool_name, is_skill, skill_name,
            is_slash, is_agent, agent_type, agent_id, exit_code, ts)
         VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,?10,?11,?12)",
        rusqlite::params![
            session, role, agent, event_type, tool_name,
            is_skill as i64, skill_name, is_agent as i64, agent_type, agent_id, exit_code, ts
        ],
    );
}

/// (session, model, input_tokens, cache_creation, output, cost, ts)
type UsageRow = (String, String, u64, u64, u64, f64, f64);

/// cutoff 이후 usage_records를 ts 오름차순으로 읽는다(부트 리플레이용).
fn load_recent(conn: &Connection, cutoff: f64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, model, input_tokens, cache_creation, output_tokens, cost_usd, ts
         FROM usage_records WHERE ts >= ?1 ORDER BY ts ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// epoch초 → 로컬 "YYYY-MM-DD" (record_message의 날짜 리셋 키).
fn date_of(ts: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 리플레이: 행 목록을 ts 순으로 record_message에 흘려 Consumption을 재구성(순수 — 테스트 가능).
fn replay(rows: &[UsageRow], c: &mut Consumption) {
    for (session, model, input_tokens, cache_creation, output, cost, ts) in rows {
        // 소비 input = input_tokens + cache_creation (수집기 적재와 동일 의미).
        c.record_message(session, input_tokens + cache_creation, *output, *cost, model, *ts, &date_of(*ts));
    }
}

/// 부트 시 1회 — 최근 12h usage_records로 in-memory Consumption을 재구성한다.
pub fn seed_consumption(daemon: &Daemon) {
    let now = crate::state::now_epoch();
    let rows = {
        let guard = daemon.analytics.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => load_recent(conn, now - SPARK_SPAN_SECS),
            None => return,
        }
    };
    if rows.is_empty() {
        return;
    }
    let mut c = daemon.consumption.lock().unwrap();
    replay(&rows, &mut c);
}

// ───────────────────────── E2 비용·효율 집계 (control.analytics) ─────────────────────────

/// (agent, model, input, output, cache_creation, cache_read, cost, session, ts)
pub type SummaryRow = (String, String, u64, u64, u64, u64, f64, String, f64);

/// window 문자열 → cutoff epoch. "today"(기본·로컬 자정)·"7d"·"all".
pub fn window_since(now: f64, window: &str) -> f64 {
    use chrono::{Local, TimeZone};
    match window {
        "all" => 0.0,
        "7d" => (now - 7.0 * 86_400.0).max(0.0),
        _ => Local
            .timestamp_opt(now as i64, 0)
            .single()
            .map(|dt| dt.date_naive())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|naive| Local.from_local_datetime(&naive).single())
            .map(|dt| dt.timestamp() as f64)
            .unwrap_or((now - 86_400.0).max(0.0)),
    }
}

/// since 이후 usage_records 전 행(집계용 — agent·cache_read 포함). 실패는 빈 벡터.
fn load_summary_rows(conn: &Connection, since: f64) -> Vec<SummaryRow> {
    let mut stmt = match conn.prepare(
        "SELECT agent, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, session_id, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, String>(1).unwrap_or_default(),
            r.get::<_, i64>(2)? as u64,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, f64>(6)?,
            r.get::<_, String>(7).unwrap_or_default(),
            r.get::<_, f64>(8)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 행 목록을 토큰 4분해·모델믹스·에이전트믹스·캐시절감$·생산성으로 롤업(순수 — 테스트 가능).
/// 캐시절감$ = Σ cache_read × (input단가 − cache_read단가) — 풀 input 대비 컨텍스트 재사용 할인액.
pub fn summarize(rows: &[SummaryRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    let (mut t_in, mut t_out, mut t_cc, mut t_cr) = (0u64, 0u64, 0u64, 0u64);
    let (mut t_cost, mut savings) = (0.0f64, 0.0f64);
    let mut models: HashMap<String, [f64; 6]> = HashMap::new(); // [in,out,cc,cr,cost,msgs]
    let mut agents: HashMap<String, [f64; 3]> = HashMap::new(); // [tokens,cost,msgs]
    let mut sessions: HashMap<String, (f64, f64, u64, u64, f64)> = HashMap::new(); // (min_ts,max_ts,msgs,tokens,cost)
    for (agent, model, input, output, cc, cr, cost, session, ts) in rows {
        t_in += input;
        t_out += output;
        t_cc += cc;
        t_cr += cr;
        t_cost += cost;
        let p = crate::cost::pricing_for(model);
        savings += (*cr as f64 / 1_000_000.0) * (p.input_per_m - p.cache_read_per_m);
        let tokens = input + output + cc + cr;
        let m = models.entry(model.clone()).or_insert([0.0; 6]);
        m[0] += *input as f64;
        m[1] += *output as f64;
        m[2] += *cc as f64;
        m[3] += *cr as f64;
        m[4] += *cost;
        m[5] += 1.0;
        let akey = if agent.is_empty() { "unknown".to_string() } else { agent.clone() };
        let a = agents.entry(akey).or_insert([0.0; 3]);
        a[0] += tokens as f64;
        a[1] += *cost;
        a[2] += 1.0;
        let s = sessions.entry(session.clone()).or_insert((*ts, *ts, 0, 0, 0.0));
        s.0 = s.0.min(*ts);
        s.1 = s.1.max(*ts);
        s.2 += 1;
        s.3 += tokens;
        s.4 += *cost;
    }
    let msgs = rows.len() as f64;
    let nsess = sessions.len() as f64;
    let tokens_total = (t_in + t_out + t_cc + t_cr) as f64;
    let dur_sum: f64 = sessions.values().map(|(mn, mx, ..)| mx - mn).sum();
    let div = |num: f64, den: f64| if den > 0.0 { num / den } else { 0.0 };
    let mut by_model: Vec<Value> = models
        .into_iter()
        .map(|(model, v)| {
            json!({
                "model": model, "input": v[0] as u64, "output": v[1] as u64,
                "cache_creation": v[2] as u64, "cache_read": v[3] as u64,
                "tokens": (v[0] + v[1] + v[2] + v[3]) as u64, "cost_usd": v[4], "msgs": v[5] as u64,
            })
        })
        .collect();
    by_model.sort_by(|a, b| {
        b["cost_usd"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["model"].as_str().unwrap_or("").cmp(b["model"].as_str().unwrap_or("")))
    });
    let mut by_agent: Vec<Value> = agents
        .into_iter()
        .map(|(agent, v)| json!({"agent": agent, "tokens": v[0] as u64, "cost_usd": v[1], "msgs": v[2] as u64}))
        .collect();
    by_agent.sort_by(|a, b| {
        b["tokens"].as_u64().unwrap_or(0).cmp(&a["tokens"].as_u64().unwrap_or(0))
            .then_with(|| a["agent"].as_str().unwrap_or("").cmp(b["agent"].as_str().unwrap_or("")))
    });
    json!({
        "totals": {
            "input": t_in, "output": t_out, "cache_creation": t_cc, "cache_read": t_cr,
            "tokens": tokens_total as u64, "cost_usd": t_cost, "msgs": msgs as u64, "sessions": nsess as u64,
        },
        "cache_savings_usd": savings,
        "by_model": by_model,
        "by_agent": by_agent,
        "productivity": {
            "turns_per_session": div(msgs, nsess),
            "tokens_per_turn": div(tokens_total, msgs),
            "cost_per_session": div(t_cost, nsess),
            "avg_session_duration_secs": div(dur_sum, nsess),
        },
    })
}

/// control.analytics 본체 — since 이후 usage_records를 롤업. conn 없으면 호출부가 빈 summarize 사용.
pub fn analytics_summary(conn: &Connection, since: f64) -> serde_json::Value {
    summarize(&load_summary_rows(conn, since))
}

// ───────────────────────── E3 스킬·에이전트 집계 (control.skills) ─────────────────────────

/// (event_type, role, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, ts)
pub type EventRow = (String, String, String, bool, String, bool, String, Option<i64>, f64);

/// since 이후 툴 events 전 행. 실패는 빈 벡터.
fn load_event_rows(conn: &Connection, since: f64) -> Vec<EventRow> {
    let mut stmt = match conn.prepare(
        "SELECT event_type, role, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, ts
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(7)?,
            r.get::<_, f64>(8)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 이벤트 행을 스킬·툴·에이전트 호출 TOP과 🔥실패율로 롤업(순수 — 테스트 가능).
/// calls = PRE_TOOL(호출 시도) 기준 · fail = POST_TOOL exit_code≠0 기준(둘은 PRE/POST 쌍으로 근사 정합).
/// duration p50·미사용 4주 diff는 현재 미수집(events.duration_ms NULL·축적 필요) — 후속.
pub fn summarize_skills(rows: &[EventRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    // name → [calls, fail]; roles는 별도 맵
    let mut tools: HashMap<String, [u64; 2]> = HashMap::new();
    let mut skills: HashMap<String, [u64; 2]> = HashMap::new();
    let mut skill_roles: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut agents: HashMap<String, u64> = HashMap::new();
    let mut agent_roles: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let (mut tool_calls, mut skill_calls, mut agent_calls, mut fail_calls) = (0u64, 0u64, 0u64, 0u64);
    for (etype, role, tool, is_skill, skill, is_agent, atype, exit, _ts) in rows {
        let role_key = if role.is_empty() { "?".to_string() } else { role.clone() };
        if etype == "PRE_TOOL" {
            if !tool.is_empty() {
                tools.entry(tool.clone()).or_insert([0, 0])[0] += 1;
                tool_calls += 1;
            }
            if *is_skill && !skill.is_empty() {
                skills.entry(skill.clone()).or_insert([0, 0])[0] += 1;
                *skill_roles.entry(skill.clone()).or_default().entry(role_key.clone()).or_insert(0) += 1;
                skill_calls += 1;
            }
            if *is_agent && !atype.is_empty() {
                *agents.entry(atype.clone()).or_insert(0) += 1;
                *agent_roles.entry(atype.clone()).or_default().entry(role_key).or_insert(0) += 1;
                agent_calls += 1;
            }
        } else if etype == "POST_TOOL" && matches!(exit, Some(c) if *c != 0) {
            if !tool.is_empty() {
                tools.entry(tool.clone()).or_insert([0, 0])[1] += 1;
            }
            if *is_skill && !skill.is_empty() {
                skills.entry(skill.clone()).or_insert([0, 0])[1] += 1;
            }
            fail_calls += 1;
        }
    }
    let rate = |fail: u64, calls: u64| if calls > 0 { fail as f64 / calls as f64 } else { 0.0 };
    let roles_val = |m: Option<&HashMap<String, u64>>| -> Value {
        match m {
            Some(rm) => {
                let mut v: Vec<Value> = rm.iter().map(|(r, c)| json!({"role": r, "count": c})).collect();
                v.sort_by(|a, b| b["count"].as_u64().unwrap_or(0).cmp(&a["count"].as_u64().unwrap_or(0)));
                Value::Array(v)
            }
            None => Value::Array(vec![]),
        }
    };
    // calls desc, 동률은 이름 asc (결정론)
    let sort_by_calls = |list: &mut Vec<Value>| {
        list.sort_by(|a, b| {
            b["calls"].as_u64().unwrap_or(0).cmp(&a["calls"].as_u64().unwrap_or(0))
                .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
        });
    };
    let mut by_skill: Vec<Value> = skills
        .iter()
        .map(|(name, v)| json!({
            "name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0]),
            "roles": roles_val(skill_roles.get(name)),
        }))
        .collect();
    sort_by_calls(&mut by_skill);
    let mut by_tool: Vec<Value> = tools
        .iter()
        .map(|(name, v)| json!({"name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0])}))
        .collect();
    sort_by_calls(&mut by_tool);
    let mut by_agent: Vec<Value> = agents
        .iter()
        .map(|(name, c)| json!({"name": name, "calls": c, "by_role": roles_val(agent_roles.get(name))}))
        .collect();
    sort_by_calls(&mut by_agent);
    // 🔥반복실패 TOP — 툴 단위 fail>0, fail desc(관측 도구 미구현 선점)
    let mut failures: Vec<Value> = tools
        .iter()
        .filter(|(_, v)| v[1] > 0)
        .map(|(name, v)| json!({"name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0])}))
        .collect();
    failures.sort_by(|a, b| {
        b["fail"].as_u64().unwrap_or(0).cmp(&a["fail"].as_u64().unwrap_or(0))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });
    json!({
        "totals": {
            "tool_calls": tool_calls, "skill_calls": skill_calls,
            "agent_calls": agent_calls, "fail_calls": fail_calls, "fail_rate": rate(fail_calls, tool_calls),
        },
        "by_skill": by_skill,
        "by_tool": by_tool,
        "by_agent": by_agent,
        "failures": failures,
    })
}

/// control.skills 본체 — since 이후 events를 롤업. conn 없으면 호출부가 빈 summarize_skills 사용.
pub fn skills_summary(conn: &Connection, since: f64) -> serde_json::Value {
    summarize_skills(&load_event_rows(conn, since))
}

// ───────────────────────── E4 세션 타임라인 (control.sessions / session_detail) ─────────────────────────

const RIBBON_BUCKETS: usize = 24; // 활동 리본 칸 수

/// 세션 집계용 usage 행 — (session, agent, tokens(4분해합), cost, ts)
type SessUsageRow = (String, String, u64, f64, f64);
/// 세션 집계용 event 행 — (session, role, tool, is_skill, skill_name, is_agent, exit_code, event_type, ts)
type SessEventRow = (String, String, String, bool, String, bool, Option<i64>, String, f64);

fn load_session_usage(conn: &Connection, since: f64) -> Vec<SessUsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, agent, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            (r.get::<_, i64>(2)? + r.get::<_, i64>(3)? + r.get::<_, i64>(4)? + r.get::<_, i64>(5)?) as u64,
            r.get::<_, f64>(6)?,
            r.get::<_, f64>(7)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn load_session_events(conn: &Connection, since: f64) -> Vec<SessEventRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, role, tool_name, is_skill, skill_name, is_agent, exit_code, event_type, ts
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| -> rusqlite::Result<SessEventRow> {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, String>(7).unwrap_or_default(),
            r.get::<_, f64>(8)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 활동 리본 — [start,end]를 buckets칸으로 나눠 각 칸의 활동 수를 센다(순수).
fn ribbon(ts_list: &[f64], start: f64, end: f64, buckets: usize) -> Vec<u64> {
    let mut out = vec![0u64; buckets];
    let span = (end - start).max(1e-9);
    for &t in ts_list {
        let mut idx = (((t - start) / span) * buckets as f64) as isize;
        if idx < 0 {
            idx = 0;
        }
        if idx as usize >= buckets {
            idx = buckets as isize - 1;
        }
        out[idx as usize] += 1;
    }
    out
}

/// usage+event 행을 세션 단위로 병합·요약(순수·테스트). ended_at 내림차순(최신 먼저).
pub fn summarize_sessions(
    usage: &[SessUsageRow],
    events: &[SessEventRow],
    starred: &std::collections::HashSet<String>,
) -> serde_json::Value {
    use serde_json::json;
    use std::collections::HashMap;
    struct Agg {
        agent: String,
        role: String,
        tokens: u64,
        cost: f64,
        msgs: u64,
        tool_calls: u64,
        skill_calls: u64,
        agent_calls: u64,
        fail_calls: u64,
        min_ts: f64,
        max_ts: f64,
        ts_list: Vec<f64>,
        skills: HashMap<String, u64>,
    }
    let mut m: HashMap<String, Agg> = HashMap::new();
    let ensure = |m: &mut HashMap<String, Agg>, sid: &str, ts: f64| {
        m.entry(sid.to_string()).or_insert_with(|| Agg {
            agent: String::new(), role: String::new(), tokens: 0, cost: 0.0, msgs: 0,
            tool_calls: 0, skill_calls: 0, agent_calls: 0, fail_calls: 0,
            min_ts: ts, max_ts: ts, ts_list: Vec::new(), skills: HashMap::new(),
        });
    };
    for (sid, agent, tokens, cost, ts) in usage {
        ensure(&mut m, sid, *ts);
        let a = m.get_mut(sid).unwrap();
        if a.agent.is_empty() && !agent.is_empty() {
            a.agent = agent.clone();
        }
        a.tokens += tokens;
        a.cost += cost;
        a.msgs += 1;
        a.min_ts = a.min_ts.min(*ts);
        a.max_ts = a.max_ts.max(*ts);
        a.ts_list.push(*ts);
    }
    for (sid, role, _tool, is_skill, skill, is_agent, exit, etype, ts) in events {
        ensure(&mut m, sid, *ts);
        let a = m.get_mut(sid).unwrap();
        if a.role.is_empty() && !role.is_empty() {
            a.role = role.clone();
        }
        a.min_ts = a.min_ts.min(*ts);
        a.max_ts = a.max_ts.max(*ts);
        // PRE_TOOL/POST_TOOL 둘 다 ts_list엔 활동으로 — 리본은 활동 밀도이므로 무방.
        a.ts_list.push(*ts);
        // 호출 수는 PRE_TOOL(실제 호출 시도)만 — POST 중복 카운트 방지(control.skills와 일관).
        if etype == "PRE_TOOL" {
            a.tool_calls += 1;
            if *is_skill {
                a.skill_calls += 1;
                if !skill.is_empty() {
                    *a.skills.entry(skill.clone()).or_insert(0) += 1;
                }
            }
            if *is_agent {
                a.agent_calls += 1;
            }
        }
        // 실패는 POST_TOOL exit_code≠0.
        if matches!(exit, Some(c) if *c != 0) {
            a.fail_calls += 1;
        }
    }
    let mut sessions: Vec<serde_json::Value> = m
        .into_iter()
        .map(|(sid, a)| {
            let top_skill = a
                .skills
                .iter()
                .max_by(|x, y| x.1.cmp(y.1).then_with(|| y.0.cmp(x.0)))
                .map(|(k, _)| k.clone());
            json!({
                "session_id": sid,
                "agent": a.agent,
                "role": a.role,
                "started_at": a.min_ts,
                "ended_at": a.max_ts,
                "duration_secs": (a.max_ts - a.min_ts).max(0.0),
                "msgs": a.msgs,
                "tokens": a.tokens,
                "cost_usd": a.cost,
                "tool_activity": a.tool_calls,
                "skill_calls": a.skill_calls,
                "agent_calls": a.agent_calls,
                "fail_calls": a.fail_calls,
                "top_skill": top_skill,
                "ribbon": ribbon(&a.ts_list, a.min_ts, a.max_ts, RIBBON_BUCKETS),
                "starred": starred.contains(&sid),
            })
        })
        .collect();
    sessions.sort_by(|a, b| {
        b["ended_at"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["ended_at"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["session_id"].as_str().unwrap_or("").cmp(b["session_id"].as_str().unwrap_or("")))
    });
    json!({ "sessions": sessions })
}

/// ⭐ 즐겨찾기 세션 집합.
pub fn starred_set(conn: &Connection) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT session_id FROM stars") {
        if let Ok(it) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            out.extend(it.filter_map(|x| x.ok()));
        }
    }
    out
}

/// ⭐ 토글 — starred=true면 upsert, false면 삭제. 실패는 무해히 무시.
pub fn set_star(conn: &Connection, session_id: &str, starred: bool, note: &str, ts: f64) {
    if starred {
        let _ = conn.execute(
            "INSERT INTO stars(session_id, note, starred_at) VALUES(?1,?2,?3)
             ON CONFLICT(session_id) DO UPDATE SET note=?2",
            rusqlite::params![session_id, note, ts],
        );
    } else {
        let _ = conn.execute("DELETE FROM stars WHERE session_id = ?1", rusqlite::params![session_id]);
    }
}

/// control.sessions 본체 — since 이후 세션 목록.
pub fn session_list(conn: &Connection, since: f64) -> serde_json::Value {
    let usage = load_session_usage(conn, since);
    let events = load_session_events(conn, since);
    summarize_sessions(&usage, &events, &starred_set(conn))
}

/// control.session_detail 본체 — 단일 세션의 이벤트 타임라인 + 토큰/비용/모델 요약.
/// ★전사 원문(HUMAN/ASSISTANT/TOOL 콘텐츠)은 미수집(messages 테이블 미적재) — 이벤트 타임라인으로 대체.
pub fn session_detail(conn: &Connection, session_id: &str) -> serde_json::Value {
    use serde_json::json;
    // 이벤트 타임라인 (ts 오름차순·원시 컬럼 그대로)
    let mut timeline: Vec<serde_json::Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT ts, event_type, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, role
         FROM events WHERE session_id = ?1 ORDER BY ts ASC LIMIT 2000",
    ) {
        if let Ok(it) = stmt.query_map(rusqlite::params![session_id], |r| {
            Ok(json!({
                "ts": r.get::<_, f64>(0)?,
                "event_type": r.get::<_, String>(1).unwrap_or_default(),
                "tool_name": r.get::<_, Option<String>>(2)?,
                "is_skill": r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                "skill_name": r.get::<_, Option<String>>(4)?,
                "is_agent": r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                "agent_type": r.get::<_, Option<String>>(6)?,
                "exit_code": r.get::<_, Option<i64>>(7)?,
                "role": r.get::<_, Option<String>>(8)?,
            }))
        }) {
            timeline.extend(it.filter_map(|x| x.ok()));
        }
    }
    // 토큰/비용/모델 요약 — 해당 세션 usage_records를 summarize 재사용
    let urows = load_summary_rows_for_session(conn, session_id);
    let summary = summarize(&urows);
    json!({
        "session_id": session_id,
        "timeline": timeline,
        "summary": summary,
    })
}

/// 단일 세션의 usage_records를 summarize 입력 형태로 로드.
fn load_summary_rows_for_session(conn: &Connection, session_id: &str) -> Vec<SummaryRow> {
    let mut stmt = match conn.prepare(
        "SELECT agent, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, session_id, ts
         FROM usage_records WHERE session_id = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![session_id], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, String>(1).unwrap_or_default(),
            r.get::<_, i64>(2)? as u64,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, f64>(6)?,
            r.get::<_, String>(7).unwrap_or_default(),
            r.get::<_, f64>(8)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

// ───────────────────────── E5 추세·주간 다이제스트 (control.weekly) ─────────────────────────

const WEEK_SECS: f64 = 7.0 * 86_400.0;

/// 주간 집계용 — (session, tokens, cost, ts)
type WeeklyUsageRow = (String, u64, f64, f64);
/// 주간 집계용 — (session, role, is_skill, skill, is_agent, event_type, ts)
type WeeklyEventRow = (String, String, bool, String, bool, String, f64);

fn load_weekly_usage(conn: &Connection, since: f64) -> Vec<WeeklyUsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            (r.get::<_, i64>(1)? + r.get::<_, i64>(2)? + r.get::<_, i64>(3)? + r.get::<_, i64>(4)?) as u64,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn load_weekly_events(conn: &Connection, since: f64) -> Vec<WeeklyEventRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, role, is_skill, skill_name, is_agent, event_type, ts
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
            r.get::<_, String>(5).unwrap_or_default(),
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 이번주(now-7d..now) vs 지난주(now-14d..now-7d)를 WoW·일별오버레이·효율리더·스킬자산으로 롤업(순수).
/// 토큰/비용은 session→role 귀속(events의 역할)으로 노드별 리더 산출. delta_pct: 지난주 0이면 null.
pub fn summarize_weekly(now: f64, usage: &[WeeklyUsageRow], events: &[WeeklyEventRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::{HashMap, HashSet};
    let this_start = now - WEEK_SECS;
    let last_start = now - 2.0 * WEEK_SECS;
    let in_this = |ts: f64| ts >= this_start;
    let in_last = |ts: f64| (last_start..this_start).contains(&ts);

    // session → role (events에서 첫 비어있지 않은 역할)
    let mut sess_role: HashMap<String, String> = HashMap::new();
    for (sid, role, _, _, _, _, _) in events {
        if !role.is_empty() {
            sess_role.entry(sid.clone()).or_insert_with(|| role.clone());
        }
    }
    let role_of = |sid: &str| sess_role.get(sid).cloned().unwrap_or_else(|| "?".to_string());

    // WoW 글로벌 + 일별 오버레이(각 주 7칸) + 세션 집합
    let (mut t_tok, mut t_cost, mut t_msgs) = (0u64, 0.0f64, 0u64);
    let (mut l_tok, mut l_cost, mut l_msgs) = (0u64, 0.0f64, 0u64);
    let mut t_sess: HashSet<&str> = HashSet::new();
    let mut l_sess: HashSet<&str> = HashSet::new();
    let mut this_daily = vec![0u64; 7];
    let mut last_daily = vec![0u64; 7];
    // 역할별 토큰/비용/세션(이번주)
    let mut role_tok: HashMap<String, u64> = HashMap::new();
    let mut role_cost: HashMap<String, f64> = HashMap::new();
    let mut role_sess: HashMap<String, HashSet<String>> = HashMap::new();
    for (sid, tokens, cost, ts) in usage {
        if in_this(*ts) {
            t_tok += tokens;
            t_cost += cost;
            t_msgs += 1;
            t_sess.insert(sid);
            let d = (((*ts - this_start) / 86_400.0) as usize).min(6);
            this_daily[d] += tokens;
            let r = role_of(sid);
            *role_tok.entry(r.clone()).or_insert(0) += tokens;
            *role_cost.entry(r.clone()).or_insert(0.0) += cost;
            role_sess.entry(r).or_default().insert(sid.clone());
        } else if in_last(*ts) {
            l_tok += tokens;
            l_cost += cost;
            l_msgs += 1;
            l_sess.insert(sid);
            let d = (((*ts - last_start) / 86_400.0) as usize).min(6);
            last_daily[d] += tokens;
        }
    }

    // 역할별 활동(이번주) + 스킬 집합(이번주/지난주). 실패율은 E3(control.skills)·E6 담당이라 주간 리더엔 미포함.
    let mut role_skilldiv: HashMap<String, HashSet<String>> = HashMap::new();
    let mut role_tool_calls: HashMap<String, u64> = HashMap::new();
    let mut this_skills: HashMap<String, u64> = HashMap::new();
    let mut last_skills: HashSet<String> = HashSet::new();
    for (sid, role, is_skill, skill, _is_agent, etype, ts) in events {
        let r = if role.is_empty() { role_of(sid) } else { role.clone() };
        if in_this(*ts) {
            if etype == "PRE_TOOL" {
                *role_tool_calls.entry(r.clone()).or_insert(0) += 1;
                if *is_skill && !skill.is_empty() {
                    role_skilldiv.entry(r.clone()).or_default().insert(skill.clone());
                    *this_skills.entry(skill.clone()).or_insert(0) += 1;
                }
            }
        } else if in_last(*ts) && *is_skill && !skill.is_empty() && etype == "PRE_TOOL" {
            last_skills.insert(skill.clone());
        }
    }

    let delta = |t: f64, l: f64| -> Value {
        if l > 0.0 {
            json!(((t - l) / l * 100.0 * 10.0).round() / 10.0)
        } else {
            Value::Null
        }
    };
    let wow = json!({
        "tokens": {"this": t_tok, "last": l_tok, "delta_pct": delta(t_tok as f64, l_tok as f64)},
        "cost":   {"this": t_cost, "last": l_cost, "delta_pct": delta(t_cost, l_cost)},
        "sessions": {"this": t_sess.len(), "last": l_sess.len(), "delta_pct": delta(t_sess.len() as f64, l_sess.len() as f64)},
        "msgs":   {"this": t_msgs, "last": l_msgs, "delta_pct": delta(t_msgs as f64, l_msgs as f64)},
    });

    // 효율 리더: 역할별 토큰·세션·스킬다양성·간결도(토큰/턴)·실패. 토큰 desc 정렬.
    let mut roles: HashSet<String> = HashSet::new();
    roles.extend(role_tok.keys().cloned());
    roles.extend(role_tool_calls.keys().cloned());
    let mut leaders: Vec<Value> = roles
        .into_iter()
        .map(|r| {
            let tok = *role_tok.get(&r).unwrap_or(&0);
            let cost = *role_cost.get(&r).unwrap_or(&0.0);
            let calls = *role_tool_calls.get(&r).unwrap_or(&0);
            let sess = role_sess.get(&r).map(|s| s.len()).unwrap_or(0);
            let div = role_skilldiv.get(&r).map(|s| s.len()).unwrap_or(0);
            json!({
                "role": r, "tokens": tok, "cost_usd": cost, "sessions": sess, "tool_calls": calls,
                "skill_diversity": div,
                "tokens_per_session": if sess > 0 { tok / sess as u64 } else { 0 },
            })
        })
        .collect();
    leaders.sort_by(|a, b| {
        b["tokens"].as_u64().unwrap_or(0).cmp(&a["tokens"].as_u64().unwrap_or(0))
            .then_with(|| a["role"].as_str().unwrap_or("").cmp(b["role"].as_str().unwrap_or("")))
    });

    // 스킬 자산: 신규(이번주만)·휴면(지난주만)·최다(이번주 호출 TOP)
    let this_set: HashSet<&String> = this_skills.keys().collect();
    let mut new_skills: Vec<String> = this_set.iter().filter(|s| !last_skills.contains(**s)).map(|s| (*s).clone()).collect();
    new_skills.sort();
    let mut dormant: Vec<String> = last_skills.iter().filter(|s| !this_set.contains(s)).cloned().collect();
    dormant.sort();
    let mut top: Vec<Value> = this_skills.iter().map(|(k, v)| json!({"name": k, "calls": v})).collect();
    top.sort_by(|a, b| {
        b["calls"].as_u64().unwrap_or(0).cmp(&a["calls"].as_u64().unwrap_or(0))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });

    json!({
        "wow": wow,
        "daily": {"this": this_daily, "last": last_daily},
        "leaders": leaders,
        "skill_asset": {"new": new_skills, "dormant": dormant, "top": top},
    })
}

/// control.weekly 본체 — 최근 14일 usage_records/events로 주간 다이제스트 집계.
pub fn weekly_summary(conn: &Connection, now: f64) -> serde_json::Value {
    let since = now - 2.0 * WEEK_SECS;
    summarize_weekly(now, &load_weekly_usage(conn, since), &load_weekly_events(conn, since))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_load_replay_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cys-analytics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("a-{}.db", line!()))).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_records(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
             model TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_creation INTEGER,
             cache_read INTEGER, cost_usd REAL, ts REAL);",
        )
        .unwrap();
        let now = 2_000_000.0;
        record_usage(&conn, "/s/a.jsonl", "claude", "claude-opus-4-8", 1000, 300, 2000, 50000, 0.42, now - 100.0);
        record_usage(&conn, "/s/a.jsonl", "claude", "claude-opus-4-8", 500, 200, 0, 0, 0.01, now);
        // 12h 밖 — 제외돼야
        record_usage(&conn, "/s/old.jsonl", "claude", "claude-haiku-4-5", 10, 10, 0, 0, 0.5, now - 50_000.0);

        let rows = load_recent(&conn, now - SPARK_SPAN_SECS);
        assert_eq!(rows.len(), 2, "12h 안쪽 2건만(오래된 1건 제외)");

        let mut c = Consumption::default();
        replay(&rows, &mut c);
        assert_eq!(c.today_msgs, 2);
        assert_eq!(c.today_input, (1000 + 2000) + 500, "input+cache_creation 합");
        assert_eq!(c.today_tokens, (1000 + 2000 + 300) + (500 + 200));
        assert!((c.today_cost_usd - 0.43).abs() < 1e-9, "비용 보존 0.42+0.01");
        assert_eq!(c.model_tokens.get("claude-opus-4-8").copied(), Some((1000 + 2000 + 300) + (500 + 200)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_and_record_event() {
        use serde_json::json;
        let (is_s, sn, is_a, at) = derive_tool("Skill", &json!({"skill": "commit"}));
        assert!(is_s && sn.as_deref() == Some("commit") && !is_a && at.is_none());
        let (is_s, _, is_a, at) = derive_tool("Task", &json!({"subagent_type": "Explore"}));
        assert!(!is_s && is_a && at.as_deref() == Some("Explore"));
        let (is_s, _, is_a, _) = derive_tool("Bash", &json!({"command": "ls"}));
        assert!(!is_s && !is_a, "일반 툴은 skill/agent 아님");
        let (_, sn, _, _) = derive_tool("Skill", &json!({"command": "/deep-research"}));
        assert_eq!(sn.as_deref(), Some("deep-research"), "/slash 접두 제거");

        let dir = std::env::temp_dir().join(format!("cys-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("e-{}.db", line!()))).unwrap();
        conn.execute_batch(
            "CREATE TABLE events(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
             event_type TEXT, tool_name TEXT, is_skill INTEGER, skill_name TEXT, is_slash INTEGER,
             is_agent INTEGER, agent_type TEXT, agent_id TEXT, exit_code INTEGER, duration_ms INTEGER, ts REAL);",
        )
        .unwrap();
        record_event(&conn, "/s/a", "worker", "claude", "PRE_TOOL", "Skill", true, Some("commit"), false, None, None, None, 1000.0);
        record_event(&conn, "/s/a", "worker", "claude", "POST_TOOL", "Bash", false, None, false, None, None, Some(1), 1001.0);
        let skills: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE is_skill=1", [], |r| r.get(0)).unwrap();
        assert_eq!(skills, 1, "스킬 호출 1건");
        let fails: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE exit_code!=0", [], |r| r.get(0)).unwrap();
        assert_eq!(fails, 1, "실패(exit!=0) 1건 — E3 반복실패 토대");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarize_costs_and_productivity() {
        // 세션 A: opus 2메시지(캐시 read 50000), 세션 B: haiku 1메시지. agent=claude/codex.
        let rows: Vec<SummaryRow> = vec![
            ("claude".into(), "claude-opus-4-8".into(), 1000, 300, 2000, 50000, 0.05, "/s/a".into(), 1000.0),
            ("claude".into(), "claude-opus-4-8".into(), 500, 200, 0, 0, 0.01, "/s/a".into(), 1100.0),
            ("codex".into(), "claude-haiku-4-5".into(), 100, 50, 0, 0, 0.00035, "/s/b".into(), 1050.0),
        ];
        let s = summarize(&rows);
        let t = &s["totals"];
        assert_eq!(t["input"], 1600, "input 합 1000+500+100");
        assert_eq!(t["cache_read"], 50000);
        assert_eq!(t["msgs"], 3);
        assert_eq!(t["sessions"], 2, "세션 A·B");
        // 토큰 4분해 합 = 1600 + 550(out) + 2000(cc) + 50000(cr) = 54150
        assert_eq!(t["tokens"], 54150u64);
        // 캐시절감$ = 50000/1e6 × (opus input 5 − cache_read 0.5) = 0.05 × 4.5 = 0.225
        assert!((s["cache_savings_usd"].as_f64().unwrap() - 0.225).abs() < 1e-9, "{}", s["cache_savings_usd"]);
        // by_model: opus가 비용 우선 정렬 1위
        assert_eq!(s["by_model"][0]["model"], "claude-opus-4-8");
        assert_eq!(s["by_model"][0]["msgs"], 2);
        // 생산성: 턴/세션 = 3/2 = 1.5, 비용/세션 = (0.06035)/2
        let prod = &s["productivity"];
        assert!((prod["turns_per_session"].as_f64().unwrap() - 1.5).abs() < 1e-9);
        // 세션 A duration = 1100-1000 = 100, B = 0 → 평균 50
        assert!((prod["avg_session_duration_secs"].as_f64().unwrap() - 50.0).abs() < 1e-9);
        // 빈 입력 = 0 division 안전
        let empty = summarize(&[]);
        assert_eq!(empty["totals"]["msgs"], 0);
        assert_eq!(empty["productivity"]["tokens_per_turn"], 0.0);
    }

    #[test]
    fn summarize_skills_calls_and_failrate() {
        // Bash 2호출 1실패, Skill(commit) 1호출 PRE+POST 성공, Task→Explore 위임 1.
        let ev = |t: &str, role: &str, tool: &str, sk: bool, skn: &str, ag: bool, at: &str, ex: Option<i64>| -> EventRow {
            (t.into(), role.into(), tool.into(), sk, skn.into(), ag, at.into(), ex, 1000.0)
        };
        let rows: Vec<EventRow> = vec![
            ev("PRE_TOOL", "worker", "Bash", false, "", false, "", None),
            ev("POST_TOOL", "worker", "Bash", false, "", false, "", Some(1)), // 실패
            ev("PRE_TOOL", "worker", "Bash", false, "", false, "", None),
            ev("POST_TOOL", "worker", "Bash", false, "", false, "", Some(0)), // 성공
            ev("PRE_TOOL", "master", "Skill", true, "commit", false, "", None),
            ev("POST_TOOL", "master", "Skill", true, "commit", false, "", Some(0)),
            ev("PRE_TOOL", "master", "Task", false, "", true, "Explore", None),
        ];
        let s = summarize_skills(&rows);
        let t = &s["totals"];
        assert_eq!(t["tool_calls"], 4, "PRE_TOOL: Bash2+Skill1+Task1");
        assert_eq!(t["skill_calls"], 1);
        assert_eq!(t["agent_calls"], 1);
        assert_eq!(t["fail_calls"], 1, "POST exit≠0 1건");
        assert!((t["fail_rate"].as_f64().unwrap() - 0.25).abs() < 1e-9, "1/4");
        // by_tool: Bash 1위(calls 2), fail 1, fail_rate 0.5
        assert_eq!(s["by_tool"][0]["name"], "Bash");
        assert_eq!(s["by_tool"][0]["calls"], 2);
        assert!((s["by_tool"][0]["fail_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        // by_skill: commit, 실패 0
        assert_eq!(s["by_skill"][0]["name"], "commit");
        assert_eq!(s["by_skill"][0]["fail"], 0);
        assert_eq!(s["by_skill"][0]["roles"][0]["role"], "master");
        // by_agent: Explore 위임
        assert_eq!(s["by_agent"][0]["name"], "Explore");
        assert_eq!(s["by_agent"][0]["by_role"][0]["role"], "master");
        // 🔥failures: Bash만(fail>0)
        assert_eq!(s["failures"].as_array().unwrap().len(), 1);
        assert_eq!(s["failures"][0]["name"], "Bash");
        // 빈 입력 안전
        let empty = summarize_skills(&[]);
        assert_eq!(empty["totals"]["tool_calls"], 0);
        assert_eq!(empty["totals"]["fail_rate"], 0.0);
    }

    #[test]
    fn ribbon_buckets_activity() {
        // [0,10] 10칸 — t=0→칸0, t=10(끝)→마지막 칸, t=5→중간
        let r = ribbon(&[0.0, 5.0, 10.0], 0.0, 10.0, 10);
        assert_eq!(r.len(), 10);
        assert_eq!(r[0], 1);
        assert_eq!(r[5], 1);
        assert_eq!(r[9], 1, "끝 ts는 마지막 칸으로 클램프");
        // span 0(단일 시점) 안전
        let r2 = ribbon(&[3.0, 3.0], 3.0, 3.0, 4);
        assert_eq!(r2.iter().sum::<u64>(), 2);
    }

    #[test]
    fn summarize_sessions_merges_usage_and_events() {
        let usage: Vec<SessUsageRow> = vec![
            ("/s/a".into(), "claude".into(), 1000, 0.05, 1000.0),
            ("/s/a".into(), "claude".into(), 500, 0.01, 1100.0),
            ("/s/b".into(), "codex".into(), 200, 0.001, 1050.0),
        ];
        let events: Vec<SessEventRow> = vec![
            ("/s/a".into(), "worker".into(), "Skill".into(), true, "commit".into(), false, None, "PRE_TOOL".into(), 1020.0),
            ("/s/a".into(), "worker".into(), "Bash".into(), false, "".into(), false, Some(1), "POST_TOOL".into(), 1040.0), // 실패
            ("/s/b".into(), "master".into(), "Task".into(), false, "".into(), true, None, "PRE_TOOL".into(), 1055.0),
        ];
        let mut starred = std::collections::HashSet::new();
        starred.insert("/s/a".to_string());
        let v = summarize_sessions(&usage, &events, &starred);
        let s = v["sessions"].as_array().unwrap();
        assert_eq!(s.len(), 2);
        // ended_at 내림차순 — /s/a(max 1100) 먼저, /s/b(max 1055)
        assert_eq!(s[0]["session_id"], "/s/a");
        assert_eq!(s[0]["agent"], "claude");
        assert_eq!(s[0]["role"], "worker");
        assert_eq!(s[0]["msgs"], 2);
        assert_eq!(s[0]["tokens"], 1500u64);
        assert!((s[0]["cost_usd"].as_f64().unwrap() - 0.06).abs() < 1e-9);
        assert_eq!(s[0]["skill_calls"], 1);
        assert_eq!(s[0]["fail_calls"], 1);
        assert_eq!(s[0]["top_skill"], "commit");
        assert_eq!(s[0]["starred"], true);
        assert!((s[0]["duration_secs"].as_f64().unwrap() - 100.0).abs() < 1e-9, "1100-1000");
        assert_eq!(s[0]["ribbon"].as_array().unwrap().len(), RIBBON_BUCKETS);
        // /s/b: codex·master·위임 1
        assert_eq!(s[1]["session_id"], "/s/b");
        assert_eq!(s[1]["agent_calls"], 1);
        assert_eq!(s[1]["starred"], false);
    }

    #[test]
    fn summarize_weekly_wow_and_assets() {
        let now = 2_000_000.0;
        let day = 86_400.0;
        // 이번주(now-1d): /s/a worker 2000토큰. 지난주(now-8d): /s/x worker 1000토큰.
        let usage: Vec<WeeklyUsageRow> = vec![
            ("/s/a".into(), 2000, 0.10, now - day),
            ("/s/x".into(), 1000, 0.04, now - 8.0 * day),
        ];
        // 이번주 스킬 commit·deep-research(신규), 지난주 commit·old-skill(→ old-skill 휴면)
        let events: Vec<WeeklyEventRow> = vec![
            ("/s/a".into(), "worker".into(), true, "commit".into(), false, "PRE_TOOL".into(), now - day),
            ("/s/a".into(), "worker".into(), true, "deep-research".into(), false, "PRE_TOOL".into(), now - day),
            ("/s/x".into(), "worker".into(), true, "commit".into(), false, "PRE_TOOL".into(), now - 8.0 * day),
            ("/s/x".into(), "worker".into(), true, "old-skill".into(), false, "PRE_TOOL".into(), now - 8.0 * day),
        ];
        let w = summarize_weekly(now, &usage, &events);
        // WoW: 토큰 2000 vs 1000 → +100%
        assert_eq!(w["wow"]["tokens"]["this"], 2000u64);
        assert_eq!(w["wow"]["tokens"]["last"], 1000u64);
        assert!((w["wow"]["tokens"]["delta_pct"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        // 일별 오버레이 7칸
        assert_eq!(w["daily"]["this"].as_array().unwrap().len(), 7);
        assert_eq!(w["daily"]["last"].as_array().unwrap().len(), 7);
        // 리더: worker, 이번주 토큰 2000·스킬다양성 2
        assert_eq!(w["leaders"][0]["role"], "worker");
        assert_eq!(w["leaders"][0]["tokens"], 2000u64);
        assert_eq!(w["leaders"][0]["skill_diversity"], 2);
        // 스킬 자산: 신규=deep-research, 휴면=old-skill, 최다 포함 commit
        let new: Vec<&str> = w["skill_asset"]["new"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(new.contains(&"deep-research") && !new.contains(&"commit"), "{:?}", new);
        let dorm: Vec<&str> = w["skill_asset"]["dormant"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(dorm, vec!["old-skill"]);
        // 지난주 0 분모 가드: 빈 입력 delta null
        let empty = summarize_weekly(now, &[], &[]);
        assert!(empty["wow"]["tokens"]["delta_pct"].is_null());
    }
}
