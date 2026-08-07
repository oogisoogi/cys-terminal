//! CC v2 WS-A: 계정 단위 rate limit 집계 — 노드(surface) 관측을 **계정** 차원으로 귀속한다.
//!
//! 핵심 사실(실측 2026-07-16):
//! - 계정 식별자 = 프로필 dir이 아니라 `<dir>/.claude.json`의 `oauthAccount.accountUuid`.
//!   프로필 dir은 계정에 N:1이다(~/.claude·~/.claude-work·~/.cys/claude* 가 같은 계정인 식).
//! - claude rate의 유일한 생산자는 statusline(usage.report)이다 — usage.rs claude transcript
//!   분기는 rate를 **이월**하며 updated_at을 현재로 갱신하므로, 여기(note_rate)에는
//!   **신선 생산된 rate만** 넘긴다(이월분 수용 시 stale이 최신으로 둔갑).
//! - 병합 = 창 벡터 통째 최신 승자(같은 계정 풀은 최신 관측이 진실).
//!
//! 잠금 순서 불변식: accounts → (해제) → analytics. 역순 금지(교착).

use crate::state::Daemon;
use crate::usage::RateWindow;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 스냅샷 영속 스로틀 — 같은 (계정,창)에서 pct 변화가 이 미만이면 INSERT 생략.
const SNAPSHOT_MIN_DELTA_PCT: f64 = 1.0;
/// 스냅샷 보존 창(초) — 초과분은 prune. 30일.
const SNAPSHOT_RETAIN_SECS: f64 = 30.0 * 86400.0;
/// prune 주기(초) — note 경로에서 저빈도 수행. 6시간.
const PRUNE_INTERVAL_SECS: f64 = 6.0 * 3600.0;
/// 부트 복원 창(초) — 이 안의 마지막 스냅샷으로 계정 뷰를 예열(stale 표시). 7일.
const BOOT_RESTORE_SECS: f64 = 7.0 * 86400.0;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountKey {
    pub provider: String,   // "claude" | "codex" | "antigravity" | (accounts.json 선언 provider)
    pub account_id: String, // claude: accountUuid · 그 외 단일 홈: "default"
}

/// 모델 스코프 주간 게이지 — OAuth usage API `limits[].kind=="weekly_scoped"` 유래.
///
/// ★왜 `rate`와 **다른 슬롯**인가(설계의 핵심): `rate`의 병합 규율은 「창 벡터 통째 최신 승자」다
/// (모듈 헤더). 그 규율은 **모든 생산자가 같은 창 집합을 낸다**는 전제 위에서만 옳다. statusline은
/// {5h,7d}만 내고 OAuth 프로브는 {5h,7d,모델 스코프}를 낸다 — 이 둘을 한 벡터에서 겨루게 하면
/// statusline이 이길 때마다 모델 게이지가 **사라졌다 나타났다** 한다(2~5분 주기 × 페인 턴마다).
/// ⇒ 겹치지 않는 축은 겨루게 하지 않는다. 5h·7d는 종전대로 `rate`에서 신선도 경쟁하고(OAuth도
/// 같은 자격으로 합류), 모델 스코프 게이지만 이 슬롯에서 **자기 시각을 들고** 산다.
#[derive(Clone, Debug)]
pub struct ScopedGauge {
    /// API가 준 표시 이름(`scope.model.display_name` — 예: "Fable"). ★우리가 짓지 않는다:
    /// 스코프가 걸린 모델이 바뀌면 라벨도 따라 바뀌어야 하는데, 상수로 박으면 남의 게이지에
    /// 옛 이름이 붙는다.
    pub model: String,
    pub used_pct: f64,
    pub resets_at: Option<f64>,
    /// 이 게이지 자체의 관측 시각 — `AccountView.updated_at`(rate 슬롯의 시각)과 별개다.
    pub updated_at: f64,
    pub source: String, // "oauth"
}

#[derive(Clone, Debug)]
pub struct AccountView {
    pub key: AccountKey,
    pub label: String,        // claude: 이메일 · codex: "OpenAI Codex" · agy: "Antigravity (agy)"
    pub plan: Option<String>, // oauthAccount rate limit tier — 값이 있을 때만 UI 표시
    pub profiles: BTreeSet<String>, // 이 계정으로 관측된 프로필 dir들(홈 상대 표기)
    pub rate: Vec<RateWindow>,
    pub updated_at: f64, // 0.0 = 관측 전(발견만)
    pub source: String,  // "statusline" | "rollout" | "agy-rpc" | "adapter:<p>" | "oauth" | "snapshot"(부트 복원)
    pub adapter: bool,   // false = 관측 어댑터 없음(accounts.json adapter:"none" 선언 계정)
    /// 모델 스코프 주간 게이지(위 주석) — rate 슬롯과 독립. 빈 벡터 = 관측 없음(그리지 않는다).
    pub scoped: Vec<ScopedGauge>,
}

struct IdentEntry {
    mtime: f64,
    ident: Option<(String, String, Option<String>)>, // (accountUuid, email, plan)
}

#[derive(Default)]
pub struct AccountsState {
    views: HashMap<AccountKey, AccountView>,
    ident_cache: HashMap<PathBuf, IdentEntry>,
    last_persisted: HashMap<(AccountKey, String), f64>, // (key, 창 라벨) → 마지막 기록 pct
    last_prune: f64,
}

/// 세션 파일 경로 → 프로필 dir (`…/<profile>/projects/<munged>/<sess>.jsonl`의 profile 부분).
/// `/projects/` 마커 앞이 프로필 dir — 홈 `~/.claude*`와 `~/.cys/claude*` 모두 커버.
pub fn profile_dir_from_session(path: &str) -> Option<PathBuf> {
    let norm = path.replace('\\', "/");
    let idx = norm.find("/projects/")?;
    if idx == 0 {
        return None;
    }
    Some(PathBuf::from(&norm[..idx]))
}

/// 프로필 dir의 홈 상대 표기 (라벨·중복 제거용 — 계정 식별에는 쓰지 않는다)
fn profile_short(dir: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = dir.strip_prefix(&home) {
            return rel.to_string_lossy().into_owned();
        }
    }
    dir.to_string_lossy().into_owned()
}

