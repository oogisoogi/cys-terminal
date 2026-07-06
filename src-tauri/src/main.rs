//! cys UI shell — cysd 소켓의 얇은 클라이언트.
//! 코어/UI 분리: UI가 죽어도 세션(PTY)은 데몬에 살아있다. UI 재시작 = 재attach.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

type Stream = Box<dyn AsyncReadWrite>;
trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

// 멀티마스터 F3: attach 핸들 키를 (소켓 slug, surface_id) 복합키로 — 서로 다른 데몬이 같은
// surface_id를 독립 발급하므로 단독 키는 부서 간 PTY 스트림이 충돌한다.
struct Attachments(Mutex<HashMap<(String, u64), tauri::async_runtime::JoinHandle<()>>>);

/// 소켓 경로 → 짧은 결정론 식별자(이벤트명·attach 키용). 백엔드 단일 진실 — UI는 attach 반환값/
/// daemon-event 페이로드로 이 값을 전달받아 그대로 쓴다(독립 재계산 금지, 검증 mustFix).
fn sock_slug(socket: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    socket.to_string_lossy().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// 기본 소켓 — env(CYS_SOCKET) 누수 방지를 위해 명시적 기본 경로를 쓴다(멀티마스터 F3:
/// 앱이 CYS_SOCKET 걸린 셸에서 런칭돼도 단일 데몬 사용자 하위호환이 깨지지 않게).
fn default_socket() -> std::path::PathBuf {
    cys::socket_path()
}
/// UI workspace의 socket(Option) → 실제 경로. None = 기본 데몬(하위호환의 단일 결정요인).
fn resolve_socket(opt: &Option<String>) -> std::path::PathBuf {
    opt.as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_socket)
}

#[cfg(unix)]
async fn connect_to(socket: &std::path::Path) -> Result<Stream, String> {
    tokio::net::UnixStream::connect(socket)
        .await
        .map(|s| Box::new(s) as Stream)
        .map_err(|e| format!("cannot connect to cysd at {}: {e}", socket.display()))
}

#[cfg(windows)]
async fn connect_to(socket: &std::path::Path) -> Result<Stream, String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    ClientOptions::new()
        .open(socket.to_string_lossy().as_ref())
        .map(|s| Box::new(s) as Stream)
        .map_err(|e| format!("cannot connect to cysd pipe: {e}"))
}

/// 기본 소켓 연결 (하위호환 wrapper).
async fn connect() -> Result<Stream, String> {
    connect_to(&default_socket()).await
}

/// 소켓별 영속 RPC 연결 풀 — 데몬(부서)마다 독립 연결 + 독립 락(데몬 간 직렬화 병목 제거).
type ConnCell = std::sync::Arc<tokio::sync::Mutex<Option<tokio::io::BufReader<Stream>>>>;
static RPC_POOL: std::sync::OnceLock<Mutex<HashMap<std::path::PathBuf, ConnCell>>> =
    std::sync::OnceLock::new();

/// 풀에서 소켓의 연결 셀을 얻는다 — 외부 std Mutex는 Arc 클론만 짧게 잡고 즉시 푼다(await 경계 안 넘김).
fn conn_cell(socket: &std::path::Path) -> ConnCell {
    let pool = RPC_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = pool.lock().unwrap();
    g.entry(socket.to_path_buf())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(None)))
        .clone()
}

/// rpc_once 실패 단계: 전송 전(BeforeSend)은 데몬이 요청을 못 봤으므로 재시도 안전,
/// 전송 후(AfterSend)는 처리됐을 수 있어 비멱등 명령(create·send)의 맹목 재시도 금지.
enum RpcErr {
    BeforeSend(String),
    AfterSend(String),
}

async fn rpc_once(
    socket: &std::path::Path,
    conn: &mut Option<tokio::io::BufReader<Stream>>,
    line: &[u8],
) -> Result<String, RpcErr> {
    if conn.is_none() {
        *conn = Some(BufReader::new(
            connect_to(socket).await.map_err(RpcErr::BeforeSend)?,
        ));
    }
    let c = conn.as_mut().unwrap();
    c.get_mut()
        .write_all(line)
        .await
        .map_err(|e| RpcErr::BeforeSend(e.to_string()))?;
    c.get_mut()
        .flush()
        .await
        .map_err(|e| RpcErr::AfterSend(e.to_string()))?;
    let mut resp_line = String::new();
    let n = c
        .read_line(&mut resp_line)
        .await
        .map_err(|e| RpcErr::AfterSend(e.to_string()))?;
    if n == 0 {
        return Err(RpcErr::AfterSend("connection closed".into()));
    }
    Ok(resp_line)
}

/// 기본 소켓 RPC (하위호환 wrapper).
async fn rpc(method: &str, params: Value) -> Result<Value, String> {
    rpc_on(&default_socket(), method, params).await
}

/// 소켓 지정 RPC — 풀의 소켓별 연결을 잠가 직렬화(다른 데몬 RPC를 막지 않음).
async fn rpc_on(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let cell = conn_cell(socket);
    let mut conn = cell.lock().await;
    let resp_line = match rpc_once(socket, &mut conn, &line).await {
        Ok(r) => r,
        Err(RpcErr::BeforeSend(_)) => {
            // 풀링된 연결이 끊겨 전송 자체가 실패 — 데몬이 요청을 못 봤으니 재시도 안전
            *conn = None;
            match rpc_once(socket, &mut conn, &line).await {
                Ok(r) => r,
                Err(RpcErr::BeforeSend(e)) | Err(RpcErr::AfterSend(e)) => {
                    *conn = None;
                    return Err(e);
                }
            }
        }
        Err(RpcErr::AfterSend(e)) => {
            // 데몬이 이미 처리했을 수 있음 — 중복 surface 생성·키 이중 주입을 막기 위해
            // 재전송하지 않고 에러를 그대로 올린다
            *conn = None;
            return Err(e);
        }
    };
    let resp: Value = serde_json::from_str(resp_line.trim()).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(resp["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_string())
    }
}

#[tauri::command]
async fn daemon_status(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "system.identify", json!({"caller": "ui"})).await
}

/// GUI(cys-app) 자기 버전 — 데몬 버전(system.identify .version)과 비교해 rename-swap 후
/// lame-duck 스큐(구 데몬 + 새 앱)를 UI 배지로 알리는 용도(P2 · 비차단·강제 재시작 없음).
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn list_surfaces(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "surface.list", json!({})).await
}

/// org.status 브리지 — 사이드바 라이브 신호(B3)·command palette(07) 공유 소스.
#[tauri::command]
async fn org_status(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "org.status", json!({})).await
}

/// 풀 비경유 일회성 RPC — org_fleet fan-out 전용. timeout 취소가 발생해도 이 연결만 드롭(폐기)되어
/// 공유 풀(conn_cell)을 desync로 오염시키지 않는다(같은 부서로 가는 send_key/org_status 응답 귀속 보호).
/// 적대검증 R-1 교정: rpc_on을 timeout으로 감싸면 취소 시 풀 연결이 미수신 응답을 남겨 후속 RPC가
/// stale 응답을 잘못 읽는다 — 일회성 연결은 드롭이 곧 연결 종료라 공유 상태를 건드리지 않는다.
async fn rpc_oneshot(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut stream = connect_to(socket).await?;
    stream.write_all(&line).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    let n = reader.read_line(&mut resp).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    let resp: Value = serde_json::from_str(resp.trim()).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(resp["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_string())
    }
}

/// Tasks Control Center — 모든 부서의 모든 노드를 한 콜로 집계한다("부서 다중소켓 보드").
/// depts.json을 읽어 본부(기본 소켓)+각 부서 소켓에 org.status를 순회 호출하고, 부서 라벨을
/// 호출자(여기)에서 주입한다(단일 데몬은 자기가 어느 부서인지 모름 — socket_slug 사상과 동일).
/// 데몬은 outbound 클라이언트가 없어 집계는 이 Tauri 층(기존 rpc_on)에서 한다. 도달 실패 부서는
/// 드롭하지 않고 error로 표기한다(오너이 "부서가 죽었다"를 봐야 함). 부서 수가 적어(4~6) 순차
/// 호출이며 부서별 2초 timeout으로 hung 부서가 전체 함대를 막지 않는다.
#[tauri::command]
async fn org_fleet() -> Result<Value, String> {
    use std::time::Duration;
    // (소켓, name, display_name) — 본부 먼저, 그다음 depts.json 등록순.
    let mut targets: Vec<(std::path::PathBuf, String, String)> =
        vec![(default_socket(), "_hq".to_string(), "본부 · CEO".to_string())];
    if let Ok(reg) = list_depts() {
        if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
            for (name, meta) in depts {
                let sock = meta
                    .get("socket")
                    .and_then(|s| s.as_str())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| dept_socket_path(name));
                let disp = meta
                    .get("display_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or(name)
                    .to_string();
                targets.push((sock, name.clone(), disp));
            }
        }
    }
    let mut departments: Vec<Value> = Vec::new();
    for (sock, name, display_name) in targets {
        let slug = sock_slug(&sock);
        let socket_str = sock.to_string_lossy().to_string();
        // R-1 교정: 공유 풀(rpc_on) 대신 일회성 연결(rpc_oneshot) — timeout 취소가 풀을 오염시키지 않게.
        let call =
            tokio::time::timeout(Duration::from_secs(2), rpc_oneshot(&sock, "org.status", json!({})))
                .await;
        let base = json!({"name": name, "display_name": display_name,
                          "socket": socket_str, "socket_slug": slug});
        let entry = match call {
            Ok(Ok(status)) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert(
                    "surfaces".into(),
                    status.get("surfaces").cloned().unwrap_or_else(|| json!([])),
                );
                m.insert(
                    "paused".into(),
                    status.get("paused").cloned().unwrap_or(json!(false)),
                );
                o
            }
            Ok(Err(e)) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert("error".into(), json!(e));
                m.insert("surfaces".into(), json!([]));
                o
            }
            Err(_) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert("error".into(), json!("timeout"));
                m.insert("surfaces".into(), json!([]));
                o
            }
        };
        departments.push(entry);
    }
    Ok(json!({ "departments": departments }))
}

/// Tasks Control Center 실시간성: depts.json의 모든 부서 소켓에 이벤트 forwarder를 보장한다
/// (멱등 — 이미 도는 forwarder는 no-op). 앱 시작 시엔 기본 소켓 forwarder만 떠 있어(setup),
/// 이미 가동 중인 부서 데몬의 task.changed/status.changed가 UI로 안 흐를 수 있다 — 작업 탭이
/// 열릴 때 1회 호출해 전 부서 실시간 push를 보장한다.
#[tauri::command]
fn ensure_dept_forwarders(app: AppHandle) {
    if let Ok(reg) = list_depts() {
        if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
            for (name, meta) in depts {
                let sock = meta
                    .get("socket")
                    .and_then(|s| s.as_str())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| dept_socket_path(name));
                spawn_event_forwarder(app.clone(), sock);
            }
        }
    }
}

