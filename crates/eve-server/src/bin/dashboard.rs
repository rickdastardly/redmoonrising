//! The public dashboard (Dashboard.bat): a login in front of the game
//! server's own dashboard, which only ever listens on 127.0.0.1.
//!
//! - Every page and API call needs a signed-in session; one account, set up by
//!   Dashboard.bat (`--set-login`, the name and password on stdin), stored as a
//!   salted, many-times-hashed digest in `.rmr/dashboard-login.json`.
//! - Five wrong passwords from one address lock it out for fifteen minutes.
//! - Signed in, requests go through to the game server's dashboard (its port
//!   from `.rmr/settings.json`). When the server is down, a page says so and
//!   offers to start it (Start.bat --server-only), then shows its progress.
//!
//! Usage: dashboard --root <folder> [--bind 0.0.0.0:18090]
//!        dashboard --root <folder> --set-login      (name, then password, on stdin)

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::io::BufRead;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SESSION_HOURS: u64 = 12;
const ROUNDS: u32 = 100_000;
const MAX_FAILURES: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(15 * 60);
const HEAD_LIMIT: usize = 64 * 1024;
const COOKIE: &str = "rmr_session";

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Login {
    user: String,
    salt: String,
    hash: String,
    rounds: u32,
}

struct State {
    root: PathBuf,
    login: Login,
    sessions: Mutex<HashMap<String, Instant>>,
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    start_requested: Mutex<Option<Instant>>,
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let root = PathBuf::from(value("--root").unwrap_or_else(|| ".".into()));
    let login_path = root.join(".rmr").join("dashboard-login.json");
    if args.iter().any(|a| a == "--set-login") {
        std::process::exit(match set_login(&login_path) {
            Ok(()) => {
                println!("Dashboard login saved.");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        });
    }
    let login: Login = match std::fs::read_to_string(&login_path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(l) => l,
        None => {
            eprintln!("No dashboard sign-in is set up. Start the dashboard with Dashboard.bat.");
            std::process::exit(1);
        }
    };
    // The sign-in lives only in memory from here: nothing is left on disk
    // once this program stops, and the next Dashboard.bat asks again.
    let _ = std::fs::remove_file(&login_path);
    // One or more --bind addresses: Dashboard.bat passes each of this PC's
    // own addresses, so the public dashboard can share a port number with the
    // game server's dashboard on 127.0.0.1.
    let mut binds: Vec<String> = args.windows(2).filter(|w| w[0] == "--bind").map(|w| w[1].clone()).collect();
    if binds.is_empty() {
        binds.push("0.0.0.0:18090".into());
    }
    let state = Arc::new(State {
        root,
        login,
        sessions: Mutex::new(HashMap::new()),
        failures: Mutex::new(HashMap::new()),
        start_requested: Mutex::new(None),
    });
    let mut serving = Vec::new();
    for bind in &binds {
        match TcpListener::bind(bind).await {
            Ok(listener) => {
                println!("Listening on http://{bind}/");
                let state = state.clone();
                serving.push(tokio::spawn(async move {
                    loop {
                        let Ok((stream, peer)) = listener.accept().await else { continue };
                        let state = state.clone();
                        tokio::spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(3600), handle(state, stream, peer)).await;
                        });
                    }
                }));
            }
            Err(e) => eprintln!("Cannot listen on {bind}: {e}"),
        }
    }
    if serving.is_empty() {
        eprintln!("The dashboard could not listen anywhere. Is another program using that port?");
        std::process::exit(1);
    }
    println!("Red Moon Rising dashboard is up (signed-in access only).");
    for task in serving {
        let _ = task.await;
    }
}

fn set_login(path: &Path) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let user = lines.next().and_then(Result::ok).unwrap_or_default().trim().to_string();
    let password = lines.next().and_then(Result::ok).unwrap_or_default();
    let password = password.trim_end_matches(['\r', '\n']);
    if user.is_empty() || password.chars().count() < 8 {
        return Err("A name and a password of at least 8 characters are needed.".into());
    }
    let salt = random_hex(16);
    let login = Login { hash: digest(&salt, password, ROUNDS), user, salt, rounds: ROUNDS };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&login).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// ---- requests ---------------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    head_len: usize,
    buf: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.header("cookie")?.split(';').find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_len = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > HEAD_LIMIT {
            return None;
        }
        let n = tokio::time::timeout(Duration::from_secs(20), stream.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_len]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let path = target.split('?').next().unwrap_or("/").to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    Some(Request { method, path, headers, head_len, buf })
}