/// `<dir>/.claude.json` → oauthAccount 신원. 잡동사니 dir(.claude-worktrees·백업 등)은
/// 파일 부재/uuid 부재로 None → 관측 미귀속(유령 계정 0). 자격증명(.credentials.json)은 읽지 않는다.
fn claude_identity(
    state: &mut AccountsState,
    dir: &Path,
) -> Option<(String, String, Option<String>)> {
    let f = dir.join(".claude.json");
    let mtime = std::fs::metadata(&f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())?;
    if let Some(e) = state.ident_cache.get(dir) {
        if e.mtime == mtime {
            return e.ident.clone();
        }
    }
    let ident = std::fs::read_to_string(&f)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| {
            let oa = v.get("oauthAccount")?;
            let uuid = oa.get("accountUuid")?.as_str()?.to_string();
            let email = oa
                .get("emailAddress")
                .and_then(|x| x.as_str())
                .unwrap_or(&uuid)
                .to_string();
            let plan = oa
                .get("userRateLimitTier")
                .or_else(|| oa.get("organizationRateLimitTier"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            Some((uuid, email, plan))
        });
    state
        .ident_cache
        .insert(dir.to_path_buf(), IdentEntry { mtime, ident: ident.clone() });
    ident
}

/// agent + 세션 파일 → (키, 라벨, plan, 프로필 표기). claude는 신원 해석 실패 시 None(스킵).
fn resolve(
    state: &mut AccountsState,
    agent: &str,
    session_file: &str,
) -> Option<(AccountKey, String, Option<String>, Option<String>)> {
    match agent {
        "claude" => {
            let dir = profile_dir_from_session(session_file)?;
            let (uuid, email, plan) = claude_identity(state, &dir)?;
            Some((
                AccountKey { provider: "claude".into(), account_id: uuid },
                email,
                plan,
                Some(profile_short(&dir)),
            ))
        }
        "codex" => Some((
            AccountKey { provider: "codex".into(), account_id: "default".into() },
            "OpenAI Codex".into(),
            None,
            Some(".codex".into()),
        )),
        "gemini" | "agy" | "antigravity" => Some((
            AccountKey { provider: "antigravity".into(), account_id: "default".into() },
            "Antigravity (agy)".into(),
            None,
            Some(".antigravity".into()),
        )),
        _ => None,
    }
}

/// 신선 생산된 rate 관측을 계정에 귀속·병합하고 스냅샷을 영속한다(스로틀·prune 포함).
/// **호출 계약: rate는 이번 관측이 실제 생산한 값만** — 이월(carryover) 금지(모듈 헤더 참조).
pub fn note_rate(
    daemon: &Arc<Daemon>,
    agent: &str,
    session_file: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) {
    if rate.is_empty() {
        return;
    }
    // 1) accounts 락 안에서 병합 + 영속 대상 수집 (analytics 락은 여기서 잡지 않는다 — 잠금 순서)
    let mut to_persist: Vec<(AccountKey, String, String, f64, Option<f64>)> = Vec::new();
    let mut do_prune = false;
    {
        let mut st = daemon.accounts.lock().unwrap();
        let Some((key, label, plan, profile)) = resolve(&mut st, agent, session_file) else {
            return; // 미귀속(신원 불명) — 유령 계정을 만들지 않는다
        };
        let view = st.views.entry(key.clone()).or_insert_with(|| AccountView {
            key: key.clone(),
            label: label.clone(),
            plan: plan.clone(),
            profiles: BTreeSet::new(),
            rate: Vec::new(),
            updated_at: 0.0,
            source: String::new(),
            adapter: true,
            scoped: Vec::new(),
        });
        view.label = label;
        if plan.is_some() {
            view.plan = plan;
        }
        if let Some(p) = profile {
            view.profiles.insert(p);
        }
        // 최신 승자 — note는 신선 생산분만 받으므로 timestamp 비교로 충분
        if now >= view.updated_at {
            view.rate = rate.to_vec();
            view.updated_at = now;
            view.source = source.into();
        }
        for w in rate {
            let pk = (key.clone(), w.label.clone());
            let prev = st.last_persisted.get(&pk).copied();
            if prev.map_or(true, |p| (w.used_pct - p).abs() >= SNAPSHOT_MIN_DELTA_PCT) {
                st.last_persisted.insert(pk, w.used_pct);
                to_persist.push((
                    key.clone(),
                    st.views[&key].label.clone(),
                    w.label.clone(),
                    w.used_pct,
                    w.resets_at,
                ));
            }
        }
        if now - st.last_prune > PRUNE_INTERVAL_SECS {
            st.last_prune = now;
            do_prune = true;
        }
    }
    // 2) analytics 영속 (accounts 락 해제 후)
    if to_persist.is_empty() && !do_prune {
        return;
    }
    let guard = daemon.analytics.lock().unwrap();
    if let Some(conn) = guard.as_ref() {
        for (key, label, win, pct, resets) in &to_persist {
            crate::analytics::record_rate_snapshot(
                conn, now, &key.provider, &key.account_id, label, win, *pct, *resets,
            );
        }
        if do_prune {
            crate::analytics::prune_rate_snapshots(conn, now - SNAPSHOT_RETAIN_SECS);
        }
    }
}

/// 부트 시드 — ① 알려진 프로필 dir 스캔으로 계정 **발견**(관측 전에도 3계정이 다 보이게),
/// ② analytics 마지막 스냅샷(7d)으로 rate 예열(source:"snapshot"·stale 표시),
/// ③ ~/.cys/accounts.json 선언 계정 등록(미래 provider — adapter:"none"은 '관측 없음' 상주).
pub fn seed_known(daemon: &Arc<Daemon>) {
    let mut dirs_to_check: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for e in std::fs::read_dir(&home).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".claude" || name.starts_with(".claude-") {
                dirs_to_check.push(e.path());
            }
        }
        for e in std::fs::read_dir(home.join(".cys")).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "claude" || name.starts_with("claude-") {
                dirs_to_check.push(e.path());
            }
        }
        {
            let mut st = daemon.accounts.lock().unwrap();
            for dir in dirs_to_check {
                if let Some((uuid, email, plan)) = claude_identity(&mut st, &dir) {
                    let key = AccountKey { provider: "claude".into(), account_id: uuid };
                    let short = profile_short(&dir);
                    let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
                        key,
                        label: email.clone(),
                        plan: plan.clone(),
                        profiles: BTreeSet::new(),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                        scoped: Vec::new(),
                    });
                    v.profiles.insert(short);
                }
            }
            if home.join(".codex").is_dir() {
                st.views
                    .entry(AccountKey { provider: "codex".into(), account_id: "default".into() })
                    .or_insert_with(|| AccountView {
                        key: AccountKey { provider: "codex".into(), account_id: "default".into() },
                        label: "OpenAI Codex".into(),
                        plan: None,
                        profiles: BTreeSet::from([".codex".to_string()]),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                        scoped: Vec::new(),
                    });
            }
            if home.join(".antigravity").is_dir() {
                st.views
                    .entry(AccountKey {
                        provider: "antigravity".into(),
                        account_id: "default".into(),
                    })
                    .or_insert_with(|| AccountView {
                        key: AccountKey {
                            provider: "antigravity".into(),
                            account_id: "default".into(),
                        },
                        label: "Antigravity (agy)".into(),
                        plan: None,
                        profiles: BTreeSet::from([".antigravity".to_string()]),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                        scoped: Vec::new(),
                    });
            }
        }
        // 선언 계정(~/.cys/accounts.json — pack 밖: pack 스윕/치유 사정권 회피)
        let decl = home.join(".cys/accounts.json");
        if let Ok(s) = std::fs::read_to_string(&decl) {
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                let mut st = daemon.accounts.lock().unwrap();
                for a in v.get("accounts").and_then(|x| x.as_array()).into_iter().flatten() {
                    let Some(provider) = a.get("provider").and_then(|x| x.as_str()) else {
                        continue;
                    };
                    let label = a
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or(provider)
                        .to_string();
                    let adapter =
                        a.get("adapter").and_then(|x| x.as_str()).unwrap_or("none") != "none";
                    let key =
                        AccountKey { provider: provider.into(), account_id: "default".into() };
                    st.views.entry(key.clone()).or_insert_with(|| AccountView {
                        key,
                        label,
                        plan: None,
                        profiles: BTreeSet::new(),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter,
                        scoped: Vec::new(),
                    });
                }
            }
        }
    }
    // 마지막 스냅샷으로 예열 — updated_at은 스냅샷 시각 그대로(신선한 척 금지)
    let rows = {
        let guard = daemon.analytics.lock().unwrap();
        guard.as_ref().map(|conn| {
            crate::analytics::last_rate_snapshots(
                conn,
                crate::state::now_epoch() - BOOT_RESTORE_SECS,
            )
        })
    };
    if let Some(rows) = rows {
        let mut st = daemon.accounts.lock().unwrap();
        for (ts, provider, account, label, win, pct, resets) in rows {
            let key = AccountKey { provider, account_id: account };
            let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
                key,
                label: label.clone(),
                plan: None,
                profiles: BTreeSet::new(),
                rate: Vec::new(),
                updated_at: 0.0,
                source: String::new(),
                adapter: true,
                scoped: Vec::new(),
            });
            // 라이브 관측 전(발견만·또는 스냅샷 예열 중)에만 덮는다 — 신선 관측 우선.
            let seeded = v.source.is_empty() || v.source == "snapshot";
            if seeded {
                if let Some(w) = v.rate.iter_mut().find(|w| w.label == win) {
                    w.used_pct = pct;
                    w.resets_at = resets;
                } else {
                    v.rate.push(RateWindow { label: win, used_pct: pct, resets_at: resets });
                }
                v.source = "snapshot".into();
                if ts > v.updated_at {
                    v.updated_at = ts;
                }
            }
        }
    }
}