#[tauri::command]
async fn control_dashboard() -> Result<Value, String> {
    rpc("control.dashboard", json!({})).await
}

#[tauri::command]
async fn control_hw() -> Result<Value, String> {
    rpc("control.hw", json!({})).await
}

#[tauri::command]
async fn control_analytics(window: Option<String>) -> Result<Value, String> {
    rpc("control.analytics", json!({ "window": window })).await
}

#[tauri::command]
async fn control_skills(window: Option<String>) -> Result<Value, String> {
    rpc("control.skills", json!({ "window": window })).await
}

#[tauri::command]
async fn control_cost_baseline(window: Option<String>) -> Result<Value, String> {
    rpc("control.cost_baseline", json!({ "window": window })).await
}

#[tauri::command]
async fn control_alerts() -> Result<Value, String> {
    rpc("control.alerts", json!({})).await
}

#[tauri::command]
async fn control_weekly() -> Result<Value, String> {
    rpc("control.weekly", json!({})).await
}

#[tauri::command]
async fn control_sessions(window: Option<String>, redact: Option<bool>) -> Result<Value, String> {
    rpc("control.sessions", json!({ "window": window, "redact": redact })).await
}

#[tauri::command]
async fn control_session_detail(session_id: String) -> Result<Value, String> {
    rpc("control.session_detail", json!({ "session_id": session_id })).await
}

#[tauri::command]
async fn control_session_star(session_id: String, starred: bool, note: Option<String>) -> Result<Value, String> {
    rpc("control.session_star", json!({ "session_id": session_id, "starred": starred, "note": note })).await
}

#[tauri::command]
async fn learn_status() -> Result<Value, String> {
    rpc("learn.status", json!({})).await
}

#[tauri::command]
async fn create_surface(
    socket: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<Value, String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.create",
        json!({"cwd": cwd, "title": title, "rows": rows, "cols": cols}),
    )
    .await
}

/// 한글 IME 계측(디버그 전용): UI가 localStorage.cysImeDebug==="1"일 때만 호출 —
/// 입력 이벤트 시퀀스를 /tmp/cys-ime.log에 append해 유실 경로를 결정론으로 확정한다
/// (WKWebView 콘솔 접근이 어려운 환경의 실측 채널 · 2026-06-13 한글 4자→2자 유실 조사).
#[tauri::command]
fn log_ime(line: String) {
    use std::io::Write;
    // RC-10: /tmp 하드코딩 → OS중립 temp_dir(Windows엔 /tmp 없어 디버그 로그 무음 유실이던 것 수정).
    let log_path = std::env::temp_dir().join("cys-ime.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "{ts} {line}");
    }
}

/// IME 디버그 게이트(파일/환경변수): 릴리스 빌드엔 devtools가 없어 localStorage.cysImeDebug를
/// 최종 사용자가 켤 수 없다 → ~/.cys/ime-debug 파일 존재 또는 CYS_IME_DEBUG=1이면 계측 활성.
#[tauri::command]
fn ime_debug_enabled() -> bool {
    std::env::var("CYS_IME_DEBUG").map(|v| v == "1").unwrap_or(false)
        || cys::home_dir().join(".cys/ime-debug").exists()
}

#[tauri::command]
async fn send_input(socket: Option<String>, surface_id: u64, data: String) -> Result<(), String> {
    // human=true: T3-13 타이핑 가드의 신호 — UI 키 입력을 '사람'으로 표시해
    // 원격 주입이 사람의 미완성 입력을 오염시키지 못하게 한다
    rpc_on(
        &resolve_socket(&socket),
        "surface.send_text",
        json!({"surface_id": surface_id, "text": data, "quiet": true, "human": true}),
    )
    .await
    .map(|_| ())
}

/// 클립보드 이미지 붙여넣기(F): base64 이미지를 임시 파일로 저장하고 절대경로를 반환한다.
/// UI가 이 경로를 셸 인용해 PTY로 타이핑한다(iTerm2 동작 — 붙여넣기로 이미지 경로 주입).
#[tauri::command]
fn save_pasted_image(data_b64: String, ext: String) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|e| e.to_string())?;
    // ext는 UI가 MIME에서 유도(png/jpg/gif/webp) — 경로 조작 방지로 영숫자만 통과, 아니면 png.
    let safe_ext = if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.as_str()
    } else {
        "png"
    };
    let dir = std::env::temp_dir().join("cys-paste");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("paste-{ms}.{safe_ext}"));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 파일 트리 패널용 디렉토리 나열 — dirs 먼저, 이름순.
#[tauri::command]
fn list_dir(path: String) -> Result<Value, String> {
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (e.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    Ok(json!(entries
        .into_iter()
        .map(|(name, is_dir)| json!({"name": name, "is_dir": is_dir}))
        .collect::<Vec<_>>()))
}

/// 파일을 시스템 기본 앱으로 연다 (macOS open / Windows start).
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    // 실재하는 로컬 경로만 허용 — URL 스킴·존재하지 않는 문자열이 OS 런처에 닿지 않게
    std::fs::metadata(&path).map_err(|e| format!("not a local path: {e}"))?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&path).spawn();
    // explorer는 인자를 셸 파싱하지 않는다 — cmd /C start의 메타문자 주입 경로 제거
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(&path).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// HUD-2: 외부 URL HARD 화이트리스트 — https만·도메인 allowlist. 통과 시 Ok(spawn 없음·테스트 가능).
/// url crate 부재 → 수동 host 파싱(https:// strip → 첫 '/' 전 host, userinfo(@)·port(:) 제거 = 위장 host 차단).
/// 기본 목록은 코드 봉인, 사용자 도메인은 로컬 설정으로 확장(공개 배포에서 기관 도메인 하드코딩 제거):
/// ~/.cys/url-allow-hosts(줄당 1도메인 — GUI 경로) 또는 $CYS_URL_ALLOW_HOSTS(콤마 구분).
fn url_host_allowed(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| "https only".to_string())?;
    // authority는 첫 '/', '?'(query), '#'(fragment) 전까지(RFC 3986) — query/fragment 사칭 우회 차단.
    let authority = rest.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority); // userinfo(@) 제거 — 위장 host 차단
    let host = host.split(':').next().unwrap_or(host); // port 제거
    let extras = user_allow_hosts();
    if host_in_allowlist(host, &extras) {
        Ok(())
    } else {
        Err(format!("domain not allowed: {host}"))
    }
}

/// 순수 판정(테스트 핀) — 기본 allowlist + 사용자 확장 도메인, 정확일치 또는 서브도메인.
fn host_in_allowlist(host: &str, extras: &[String]) -> bool {
    const ALLOW: &[&str] = &["notebooklm.google.com", "github.com"];
    ALLOW
        .iter()
        .map(|d| *d)
        .chain(extras.iter().map(|s| s.as_str()))
        .any(|d| !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))))
}

