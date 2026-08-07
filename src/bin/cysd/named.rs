//! 이름 있는 보고자(named reporter) — surface 없는 Claude의 ctx 관측.
//!
//! 오너 요청 2026-08-07: 「master·CSO 의 CTX 도 사이드바 페인 CTX 절에 표시. 라벨은 번호 대신
//! master·cso」.
//!
//! ★왜 새 저장소가 필요한가(실측 근거 2026-08-07): master·CSO는 cys surface가 아니라 cmux 페인의
//! Claude다. 이들의 statusline은 `cys usage-report-stdin`을 타지만, 그 함수는
//! `if let Ok(sid) = target_surface(..)` 안에서만 push한다(cys.rs). cmux 페인에는 `--surface`도
//! `CYS_SURFACE_ID`도 없으므로 **push 자체가 실행되지 않는다** — `env -u CYS_SURFACE_ID`로 재현해
//! 확인했다(사람용 줄만 나오고 RPC는 0건). 데몬의 `usage.report` 처리도 전부 surface 조회 블록
//! 안이라 sid 없이는 들어올 문이 없다.
//!
//! ⚠계정 저장소가 차 있는 것은 이 경로의 증거가 **아니다**: accounts.rs `seed_known`이 부팅 때
//! 홈의 `.claude*`·`.cys/claude*` 디렉터리를 직접 스캔해 계정 신원을 심기 때문이다.
//! 문 앞까지 온 것은 **계정 이름**이지 ctx가 아니었다. 그래서 전송로부터 새로 낸다.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 보고자 1건의 최신 관측. surface의 ObservedUsage와 같은 축을 갖되 rate는 없다 —
/// rate(5h·7d)는 계정 저장소가 이미 계정 단위로 들고 있어 여기서 또 세면 경계가 섞인다.
#[derive(Clone, Debug)]
pub struct NamedReport {
    pub ctx_pct: Option<f64>,
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub source: String,
    pub updated_at: f64,
}

#[derive(Default)]
pub struct NamedState {
    /// 이름 → 최신 관측. BTreeMap이라 노출 순서가 이름순으로 결정론이다.
    pub reports: BTreeMap<String, NamedReport>,
    /// 마지막으로 **디스크에 쓴** 시각(epoch). 0 = 이번 기동에서 아직 안 씀.
    last_persist_at: f64,
    /// 마지막으로 디스크에 쓴 (이름 → 반올림 ctx%). 변화 판정용 — 값이 안 바뀌면 안 쓴다.
    /// None = 그때 ctx가 없었다(관측은 있었으나 %가 비었다).
    last_persisted_pct: BTreeMap<String, Option<i64>>,
}

/// 디스크 기록 최소 간격(초). 값이 안 변해도 이 간격이 지나면 한 번 쓴다 —
/// 기록의 목적이 %뿐 아니라 **관측 시각**이라, 시각만 흐르는 구간에도 갱신이 필요하다.
///
/// ★왜 매 보고마다 쓰지 않는가: statusline은 Claude가 화면을 그릴 때마다 보고한다(초당 여러 번도
/// 가능). 그때마다 fsync를 동반한 atomic rename을 돌리면 관측이 관측 대상을 방해한다.
/// ★이 지연이 만드는 오차의 **방향**이 중요하다: 디스크의 updated_at은 실제보다 최대 이 간격만큼
/// **과거**다 ⇒ 복원된 행은 실제보다 「더 낡게」 보인다. 절대 「더 신선하게」 보이지 않는다.
/// 이 코드베이스가 반복해 지켜 온 규율(거짓 신선 금지)과 같은 방향이다.
const PERSIST_MIN_SECS: f64 = 60.0;

/// 상태 파일 경로 — 데몬 상태 디렉터리 관례(state_dir)를 그대로 따른다.
/// 기본 데몬 = `~/.local/state/cys/named_reporters.json`,
/// 부서 데몬 = `~/.local/state/cys-dept-<name>/named_reporters.json`(소켓 부모 디렉터리).
///
/// ★전용 사이드카를 쓰는 이유는 dept_tombstones.json과 같다(governance.rs 주석): writer가
/// 이 데몬 하나뿐인 신규 파일이라 구 바이너리가 덮어써 소실시키는 경로가 원리적으로 없다.
pub fn state_path(socket_path: &Path) -> PathBuf {
    crate::state::state_dir(socket_path).join("named_reporters.json")
}