/// accounts.json의 adapter:"cmd" 계정 — 주기 실행해 rate JSON을 흡수하는 범용 풀 어댑터.
/// 출력 계약: `[{"label":"5h","used_pct":12.3,"resets_at":1234.0}, …]`. grok/GLM CLI 합류 지점.
pub fn spawn_custom_adapters(daemon: Arc<Daemon>) {
    let Some(home) = dirs::home_dir() else { return };
    let decl = home.join(".cys/accounts.json");
    let Ok(s) = std::fs::read_to_string(&decl) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&s) else { return };
    for a in v.get("accounts").and_then(|x| x.as_array()).into_iter().flatten() {
        let (Some(provider), Some(cmd)) = (
            a.get("provider").and_then(|x| x.as_str()).map(|s| s.to_string()),
            a.get("cmd").and_then(|x| x.as_str()).map(|s| s.to_string()),
        ) else {
            continue;
        };
        if a.get("adapter").and_then(|x| x.as_str()) != Some("cmd") {
            continue;
        }
        let interval = a
            .get("interval_secs")
            .and_then(|x| x.as_u64())
            .unwrap_or(300)
            .max(60);
        let d = daemon.clone();
        tokio::spawn(async move {
            loop {
                // 플랫폼별 셸 위임 — Windows는 sh 부재(cmd /C). 실패는 무해(다음 주기 재시도).
                let fut = if cfg!(windows) {
                    tokio::process::Command::new("cmd").args(["/C", &cmd]).output()
                } else {
                    tokio::process::Command::new("sh").args(["-c", &cmd]).output()
                };
                if let Ok(Ok(out)) =
                    tokio::time::timeout(std::time::Duration::from_secs(10), fut).await
                {
                    if out.status.success() {
                        if let Ok(arr) = serde_json::from_slice::<Value>(&out.stdout) {
                            let rate: Vec<RateWindow> = arr
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|w| {
                                    Some(RateWindow {
                                        label: w.get("label")?.as_str()?.to_string(),
                                        used_pct: w.get("used_pct")?.as_f64()?,
                                        resets_at: w.get("resets_at").and_then(|x| x.as_f64()),
                                    })
                                })
                                .collect();
                            if !rate.is_empty() {
                                let now = crate::state::now_epoch();
                                let src = format!("adapter:{provider}");
                                note_custom(&d, &provider, &rate, &src, now);
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        });
    }
}

/// 선언 provider(비 내장) 계정에 rate 반영 — note_rate의 resolve를 우회하는 직접 키 경로.
fn note_custom(daemon: &Arc<Daemon>, provider: &str, rate: &[RateWindow], source: &str, now: f64) {
    let mut st = daemon.accounts.lock().unwrap();
    let key = AccountKey { provider: provider.into(), account_id: "default".into() };
    let label = st.views.get(&key).map(|v| v.label.clone()).unwrap_or_else(|| provider.into());
    let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
        key,
        label,
        plan: None,
        profiles: BTreeSet::new(),
        rate: Vec::new(),
        updated_at: 0.0,
        source: String::new(),
        adapter: true,
        scoped: Vec::new(),
    });
    if now >= v.updated_at {
        v.rate = rate.to_vec();
        v.updated_at = now;
        v.source = source.into();
        v.adapter = true;
    }
}

// ── Claude OAuth usage API 프로브 (오너 승인 2026-08-07 티켓⑤)
//
// 무엇을 푸는가: 5h·7d는 statusline이 주지만 **Claude 페인이 턴을 돌 때만** 온다. 모델 스코프
// 주간 게이지(Fable)는 statusline JSON에 **아예 없다**(실측 — five_hour·seven_day 둘뿐).
// ⇒ Claude Code의 /usage가 쓰는 서버 API를 우리도 직접 조회해 계정 저장소에 넣는다.
//
// 실측(2026-08-07 02:1x · 재검증 완료): `GET https://api.anthropic.com/api/oauth/usage`
//   헤더 `Authorization: Bearer <accessToken>` + `anthropic-beta: oauth-2025-04-20` → 200
//   `limits[]` = {kind: session|weekly_all|weekly_scoped, percent, resets_at(RFC3339), scope, …}
//   weekly_scoped.scope.model.display_name = "Fable" · 값이 오너 /usage 화면과 일치.
//
// ★신선도 실측 단서: 창이 굴러가는 순간(5h 리셋) API가 **약 1~2분간 직전 창을 계속 보고**한다
//   (02:11:35 조회 = 61%/리셋 02:10(과거) · 02:12:07 조회 = 0%/리셋 07:10). 그러므로 이 값을
//   statusline보다 무조건 우선시키지 않는다 — `rate`에서 신선도로 겨루게 두면 자연히 해소된다.
//
// ⛔토큰 규율: 토큰은 **프로세스 메모리와 파이프에만** 존재한다. 디스크·로그·환경변수·argv 어디에도
//   남기지 않는다. curl에 `-H "Authorization: …"`을 쓰면 argv에 실려 `ps`로 온 시스템에 보이므로,
//   헤더는 `--config -`(stdin)로 넣는다. 실패 로그에도 응답 본문을 찍지 않는다(토큰은 아니지만
//   계정 정보가 섞일 수 있고, 로그는 우리가 지우지 않는 곳이다).

/// 프로브 주기(초) — master 지정 2~5분의 중앙. 계정 한도는 분 단위로 움직이므로 이보다 촘촘할 이유가 없다.
const OAUTH_PROBE_INTERVAL_SECS: u64 = 180;
/// 연속 실패 시 주기 배수 상한 — 180s × 2^3 = 24분. 「재시도 폭주 금지」(master 규율).
const OAUTH_PROBE_MAX_BACKOFF_SHIFT: u32 = 3;
/// 외부 명령 1회 타임아웃(초) — 키체인·네트워크 모두. 매달리지 않는다.
const OAUTH_PROBE_CMD_TIMEOUT_SECS: u64 = 10;

/// OAuth usage 응답 → (rate 창들, 모델 스코프 게이지들). **응답 형태를 아는 유일한 자리**다.
///
/// ★순수 함수로 뽑아 둔 이유: 결함이 나는 곳은 늘 「필드 경로를 아는 지식」인데, 그 지식이
/// 네트워크·프로세스와 뒤엉킨 자리에 있으면 테스트가 닿지 못한다. (같은 이유로 뽑혀 나왔던
/// UI 쪽 짝 `wsusage.fableFromAnalytics`는 티켓⑥에서 그 줄과 함께 삭제됐다 — 교훈만 남는다.)
/// 아래 테스트는 **실물 응답 형태 그대로**를 픽스처로 쓴다.
///
/// 형태가 바뀌면 rate가 비고, 호출자는 그것을 「원천 소실」로 다룬다(경보가 아니라 조용한 강등).
pub fn parse_oauth_usage(v: &Value, now: f64) -> (Vec<RateWindow>, Vec<ScopedGauge>) {
    let iso = |x: &Value| -> Option<f64> {
        chrono::DateTime::parse_from_rfc3339(x.as_str()?).ok().map(|d| d.timestamp() as f64)
    };
    let mut rate = Vec::new();
    let mut scoped = Vec::new();
    for l in v.get("limits").and_then(|x| x.as_array()).into_iter().flatten() {
        let Some(pct) = l.get("percent").and_then(|x| x.as_f64()) else { continue };
        let resets_at = l.get("resets_at").and_then(iso);
        match l.get("kind").and_then(|x| x.as_str()) {
            // 라벨은 statusline·codex·agy와 **같은 어휘**를 쓴다 — 한 표 안에서 같은 창이 다른
            // 이름으로 두 줄 나오면 사용자는 그것을 두 한도로 읽는다.
            Some("session") => rate.push(RateWindow { label: "5h".into(), used_pct: pct, resets_at }),
            Some("weekly_all") => rate.push(RateWindow { label: "7d".into(), used_pct: pct, resets_at }),
            Some("weekly_scoped") => {
                // ★모델 이름이 없으면 게이지를 만들지 않는다. 이름 없는 게이지는 「무엇의 5%인지」를
                //   말할 수 없고, 우리가 이름을 지어 넣으면 없는 사실을 만드는 것이다.
                let Some(model) = l
                    .pointer("/scope/model/display_name")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                scoped.push(ScopedGauge {
                    model: model.to_string(),
                    used_pct: pct,
                    resets_at,
                    updated_at: now,
                    source: "oauth".into(),
                });
            }
            _ => {}
        }
    }
    // limits[]가 없는 옛/새 형태를 위한 보조 경로 — 최상위 five_hour·seven_day.
    // ★보조 경로에는 모델 스코프가 없다(실측: 최상위 seven_day_* 필드들은 전부 null). 즉 이 경로로
    //   떨어지면 Fable 줄은 조용히 사라진다 — 그것이 정직한 표현이다(없는 값을 지어내지 않는다).
    if rate.is_empty() {
        for (k, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            let Some(o) = v.get(k).filter(|x| x.is_object()) else { continue };
            let Some(pct) = o.get("utilization").and_then(|x| x.as_f64()) else { continue };
            rate.push(RateWindow { label: label.into(), used_pct: pct, resets_at: o.get("resets_at").and_then(iso) });
        }
    }
    rate.sort_by_key(|r| u8::from(r.label != "5h")); // 5h 먼저 (배지·사이드바 순서 안정)
    scoped.sort_by(|a, b| a.model.cmp(&b.model));
    (rate, scoped)
}

/// OAuth 프로브 관측을 claude 계정에 반영 — `rate`는 종전 규율대로 겨루고, `scoped`는 자기 슬롯에 산다.
///
/// ★`scoped`를 무조건 덮는 이유: 이 슬롯의 생산자는 프로브 하나뿐이다. 경쟁자가 없으므로 최신이
/// 곧 진실이고, 시각도 게이지 자신이 들고 있어 UI가 따로 나이를 잰다.
fn note_oauth(
    daemon: &Arc<Daemon>,
    account_id: &str,
    label: &str,
    rate: &[RateWindow],
    scoped: &[ScopedGauge],
    now: f64,
) {
    let key = AccountKey { provider: "claude".into(), account_id: account_id.into() };
    let mut to_persist: Vec<(AccountKey, String, String, f64, Option<f64>)> = Vec::new();
    {
        let mut st = daemon.accounts.lock().unwrap();
        let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
            key: key.clone(),
            label: label.into(),
            plan: None,
            profiles: BTreeSet::new(),
            rate: Vec::new(),
            updated_at: 0.0,
            source: String::new(),
            adapter: true,
            scoped: Vec::new(),
        });
        // scoped는 rate 승패와 무관하게 갱신한다(위 주석) — statusline이 이겨도 살아남는 축.
        if !scoped.is_empty() {
            v.scoped = scoped.to_vec();
        }
        if !rate.is_empty() && now >= v.updated_at {
            v.rate = rate.to_vec();
            v.updated_at = now;
            v.source = "oauth".into();
        }
        // 스냅샷 영속은 statusline 경로와 **같은 스로틀**을 쓴다(원천이 둘이어도 시계열은 하나다).
        for w in rate {
            let pk = (key.clone(), w.label.clone());
            let prev = st.last_persisted.get(&pk).copied();
            if prev.map_or(true, |p| (w.used_pct - p).abs() >= SNAPSHOT_MIN_DELTA_PCT) {
                st.last_persisted.insert(pk, w.used_pct);
                let lbl = st.views[&key].label.clone();
                to_persist.push((key.clone(), lbl, w.label.clone(), w.used_pct, w.resets_at));
            }
        }
    }
    if to_persist.is_empty() {
        return;
    }
    let guard = daemon.analytics.lock().unwrap(); // 잠금 순서: accounts 해제 후 analytics
    if let Some(conn) = guard.as_ref() {
        for (key, label, win, pct, resets) in &to_persist {
            crate::analytics::record_rate_snapshot(
                conn, now, &key.provider, &key.account_id, label, win, *pct, *resets,
            );
        }
    }
}