/// 사용자 확장 allowlist — 파일(~/.cys/url-allow-hosts, 줄당 1개) ∪ env(콤마 구분).
/// 로컬 사용자 자신의 동의 하에 자기 머신에서만 확장된다(원격 주입 경로 없음).
fn user_allow_hosts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(s) = std::fs::read_to_string(cys::home_dir().join(".cys/url-allow-hosts")) {
        out.extend(s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')));
    }
    if let Ok(env) = std::env::var("CYS_URL_ALLOW_HOSTS") {
        out.extend(env.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    }
    out
}

/// HUD-2: SOT 근거 URL을 시스템 브라우저로 연다 — 화이트리스트 통과 https만(비가역 외부개방의 최후 게이트).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    url_host_allowed(&url)?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// D5: cys 사이드카 바이너리 해소 — exe 옆(production 번들) 우선, 없으면 PATH 폴백(ensure_daemon 패턴).
fn resolve_sidecar(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

// ── CLI PATH 설치(명시 메뉴) — 가드/스크립트 순수 헬퍼 ─────────────────
#[derive(PartialEq, Debug)]
enum BundleKind {
    Canonical,    // /Applications/cys.app 또는 ~/Applications/cys.app
    Translocated, // Gatekeeper AppTranslocation 휘발 경로
    Backup,       // cys.app.bak-*/*.prev*
    NonStandard,  // 그 외(Downloads 등) — 경고와 함께 진행
}

/// 셸 단일따옴표 이스케이프(경로의 공백·특수문자·따옴표 안전).
fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// AppleScript 문자열 리터럴 이스케이프(큰따옴표). `osascript`의 `do shell script`는
/// 작은따옴표가 아니라 **큰따옴표 리터럴**을 요구한다 — 백슬래시·큰따옴표만 이스케이프하면 되고,
/// 내부 셸 경로 인용은 sh_squote(작은따옴표)가 따로 담당한다.
fn applescript_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `<bundle>/Contents/MacOS` 디렉토리를 분류한다.
fn classify_bundle_dir(macos_dir: &std::path::Path) -> BundleKind {
    let s = macos_dir.to_string_lossy();
    if s.contains("/AppTranslocation/") {
        return BundleKind::Translocated;
    }
    // macos_dir = <bundle>.app/Contents/MacOS → bundle = parent.parent
    let bundle = macos_dir.parent().and_then(|p| p.parent());
    if let Some(b) = bundle {
        let name = b
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with("cys.app.bak") || name.starts_with("cys.app.prev") {
            return BundleKind::Backup;
        }
        if name == "cys.app" {
            let parent = b
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if parent == "/Applications" || parent.ends_with("/Applications") {
                return BundleKind::Canonical;
            }
        }
    }
    BundleKind::NonStandard
}

/// `do shell script` 본문: target_dir 생성 + cys·cysd 심볼릭 멱등 생성(`ln -sf`).
fn build_install_script(
    cys: &std::path::Path,
    cysd: &std::path::Path,
    target_dir: &str,
) -> String {
    format!(
        "mkdir -p {td} && ln -sf {c} {tc} && ln -sf {d} {tdd}",
        td = sh_squote(target_dir),
        c = sh_squote(&cys.to_string_lossy()),
        tc = sh_squote(&format!("{target_dir}/cys")),
        d = sh_squote(&cysd.to_string_lossy()),
        tdd = sh_squote(&format!("{target_dir}/cysd")),
    )
}

/// `which -a cys` 출력 → precedence 순 경로 리스트(공백줄 제거).
fn parse_which_a(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 설치 계획(순수): 가드 판정 + 소스 경로 + osascript 인자 + 경고. osascript 실행은 포함하지 않는다.
struct CliInstallPlan {
    cys_src: std::path::PathBuf,
    cysd_src: std::path::PathBuf,
    osascript_arg: String, // `do shell script "..." with administrator privileges` (AppleScript 큰따옴표 리터럴)
    warnings: Vec<String>,
}

fn plan_cli_install(
    macos_dir: &std::path::Path,
    target_dir: &str,
) -> Result<CliInstallPlan, String> {
    match classify_bundle_dir(macos_dir) {
        BundleKind::Translocated => {
            return Err("cys.app이 Gatekeeper에 의해 임시 위치에서 실행 중입니다. \
Finder에서 cys.app을 Applications 폴더로 옮긴 뒤 다시 열고 시도하세요."
                .into());
        }
        BundleKind::Backup => {
            return Err("백업 번들에서 실행 중입니다. \
정규 cys.app(Applications)에서 실행한 뒤 시도하세요."
                .into());
        }
        BundleKind::Canonical | BundleKind::NonStandard => {}
    }
    let mut warnings = vec![];
    if classify_bundle_dir(macos_dir) == BundleKind::NonStandard {
        warnings.push(
            "cys.app이 표준 위치(Applications)가 아닌 곳에서 실행 중입니다. \
앱을 옮기면 심볼릭이 깨지니 Applications로 이동을 권장합니다."
                .into(),
        );
    }
    let cys_src = macos_dir.join("cys");
    let cysd_src = macos_dir.join("cysd");
    let script = build_install_script(&cys_src, &cysd_src, target_dir);
    // AppleScript `do shell script`는 큰따옴표 문자열 리터럴을 요구한다 — 작은따옴표로 감싸면
    // 실행 전 파스 단계에서 syntax error -2741로 거부된다(내부 셸 경로 인용은 build_install_script의
    // sh_squote가 담당). 따라서 바깥 래핑은 반드시 applescript_str(큰따옴표)여야 한다.
    let osascript_arg = format!(
        "do shell script {} with administrator privileges",
        applescript_str(&script)
    );
    Ok(CliInstallPlan {
        cys_src,
        cysd_src,
        osascript_arg,
        warnings,
    })
}

#[derive(serde::Serialize)]
struct InstallCliReport {
    ok: bool,
    target_dir: String,
    cys_link: String,
    cysd_link: String,
    source_cys: String,
    effective_cys: Option<String>, // which -a cys 1순위
    shadowed_by: Option<String>,   // /usr/local/bin/cys 앞을 가리는 다른 cys
    warnings: Vec<String>,
}

/// 명시 메뉴 트리거. macOS에서 cys·cysd를 /usr/local/bin에 1회 승격으로 심볼릭한다.
#[tauri::command]
fn install_cli_to_path() -> Result<InstallCliReport, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("이 기능은 macOS 전용입니다.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let target_dir = "/usr/local/bin";
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let macos_dir = exe
            .parent()
            .ok_or("번들 디렉토리 해석 실패")?
            .to_path_buf();

        let plan = plan_cli_install(&macos_dir, target_dir)?;
        if !plan.cys_src.exists() || !plan.cysd_src.exists() {
            return Err("번들 내 cys/cysd 바이너리를 찾지 못했습니다.".into());
        }

        // osascript 1회 승격(cys·cysd 동시 → 단일 프롬프트).
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&plan.osascript_arg)
            .output()
            .map_err(|e| format!("osascript 실행 실패: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") || err.contains("User canceled") {
                return Err("설치가 취소되었습니다.".into());
            }
            return Err(format!("심볼릭 생성 실패: {}", err.trim()));
        }

        // 검증: 로그인 PATH 기준 which -a cys.
        let which = std::process::Command::new("bash")
            .arg("-lc")
            .arg("which -a cys")
            .output()
            .ok();
        let entries = which
            .as_ref()
            .map(|o| parse_which_a(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        let effective_cys = entries.first().cloned();
        let target_cys = format!("{target_dir}/cys");
        let shadowed_by = match &effective_cys {
            Some(p) if *p != target_cys => Some(p.clone()),
            _ => None,
        };

        let mut warnings = plan.warnings;
        if let Some(sh) = &shadowed_by {
            warnings.push(format!(
                "PATH 선행 위치의 다른 cys가 우선합니다: {sh} \
(예: dev deploy_gate의 /opt/homebrew/bin). 새로 설치한 {target_cys}는 그 뒤에 있습니다."
            ));
        }

        Ok(InstallCliReport {
            ok: true,
            target_dir: target_dir.to_string(),
            cys_link: target_cys,
            cysd_link: format!("{target_dir}/cysd"),
            source_cys: plan.cys_src.to_string_lossy().to_string(),
            effective_cys,
            shadowed_by,
            warnings,
        })
    }
}

/// 업데이트 재시작 후 자동복귀 마커 경로 — install_update(재시작 직전)가 쓰고, 재시작된 cys-app
/// setup이 읽는다. 두 프로세스가 공유하는 ~/.cys 아래에 둔다.
fn pending_restore_path() -> std::path::PathBuf {
    cys::home_dir().join(".cys/.pending-restore")
}

/// 업데이트로 인한 재시작이면(마커 존재) 두 가지를 한다:
///  ① 새 기능 배포 — 새 cys 바이너리에 embed된 팩(pack.rs include_str! + build.rs PACK_SKILLS)을
///     `cys init-pack --no-install-hook`으로 ~/.cys/pack에 반영한다. --no-install-hook: hook 등록은
///     최초 설치/launch-agent에서 끝나므로 매 업데이트마다 settings.json을 건드리지 않는다(.bak-cys
///     백업 파괴·활성 프로필 재직렬화 방지 — 적대검증 serious). force 없이 호출하므로 preserve-gate가
///     사용자 수정 파일을 보존하고 비수정·신규만 갱신한다.
///  ② 자동복귀 — 팩 반영 성공 시에만 `cys restore --include-master`로 노드를 session_id resume
///     재런칭(작업 무손실). init-pack 실패 시 마커를 보존하고 복원을 보류해, 노드가 구 디렉티브로
///     조용히 각성하는 침묵 실패를 막는다(적대검증 fatal). restore는 멱등(run_restore cys.rs:3791).
fn maybe_apply_pending_update(app: &AppHandle) {
    let marker = pending_restore_path();
    if !marker.exists() {
        return;
    }
    // ① 새 팩(새 기능) 반영 — 성공 여부를 검사한다(침묵 실패 차단).
    let mut init_cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
    init_cmd.arg("init-pack").arg("--no-install-hook");
    no_console(&mut init_cmd);
    let pack_ok = init_cmd
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !pack_ok {
        // 실패 — 마커를 보존(다음 재시작에 재시도)하고 노드 복원을 보류한다. 구 디렉티브로
        // 조용히 각성하는 것을 막고 사용자에게 알린다.
        let _ = app.emit(
            "update-error",
            "새 팩 반영(init-pack) 실패 — 노드 복원 보류, 다음 재시작에 재시도",
        );
        return;
    }
    // 성공 후에만 마커 제거 + 자동복귀.
    let _ = std::fs::remove_file(&marker);
    let mut restore_cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
    restore_cmd.arg("restore").arg("--include-master");
    no_console(&mut restore_cmd);
    let _ = restore_cmd.spawn();
}

/// D5/P1: UI 발 키 전송 — surface.send_key RPC 래퍼. send_input(send_text)과 달리 Return 등 키 전송 가능.
/// human 플래그 미사용(데몬 send_key 핸들러는 전부 프로그램 경로 — 읽지 않음).
#[tauri::command]
async fn send_key(socket: Option<String>, surface_id: u64, key: String) -> Result<(), String> {
    rpc_on(&resolve_socket(&socket), "surface.send_key",
        json!({"surface_id": surface_id, "key": key})).await.map(|_| ())
}

/// D5/SB-1: 스킬 버튼 보드 카탈로그 읽기(pack/board-catalog.json) — 정적 파일 read(데몬 무변경).
#[tauri::command]
fn read_board_catalog() -> Result<Value, String> {
    let path = cys::pack::pack_dir().join("board-catalog.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("board-catalog.json 없음 ({}): {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("카탈로그 파싱 실패: {e}"))
}

/// D6: 청중 프로파일(~/.cys/profile.json·사용자 로컬·pack 밖) audience 읽기 — 없으면 "custom"(전체보기 폴백·안전).
fn read_profile_audience() -> String {
    let path = cys::home_dir().join(".cys/profile.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("audience").and_then(|a| a.as_str()).map(String::from))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "custom".to_string())
}

/// D5: 무계약 차단의 결정론 강제점 — task-prompt 티켓(성공기준·4규칙)을 생성한다(UI가 직접 워커에 명령 못 함).
/// --no-survival-gate(B2): fresh 경로는 surface를 실행 시점에 만들므로 지금 워커 생존 확인 불요.
/// D6: 청중 프로파일 audience를 scope에 주입 — 스킬이 Implications Domain 질문을 건너뛴다(custom=전체보기).
#[tauri::command]
fn make_ticket(task: String, scope: String, success: String, to: String) -> Result<String, String> {
    let script = cys::pack::pack_dir().join("bin").join("javis_orchestra.py");
    let out_fmt = "산출물을 ~/.cys/_round/skill-out/<작업slug>/ (절대경로) 아래에 저장하라(결정론 회수 위치·SB-6). \
                   산출물에 '🔒 AI 보조 생성 · 오너 검수 전' 신뢰선 라벨을 부착하라(과대약속 금지).";
    let audience = read_profile_audience();
    let scope_full = if audience != "custom" {
        format!("{scope} · 청중 프로파일: {audience}(이 청중 맞춤으로 산출·Implications Domain 질문 생략)")
    } else {
        scope.clone()
    };
    let mut orch_cmd = std::process::Command::new("python3");
    inject_runtime_path(&mut orch_cmd); // RC-5: 동봉 runtime(python3.exe) PATH 주입
    orch_cmd
        .arg(&script)
        .arg("task-prompt")
        .args(["--task", &task, "--scope", &scope_full, "--success", &success, "--to", &to])
        .arg("--no-survival-gate")
        .args(["--output-format", out_fmt]);
    no_console(&mut orch_cmd);
    let output = orch_cmd
        .output()
        .map_err(|e| format!("javis_orchestra 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err(format!("task-prompt 실패: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// D5/SB-2: 보이는 일회용 워커로 스킬 실행 — cys skill run(schedule --fresh) spawn(새 RPC 0·invisible -p 금지).
#[tauri::command]
fn run_skill(name: String, ticket: String, agent: Option<String>, close_after: Option<u64>) -> Result<Value, String> {
    if ticket.trim().is_empty() {
        return Err("ticket 비어 있음 — 무계약 실행 금지".into());
    }
    let cys = resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" });
    let mut cmd = std::process::Command::new(&cys);
    cmd.arg("skill").arg("run").arg(&name)
        .args(["--ticket", &ticket])
        .args(["--agent", agent.as_deref().unwrap_or("claude")]);
    if let Some(ca) = close_after {
        cmd.args(["--close-after", &ca.to_string()]);
    }
    cmd.stdin(std::process::Stdio::null());
    no_console(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("cys skill run 실행 실패 ({}): {e}", cys.display()))?;
    Ok(json!({"ok": true, "name": name}))
}

/// D5/SB-6: 산출물 회수 결정론 위치(~/.cys/_round/skill-out) — make_ticket output_format과 정합.
#[tauri::command]
fn skill_out_dir() -> String {
    cys::home_dir()
        .join(".cys/_round/skill-out")
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
async fn rename_surface(socket: Option<String>, surface_id: u64, title: String) -> Result<(), String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.rename",
        json!({"surface_id": surface_id, "title": title}),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
async fn resize_surface(
    socket: Option<String>,
    surface_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.resize",
        json!({"surface_id": surface_id, "rows": rows, "cols": cols}),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
async fn close_surface(
    state: State<'_, Attachments>,
    socket: Option<String>,
    surface_id: u64,
) -> Result<(), String> {
    let sock = resolve_socket(&socket);
    let key = (sock_slug(&sock), surface_id);
    if let Some(handle) = state.0.lock().unwrap().remove(&key) {
        handle.abort();
    }
    rpc_on(&sock, "surface.close", json!({"surface_id": surface_id}))
        .await
        .map(|_| ())
}

#[tauri::command]
async fn feed_list(status: Option<String>) -> Result<Value, String> {
    rpc("feed.list", json!({"status": status})).await
}

#[tauri::command]
async fn feed_reply(request_id: String, decision: String) -> Result<(), String> {
    rpc(
        "feed.reply",
        json!({"request_id": request_id, "decision": decision}),
    )
    .await
    .map(|_| ())
}

/// Attach: 부서 소켓의 surface PTY 출력을 base64 이벤트로 webview에 스트리밍.
/// 이벤트명은 (소켓 slug, surface_id)로 데몬 간 충돌을 막고, 그 이름을 반환해 UI가 구독한다
/// (백엔드 단일 진실 — UI 독립 재계산 금지, 검증 mustFix).
#[tauri::command]
async fn attach_surface(socket: Option<String>, surface_id: u64) -> Result<Value, String> {
    // 이벤트명만 반환 — 실제 스트림은 start_surface_stream이 시작한다. UI가 이 이름으로 listen을
    // 먼저 등록한 뒤 start를 호출해야, 데몬이 attach 직후 보내는 초기 화면 snapshot(프롬프트)이
    // listen 등록 전에 emit돼 유실되는 race(런치 시 첫 pane 빈 화면)를 차단한다.
    let sock = resolve_socket(&socket);
    let slug = sock_slug(&sock);
    Ok(json!({
        "output_event": format!("surface-output-{slug}-{surface_id}"),
        "exited_event": format!("surface-exited-{slug}-{surface_id}"),
    }))
}

/// 실제 PTY 스트림 시작 — 이전 핸들 abort + connect + surface.attach + 초기 화면 snapshot + live 스트림.
/// UI는 attach_surface로 이벤트명을 받아 listen을 등록한 뒤 이 명령을 호출한다(snapshot 유실 방지).
#[tauri::command]
async fn start_surface_stream(
    app: AppHandle,
    state: State<'_, Attachments>,
    socket: Option<String>,
    surface_id: u64,
) -> Result<(), String> {
    let sock = resolve_socket(&socket);
    let slug = sock_slug(&sock);
    let key = (slug.clone(), surface_id);
    if let Some(prev) = state.0.lock().unwrap().remove(&key) {
        prev.abort();
    }
    let event_name = format!("surface-output-{slug}-{surface_id}");
    let event_exited = format!("surface-exited-{slug}-{surface_id}");
    let (en, ee) = (event_name.clone(), event_exited.clone());
    let handle = tauri::async_runtime::spawn(async move {
        let Ok(mut stream) = connect_to(&sock).await else {
            let _ = app.emit(&ee, ());
            return;
        };
        let req =
            json!({"id": 1, "method": "surface.attach", "params": {"surface_id": surface_id}});
        let mut line = serde_json::to_vec(&req).unwrap_or_default();
        line.push(b'\n');
        if stream.write_all(&line).await.is_err() {
            let _ = app.emit(&ee, ());
            return;
        }
        let mut reader = BufReader::new(stream);
        let mut ack = String::new();
        // ack 검증 — not_found 등 에러 ack에서 read 블록·무신호 죽은 pane이 되지 않게
        if reader.read_line(&mut ack).await.unwrap_or(0) == 0 {
            let _ = app.emit(&ee, ());
            return;
        }
        let ack_v: Value = serde_json::from_str(ack.trim()).unwrap_or(Value::Null);
        if ack_v["ok"].as_bool() != Some(true) {
            let _ = app.emit(&ee, ());
            return;
        }
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if app.emit(&en, b64).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = app.emit(&ee, ());
    });
    state.0.lock().unwrap().insert(key, handle);
    Ok(())
}

/// 데몬 소켓이 준비될 때까지 connect를 폴링(수동 spawn 없음). `attempts`×100ms.
async fn wait_for_connect(attempts: u32) -> bool {
    for _ in 0..attempts {
        if connect().await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// 앱 첫 실행 시 cysd를 launchd에 자동등록(RunAtLoad·KeepAlive) — 재부팅 후에도 데몬 생존.
/// 수동 `cys daemon install`의 opt-in을 자동화한다(`cys::launchd`와 plist 포맷 단일화).
/// 반환값 = **launchd가 cysd 기동을 책임지는가**. true면 setter가 수동 spawn을 건너뛰고
/// launchd-owned cysd의 socket-ready를 폴링해야 한다(중복 spawn·flock 경합 방지 — codex BLOCKER).
#[cfg(target_os = "macos")]
async fn maybe_autoregister_launchd() -> bool {
    // 번들 동봉 cysd 절대경로(ensure_daemon과 동일 규칙) — 형제 cysd가 없으면 보류.
    let daemon = match std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("cysd")))
    {
        Some(p) if p.exists() => p,
        _ => return false,
    };
    let running = connect().await.is_ok();
    match cys::launchd::register_if_absent(&daemon, running) {
        Ok(outcome) => {
            eprintln!("[cys-app] launchd autoregister: {outcome:?}");
            cys::launchd::launchd_will_serve(outcome)
        }
        Err(e) => {
            eprintln!("[cys-app] launchd autoregister skipped: {e}");
            false
        }
    }
}

/// 첫 기동 온보딩 공용 단계 — `cys init-pack`으로 팩 파일 + Claude SessionStart hook 등록.
/// install은 preserve, hook은 중복 dedup(already→skip·.bak-cys 무변경)이라 **멱등** — 매 기동
/// 실행해도 안전(팩 삭제 시 자가치유). Windows·macOS 온보딩이 공유한다(autostart는 OS별로 분리:
/// Windows=schtasks·macOS=launchd). best-effort — 실패해도 세션은 진행.
#[cfg(any(windows, target_os = "macos"))]
fn onboard_init_pack(cys: &std::path::Path) {
    let mut init = std::process::Command::new(cys);
    init.arg("init-pack");
    no_console(&mut init);
    match init.status() {
        Ok(s) if s.success() => eprintln!("[cys-app] onboarding: init-pack ok"),
        Ok(s) => eprintln!("[cys-app] onboarding: init-pack exited {s}"),
        Err(e) => eprintln!("[cys-app] onboarding: init-pack spawn failed: {e}"),
    }
}

/// Windows 첫 기동 온보딩(RC-1) — 순정 Windows엔 hook 자동등록 경로가 없어 "너는 마스터다"
/// 부트스트랩(SessionStart hook)이 미발동했다(T1 증상①).
/// ① `onboard_init_pack`: 팩 + Claude hook 등록(멱등).
/// ② `cys daemon install`: 기존 schtasks ONLOGON 자동기동 등록 재사용(cys.rs:3139·/F 멱등).
#[cfg(windows)]
fn maybe_windows_onboard() {
    let cys = resolve_sidecar("cys.exe");
    onboard_init_pack(&cys);
    // ② autostart 등록 (기존 cys daemon install = schtasks ONLOGON 재사용, /F 멱등)
    let mut reg = std::process::Command::new(&cys);
    reg.arg("daemon").arg("install");
    no_console(&mut reg);
    match reg.status() {
        Ok(s) if s.success() => eprintln!("[cys-app] windows onboarding: daemon install (schtasks) ok"),
        Ok(s) => eprintln!("[cys-app] windows onboarding: daemon install exited {s}"),
        Err(e) => eprintln!("[cys-app] windows onboarding: daemon install spawn failed: {e}"),
    }
}

/// macOS 첫 기동 온보딩 — Windows 온보딩의 대칭(RC-17·T5). macOS DMG 소비자는 launchd
/// 자동시작(maybe_autoregister_launchd)만 있고 hook 자동등록 경로가 없어 "너는 마스터다"
/// 부트스트랩이 미발동했다. autostart는 launchd가 담당하므로 여기서는 Windows와 대칭으로
/// 팩+Claude hook만 등록한다. init-pack 멱등 — 기존 사용자에 재실행돼도 무해(already→skip·.bak-cys 불변).
#[cfg(target_os = "macos")]
fn maybe_macos_onboard() {
    let cys = resolve_sidecar("cys");
    onboard_init_pack(&cys);
}

/// Windows: GUI(windows_subsystem)가 콘솔 바이너리(cys/cysd/python3)를 스폰할 때 콘솔 창이
/// 뜨지 않게 CREATE_NO_WINDOW 를 붙인다(검은 빈 Windows Terminal 창·ConPTY 오염 방지). 타 OS 무동작.
fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// RC-5: GUI 직스폰(bash/python3)에 동봉 runtime PATH 주입. GUI(Explorer/Finder) 프로세스 PATH엔
/// runtime이 없어 순정 Windows서 bash/python3 lookup 실패 → ＋부서·티켓 무반응이었다(cysd PTY 자식만
/// 주입 수혜). cysd와 동일한 공용 로직(cys::runtime_prefixed_path) 사용 — 중복 구현 금지.
/// 타 OS는 exe_dir만 얹혀 사실상 무영향(제거 없음).
fn inject_runtime_path(cmd: &mut std::process::Command) {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        let cur = std::env::var("PATH").unwrap_or_default();
        if let Some(newp) = cys::runtime_prefixed_path(&exe_dir, &cur) {
            cmd.env("PATH", newp);
        }
    }
}

/// Ensure aitermd is running: try to connect, otherwise spawn the bundled/sibling binary.
async fn ensure_daemon() -> Result<(), String> {
    if connect().await.is_ok() {
        return Ok(());
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let daemon_name = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    let candidate = exe_dir.as_ref().map(|d| d.join(daemon_name));
    let program = match candidate {
        Some(p) if p.exists() => p,
        _ => std::path::PathBuf::from(daemon_name), // fall back to PATH
    };
    let mut command = std::process::Command::new(&program);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    no_console(&mut command);
    command
        .spawn()
        .map_err(|e| format!("failed to start cysd ({}): {e}", program.display()))?;
    if wait_for_connect(40).await {
        Ok(())
    } else {
        Err("cysd did not come up within 4s".into())
    }
}

/// Background: 한 데몬의 push 이벤트 스트림을 구독해 webview로 전달.
/// 데몬별 event forwarder 중복 spawn 방지 — restore가 ws마다 launch_dept_daemon을 재호출해도
/// socket당 forwarder 1개만 유지(태스크 누수·daemon-event 중복 emit 차단).
static FORWARDERS: std::sync::OnceLock<Mutex<std::collections::HashSet<std::path::PathBuf>>> =
    std::sync::OnceLock::new();

/// 데몬마다 spawn — 페이로드에 출처 socket_slug를 주입해 UI가 부서를 구분한다(멀티마스터 F3).
fn spawn_event_forwarder(app: AppHandle, socket: std::path::PathBuf) {
    // 멱등 가드: 이 socket의 forwarder가 이미 돌고 있으면 no-op.
    {
        let set = FORWARDERS.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
        if !set.lock().unwrap().insert(socket.clone()) {
            return;
        }
    }
    let slug = sock_slug(&socket);
    tauri::async_runtime::spawn(async move {
        let mut after_seq: Option<u64> = None;
        let mut fails: u32 = 0;
        loop {
            let mut connected = false;
            let attempt: Result<(), String> = async {
                let mut stream = connect_to(&socket).await?;
                connected = true; // 연결 수립 — dead-socket 아님
                let req = json!({"id": 1, "method": "events.stream",
                                 "params": {"after_seq": after_seq}});
                let mut line = serde_json::to_vec(&req).unwrap_or_default();
                line.push(b'\n');
                stream.write_all(&line).await.map_err(|e| e.to_string())?;
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&l) {
                        if v["type"] == "event" {
                            if let Some(seq) = v["seq"].as_u64() {
                                after_seq = Some(seq);
                            }
                            if let Some(obj) = v.as_object_mut() {
                                obj.insert("socket_slug".into(), json!(slug));
                            }
                            let _ = app.emit("daemon-event", v);
                        }
                    }
                }
                Err("event stream closed".into())
            }
            .await;
            let _ = attempt;
            // dead-socket 회수: 연속 연결 실패(스트림 수립 실패)가 ~30s 넘으면 forwarder 종료.
            // 스트림 수립 후 종료(데몬 재시작 등)는 정상 재연결 대상이라 카운터를 리셋한다.
            if connected {
                fails = 0;
            } else {
                fails += 1;
                if fails >= 30 {
                    if let Some(set) = FORWARDERS.get() {
                        set.lock().unwrap().remove(&socket);
                    }
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// 부서 운용 정식 도구 cys-dept 경로(pack_dir/bin/cys-dept).
fn dept_tool() -> std::path::PathBuf {
    cys::pack::pack_dir().join("bin").join("cys-dept")
}

/// 부서 데몬 소켓 경로 — RC-4: 공용 규약(cys::dept_socket_path)에 위임.
/// Windows=named pipe `\\.\pipe\cys-dept-<name>`, unix=~/.local/state/cys-dept-<name>/cys.sock.
/// (구: HOME 직접사용 unix .sock 고정 → Windows named pipe 미대응·HOME 미설정 이중결함 RC-4/RC-7.)
fn dept_socket_path(name: &str) -> std::path::PathBuf {
    cys::dept_socket_path(name)
}

/// 새 부서 workspace 런칭 = 부서 데몬 spawn. 단일 진입점 cys-dept launch를 OS 호출해
/// 레지스트리·ACL 시드·CEO 승격을 일임한다(직접 cysd spawn 금지, 검증 mustFix). 성공 시
/// 그 데몬용 이벤트 forwarder를 추가 spawn하고 socket·slug·identify를 반환한다.
#[tauri::command]
async fn launch_dept_daemon(app: AppHandle, name: String) -> Result<Value, String> {
    let tool = dept_tool();
    let n = name.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("launch").arg(&n);
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    let sock = dept_socket_path(&name);
    spawn_event_forwarder(app.clone(), sock.clone());
    let mut info = rpc_on(&sock, "system.identify", json!({"caller": "ui"})).await?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("socket".into(), json!(sock.to_string_lossy()));
        obj.insert("socket_slug".into(), json!(sock_slug(&sock)));
    }
    Ok(info)
}

/// 새 부서 번호 백엔드 원자 발급. 번호 계산을 UI가 아닌 레지스트리 flock RMW에 일임해
/// lowest-unused 재사용 + 멀티창 충돌0을 보장한다. stdout 마지막 줄이 확정 name(dept-N).
/// ＋부서 자동화(패치5): `catalog_key`=Some(k) → `cys-dept create <k>`(카탈로그 기반 부서명·계정·미션·각성),
/// None → `cys-dept allocate`(레거시 무변경). create 경로는 레지스트리에서 display_name 을 조회해 반환한다.
#[tauri::command]
async fn allocate_dept_daemon(app: AppHandle, catalog_key: Option<String>) -> Result<Value, String> {
    let tool = dept_tool();
    let ck = catalog_key.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool);
        match &ck {
            Some(k) => {
                cmd.arg("create").arg(k);
            } // ＋부서 자동화: 카탈로그 키 기반 생성(stdout 마지막 줄=name)
            None => {
                cmd.arg("allocate");
            } // 레거시: 번호만 발급(회귀 무변경)
        }
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        // ＋부서 자동화(gemini R2 ①): create 경로는 exit code 를 'dept-create:<code>:<stderr>' 로 GUI 에 전달해
        //   보안 분기를 가능케 한다 — exit5(account dir 미존재)=계정누수 → 레거시 폴백 절대 금지(하드 에러)·
        //   exit4(키 부재)=에러·exit3(카탈로그 부재)=레거시 허용. 레거시 allocate(None) 경로는 평문 stderr 유지.
        if catalog_key.is_some() {
            let code = out.status.code().unwrap_or(-1);
            return Err(format!("dept-create:{code}:{stderr}"));
        }
        return Err(stderr);
    }
    let name = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("allocate: empty name".into());
    }
    let sock = dept_socket_path(&name);
    spawn_event_forwarder(app.clone(), sock.clone());
    let mut info = rpc_on(&sock, "system.identify", json!({"caller": "ui"})).await?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("socket".into(), json!(sock.to_string_lossy()));
        obj.insert("socket_slug".into(), json!(sock_slug(&sock)));
        obj.insert("name".into(), json!(name));
        // ＋부서 자동화: create 경로면 레지스트리(cys-dept reg_set_meta 가 기록)에서 display_name 조회 →
        // 탭 표시명. create stdout 은 name only(cys-dept 코어 재구현 금지)이므로 depts.json 이 표시명 진실원.
        if catalog_key.is_some() {
            if let Some(disp) = dept_display_name(&name) {
                obj.insert("display_name".into(), json!(disp));
            }
        }
    }
    Ok(info)
}

/// 부서 workspace 닫기 = 부서 데몬 teardown. cys-dept down에 일임(SIGTERM·소켓 정리·레지스트리·CEO 강등).
#[tauri::command]
async fn stop_dept_daemon(name: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("down").arg(&name);
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    Ok(())
}

/// 부서 레지스트리(depts.json) 조회 — restore가 등록된 부서(진실원)와 대조해 죽은 socket의 유령 ws를
/// 무비판 재-launch하지 않게 한다(옛 테스트 잔재·삭제된 부서 차단). 부재 시 빈 depts.
#[tauri::command]
fn list_depts() -> Result<Value, String> {
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir().join(".cys/depts.json")
        });
    match std::fs::read_to_string(&reg) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()),
        Err(_) => Ok(json!({ "depts": {} })),
    }
}

/// 부서 레지스트리(depts.json)에서 표시명 조회 — cys-dept reg_set_meta 가 기록한 display_name.
/// create stdout 은 name only 이므로 표시명의 진실원은 레지스트리다. 부재/오류 시 None(=name 폴백).
fn dept_display_name(name: &str) -> Option<String> {
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir().join(".cys/depts.json")
        });
    let s = std::fs::read_to_string(&reg).ok()?;
    let v: Value = serde_json::from_str(&s).ok()?;
    v.get("depts")?
        .get(name)?
        .get("display_name")?
        .as_str()
        .map(|s| s.to_string())
}

/// 부서 카탈로그(dept-catalog.json) 조회 — ＋부서 선택 팝업용. cys-dept 와 동일 경로 규약
/// (CYS_DEPT_CATALOG 또는 $HOME/.cys/dept-catalog.json). 부재/손상 시 빈 departments 반환(팝업=레거시 폴백).
#[tauri::command]
fn read_dept_catalog() -> Result<Value, String> {
    let cat = std::env::var("CYS_DEPT_CATALOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir()
                .join(".cys/dept-catalog.json")
        });
    match std::fs::read_to_string(&cat) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()),
        Err(_) => Ok(json!({ "departments": {} })),
    }
}

