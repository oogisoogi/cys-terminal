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

#[tauri::command]
async fn list_surfaces(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "surface.list", json!({})).await
}

#[tauri::command]
async fn control_dashboard() -> Result<Value, String> {
    rpc("control.dashboard", json!({})).await
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
async fn control_session_star(session_id: String, starred: bool) -> Result<Value, String> {
    rpc("control.session_star", json!({ "session_id": session_id, "starred": starred })).await
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
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cys-ime.log")
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "{ts} {line}");
    }
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
async fn attach_surface(
    app: AppHandle,
    state: State<'_, Attachments>,
    socket: Option<String>,
    surface_id: u64,
) -> Result<Value, String> {
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
    Ok(json!({"output_event": event_name, "exited_event": event_exited}))
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
    std::process::Command::new(&program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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

/// 부서 데몬 소켓 경로 — cys-dept 규약과 동일(~/.local/state/cys-dept-<name>/cys.sock).
fn dept_socket_path(name: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".local/state")
        .join(format!("cys-dept-{name}"))
        .join("cys.sock")
}

/// 새 부서 workspace 런칭 = 부서 데몬 spawn. 단일 진입점 cys-dept launch를 OS 호출해
/// 레지스트리·ACL 시드·CEO 승격을 일임한다(직접 cysd spawn 금지, 검증 mustFix). 성공 시
/// 그 데몬용 이벤트 forwarder를 추가 spawn하고 socket·slug·identify를 반환한다.
#[tauri::command]
async fn launch_dept_daemon(app: AppHandle, name: String) -> Result<Value, String> {
    let tool = dept_tool();
    let n = name.clone();
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&tool)
            .arg("launch")
            .arg(&n)
            .output()
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

/// 부서 workspace 닫기 = 부서 데몬 teardown. cys-dept down에 일임(SIGTERM·소켓 정리·레지스트리·CEO 강등).
#[tauri::command]
async fn stop_dept_daemon(name: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&tool)
            .arg("down")
            .arg(&name)
            .output()
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
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cys/depts.json")
        });
    match std::fs::read_to_string(&reg) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()),
        Err(_) => Ok(json!({ "depts": {} })),
    }
}

/// 부서 데몬 teardown(socket 기준) — ws 이름 변경(rename)으로 name→socket 매핑이 끊겨도 정확히 종료.
/// cys-dept down-sock에 일임(레지스트리 역인덱스로 부서명 해석 후 teardown).
#[tauri::command]
async fn stop_dept_daemon_by_socket(socket: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&tool)
            .arg("down-sock")
            .arg(&socket)
            .output()
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
    let _ = app.emit("update-progress", json!({"phase": "handoff"}));
    stop_running_daemon().await;
    // 4) 앱 재시작 — setup의 ensure_daemon이 새 cysd를 자동 기동한다
    app.restart();
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
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
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
        .manage(Attachments(Mutex::new(HashMap::new())))
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            list_surfaces,
            control_analytics,
            control_skills,
            control_alerts,
            control_weekly,
            control_sessions,
            control_session_detail,
            control_session_star,
            control_dashboard,
            learn_status,
            create_surface,
            send_input,
            log_ime,
            rename_surface,
            resize_surface,
            close_surface,
            attach_surface,
            feed_list,
            feed_reply,
            list_dir,
            open_path,
            check_update,
            live_session_count,
            install_update,
            launch_dept_daemon,
            stop_dept_daemon,
            stop_dept_daemon_by_socket,
            list_depts,
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
                spawn_event_forwarder(handle, default_socket());
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aiterm");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