/// 외부 명령 1회 실행 — 표준입력을 주고 stdout을 받는다. 실패는 사유 문자열로.
async fn run_capture(program: &str, args: &[&str], stdin_data: Option<&str>) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(if stdin_data.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{program} spawn: {e}"))?;
    if let Some(data) = stdin_data {
        let mut si = child.stdin.take().ok_or_else(|| format!("{program}: stdin 없음"))?;
        si.write_all(data.as_bytes()).await.map_err(|e| format!("{program} stdin: {e}"))?;
        drop(si); // EOF — 안 닫으면 curl이 설정 끝을 못 보고 매달린다
    }
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(OAUTH_PROBE_CMD_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("{program}: 타임아웃"))?
    .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        // ⛔stderr 본문을 로그로 흘리지 않는다 — 종료코드만 말한다.
        return Err(format!("{program}: 종료코드 {:?}", out.status.code()));
    }
    Ok(out.stdout)
}

/// 키체인에서 Claude Code OAuth 액세스 토큰. **반환값을 로그에 찍지 마라.**
///
/// ★키체인 서비스명 `Claude Code-credentials`(접미 없음) = 기본 설정 dir(`~/.claude`)의 자격증명이다.
/// 접미가 붙은 항목들(`…-<hash>`)은 `CLAUDE_CONFIG_DIR`로 갈라 둔 다른 설정 dir의 것이다.
/// 그래서 계정 귀속도 `~/.claude/.claude.json`의 accountUuid로 잡는다(짝이 맞는 쌍을 쓴다).
async fn keychain_token() -> Result<String, String> {
    let raw = run_capture(
        "security",
        &["find-generic-password", "-s", "Claude Code-credentials", "-w"],
        None,
    )
    .await?;
    let v: Value = serde_json::from_slice(&raw).map_err(|_| "키체인 항목이 JSON이 아니다".to_string())?;
    v.pointer("/claudeAiOauth/accessToken")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "키체인 항목에 accessToken이 없다".to_string())
}