/// 부서 데몬 teardown(socket 기준) — ws 이름 변경(rename)으로 name→socket 매핑이 끊겨도 정확히 종료.
/// cys-dept down-sock에 일임(레지스트리 역인덱스로 부서명 해석 후 teardown).
#[tauri::command]
async fn stop_dept_daemon_by_socket(socket: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("down-sock").arg(&socket);
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    Ok(())
}

/// 업데이트 확인: 새 버전이 있으면 (version, notes)를 반환, 없으면 null.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<Value>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(update) => Ok(Some(json!({
            "version": update.version,
            "current": update.current_version,
            "notes": update.body,
        }))),
        None => Ok(None),
    }
}

/// 기본 원격 pack-manifest.json URL — tauri.conf updater endpoint(latest.json)와 같은
/// release 'latest' 자산에 동봉된다(release.yml이 함께 업로드, DESIGN §5 파일맵).
fn default_pack_manifest_url() -> String {
    // Phase 2 릴리스 통합(2026-07-03): 배포 원본 = 공개 소스 repo. 구 repo는 전환기 미러.
    "https://github.com/idoforgod/cys-terminal/releases/latest/download/pack-manifest.json"
        .to_string()
}

/// 무중단 팩 업데이트 가용성 확인(DESIGN §7-④ 3축 게이트) — 원격 pack-manifest.json만 경량
/// 페치(curl)해 디스크 `.pack-version` 및 실행 바이너리 버전과 비교한다. ★pack.tar.gz·서명은
/// 받지 않는다(폴링 비용 최소화) — 실제 다운로드·서명검증·원자적 반영·reinject는
/// install_pack_update(사이드카 cys pack-update)가 전담한다(불가침).
/// 반환(★3상태 — UI가 'transient 장애'와 '확인된 no-update'를 구분해 fail-safe 상태보존):
///   - Ok(Some({pack_version, manifest_url, min_binary_version, binary_too_old}))
///       → 확인된 새 팩 있음. binary_too_old=false=무중단 가능(install_pack_update 경로) /
///         true=min_binary_version > 실행 바이너리 = 무중단 거부, 바이너리(재시작) 경로 안내.
///   - Ok(None)  → ① 정상 no-update(원격을 받아·파싱해 비교했고 디스크보다 새것이 아님) 또는
///                 ② 미서명/필수필드 부재 manifest의 fail-closed 거부(보안 경계 — 받았으나 신뢰 불가,
///                 설치 안 함). UI는 이때만 packUpdateAvailable을 해제한다(확인된 '새 팩 없음').
///   - Err(..)   → ★일시 fetch 장애(spawn/join·curl 실행·HTTP 비정상). UI의 기존 catch가
///                 packCheckFailed=true로 잡아 마지막 검증 상태를 보존하고 토스트는 띄우지 않는다
///                 (silent 폴링). '확인된 no-update'와 섞지 않는 게 핵심 — 일시 장애로
///                 packUpdateAvailable이 소거돼 배지가 사라지는 것을 막는다.
#[tauri::command]
async fn check_pack_update(manifest_url: Option<String>) -> Result<Option<Value>, String> {
    let url = manifest_url.unwrap_or_else(default_pack_manifest_url);
    // 경량 페치: manifest JSON만 stdout으로. blocking 풀에서 실행(install_pack_update curl 패턴 동형).
    let fetch_url = url.clone();
    // ★transient 실패(spawn/join·curl 실행·HTTP 비정상)는 Err로 돌린다 — UI catch가 상태보존(silent).
    //   Ok(None)으로 접으면 '확인된 no-update'와 구분 불가 → 일시 장애에 배지 소거(codex R2 #1).
    let joined = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-fsSL", &fetch_url]);
        // startup + 6시간 폴링마다 실행 — GUI(무콘솔)가 콘솔 자식(curl)을 그냥 스폰하면
        // Win11(기본터미널=WT)에서 검은 창이 깜빡인다. 첫 실행 flash의 단일 최우선 원인.
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    let out = match joined {
        Ok(Ok(out)) if out.status.success() => out,
        Ok(Ok(out)) => return Err(format!("pack-manifest HTTP 실패(code {:?})", out.status.code())),
        Ok(Err(e)) => return Err(format!("curl 실행 실패: {e}")),
        Err(e) => return Err(format!("curl join 실패: {e}")),
    };
    // 미서명/필수필드 부재 manifest = packsig PackManifest 역직렬화 fail-closed(거부) = 보안 경계.
    //   받았으나 신뢰 불가 → '새 팩 없음'으로 취급(Ok(None), 설치 안 함). fetch 장애(Err·상태보존)와
    //   달리 재시도해도 동일하므로 unknown이 아닌 확정 거부 — UI는 packUpdateAvailable을 해제한다.
    let manifest: cys::packsig::PackManifest = match serde_json::from_slice(&out.stdout) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    let disk = std::fs::read_to_string(cys::pack::pack_dir().join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // 축1 반영 판정: remote가 디스크보다 strictly-newer 여야. ★여기서 false면 '확인된 no-update' = Ok(None).
    if !cys::pack::remote_is_newer(&manifest.pack_version, &disk) {
        return Ok(None);
    }
    // 축2 호환 게이트: min_binary_version ≤ 실행 바이너리(env CARGO_PKG_VERSION = 단일 버전선).
    let binary_too_old = pack_binary_too_old(&manifest.min_binary_version, env!("CARGO_PKG_VERSION"));
    Ok(Some(json!({
        "pack_version": manifest.pack_version,
        "min_binary_version": manifest.min_binary_version,
        "manifest_url": url,
        "binary_too_old": binary_too_old,
    })))
}

/// 무중단 호환 게이트(DESIGN §7-④ 축2) 순수 판정 — min_binary_version > 실행 바이너리면 true(무중단
/// 거부=바이너리 경로). 빈 값=제약 없음(false), 어느 쪽이든 파싱 실패=거부(true, 보수적).
/// cys.rs version_gates의 호환 게이트와 동일 의미 — 단위테스트 대상.
fn pack_binary_too_old(min_binary: &str, running: &str) -> bool {
    let min = min_binary.trim();
    if min.is_empty() {
        return false;
    }
    match (cys::pack::parse_semver(min), cys::pack::parse_semver(running)) {
        (Some(m), Some(r)) => m > r,
        _ => true,
    }
}

/// 데몬 핸드오프 정책(오너 결정): 살아있는 세션 0개면 데몬 종료까지 자동,
/// 있으면 거부하고 세션 수를 알려 UI가 확인을 받게 한다(force=true면 강행).
/// 반환: 종료된 세션 수.
#[tauri::command]
async fn live_session_count() -> Result<u64, String> {
    let r = rpc("surface.list", json!({})).await?;
    let n = r["surfaces"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| !s["exited"].as_bool().unwrap_or(true))
                .count() as u64
        })
        .unwrap_or(0);
    Ok(n)
}