async fn read_body(stream: &mut TcpStream, req: &mut Request, limit: usize) -> Option<Vec<u8>> {
    let want: usize = req.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if want > limit {
        return None;
    }
    let mut body = req.buf[req.head_len..].to_vec();
    let mut chunk = [0u8; 4096];
    while body.len() < want {
        let n = tokio::time::timeout(Duration::from_secs(20), stream.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(want);
    Some(body)
}

async fn handle(state: Arc<State>, mut client: TcpStream, peer: SocketAddr) -> std::io::Result<()> {
    let Some(mut req) = read_head(&mut client).await else { return Ok(()) };
    let ip = peer.ip();
    match (req.method.as_str(), req.path.as_str()) {
        (_, "/rmr-login") if req.method == "POST" => {
            let body = read_body(&mut client, &mut req, 8 * 1024).await.unwrap_or_default();
            return sign_in(&state, &mut client, ip, &body).await;
        }
        // The server's news, for game launchers: public, read-only.
        ("GET", "/rmr-news") => {
            let body = news(&state).await;
            return send(&mut client, 200, "application/json; charset=utf-8", &["Access-Control-Allow-Origin: *".to_string()], body.as_bytes()).await;
        }
        ("GET", "/rmr-login") => return send(&mut client, 200, "text/html; charset=utf-8", &[], login_page("").as_bytes()).await,
        (_, "/rmr-logout") => {
            if let Some(token) = req.cookie(COOKIE) {
                state.sessions.lock().unwrap().remove(&token);
            }
            let clear = format!("Set-Cookie: {COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
            return redirect(&mut client, "/rmr-login", &[clear]).await;
        }
        _ => {}
    }
    if !signed_in(&state, &req) {
        if req.path.starts_with("/api/") {
            return send(&mut client, 401, "application/json", &[], br#"{"error":"Sign in to the dashboard first."}"#).await;
        }
        return redirect(&mut client, "/rmr-login", &[]).await;
    }
    if req.path == "/rmr-start" && req.method == "POST" {
        start_server(&state);
        return redirect(&mut client, "/", &[]).await;
    }
    if req.path == "/rmr-status" {
        let body = serde_json::json!({ "online": backend(&state).await.is_some(), "status": launcher_status(&state.root) }).to_string();
        return send(&mut client, 200, "application/json", &[], body.as_bytes()).await;
    }
    // Through to the game server's own dashboard.
    let Some(mut upstream) = backend(&state).await else {
        if req.path.starts_with("/api/") {
            return send(&mut client, 503, "application/json", &[], br#"{"error":"The game server is offline."}"#).await;
        }
        let page = offline_page(&state);
        return send(&mut client, 503, "text/html; charset=utf-8", &[], page.as_bytes()).await;
    };
    let head = String::from_utf8_lossy(&req.buf[..req.head_len]).to_string();
    let mut lines = head.split("\r\n");
    let mut out = String::with_capacity(head.len() + 64);
    out.push_str(lines.next().unwrap_or(""));
    out.push_str("\r\n");
    for line in lines {
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if line.is_empty() || matches!(name.as_str(), "connection" | "cookie" | "keep-alive" | "x-forwarded-for") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("X-Forwarded-For: {ip}\r\nConnection: close\r\n\r\n"));
    upstream.write_all(out.as_bytes()).await?;
    upstream.write_all(&req.buf[req.head_len..]).await?;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut up_read, mut up_write) = upstream.into_split();
    let to_server = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut client_read, &mut up_write).await;
    });
    let _ = tokio::io::copy(&mut up_read, &mut client_write).await;
    to_server.abort();
    let _ = client_write.shutdown().await;
    Ok(())
}

fn signed_in(state: &State, req: &Request) -> bool {
    let Some(token) = req.cookie(COOKIE) else { return false };
    let mut sessions = state.sessions.lock().unwrap();
    let now = Instant::now();
    sessions.retain(|_, until| *until > now);
    sessions.contains_key(&token)
}

async fn sign_in(state: &State, client: &mut TcpStream, ip: IpAddr, body: &[u8]) -> std::io::Result<()> {
    let locked = {
        let mut failures = state.failures.lock().unwrap();
        failures.retain(|_, (_, at)| at.elapsed() < LOCKOUT);
        failures.get(&ip).is_some_and(|(n, _)| *n >= MAX_FAILURES)
    };
    if locked {
        let page = login_page("Too many wrong passwords from your address. Try again in fifteen minutes.");
        return send(client, 429, "text/html; charset=utf-8", &[], page.as_bytes()).await;
    }
    let form = String::from_utf8_lossy(body);
    let field = |name: &str| form.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| url_decode(v))).unwrap_or_default();
    let (user, password) = (field("user"), field("password"));
    let login = &state.login;
    let ok = same(user.as_bytes(), login.user.as_bytes()) & same(digest(&login.salt, &password, login.rounds).as_bytes(), login.hash.as_bytes());
    if !ok {
        {
            let mut failures = state.failures.lock().unwrap();
            let entry = failures.entry(ip).or_insert((0, Instant::now()));
            entry.0 += 1;
            entry.1 = Instant::now();
            println!("{}: failed sign-in from {ip} ({} of {MAX_FAILURES})", stamp(), entry.0);
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        let page = login_page("Wrong name or password.");
        return send(client, 401, "text/html; charset=utf-8", &[], page.as_bytes()).await;
    }
    state.failures.lock().unwrap().remove(&ip);
    let token = random_hex(32);
    state.sessions.lock().unwrap().insert(token.clone(), Instant::now() + Duration::from_secs(SESSION_HOURS * 3600));
    println!("{}: signed in from {ip}", stamp());
    let cookie = format!("Set-Cookie: {COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}", SESSION_HOURS * 3600);
    redirect(client, "/", &[cookie]).await
}