/// usage API 1회 조회. 토큰은 argv가 아니라 **stdin(curl --config -)** 으로만 건넨다.
async fn fetch_oauth_usage(token: &str) -> Result<Value, String> {
    // curl 설정 파일 문법: `key = "value"`. 값 안의 큰따옴표만 이스케이프하면 된다.
    // 토큰은 `sk-ant-oat…` 형태라 따옴표가 없지만, 형태를 믿지 않고 escape한다.
    let esc = token.replace('\\', "\\\\").replace('"', "\\\"");
    let cfg = format!(
        concat!(
            "url = \"https://api.anthropic.com/api/oauth/usage\"\n",
            "header = \"Authorization: Bearer {}\"\n",
            "header = \"anthropic-beta: oauth-2025-04-20\"\n",
            "silent\n",
            "show-error\n",
            "max-time = {}\n",
            "write-out = \"\\n%{{http_code}}\"\n",
        ),
        esc, OAUTH_PROBE_CMD_TIMEOUT_SECS
    );
    let out = run_capture("curl", &["--config", "-"], Some(&cfg)).await?;
    let text = String::from_utf8_lossy(&out);
    let (body, code) = text.rsplit_once('\n').ok_or_else(|| "응답 형태 불명".to_string())?;
    if code.trim() != "200" {
        // ★본문은 찍지 않는다. 401은 토큰 만료(Claude Code가 갱신하면 다음 주기에 저절로 낫는다).
        return Err(format!("HTTP {}", code.trim()));
    }
    serde_json::from_str(body).map_err(|_| "응답이 JSON이 아니다".to_string())
}