/// 업데이트 다운로드·설치 후 데몬 핸드오프 + 재시작.
/// force=false: 살아있는 세션이 있으면 설치 전에 거부(UI가 확인 후 force=true로 재호출).
#[tauri::command]
async fn install_update(app: AppHandle, force: bool) -> Result<(), String> {
    // 1) 세션 가드 (오너 정책: 없으면 자동·있으면 확인)
    let sessions = live_session_count().await.unwrap_or(0);
    if sessions > 0 && !force {
        return Err(format!("live_sessions:{sessions}"));
    }
    // 2) 업데이트 받아 설치 (.app 번들 교체 — 새 cysd/cys 동봉)
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no update available")?;
    let _ = app.emit("update-progress", json!({"phase": "download"}));
    update
        .download_and_install(
            |chunk, total| {
                let _ = app.emit(
                    "update-progress",
                    json!({"phase": "download", "chunk": chunk, "total": total}),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // 3) 데몬 핸드오프: 구 데몬을 정상 종료(SIGTERM — scoped 정리·소켓 제거)해야
    //    재시작 후 새 번들의 cysd가 뜬다. 종료 안 하면 구 데몬이 계속 세션을 들고 돈다.
    // drain(best-effort): 재시작 전 살아있는 노드에 저장 신호 + 유예를 준다. 노드 LLM 협조 의존이라
    // 무손실 보장은 아니며(마지막 미저장분은 손실 가능), 주 복원 경로는 재시작 후 resume이다.
    // spawn_blocking으로 tokio 워커 점유를 막는다(파일 내 launch_dept_daemon 패턴과 일치). cys drain은
    // 자체 watchdog(12s)로 hang 시에도 종료되므로 별도 timeout 없이 await해도 업데이트가 멈추지 않는다.
    let _ = app.emit("update-progress", json!({"phase": "drain"}));
    let _ = tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
        cmd.arg("drain");
        no_console(&mut cmd);
        cmd.status()
    })
    .await;
    let _ = app.emit("update-progress", json!({"phase": "handoff"}));
    // 재시작 후 자동복귀 예약 — 새 cys-app setup이 이 마커를 보고 cys restore로 노드를 resume 재런칭한다.
    let _ = std::fs::write(pending_restore_path(), "");
    stop_running_daemon().await;
    // 4) 앱 재시작 — setup의 ensure_daemon이 새 cysd를 자동 기동, maybe_restore_after_update가 노드 복원
    app.restart();
}

/// 데몬 세대교체(업데이트 없이) — Windows rename-swap 후 lame-duck 스큐(구 데몬 + 새 앱)의
/// 지연 핸드오프 완결(P2 스큐 배지의 짝). NSIS 경로는 install_update의 핸드오프 코드가 실행될 수
/// 없어(인스톨러가 앱을 죽임) 디스크만 새 버전·프로세스는 구 버전으로 남는다 — 이 command가
/// install_update 3~4단계를 업데이트 없이 재현한다: drain → 복귀 마커 → 구 데몬 종료 →
/// 디스크의 새 cysd 기동. app.restart()가 없어 setup이 다시 돌지 않으므로
/// maybe_apply_pending_update(팩 반영 + cys restore 노드 복원)를 여기서 직접 수행한다.
/// ★update-progress는 emit하지 않는다 — drain/handoff 페이즈가 UI "업데이트 설치" sticky 토스트를
/// 만드는데 이 경로엔 재시작이 없어 영구 잔류한다. 진행 표시는 UI(promptRotateDaemon) 토스트 담당.
/// force=false: 살아있는 세션이 있으면 거부(UI가 확인 후 force=true로 재호출) — install_update 가드 동형.
#[tauri::command]
async fn rotate_daemon(app: AppHandle, force: bool) -> Result<(), String> {
    let sessions = live_session_count().await.unwrap_or(0);
    if sessions > 0 && !force {
        return Err(format!("live_sessions:{sessions}"));
    }
    // drain(best-effort): 교대 전 살아있는 노드에 저장 신호 + 유예 (install_update 3단계 동형).
    let _ = tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
        cmd.arg("drain");
        no_console(&mut cmd);
        cmd.status()
    })
    .await;
    let _ = std::fs::write(pending_restore_path(), "");
    stop_running_daemon().await;
    ensure_daemon().await?;
    // init-pack이 blocking Command::status()라 blocking 풀에서 실행(위 drain 패턴과 일치).
    let app2 = app.clone();
    let _ = tokio::task::spawn_blocking(move || maybe_apply_pending_update(&app2)).await;
    Ok(())
}