/// 디스크에 쓸 본문. **여기 없는 필드는 디스크에 남지 않는다** — 목록이 곧 계약이다.
///
/// ⚠민감치 배제(오너 지시 2026-08-07): 이름·ctx%·출처·관측 시각만 쓴다.
/// ctx_tokens·ctx_window(토큰 수)는 **일부러 뺐다** — 이 파일은 재기동을 넘어 남는 기록이라
/// 화면이 쓰지 않는 값을 굳이 디스크에 눕힐 이유가 없다(namedCtxRows가 읽는 것은 이 넷뿐이다).
/// 그래서 복원된 행의 ctx_tokens·ctx_window는 None이다 — 「기록하지 않았다」가 정직한 상태다.
pub fn persist_body_of(state: &NamedState) -> String {
    let rows: Vec<Value> = state
        .reports
        .iter()
        .map(|(name, r)| {
            json!({
                "name": name,
                "ctx_pct": r.ctx_pct,
                "source": r.source,
                "updated_at": r.updated_at,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({"named": rows})).unwrap_or_default()
}

/// 본문을 디스크에 원자적으로 쓴다(temp → fsync → rename). note()가 참을 준 뒤에만 부른다.
///
/// ★본문을 인자로 받는 이유: 호출자가 락을 놓은 뒤에 쓰게 하기 위해서다. 락을 쥔 채 fsync를
/// 돌리면 보고 한 건이 조회 전체를 잡아 세운다(handlers.rs 호출부 주석).
///
/// 실패는 삼키되 조용하지 않다 — 다음 기동에서 행이 사라지는 결과로 이어지므로 로그를 남긴다.
pub fn write_state(socket_path: &Path, body: &str) {
    let dir = crate::state::state_dir(socket_path);
    if let Err(e) = crate::governance::write_json_atomic(&dir, "named_reporters.json", body) {
        eprintln!(
            "[cysd] named_reporters.json 기록 실패({e}) — {} · 이번 관측은 재기동을 넘지 못한다",
            state_path(socket_path).display()
        );
    }
}

/// 기본 매핑 — 경로 접두 → 보고자 이름.
///
/// ★오너가 지정한 두 개가 기본값이다. env `CYS_NAMED_REPORTERS`로 재정의할 수 있게 둔 이유는
/// 이 경로가 이 기계의 사정이기 때문이다 — 코드에 박으면 다른 기계에서 조용히 아무도 안 잡힌다.
const DEFAULT_MAP: [(&str, &str); 2] = [
    ("/Users/oogisoogi/axdev/cso", "cso"),
    ("/Users/oogisoogi/axdev", "master"),
];

/// env 형식: `<경로>=<이름>` 을 `:` 로 이어 붙인다.
/// 예) `CYS_NAMED_REPORTERS=/Users/o/axdev=master:/Users/o/axdev/cso=cso`
fn mapping() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = match std::env::var("CYS_NAMED_REPORTERS") {
        Ok(s) if !s.trim().is_empty() => s
            .split(':')
            .filter_map(|pair| {
                let (p, n) = pair.split_once('=')?;
                let (p, n) = (p.trim(), n.trim());
                if p.is_empty() || n.is_empty() {
                    return None;
                }
                Some((p.trim_end_matches('/').to_string(), n.to_string()))
            })
            .collect(),
        _ => DEFAULT_MAP
            .iter()
            .map(|(p, n)| (p.to_string(), n.to_string()))
            .collect(),
    };
    // ★긴 경로가 먼저 오게 정렬한다 — 최장 접두 일치를 위해서다.
    //   이게 없으면 `/…/axdev` 가 `/…/axdev/cso` 를 먼저 먹어 CSO 가 영원히 master로 보고된다
    //   (부모가 자식을 삼키는 고전적 접두 버그).
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    out
}

/// cwd → 보고자 이름. 판별 불가면 None.
///
/// ★None일 때 이름을 지어내지 않는다(오너 지시). 모르는 보고자를 「unknown」 같은 라벨로 띄우면
/// 사용자는 그것을 실재하는 노드로 읽는다 — 관측되지 않은 것을 화면에 만들어 내는 셈이다.
pub fn resolve_name(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let cwd = cwd.trim_end_matches('/');
    for (prefix, name) in mapping() {
        // 정확히 그 경로이거나 그 아래여야 한다. 단순 starts_with면 `/axdev-old`가
        // `/axdev`에 걸린다(형제 디렉터리 오인) — 경계 문자까지 봐야 한다.
        if cwd == prefix || cwd.starts_with(&format!("{prefix}/")) {
            return Some(name);
        }
    }
    None
}

/// 관측 적재. 이름이 판별된 경우에만 호출된다.
///
/// 반환값 = **디스크에 써야 하는가.** 판정을 호출자에게 맡기지 않고 여기서 내리는 이유는
/// 판정에 필요한 상태(직전 기록 시각·직전 기록 %)가 전부 여기 있기 때문이다 — 밖으로 내보내면
/// 두 곳이 같은 규칙을 따로 들고 있게 되고, 그 둘은 언젠가 갈라진다.
///
/// 참으로 판정하는 세 경우:
///   ⑴ 디스크에 없던 보고자다(재기동 후 첫 관측 포함) — 없던 행은 즉시 남겨야 한다.
///   ⑵ 반올림한 ctx%가 달라졌다 — 화면에 보이는 값이 바뀌었다는 뜻이다.
///   ⑶ 마지막 기록 후 PERSIST_MIN_SECS가 지났다 — 값은 그대로여도 관측 시각이 흘렀다.
#[must_use = "디스크 기록이 필요한지 알려 준다 — 무시하면 관측이 재기동을 넘지 못한다"]
pub fn note(state: &mut NamedState, name: &str, r: NamedReport) -> bool {
    let now = r.updated_at;
    let pct = r.ctx_pct.map(|p| p.round() as i64);
    state.reports.insert(name.to_string(), r);
    let due = match state.last_persisted_pct.get(name) {
        None => true,                                            // ⑴ 처음 보는 보고자
        Some(prev) => *prev != pct || now - state.last_persist_at >= PERSIST_MIN_SECS, // ⑵⑶
    };
    if due {
        // 기록은 지금 상태 **전체**를 쓰므로, 직전 기록 표도 전체를 갱신한다.
        // (A의 보고가 B의 행까지 함께 디스크로 내보내기 때문이다 — 표만 A로 갱신하면
        //  B가 안 바뀌었는데도 「바뀐 것」으로 남아 다음 보고마다 불필요한 기록이 한 번씩 더 난다.)
        state.last_persisted_pct = state
            .reports
            .iter()
            .map(|(n, rep)| (n.clone(), rep.ctx_pct.map(|p| p.round() as i64)))
            .collect();
        state.last_persist_at = now;
    }
    due
}

/// 기동 시 로드. 부재 = 정상(첫 기동), 손상 = `.corrupt-<ts>` 격리 + 빈 상태.
///
/// ★손상을 조용한 빈 상태로 흘리지 않는 이유는 dept_tombstones와 같다: 소실을 디스크에
/// 확정해 버리면 무엇이 있었는지 아무도 못 본다. 다만 세대 스냅샷까지 두지는 않는다 —
/// 이 값은 **다음 statusline 보고 한 번으로 저절로 다시 찬다**(정직한 한계).
///
/// ⚠복원된 행의 나이는 관측 시각(updated_at)으로 UI가 다시 잰다. 재기동 직후 오래된 행은
/// 자연히 stale(120초 문턱)로 표시된다 — 「살아 있는 값」으로 위장시키지 않는다.
pub fn load_from_disk(socket_path: &Path) -> NamedState {
    let p = state_path(socket_path);
    let raw = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(_) => return NamedState::default(), // 부재 = fresh install 정상
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            let ts = crate::state::now_epoch() as u64;
            let corrupt = p.with_file_name(format!("named_reporters.json.corrupt-{ts}"));
            let _ = std::fs::rename(&p, &corrupt);
            eprintln!(
                "[cysd] named_reporters.json 손상({e}) — {} isolate·빈 상태로 시작(다음 보고에 다시 찬다)",
                corrupt.display()
            );
            return NamedState::default();
        }
    };
    let mut st = NamedState::default();
    for row in v.get("named").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        let Some(name) = row.get("name").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) else {
            continue; // 이름 없는 행은 만들지 않는다(지어낸 라벨 금지 — resolve_name과 같은 규율)
        };
        let updated_at = row.get("updated_at").and_then(|x| x.as_f64()).unwrap_or(0.0);
        // ★관측 시각이 없는 행은 버린다. 0으로 살려 두면 UI(namedCtxRows)가 그 행을 그리지
        //   않으므로 무해해 보이지만, 상태에 남으면 이후 판정(직전 기록 표)에 섞인다.
        if !(updated_at > 0.0) {
            continue;
        }
        st.reports.insert(
            name.to_string(),
            NamedReport {
                ctx_pct: row.get("ctx_pct").and_then(|x| x.as_f64()),
                // 디스크에 안 남기는 값이다(persist_body 주석) — 복원본은 None이 정직하다.
                ctx_tokens: None,
                ctx_window: None,
                source: row.get("source").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                updated_at,
            },
        );
    }
    st.last_persisted_pct = st
        .reports
        .iter()
        .map(|(n, r)| (n.clone(), r.ctx_pct.map(|p| p.round() as i64)))
        .collect();
    // last_persist_at은 0으로 둔다 — 이번 기동의 첫 보고가 곧바로 기록을 한 번 내
    // 디스크의 관측 시각을 현재로 끌어올리고, 기록 경로가 살아 있음도 그때 드러난다.
    st
}