/// 프로브 1회 — 토큰 → 조회 → 파싱 → 계정 반영.
async fn oauth_probe_once(daemon: &Arc<Daemon>) -> Result<(), String> {
    let token = keychain_token().await?;
    let v = fetch_oauth_usage(&token).await?;
    drop(token); // 필요 이상으로 들고 있지 않는다
    let now = crate::state::now_epoch();
    let (rate, scoped) = parse_oauth_usage(&v, now);
    if rate.is_empty() && scoped.is_empty() {
        return Err("응답에 한도 정보가 없다(형태 변경?)".into());
    }
    // 계정 귀속 — 짝이 맞는 설정 dir(~/.claude)의 신원. 없으면 **쓰지 않는다**(유령 계정 0).
    let home = dirs::home_dir().ok_or_else(|| "홈 dir 불명".to_string())?;
    let ident = {
        let mut st = daemon.accounts.lock().unwrap();
        claude_identity(&mut st, &home.join(".claude"))
    };
    let Some((uuid, email, _plan)) = ident else {
        return Err("~/.claude/.claude.json에 oauthAccount 신원이 없다".into());
    };
    note_oauth(daemon, &uuid, &email, &rate, &scoped, now);
    Ok(())
}

/// 실패 1줄의 정본 문구 — 상주 프로브와 강제발화가 **같은 문장**을 쓴다.
/// (두 곳이 따로 문장을 지으면, 강제발화로 확인한 실패 표현이 운영 로그의 표현과 달라져
///  「내가 본 것」과 「로그에 남는 것」이 어긋난다.)
fn oauth_lost_line(e: &str) -> String {
    format!("[cysd] oauth-usage: 원천 소실 — {e}")
}

/// 강제발화 — `cysd --oauth-usage-probe`. 데몬을 띄우지 않고 프로브 1회만 돌고 끝난다.
///
/// ★왜 필요한가: 이 경로에서 깨질 수 있는 두 가지(키체인 접근·외부 HTTPS)는 **실행 컨텍스트에
/// 좌우된다** — 사람이 로그인한 셸에서 된다는 것은 launchd 아래 cysd에서 된다는 증거가 아니다.
/// 데몬 본체를 띄우지 않고 **같은 코드**로 그 컨텍스트를 찍어 볼 수 있어야 검증이 성립한다.
/// ⛔출력에는 값(%·리셋 시각)만 싣는다. 토큰은 어떤 경로로도 나가지 않는다.
/// 반환 = 프로세스 종료코드(0 정상 · 1 원천 소실).
pub async fn oauth_probe_report() -> i32 {
    let now = crate::state::now_epoch();
    let r = async {
        let token = keychain_token().await?;
        let v = fetch_oauth_usage(&token).await?;
        Ok::<_, String>(parse_oauth_usage(&v, now))
    }
    .await;
    match r {
        Err(e) => {
            eprintln!("{}", oauth_lost_line(&e));
            1
        }
        Ok((rate, scoped)) if rate.is_empty() && scoped.is_empty() => {
            eprintln!("{}", oauth_lost_line("응답에 한도 정보가 없다(형태 변경?)"));
            1
        }
        Ok((rate, scoped)) => {
            for w in &rate {
                println!("[cysd] oauth-usage: {} {:.0}% resets_at={:?}", w.label, w.used_pct, w.resets_at);
            }
            for g in &scoped {
                println!(
                    "[cysd] oauth-usage: 7d·{} {:.0}% resets_at={:?}",
                    g.model, g.used_pct, g.resets_at
                );
            }
            0
        }
    }
}

/// claude 계정 OAuth usage 프로브 상주 — 주기 조회·실패 시 백오프.
///
/// 실패는 **경보가 아니라 「원천 소실」 1줄**이다(master 규율): 이 값이 없어도 statusline 원천이
/// 그대로 살아 있고, 프로브 유래 행은 나이가 자라 자연히 stale로 강등된다. 시끄럽게 굴 이유가 없다.
/// ★로그는 상태가 **바뀔 때만** 찍는다 — 매 주기 찍으면 24분마다 같은 줄이 쌓여 로그가 신호를 잃는다.
pub fn spawn_claude_oauth_probe(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut fails: u32 = 0;
        loop {
            match oauth_probe_once(&daemon).await {
                Ok(()) => {
                    if fails > 0 {
                        eprintln!("[cysd] oauth-usage: 원천 복구 (연속 실패 {fails}회 후)");
                    }
                    fails = 0;
                }
                Err(e) => {
                    if fails == 0 {
                        eprintln!("{}", oauth_lost_line(&e));
                    }
                    fails = fails.saturating_add(1);
                }
            }
            let shift = fails.min(OAUTH_PROBE_MAX_BACKOFF_SHIFT);
            let secs = OAUTH_PROBE_INTERVAL_SECS.saturating_mul(1u64 << shift);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        }
    });
}

/// 소진 예측 최소 표본 수·스팬(초) — 미달 시 예측 미표시(표본 2개 기울기의 황당 예측 차단).
const PREDICT_MIN_POINTS: usize = 3;
const PREDICT_MIN_SPAN_SECS: f64 = 600.0;
/// 예측 대상 신선도(초) — stale 관측으로 예측하지 않는다.
const PREDICT_FRESH_SECS: f64 = 600.0;