/// P5: 무중단 팩 업데이트 UI 브리지(DESIGN-noshutdown-pack-update §2-②·§7-③/④).
/// UI "업데이트 버튼"이 호출 → `cys pack-update`(P4) 사이드카를 실행해 서명검증→디스크 반영→
/// 살아있는 노드 reinject를 시킨다. ★`app.restart()`를 **절대 호출하지 않는다** — cysd·cys-app·
/// 세션이 단 한 번도 죽지 않는 게 install_update(재시작)와의 핵심 차이(무중단).
/// 오케스트레이션은 cys(Rust)에 있고 cys CLI엔 AppHandle이 없으므로, **이 command가 사이드카를
/// 래핑**해(make_ticket/run_skill 패턴 동형) 성공 종료 후 자신이 `app.emit("pack-updated", …)`
/// 한다 — 프런트가 read_board_catalog 등 캐시 의존 호출을 재실행해 stale 캐시를 갱신(§2-② UI 브리지).
/// 인자: from(로컬 디렉터리) 우선, 없으면 manifest_url(원격) — cys pack-update의 --from/--manifest-url에 전달.
#[tauri::command]
async fn install_pack_update(
    app: AppHandle,
    manifest_url: Option<String>,
    from: Option<String>,
) -> Result<String, String> {
    let _ = app.emit("pack-progress", json!({"phase": "start"}));
    let cys = resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" });
    let mut cmd = std::process::Command::new(&cys);
    cmd.arg("pack-update");
    no_console(&mut cmd);
    match (&from, &manifest_url) {
        (Some(d), _) => {
            cmd.args(["--from", d]);
        }
        (None, Some(u)) => {
            cmd.args(["--manifest-url", u]);
        }
        (None, None) => return Err("from 또는 manifest_url 인자 필요".into()),
    }
    // 네트워크·디스크 작업이 길 수 있어 blocking 풀에서 실행(tokio 워커 점유 방지 — install_update drain 패턴).
    let out = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .map_err(|e| format!("pack-update join 실패: {e}"))?
        .map_err(|e| format!("cys pack-update 실행 실패 ({}): {e}", cys.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    // 종료코드 구분: EXIT_REINJECT_DEGRADED = 디스크 팩은 반영됐으나 라이브 노드 reinject 실패
    // (일부 미각성) — 디스크는 성공이므로 pack-updated를 emit하되 update-warning을 함께 띄운다.
    // 그 외 비0 = 실제 실패(디스크 미반영) → 구 캐시 유지가 안전하므로 update-error만.
    let degraded = out.status.code() == Some(cys::pack::EXIT_REINJECT_DEGRADED);
    if !out.status.success() && !degraded {
        // ★실패 — "pack-updated"는 emit하지 않는다(구 캐시 유지가 stale 갱신보다 안전). update-error만.
        let _ = app.emit(
            "update-error",
            json!({"phase": "pack-update", "message": stderr.clone()}),
        );
        return Err(format!("pack-update 실패: {stderr}"));
    }
    // ★디스크 반영 성공(success 또는 degraded) — .pack-version을 읽어 새 팩 버전으로 브로드캐스트(§2-②/§7-③).
    //   read_board_catalog가 pack_dir의 정적 파일을 읽는 것과 동일 SOT(pack_dir).
    let pack_version = std::fs::read_to_string(cys::pack::pack_dir().join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // 사이드카 구조화 출력에서 reinject failed/deferred 집계 — 라이브 미각성을 사용자에게 경고.
    let (failed, deferred) = parse_reinject_counts(&stdout);
    if failed > 0 || deferred > 0 {
        // ★성공으로만 포장 금지 — 디스크는 갱신됐으나 라이브 노드 일부 미각성/보류를 경고한다.
        //   (app.restart는 여전히 미호출 — 무중단 불변식 유지.)
        let _ = app.emit(
            "update-warning",
            json!({
                "phase": "pack-update",
                "pack_version": pack_version,
                "reinject_failed": failed,
                "reinject_deferred": deferred,
                "message": format!(
                    "디스크 팩은 {pack_version} 로 갱신됐으나 reinject {failed} 실패·{deferred} 보류 — \
                     일부 노드 미각성(라이브 무중단 유지, 재시작 안 함). 다음 pack-update에서 재시도됩니다."
                ),
            }),
        );
    }
    let _ = app.emit(
        "pack-updated",
        json!({
            "pack_version": pack_version,
            "reinject_failed": failed,
            "reinject_deferred": deferred,
        }),
    );
    Ok(pack_version)
}

/// 사이드카(cys pack-update) stdout에서 `PACK_UPDATE_RESULT … failed=N deferred=N` 토큰을 파싱해
/// (failed, deferred)를 돌려준다. 토큰 부재(구버전 사이드카·reinject 스킵 등)면 (0,0) — 보수적.
/// 사람용 메시지와 독립한 안정 토큰(REINJECT_RESULT_PREFIX)만 신뢰한다.
fn parse_reinject_counts(stdout: &str) -> (u64, u64) {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(cys::pack::REINJECT_RESULT_PREFIX) {
            let (mut failed, mut deferred) = (0u64, 0u64);
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("failed=") {
                    failed = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("deferred=") {
                    deferred = v.parse().unwrap_or(0);
                }
            }
            return (failed, deferred);
        }
    }
    (0, 0)
}

/// `ledger.list` 응답에서 scoped 프로세스 pid만 추린다.
/// windows 핸드오프(taskkill /F=TerminateProcess)는 데몬의 콘솔 이벤트 핸들러를
/// 못 깨워 shutdown_cleanup이 실행되지 않으므로, 데몬이 살아있는 동안 UI가
/// 직접 이 pid들을 ledger.kill로 회수해야 한다 (cysd shutdown_cleanup와 동일 선별).
/// (호출은 windows 경로 한정 — non-windows 빌드에선 테스트만 사용한다.)
#[cfg_attr(not(windows), allow(dead_code))]
fn scoped_pids_from_ledger_list(resp: &Value) -> Vec<u64> {
    resp["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| e["scoped"].as_bool().unwrap_or(false))
                .filter_map(|e| e["pid"].as_u64())
                .collect()
        })
        .unwrap_or_default()
}

/// 구 데몬 정상 종료: system.identify로 pid를 받아 SIGTERM(unix)/taskkill(win).
async fn stop_running_daemon() {
    let pid = rpc("system.identify", json!({}))
        .await
        .ok()
        .and_then(|r| r["daemon_pid"].as_u64());
    if let Some(pid) = pid {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            // taskkill /F는 TerminateProcess라 데몬이 어떤 콘솔 이벤트도 못 받아
            // shutdown_cleanup이 실행되지 않는다 → ledger의 scoped 프로세스(=cys CLI의
            // 자식, 데몬 트리 밖이라 /T로도 닿지 않음)가 영구 고아로 남는다. 데몬이
            // 아직 살아있는 지금 직접 회수한 뒤 데몬을 종료한다 (unix SIGTERM 경로 대칭).
            if let Ok(r) = rpc("ledger.list", json!({})).await {
                for spid in scoped_pids_from_ledger_list(&r) {
                    let _ = rpc("ledger.kill", json!({ "pid": spid })).await;
                }
            }
            let mut kill = std::process::Command::new("taskkill");
            kill.args(["/PID", &pid.to_string(), "/F"]);
            no_console(&mut kill);
            let _ = kill.output();
        }
        // 종료·소켓 unlink 대기 (최대 3초)
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if connect().await.is_err() {
                break;
            }
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Attachments(Mutex::new(HashMap::new())))
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            list_surfaces,
            org_status,
            org_fleet,
            ensure_dept_forwarders,
            control_analytics,
            control_skills,
            control_cost_baseline,
            control_alerts,
            control_weekly,
            control_sessions,
            control_session_detail,
            control_session_star,
            control_dashboard,
            control_hw,
            learn_status,
            create_surface,
            send_input,
            save_pasted_image,
            log_ime,
            ime_debug_enabled,
            rename_surface,
            resize_surface,
            close_surface,
            attach_surface,
            start_surface_stream,
            feed_list,
            feed_reply,
            list_dir,
            open_path,
            open_url,
            send_key,
            read_board_catalog,
            make_ticket,
            run_skill,
            skill_out_dir,
            check_update,
            check_pack_update,
            live_session_count,
            install_update,
            rotate_daemon,
            install_pack_update,
            launch_dept_daemon,
            allocate_dept_daemon,
            stop_dept_daemon,
            stop_dept_daemon_by_socket,
            list_depts,
            read_dept_catalog,
            install_cli_to_path,
            app_version,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                #[cfg(target_os = "macos")]
                let launchd_owns = maybe_autoregister_launchd().await;
                #[cfg(not(target_os = "macos"))]
                let launchd_owns = false;
                let result = if launchd_owns {
                    // launchd가 cysd를 소유·기동한다 — 수동 spawn 금지(중복 spawn·flock 경합 방지,
                    // codex BLOCKER). launchctl load는 비동기라 socket-ready를 최대 5초 폴링.
                    if wait_for_connect(50).await {
                        Ok(())
                    } else {
                        Err("launchd-owned cysd did not become ready within 5s".to_string())
                    }
                } else {
                    ensure_daemon().await
                };
                if let Err(e) = result {
                    let _ = handle.emit("daemon-error", e);
                    return;
                }
                let _ = handle.emit("daemon-ready", ());
                // event-forwarder를 먼저 띄워 init-pack 블로킹이 양방향 이벤트 파이프를 막지 않게 한다(반쪽 부팅 방지).
                spawn_event_forwarder(handle.clone(), default_socket());
                // RC-1: Windows 첫 기동 온보딩(팩+hook+autostart schtasks). 멱등.
                #[cfg(windows)]
                maybe_windows_onboard();
                // RC-17(T5): macOS 첫 기동 온보딩(팩+hook) — Windows 대칭. autostart는 위 launchd. 멱등.
                #[cfg(target_os = "macos")]
                maybe_macos_onboard();
                // 업데이트 재시작 시: 새 팩(새 기능) 반영 + 노드 자동복귀(마커가 있을 때만).
                maybe_apply_pending_update(&handle);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aiterm");
}