/// UI 노출용 JSON. surface 행과 같은 축(신선도·출처)을 실어 보내 같은 규율로 그릴 수 있게 한다.
pub fn to_json(state: &NamedState) -> Value {
    Value::Array(
        state
            .reports
            .iter()
            .map(|(name, r)| {
                json!({
                    "name": name,
                    "ctx_pct": r.ctx_pct,
                    "ctx_tokens": r.ctx_tokens,
                    "ctx_window": r.ctx_window,
                    "source": r.source,
                    "updated_at": r.updated_at,
                })
            })
            .collect::<Vec<_>>(),
    )
}

/// `CYS_NAMED_REPORTERS`를 읽는 경로(resolve_name)를 타는 테스트의 **직렬화 락**.
///
/// ★env는 프로세스 전역인데 cargo는 테스트를 스레드로 병렬 실행한다 — 아래 tests 모듈 머리주석의
/// 「가끔 빨간 테스트」와 같은 위험이다. 그 주석은 *한 모듈 안에서* 해결했지만, 다른 모듈
/// (handlers의 배선 회귀)도 같은 env를 읽으므로 락을 모듈 밖으로 낸다.
/// 잠금 획득은 poison을 무시한다 — 앞선 테스트가 패닉해도 뒤 테스트까지 연쇄로 죽일 이유가 없다.
#[cfg(test)]
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// ★env를 읽는 경로의 테스트는 **한 함수 안에서 순차로** 돌린다.
    ///
    /// 초판은 이것을 세 개의 #[test]로 나눴다가 간헐 적색을 만들었다 — 환경변수는 프로세스
    /// 전역인데 cargo는 테스트를 스레드로 병렬 실행하므로, 한 테스트가 set_var 한 값을
    /// 다른 테스트가 보고 넘어진다. 스케줄에 따라 통과·실패가 갈려 「가끔 빨간」 테스트가 됐다.
    /// ★가끔 빨간 테스트는 없느니만 못하다 — 사람이 빨간불을 무시하는 법을 배우게 만든다.
    #[test]
    fn resolve_name_mapping_rules() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CYS_NAMED_REPORTERS");
        // ① 기본 매핑 — 오너가 지정한 두 경로.
        assert_eq!(resolve_name("/Users/oogisoogi/axdev").as_deref(), Some("master"));
        // ★최장 접두 — cso가 master에 먹히면 안 된다(정렬이 없으면 여기서 master가 나온다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/cso").as_deref(), Some("cso"));
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/cso/sub").as_deref(), Some("cso"));
        // 하위 경로도 그 역할로 본다(세션이 하위 폴더에서 열려 있을 수 있다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/eduscan").as_deref(), Some("master"));

        // ② 판별 불가 — 라벨을 짓지 않는다.
        assert_eq!(resolve_name("/Users/oogisoogi/cys-terminal-src"), None);
        assert_eq!(resolve_name(""), None);
        // 형제 디렉터리를 접두로 오인하지 않는다(`/axdev-old`가 `/axdev`에 걸리면 안 된다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev-old"), None);

        // ③ env 재정의 — 치환이지 병합이 아니다.
        std::env::set_var("CYS_NAMED_REPORTERS", "/tmp/a=alpha:/tmp/a/b=beta");
        assert_eq!(resolve_name("/tmp/a").as_deref(), Some("alpha"));
        assert_eq!(resolve_name("/tmp/a/b").as_deref(), Some("beta"));
        assert_eq!(resolve_name("/Users/oogisoogi/axdev"), None, "재정의하면 기본값은 안 걸린다");
        std::env::remove_var("CYS_NAMED_REPORTERS");
    }

    /// 테스트용 격리 상태 디렉터리 + 그 안의 가짜 소켓 경로.
    /// state_dir은 unix에서 **소켓의 부모 디렉터리**이므로, 소켓 경로를 여기 두면
    /// named_reporters.json도 이 디렉터리에 떨어진다(실물과 같은 배치).
    #[cfg(not(windows))]
    fn drill_sock(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cys_named_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("cys.sock")
    }

    #[cfg(not(windows))]
    fn rep(pct: Option<f64>, at: f64) -> NamedReport {
        NamedReport {
            ctx_pct: pct,
            ctx_tokens: Some(22_000),
            ctx_window: Some(200_000),
            source: "statusline".into(),
            updated_at: at,
        }
    }

    /// ★티켓⑥의 요구 그 자체: 데몬이 죽었다 살아나도 마지막 관측이 남아 있어야 한다.
    /// 「재기동」은 NamedState를 버리고 load_from_disk로 새로 만드는 것으로 재현한다.
    #[cfg(not(windows))]
    #[test]
    fn survives_restart_with_its_observation_time() {
        let sock = drill_sock("restart");
        let mut st = NamedState::default();
        assert!(note(&mut st, "cso", rep(Some(7.4), 1_000.0)), "처음 보는 보고자는 즉시 기록 대상");
        write_state(&sock, &persist_body_of(&st));
        assert!(state_path(&sock).exists(), "상태 파일이 생기지 않았다");

        // ── 여기서 데몬이 죽는다. 새 프로세스가 디스크만 보고 다시 선다.
        let restored = load_from_disk(&sock);
        let v = to_json(&restored);
        assert_eq!(v.as_array().unwrap().len(), 1, "재기동 후 행이 사라졌다(티켓⑥ 결함 재현)");
        assert_eq!(v[0]["name"], "cso");
        assert_eq!(v[0]["ctx_pct"], 7.4);
        assert_eq!(v[0]["source"], "statusline");
        // ★나이를 재는 근거인 관측 시각이 함께 살아야 한다 — 없으면 UI가 그 행을 못 그린다
        //   (namedCtxRows: updated_at 없는 행은 「거짓 신선」 방지로 버린다).
        assert_eq!(v[0]["updated_at"], 1_000.0);
    }

    /// ⚠민감치 배제(오너 지시): 디스크에는 이름·ctx%·출처·시각만 남는다.
    /// ★이 단언이 없으면 나중에 누가 필드를 하나 더 실어도 아무도 모른다 — 목록이 계약이다.
    #[cfg(not(windows))]
    #[test]
    fn persisted_file_holds_no_token_counts() {
        let sock = drill_sock("nosecret");
        let mut st = NamedState::default();
        let _ = note(&mut st, "master", rep(Some(11.0), 1_000.0));
        let body = persist_body_of(&st);
        write_state(&sock, &body);
        let raw = std::fs::read_to_string(state_path(&sock)).unwrap();
        for banned in ["ctx_tokens", "ctx_window", "22000", "200000"] {
            assert!(!raw.contains(banned), "상태 파일에 {banned} 가 남았다(민감치 배제 위반):\n{raw}");
        }
        // 남아야 하는 넷은 실제로 남는다(배제가 과해서 다 지운 것이 아님을 함께 못박는다).
        for keep in ["master", "ctx_pct", "source", "updated_at"] {
            assert!(raw.contains(keep), "{keep} 가 없다 — 배제가 필요한 값까지 지웠다");
        }
        // 복원본은 토큰 수를 None으로 든다 — 「기록하지 않았다」가 정직한 상태다.
        let restored = load_from_disk(&sock);
        let r = restored.reports.get("master").unwrap();
        assert!(r.ctx_tokens.is_none() && r.ctx_window.is_none());
        assert_eq!(r.ctx_pct, Some(11.0));
    }

    /// 기록 스로틀 — 값이 그대로면 매 보고마다 쓰지 않는다(관측이 대상을 방해하지 않게).
    /// ★단, 값이 바뀌거나 최소 간격이 지나면 반드시 쓴다. 「안 쓴다」만 검사하면
    ///   영원히 안 쓰는 구현도 초록이 된다 — 두 방향을 함께 단언한다.
    #[cfg(not(windows))]
    #[test]
    fn persist_is_throttled_but_not_silenced() {
        let mut st = NamedState::default();
        assert!(note(&mut st, "master", rep(Some(11.0), 1_000.0)), "첫 관측");
        assert!(!note(&mut st, "master", rep(Some(11.0), 1_010.0)), "값 동일·간격 미달 → 기록 없음");
        assert!(!note(&mut st, "master", rep(Some(11.4), 1_020.0)), "반올림 후 같은 11% → 기록 없음");
        assert!(note(&mut st, "master", rep(Some(12.0), 1_030.0)), "화면 값이 바뀌면 기록");
        assert!(!note(&mut st, "master", rep(Some(12.0), 1_040.0)), "다시 안정 구간");
        // 값은 그대로여도 최소 간격이 지나면 쓴다 — 기록의 목적에 「관측 시각」도 있다.
        assert!(
            note(&mut st, "master", rep(Some(12.0), 1_030.0 + PERSIST_MIN_SECS)),
            "최소 간격 경과 후에는 값이 같아도 기록해야 한다(시각이 흘렀다)"
        );
        // 다른 보고자가 처음 나타나면 그 즉시 기록 — 없던 행은 기다리지 않는다.
        assert!(note(&mut st, "cso", rep(Some(3.0), 1_095.0)), "새 보고자는 즉시 기록");
    }

    /// 손상 파일은 조용한 빈 상태가 아니라 `.corrupt-<ts>` 격리 + 빈 상태.
    /// 부재(첫 기동)는 손상과 구별한다 — 격리 흔적을 남기면 안 된다.
    #[cfg(not(windows))]
    #[test]
    fn corrupt_isolated_missing_is_clean() {
        let sock = drill_sock("corrupt");
        let dir = sock.parent().unwrap().to_path_buf();
        // ① 부재 = 정상 빈 상태.
        assert!(load_from_disk(&sock).reports.is_empty());
        let has_corrupt = |dir: &std::path::Path| {
            std::fs::read_dir(dir)
                .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().contains(".corrupt-")))
                .unwrap_or(false)
        };
        assert!(!has_corrupt(&dir), "부재를 손상으로 오판해 격리하면 안 된다");

        // ② 손상 = 격리 + 빈 상태(원본을 그 자리에 남겨 두지 않는다).
        std::fs::write(state_path(&sock), "{ 이건 json 이 아니다 ]]]").unwrap();
        assert!(load_from_disk(&sock).reports.is_empty());
        assert!(has_corrupt(&dir), "손상 파일이 격리되지 않았다(조용한 소실)");
        assert!(!state_path(&sock).exists(), "손상 원본이 그대로 남아 매 기동 같은 실패를 반복한다");
    }

    /// 이름·관측 시각이 없는 행은 복원하지 않는다 — 지어낸 라벨·거짓 신선 금지 규율의 연장.
    #[cfg(not(windows))]
    #[test]
    fn restore_drops_nameless_and_timeless_rows() {
        let sock = drill_sock("bogus");
        std::fs::write(
            state_path(&sock),
            r#"{"named":[
                {"name":"","ctx_pct":5,"source":"statusline","updated_at":10},
                {"name":"ghost","ctx_pct":5,"source":"statusline","updated_at":0},
                {"name":"ghost2","ctx_pct":5,"source":"statusline"},
                {"name":"master","ctx_pct":null,"source":"statusline","updated_at":10}
            ]}"#,
        )
        .unwrap();
        let st = load_from_disk(&sock);
        assert_eq!(st.reports.keys().cloned().collect::<Vec<_>>(), vec!["master".to_string()]);
        // ctx가 없는 행은 살린다 — 「미관측」과 「그런 보고자 없음」은 다르다(UI가 「—」로 그린다).
        assert!(st.reports["master"].ctx_pct.is_none());
    }

    #[test]
    fn note_and_expose_roundtrip() {
        let mut st = NamedState::default();
        let _ = note(
            &mut st,
            "master",
            NamedReport {
                ctx_pct: Some(11.0),
                ctx_tokens: Some(22_000),
                ctx_window: Some(200_000),
                source: "statusline".into(),
                updated_at: 1000.0,
            },
        );
        let v = to_json(&st);
        assert_eq!(v[0]["name"], "master");
        assert_eq!(v[0]["ctx_pct"], 11.0);
        assert_eq!(v[0]["source"], "statusline");
        // 같은 이름의 새 관측은 덮어쓴다(최신만 남는다 — 행이 늘어나지 않는다).
        let _ = note(
            &mut st,
            "master",
            NamedReport {
                ctx_pct: Some(12.0),
                ctx_tokens: None,
                ctx_window: None,
                source: "statusline".into(),
                updated_at: 2000.0,
            },
        );
        let v = to_json(&st);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["ctx_pct"], 12.0);
    }
}