// ---- the game server ----------------------------------------------------------------------------

/// The game server's dashboard port, from the launcher's settings.
fn web_port(root: &Path) -> u16 {
    std::fs::read_to_string(root.join(".rmr").join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v.get("webPort").and_then(|p| p.as_u64()))
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(18080)
}

async fn backend(state: &State) -> Option<TcpStream> {
    let port = web_port(&state.root);
    tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(("127.0.0.1", port))).await.ok()?.ok()
}

/// The newest published articles (title, time in ms, HTML body), or [] when
/// the game server is down.
async fn news(state: &State) -> String {
    let Some(mut upstream) = backend(state).await else { return "[]".into() };
    if upstream.write_all(b"GET /api/articles HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.is_err() {
        return "[]".into();
    }
    let mut raw = Vec::new();
    if tokio::time::timeout(Duration::from_secs(5), upstream.read_to_end(&mut raw)).await.is_err() {
        return "[]".into();
    }
    let Some(start) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return "[]".into() };
    let Ok(serde_json::Value::Array(articles)) = serde_json::from_slice::<serde_json::Value>(&raw[start + 4..]) else { return "[]".into() };
    let trimmed: Vec<serde_json::Value> = articles
        .into_iter()
        .take(12)
        .map(|a| serde_json::json!({ "title": a.get("title"), "time": a.get("time"), "body": a.get("body") }))
        .collect();
    serde_json::Value::Array(trimmed).to_string()
}

fn launcher_status(root: &Path) -> serde_json::Value {
    std::fs::read_to_string(root.join(".rmr").join("status.json"))
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn starting(state: &State) -> bool {
    state.start_requested.lock().unwrap().is_some_and(|at| at.elapsed() < Duration::from_secs(15 * 60))
}

fn start_server(state: &State) {
    {
        let mut asked = state.start_requested.lock().unwrap();
        if asked.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            return;
        }
        *asked = Some(Instant::now());
    }
    let launcher = state.root.join("Start.bat");
    let result = std::process::Command::new("cmd.exe")
        .args(["/c", "start", "Red Moon Rising server", "/min"])
        .arg(&launcher)
        .arg("--server-only")
        .current_dir(&state.root)
        .spawn();
    match result {
        Ok(_) => println!("{}: starting the server ({})", stamp(), launcher.display()),
        Err(e) => println!("{}: could not start the server: {e}", stamp()),
    }
}

// ---- pages --------------------------------------------------------------------------------------