#[cfg(test)]
mod tests {
    use super::*;

    // HUD-2: open_url 화이트리스트 — https·허용 도메인만 통과, 위장 host(userinfo/서브도메인 사칭) 차단.
    #[test]
    fn open_url_whitelist_blocks_spoofed_and_nonhttps() {
        assert!(url_host_allowed("https://notebooklm.google.com/notebook/abc").is_ok());
        assert!(url_host_allowed("https://github.com/cys/repo").is_ok());
        assert!(url_host_allowed("http://notebooklm.google.com/").is_err(), "http 차단");
        assert!(url_host_allowed("https://evil.com/notebooklm.google.com").is_err(), "경로 사칭 차단");
        assert!(url_host_allowed("https://notebooklm.google.com.evil.com/").is_err(), "서브도메인 사칭 차단");
        assert!(url_host_allowed("https://notebooklm.google.com@evil.example.com/").is_err(), "userinfo 사칭 차단");
        assert!(url_host_allowed("https://evil.com#.github.com/").is_err(), "fragment 사칭 차단");
        assert!(url_host_allowed("https://evil.com?.github.com").is_err(), "query 사칭 차단");
        assert!(url_host_allowed("https://evil.com?x=.github.com").is_err(), "query 파라미터 사칭 차단");
    }

    // 사용자 확장 allowlist(순수 판정) — 정확일치·서브도메인 허용, 사칭·빈 항목 차단.
    #[test]
    fn host_allowlist_user_extension() {
        let extras = vec!["example-inst.org".to_string()];
        assert!(host_in_allowlist("example-inst.org", &extras));
        assert!(host_in_allowlist("docs.example-inst.org", &extras), "확장 도메인 서브도메인 허용");
        assert!(!host_in_allowlist("example-inst.org.evil.com", &extras), "사칭 차단");
        assert!(!host_in_allowlist("evil.com", &extras));
        assert!(!host_in_allowlist("anything.com", &vec!["".to_string()]), "빈 확장 항목 무시");
    }

