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
}