/// 로컬 계정 뷰 → JSON 배열 (usage.accounts RPC·control.dashboard "accounts" 공용).
/// stale_secs는 읽기 시점 계산 — updated_at==0.0은 null(관측 전)로 정직 표기.
/// 5h 창에는 소진 예측(exhaust_at)을 붙인다 — 최근 60분 선형 기울기, 표본 미달·기울기≤0·
/// 리셋 후 소진이면 생략(정직한 공백). 잠금 순서: accounts → 해제 → analytics.
pub fn local_json(daemon: &Arc<Daemon>, now: f64) -> Value {
    let mut rows: Vec<Value> = {
        let st = daemon.accounts.lock().unwrap();
        let mut views: Vec<&AccountView> = st.views.values().collect();
        views.sort_by(|a, b| a.key.cmp(&b.key));
        views
            .into_iter()
            .map(|v| {
                json!({
                    "provider": v.key.provider,
                    "account_id": v.key.account_id,
                    "label": v.label,
                    "plan": v.plan,
                    "profiles": v.profiles.iter().collect::<Vec<_>>(),
                    "rate": v.rate,
                    "updated_at": if v.updated_at > 0.0 { json!(v.updated_at) } else { Value::Null },
                    "stale_secs": if v.updated_at > 0.0 { json!((now - v.updated_at).max(0.0)) } else { Value::Null },
                    "source": v.source,
                    "adapter": v.adapter,
                    // 모델 스코프 게이지 — ★자기 updated_at을 들고 나간다. 계정의 updated_at(rate 슬롯)을
                    // 물려 쓰면 statusline이 rate를 갱신할 때마다 이 게이지가 「방금 관측」으로 둔갑한다.
                    "scoped": v.scoped.iter().map(|g| json!({
                        "model": g.model,
                        "used_pct": g.used_pct,
                        "resets_at": g.resets_at,
                        "updated_at": g.updated_at,
                        "source": g.source,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect()
    };
    // 소진 예측 — 신선(≤10분) 계정의 5h 창만. accounts 락 해제 후 analytics 조회(잠금 순서).
    let guard = daemon.analytics.lock().unwrap();
    if let Some(conn) = guard.as_ref() {
        for row in rows.iter_mut() {
            let fresh = row["stale_secs"].as_f64().map(|s| s <= PREDICT_FRESH_SECS).unwrap_or(false);
            if !fresh {
                continue;
            }
            let (provider, account) = (
                row["provider"].as_str().unwrap_or("").to_string(),
                row["account_id"].as_str().unwrap_or("").to_string(),
            );
            let resets_at = row["rate"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|w| w["label"] == "5h")
                .and_then(|w| w["resets_at"].as_f64());
            let series = crate::analytics::rate_series(conn, &provider, &account, "5h", now - 3600.0);
            if let Some(t) = predict_exhaust(&series, now, resets_at) {
                row["exhaust_at"] = json!(t);
            }
        }
    }
    Value::Array(rows)
}

/// 선형 소진 예측(순수 — 테스트 핀): 시계열 최소자승 기울기로 100% 도달 시각.
/// None = 표본 미달·스팬 미달·기울기≤0·이미 100%·예측이 리셋 이후(리셋이 먼저면 무의미).
pub fn predict_exhaust(series: &[(f64, f64)], now: f64, resets_at: Option<f64>) -> Option<f64> {
    if series.len() < PREDICT_MIN_POINTS {
        return None;
    }
    let span = series.last()?.0 - series.first()?.0;
    if span < PREDICT_MIN_SPAN_SECS {
        return None;
    }
    let n = series.len() as f64;
    let (sx, sy): (f64, f64) = series.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (mut num, mut den) = (0.0, 0.0);
    for (x, y) in series {
        num += (x - mx) * (y - my);
        den += (x - mx) * (x - mx);
    }
    if den <= 0.0 {
        return None;
    }
    let slope = num / den; // %/초
    let last = series.last()?;
    if slope <= 0.0 || last.1 >= 100.0 {
        return None;
    }
    let t = last.0 + (100.0 - last.1) / slope;
    if t <= now {
        return None;
    }
    match resets_at {
        Some(r) if t >= r => None, // 리셋이 먼저 — 소진 경고 무의미
        _ => Some(t),
    }
}

/// alerts용 스냅샷: (라벨, 창, pct) — 관측된 계정만.
pub fn alert_rates(daemon: &Arc<Daemon>) -> Vec<(String, String, f64)> {
    let st = daemon.accounts.lock().unwrap();
    let mut out = Vec::new();
    for v in st.views.values() {
        if v.updated_at == 0.0 {
            continue;
        }
        for w in &v.rate {
            out.push((v.label.clone(), w.label.clone(), w.used_pct));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys-acct-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn profile_dir_extraction() {
        assert_eq!(
            profile_dir_from_session("/Users/x/.claude-work/projects/-a/s.jsonl"),
            Some(PathBuf::from("/Users/x/.claude-work"))
        );
        assert_eq!(
            profile_dir_from_session("/Users/x/.cys/claude-default-dept-2/projects/-a/s.jsonl"),
            Some(PathBuf::from("/Users/x/.cys/claude-default-dept-2"))
        );
        assert_eq!(profile_dir_from_session("no-projects-marker.jsonl"), None);
        // Windows 역슬래시 경로 내성
        assert_eq!(
            profile_dir_from_session("C:\\Users\\x\\.claude\\projects\\-a\\s.jsonl"),
            Some(PathBuf::from("C:/Users/x/.claude"))
        );
    }

    #[test]
    fn identity_parse_and_junk_dir_skip() {
        let dir = tmp("ident");
        // 정상 프로필
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"a@b.c","userRateLimitTier":"max_5x"}}"#,
        )
        .unwrap();
        let mut st = AccountsState::default();
        let got = claude_identity(&mut st, &dir).unwrap();
        assert_eq!(got, ("u-1".into(), "a@b.c".into(), Some("max_5x".into())));
        // 캐시 적중(mtime 동일 → 재파싱 없이 동일 결과)
        assert_eq!(claude_identity(&mut st, &dir).unwrap().0, "u-1");
        // 잡동사니 dir(.claude.json 없음) → None
        let junk = tmp("junk");
        assert!(claude_identity(&mut st, &junk).is_none());
        // uuid 없는 파손 파일 → None (유령 계정 0)
        let broken = tmp("broken");
        std::fs::write(broken.join(".claude.json"), r#"{"oauthAccount":{}}"#).unwrap();
        assert!(claude_identity(&mut st, &broken).is_none());
    }

    #[test]
    fn predict_exhaust_pins() {
        // 표본 미달(2개) → None
        assert!(predict_exhaust(&[(0.0, 10.0), (600.0, 20.0)], 700.0, None).is_none());
        // 스팬 미달(<600s) → None
        assert!(
            predict_exhaust(&[(0.0, 10.0), (100.0, 20.0), (200.0, 30.0)], 300.0, None).is_none()
        );
        // 정상: 0→60%가 3600초 — 100% 도달 ≈ 6000초
        let s = [(0.0, 0.0), (1800.0, 30.0), (3600.0, 60.0)];
        let t = predict_exhaust(&s, 3600.0, None).unwrap();
        assert!((t - 6000.0).abs() < 1.0, "t={t}");
        // 리셋이 소진보다 먼저 → None
        assert!(predict_exhaust(&s, 3600.0, Some(5000.0)).is_none());
        // 감소 추세(slope≤0) → None
        assert!(
            predict_exhaust(&[(0.0, 60.0), (1800.0, 40.0), (3600.0, 20.0)], 3600.0, None)
                .is_none()
        );
    }

    /// 실물 응답 형태(2026-08-07 02:12 실측 · 값만 축약, 키·중첩은 원본 그대로).
    /// ★픽스처를 손으로 예쁘게 다듬지 않는다 — 실물이 안 때리는 형태로 만들면 초록불이 거짓이 된다.
    fn oauth_fixture() -> Value {
        serde_json::from_str(
            r#"{
              "five_hour": {"utilization": 0.0, "resets_at": "2026-08-07T07:10:00.688508+00:00",
                            "limit_dollars": null, "used_dollars": null, "remaining_dollars": null},
              "seven_day": {"utilization": 13.0, "resets_at": "2026-08-13T21:00:00.688530+00:00"},
              "seven_day_opus": null, "seven_day_sonnet": null, "seven_day_cowork": null,
              "extra_usage": {"is_enabled": false, "utilization": null},
              "member_dashboard_available": false,
              "limits": [
                {"kind":"session","group":"session","percent":0,"severity":"normal",
                 "resets_at":"2026-08-07T07:10:00.688508+00:00","scope":null,"is_active":false},
                {"kind":"weekly_all","group":"weekly","percent":13,"severity":"normal",
                 "resets_at":"2026-08-13T21:00:00.688530+00:00","scope":null,"is_active":true},
                {"kind":"weekly_scoped","group":"weekly","percent":6,"severity":"normal",
                 "resets_at":"2026-08-13T20:59:59.688768+00:00",
                 "scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false}
              ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn oauth_usage_parses_real_shape() {
        let (rate, scoped) = parse_oauth_usage(&oauth_fixture(), 1000.0);
        // 라벨 어휘는 statusline과 같아야 한다(한 표에서 같은 창이 두 이름으로 나오면 두 한도로 읽힌다)
        assert_eq!(rate.len(), 2);
        assert_eq!(rate[0].label, "5h", "5h 먼저 정렬");
        assert_eq!(rate[0].used_pct, 0.0);
        assert_eq!(rate[1].label, "7d");
        assert_eq!(rate[1].used_pct, 13.0);
        // RFC3339 → epoch (2026-08-13T21:00:00Z)
        assert_eq!(rate[1].resets_at, Some(1786654800.0));
        // 모델 스코프 게이지 — 이름은 응답이 준 것을 그대로 쓴다(상수 아님)
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].model, "Fable");
        assert_eq!(scoped[0].used_pct, 6.0);
        assert_eq!(scoped[0].updated_at, 1000.0, "게이지는 자기 관측 시각을 들고 나간다");
        assert_eq!(scoped[0].source, "oauth");
    }

    #[test]
    fn oauth_usage_degrades_without_lying() {
        // ① limits[] 부재 → 최상위 five_hour/seven_day 보조 경로. **스코프 게이지는 안 만든다.**
        let mut v = oauth_fixture();
        v.as_object_mut().unwrap().remove("limits");
        let (rate, scoped) = parse_oauth_usage(&v, 1000.0);
        assert_eq!(rate.len(), 2, "보조 경로로 5h·7d는 살아난다");
        assert_eq!(rate[0].label, "5h");
        assert!(scoped.is_empty(), "최상위 형태엔 모델 스코프가 없다 — 지어내지 않는다");
        // ② display_name 없는 weekly_scoped → 게이지 없음(이름 없는 게이지는 무엇의 %인지 못 말한다)
        let v2: Value = serde_json::from_str(
            r#"{"limits":[{"kind":"weekly_scoped","percent":5,"resets_at":"2026-08-13T21:00:00Z",
                           "scope":{"model":{"id":null,"display_name":""},"surface":null}}]}"#,
        )
        .unwrap();
        let (r2, s2) = parse_oauth_usage(&v2, 1000.0);
        assert!(r2.is_empty() && s2.is_empty(), "이름 없는 스코프는 버린다");
        // ③ 형태 전면 변경 → 전부 비어 호출자가 「원천 소실」로 다룰 수 있다
        let v3: Value = serde_json::from_str(r#"{"something_else": 1}"#).unwrap();
        let (r3, s3) = parse_oauth_usage(&v3, 1000.0);
        assert!(r3.is_empty() && s3.is_empty());
        // ④ resets_at이 파싱 불가여도 pct는 살린다(시각 하나 때문에 값을 버리지 않는다)
        let v4: Value = serde_json::from_str(
            r#"{"limits":[{"kind":"session","percent":42,"resets_at":"어제"}]}"#,
        )
        .unwrap();
        let (r4, _) = parse_oauth_usage(&v4, 1000.0);
        assert_eq!(r4.len(), 1);
        assert_eq!(r4[0].used_pct, 42.0);
        assert_eq!(r4[0].resets_at, None);
    }

    #[test]
    fn resolve_agents() {
        let mut st = AccountsState::default();
        // codex/agy는 세션 파일 불요·단일 계정
        let (k, l, _, _) = resolve(&mut st, "codex", "").unwrap();
        assert_eq!((k.provider.as_str(), k.account_id.as_str()), ("codex", "default"));
        assert_eq!(l, "OpenAI Codex");
        let (k, ..) = resolve(&mut st, "gemini", "").unwrap();
        assert_eq!(k.provider, "antigravity");
        // 미지 agent → None
        assert!(resolve(&mut st, "mystery", "").is_none());
        // claude인데 신원 해석 불가 → None(스킵 — 유령 계정 금지)
        assert!(resolve(&mut st, "claude", "/nonexist/projects/x/s.jsonl").is_none());
    }
}