    // #3: 사이드카 stdout의 PACK_UPDATE_RESULT 토큰에서 reinject failed/deferred를 파싱해
    // update-warning 발화 판단에 쓴다. 토큰 부재(구버전·reinject 스킵)는 (0,0)으로 보수적 처리.
    #[test]
    fn parse_reinject_counts_reads_structured_token() {
        let out = "[pack-update] 팩 2.0.0 반영 완료 (3 written, 1 preserved). 노드 reinject 점검…\n\
                   [pack-update] reinject: 2 injected, 1 skipped, 3 deferred, 4 failed.\n\
                   PACK_UPDATE_RESULT pack_version=2.0.0 injected=2 skipped=1 deferred=3 failed=4\n";
        assert_eq!(parse_reinject_counts(out), (4, 3), "failed=4 deferred=3 파싱");

        // 토큰 부재 → (0,0) 보수적(경고 미발화).
        assert_eq!(parse_reinject_counts("아무 의미 없는 출력\n"), (0, 0));
        assert_eq!(parse_reinject_counts(""), (0, 0));

        // failed=0 deferred=0 → (0,0)(완전 성공, 경고 없음).
        assert_eq!(
            parse_reinject_counts("PACK_UPDATE_RESULT pack_version=2.0.0 injected=5 skipped=0 deferred=0 failed=0"),
            (0, 0)
        );
        // deferred만 있는 경우(busy 노드) — 경고 발화 대상.
        assert_eq!(
            parse_reinject_counts("PACK_UPDATE_RESULT pack_version=1.2.3 injected=0 skipped=0 deferred=2 failed=0"),
            (0, 2)
        );
    }

    // check_pack_update 호환 게이트(DESIGN §7-④ 축2): min_binary_version > 실행 바이너리 = 무중단 거부.
    #[test]
    fn pack_binary_too_old_gate() {
        // 빈 값 = 제약 없음 → 무중단 허용.
        assert!(!pack_binary_too_old("", "0.4.2"));
        assert!(!pack_binary_too_old("   ", "0.4.2"));
        // min ≤ running → 허용.
        assert!(!pack_binary_too_old("0.4.2", "0.4.2"), "동일 버전 허용");
        assert!(!pack_binary_too_old("0.4.1", "0.4.2"), "min < running 허용");
        // min > running → 거부(바이너리 경로).
        assert!(pack_binary_too_old("0.5.0", "0.4.2"), "min > running 거부");
        assert!(pack_binary_too_old("1.0.0", "0.4.2"));
        // 파싱 실패 = 거부(보수적).
        assert!(pack_binary_too_old("not-a-version", "0.4.2"));
        assert!(pack_binary_too_old("0.5.0", "garbage"));
    }

    // 회귀: windows 업데이트 핸드오프가 데몬을 taskkill /F로 하드킬하면 cysd의
    // shutdown_cleanup이 실행되지 않아 scoped 자식(cys CLI의 자식)이 영구 고아로
    // 남는다. 그 누수를 막으려면 데몬이 살아있을 때 UI가 ledger.list에서 scoped pid를
    // 정확히 추려 ledger.kill로 회수해야 한다 — 그 선별 로직을 고정한다.
    #[test]
    fn scoped_pids_from_ledger_list_picks_only_scoped_pids() {
        let resp = json!({
            "entries": [
                {"pid": 100, "scoped": true},
                {"pid": 200, "scoped": false}, // 비-scoped → 데몬이 생명주기 보장 안 함, 회수 제외
                {"pid": 300, "scoped": true},
            ]
        });
        let mut pids = scoped_pids_from_ledger_list(&resp);
        pids.sort_unstable();
        assert_eq!(
            pids,
            vec![100, 300],
            "scoped 항목만 회수 대상이어야 하고 비-scoped는 제외돼야 한다"
        );
    }

    // scoped 플래그가 없으면(기본값 누락) 보수적으로 회수 대상에서 빼 외부 프로세스
    // 오인 킬을 막는다. entries가 비었거나 누락돼도 패닉 없이 빈 목록을 돌려준다.
    #[test]
    fn scoped_pids_from_ledger_list_empty_and_missing_fields_are_safe() {
        assert!(scoped_pids_from_ledger_list(&json!({"entries": []})).is_empty());
        assert!(scoped_pids_from_ledger_list(&json!({})).is_empty());
        // scoped 키 누락 = false 취급, pid 누락 항목은 건너뛴다
        let resp = json!({"entries": [{"pid": 100}, {"scoped": true}]});
        assert!(scoped_pids_from_ledger_list(&resp).is_empty());
    }

    // 적대검증 R-1 회귀: org_fleet fan-out은 풀 비경유 rpc_oneshot을 쓴다. (a) 정상 소켓은 응답을
    // 파싱해 반환하고, (b) 무응답(hung) 소켓은 timeout으로 깨끗이 Err를 준다 — 일회성 연결이라
    // 취소가 공유 풀(conn_cell)을 오염시키지 않는다(같은 부서 send_key/org_status 응답 귀속 보호).
    #[cfg(unix)]
    #[test]
    fn rpc_oneshot_parses_response_and_times_out_on_hung_socket() {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
        use tokio::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("cys-rpc-oneshot-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ok_sock = dir.join("ok.sock");
        let hang_sock = dir.join("hang.sock");
        let _ = std::fs::remove_file(&ok_sock);
        let _ = std::fs::remove_file(&hang_sock);

        tauri::async_runtime::block_on(async {
            // (a) 응답 소켓 — 요청 1줄 소비 후 valid 프레임 반환
            let ok = UnixListener::bind(&ok_sock).unwrap();
            tauri::async_runtime::spawn(async move {
                if let Ok((mut s, _)) = ok.accept().await {
                    let (r, mut w) = s.split();
                    let mut br = BufReader::new(r);
                    let mut l = String::new();
                    let _ = br.read_line(&mut l).await;
                    let _ = w.write_all(b"{\"ok\":true,\"result\":{\"surfaces\":[]}}\n").await;
                    let _ = w.flush().await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });
            // (b) hung 소켓 — accept만 하고 응답 없이 hold
            let hang = UnixListener::bind(&hang_sock).unwrap();
            tauri::async_runtime::spawn(async move {
                if let Ok((_s, _)) = hang.accept().await {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await; // bind 안정화

            // (a) 정상 응답 파싱
            let ok_res = rpc_oneshot(&ok_sock, "org.status", json!({})).await;
            assert!(ok_res.is_ok(), "정상 소켓 응답을 파싱해야 한다: {ok_res:?}");
            assert!(ok_res.unwrap()["surfaces"].is_array());

            // (b) hung 소켓은 timeout으로 Err — 취소가 깨끗이 일어난다(풀 비경유)
            let hung = tokio::time::timeout(
                std::time::Duration::from_millis(300),
                rpc_oneshot(&hang_sock, "org.status", json!({})),
            )
            .await;
            assert!(hung.is_err(), "무응답 소켓은 timeout(Elapsed)이어야 한다");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── CLI PATH 설치 헬퍼 ──────────────────────────────────────────
    #[test]
    fn sh_squote_escapes_spaces_and_quotes() {
        assert_eq!(sh_squote("/usr/local/bin"), "'/usr/local/bin'");
        assert_eq!(sh_squote("/Users/x/a b/cys.app"), "'/Users/x/a b/cys.app'");
        // 단일따옴표는 '\'' 시퀀스로 안전 이스케이프
        assert_eq!(sh_squote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_install_script_emits_idempotent_symlinks() {
        let cys = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cys");
        let cysd = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let s = build_install_script(cys, cysd, "/usr/local/bin");
        assert_eq!(
            s,
            "mkdir -p '/usr/local/bin' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cys' '/usr/local/bin/cys' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cysd' '/usr/local/bin/cysd'"
        );
    }

    #[test]
    fn classify_bundle_dir_distinguishes_canonical_translocated_backup_nonstandard() {
        use std::path::Path;
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new(
                "/private/var/folders/aa/bb/AppTranslocation/CCCC/d/cys.app/Contents/MacOS"
            )),
            BundleKind::Translocated
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.bak-044/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.prev-210050/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Downloads/cys.app/Contents/MacOS")),
            BundleKind::NonStandard
        );
    }

    #[test]
    fn parse_which_a_returns_precedence_ordered_paths() {
        let out = "/Users/x/.local/bin/cys\n/opt/homebrew/bin/cys\n\n/usr/local/bin/cys\n";
        assert_eq!(
            parse_which_a(out),
            vec![
                "/Users/x/.local/bin/cys".to_string(),
                "/opt/homebrew/bin/cys".to_string(),
                "/usr/local/bin/cys".to_string(),
            ]
        );
    }

    #[test]
    fn plan_cli_install_refuses_translocated_and_backup() {
        // translocated → Err
        assert!(plan_cli_install(
            std::path::Path::new("/private/var/folders/x/AppTranslocation/Y/d/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
        // backup → Err
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app.bak-044/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
    }

    #[test]
    fn plan_cli_install_warns_on_nonstandard_but_proceeds() {
        let plan = plan_cli_install(
            std::path::Path::new("/Users/x/Downloads/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("nonstandard는 경고와 함께 진행");
        assert!(plan.osascript_arg.contains("with administrator privileges"));
        assert!(plan.warnings.iter().any(|w| w.contains("표준 위치")));
        assert_eq!(plan.cys_src, std::path::PathBuf::from("/Users/x/Downloads/cys.app/Contents/MacOS/cys"));
    }

    #[test]
    fn plan_cli_install_canonical_has_no_location_warning() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("정규 번들은 진행");
        assert!(plan.warnings.iter().all(|w| !w.contains("표준 위치")));
        // osascript 인자는 do shell script + 승격 + 멱등 스크립트를 감싼다(AppleScript 큰따옴표 리터럴)
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan.osascript_arg.ends_with("\" with administrator privileges"));
    }

    #[test]
    fn applescript_str_escapes_backslash_and_doublequote() {
        assert_eq!(applescript_str("/usr/local/bin"), "\"/usr/local/bin\"");
        assert_eq!(applescript_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(applescript_str("a\\b"), "\"a\\\\b\"");
    }

    // 회귀 가드: osascript 인자는 AppleScript 큰따옴표 리터럴로 감싸야 한다(작은따옴표면 -2741로
    // 모든 호출이 admin 프롬프트 전에 실패 = dead-on-arrival). 내부 셸 경로는 작은따옴표 유지.
    #[test]
    fn osascript_arg_wraps_shell_in_applescript_double_quotes() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin",
        )
        .unwrap();
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan.osascript_arg.ends_with("\" with administrator privileges"));
        assert!(!plan.osascript_arg.starts_with("do shell script '"));
        assert!(plan.osascript_arg.contains("'/usr/local/bin/cys'"));
        assert!(plan.osascript_arg.contains("ln -sf"));
    }
}
