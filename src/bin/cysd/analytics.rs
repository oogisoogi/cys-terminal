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
}