const STYLE: &str = r#"<style>
body{margin:0;min-height:100vh;display:grid;place-items:center;background:radial-gradient(circle at 30% 20%,#2a1214,#07090c 70%);color:#c9d1d9;font:15px 'Segoe UI',sans-serif}
.box{width:min(420px,90vw);padding:32px;background:rgba(14,18,23,.92);border:1px solid #2c3540;box-shadow:0 20px 60px #000}
h1{margin:0 0 4px;font-size:18px;letter-spacing:.3em;color:#fff}p.k{margin:0 0 22px;color:#e0533f;font-size:11px;letter-spacing:.3em}
label{display:block;margin:14px 0 6px;font-size:11px;letter-spacing:.2em;color:#8c949c;text-transform:uppercase}
input{width:100%;box-sizing:border-box;padding:10px;background:#0b0f13;border:1px solid #2c3540;color:#fff;font-size:15px}
button{margin-top:22px;width:100%;padding:11px;border:0;background:#4fcab6;color:#06120f;font-weight:600;letter-spacing:.15em;cursor:pointer}
.msg{margin-top:16px;color:#e0533f}.dim{color:#8c949c;font-size:13px}.bar{height:6px;background:#1b2229;margin-top:16px}.bar i{display:block;height:100%;background:#4fcab6}
a{color:#4fcab6}
</style>"#;

fn login_page(message: &str) -> String {
    let message = if message.is_empty() { String::new() } else { format!("<p class=msg>{}</p>", html(message)) };
    format!(
        r#"<!DOCTYPE html><html><head><meta charset=utf-8><meta name=viewport content="width=device-width"><title>Sign in · Red Moon Rising</title>{STYLE}</head>
<body><form class=box method=post action="/rmr-login"><h1>RED MOON RISING</h1><p class=k>CLUSTER ADMINISTRATION</p>
<label for=u>Name</label><input id=u name=user autocomplete=username autofocus required>
<label for=p>Password</label><input id=p name=password type=password autocomplete=current-password required>
<button type=submit>SIGN IN</button>{message}</form></body></html>"#
    )
}

fn offline_page(state: &State) -> String {
    let status = launcher_status(&state.root);
    let text = |k: &str| status.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let progress = status.get("progress").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 100);
    let (stage, message) = (text("stage"), text("message"));
    let failed = status.get("error").and_then(|v| v.as_bool()).unwrap_or(false);
    let busy = starting(state) && !failed;
    let detail = if busy || (!stage.is_empty() && stage != "Stopped" && !failed && progress < 100) {
        format!(
            r#"<p class=dim>{}: {}</p><div class=bar><i style="width:{progress}%"></i></div><p class=dim>This page refreshes by itself.</p>"#,
            html(&stage),
            html(&message)
        )
    } else if failed {
        format!("<p class=msg>The last start failed: {}</p>", html(&message))
    } else {
        String::new()
    };
    let refresh = if busy { r#"<meta http-equiv=refresh content=5>"# } else { "" };
    let button = if busy { "" } else { r#"<form method=post action="/rmr-start"><button type=submit>START SERVER</button></form>"# };
    format!(
        r#"<!DOCTYPE html><html><head><meta charset=utf-8><meta name=viewport content="width=device-width">{refresh}<title>Server offline · Red Moon Rising</title>{STYLE}</head>
<body><div class=box><h1>RED MOON RISING</h1><p class=k>SERVER OFFLINE</p><p>The game server is not running.</p>{detail}{button}
<p class=dim style="margin-top:22px"><a href="/rmr-logout">Sign out</a></p></div></body></html>"#
    )
}

async fn send(stream: &mut TcpStream, code: u16, kind: &str, extra: &[String], body: &[u8]) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        401 => "Unauthorized",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut head = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Frame-Options: DENY\r\nConnection: close\r\n", body.len());
    for h in extra {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await
}

async fn redirect(stream: &mut TcpStream, to: &str, extra: &[String]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 303 See Other\r\nLocation: {to}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n");
    for h in extra {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.shutdown().await
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

// ---- secrets ------------------------------------------------------------------------------------

/// Hex of `n` unpredictable bytes: SipHash keyed from the operating system's
/// random source (std's RandomState) over a counter and the clock.
fn random_hex(n: usize) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut out = String::with_capacity(n * 2);
    let mut i = 0u64;
    while out.len() < n * 2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(nanos);
        h.write_usize(&out as *const String as usize);
        out.push_str(&format!("{:016x}", h.finish()));
        i += 1;
    }
    out.truncate(n * 2);
    out
}

/// The stored form of a password: SHA-256, salted and repeated.
fn digest(salt: &str, password: &str, rounds: u32) -> String {
    let mut h = sha256(format!("{salt}:{password}").as_bytes());
    for _ in 0..rounds {
        let mut next = Vec::with_capacity(32 + salt.len() + password.len());
        next.extend_from_slice(&h);
        next.extend_from_slice(salt.as_bytes());
        next.extend_from_slice(password.as_bytes());
        h = sha256(&next);
    }
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vectors() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn forms_decode() {
        assert_eq!(url_decode("a+b%21%3D"), "a b!=");
        assert_eq!(url_decode("100%"), "100%");
    }
}
