use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tailscale_status_reporter::Reporter;
use tempfile::TempDir;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, UnixListener},
    task::JoinHandle,
    time::{sleep, timeout},
};

const LOGIN_URL: &str = "https://login.tailscale.com/a/synthetic?first=1&second=a%2Bb";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
            delay: Duration::ZERO,
        }
    }
}

struct Request {
    headers: String,
    body: Vec<u8>,
}

struct Server {
    reply: Arc<Mutex<Reply>>,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    reply: Arc<Mutex<Reply>>,
    requests: Arc<Mutex<Vec<Request>>>,
) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0 && bytes.len() < 16384);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let content_length: usize = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    requests.lock().unwrap().push(Request {
        headers,
        body: bytes[header_end..].to_vec(),
    });
    let reply = reply.lock().unwrap().clone();
    sleep(reply.delay).await;
    let location = if (300..400).contains(&reply.status) {
        "Location: /unexpected-redirect\r\n"
    } else {
        ""
    };
    let headers = format!(
        "HTTP/1.1 {} Fixture\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.body.len()
    );
    // Timed-out clients can disconnect before the fixture writes its response.
    if stream.write_all(headers.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&reply.body).await;
    }
}

async fn local_server() -> (Server, TempDir) {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let listener = UnixListener::bind(dir.path().join("tailscaled.sock")).unwrap();
    let reply = Arc::new(Mutex::new(Reply::json(json!({
        "BackendState": "NeedsLogin", "AuthURL": LOGIN_URL, "Self": null
    }))));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let reply = reply.clone();
        let requests = requests.clone();
        async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                handle(stream, reply.clone(), requests.clone()).await;
            }
        }
    });
    (
        Server {
            reply,
            requests,
            task,
        },
        dir,
    )
}

async fn manager_server() -> (Server, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let reply = Arc::new(Mutex::new(Reply::json(json!({}))));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let reply = reply.clone();
        let requests = requests.clone();
        async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                handle(stream, reply.clone(), requests.clone()).await;
            }
        }
    });
    (
        Server {
            reply,
            requests,
            task,
        },
        origin,
    )
}

fn payload(server: &Server, index: usize) -> Value {
    serde_json::from_slice(&server.requests.lock().unwrap()[index].body).unwrap()
}

#[tokio::test]
async fn sends_private_url_in_text_body_and_clears_it_on_login() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    reporter.tick(100).await;
    assert_eq!(
        payload(&manager, 0),
        json!({
            "version": 1, "state": "NeedsLogin", "online": false, "authUrl": LOGIN_URL, "updatedAt": 100
        })
    );
    let headers = manager.requests.lock().unwrap()[0]
        .headers
        .to_ascii_lowercase();
    assert!(headers.starts_with("post /data-send?key=tailscale.status http/1.1"));
    assert!(headers.contains("content-type: text/plain\r\n"));
    assert!(!headers.contains("synthetic"));
    let local_headers = local.requests.lock().unwrap()[0]
        .headers
        .to_ascii_lowercase();
    assert!(local_headers.starts_with("get /localapi/v0/status?peers=false http/1.1"));
    assert!(local_headers.contains("host: local-tailscaled.sock\r\n"));

    *local.reply.lock().unwrap() = Reply::json(json!({
        "BackendState": "Running", "AuthURL": LOGIN_URL, "Self": {"Online": true}
    }));
    reporter.tick(102).await;
    assert_eq!(payload(&manager, 1)["state"], "Running");
    assert_eq!(payload(&manager, 1)["online"], true);
    assert!(payload(&manager, 1)["authUrl"].is_null());
    reporter.tick(104).await;
    assert_eq!(manager.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn retries_rejected_delivery_and_publishes_rotated_link() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    manager.reply.lock().unwrap().status = 503;
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    reporter.tick(100).await;
    manager.reply.lock().unwrap().status = 200;
    reporter.tick(102).await;
    assert_eq!(payload(&manager, 1)["authUrl"], LOGIN_URL);
    *local.reply.lock().unwrap() = Reply::json(json!({
        "BackendState":"NeedsLogin", "AuthURL":"https://login.tailscale.com/a/replaced"
    }));
    reporter.tick(104).await;
    assert_eq!(
        payload(&manager, 2)["authUrl"],
        "https://login.tailscale.com/a/replaced"
    );
}

#[tokio::test]
async fn malformed_oversized_and_failed_status_clear_the_link() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    for bad_reply in [
        Reply {
            status: 200,
            body: b"not JSON".to_vec(),
            delay: Duration::ZERO,
        },
        Reply {
            status: 200,
            body: vec![b'x'; 65537],
            delay: Duration::ZERO,
        },
        Reply {
            status: 500,
            body: Vec::new(),
            delay: Duration::ZERO,
        },
    ] {
        *local.reply.lock().unwrap() =
            Reply::json(json!({"BackendState":"NeedsLogin", "AuthURL":LOGIN_URL}));
        reporter.tick(100).await;
        *local.reply.lock().unwrap() = bad_reply;
        reporter.tick(102).await;
        let last = manager.requests.lock().unwrap().len() - 1;
        assert_eq!(payload(&manager, last)["state"], "Unavailable");
        assert!(payload(&manager, last)["authUrl"].is_null());
    }
}

#[tokio::test]
async fn survives_socket_disappearance_and_reconnects_after_daemon_restart() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    let socket = dir.path().join("tailscaled.sock");
    let mut reporter = Reporter::new(&socket, origin.parse().unwrap()).unwrap();
    reporter.tick(100).await;
    local.task.abort();
    std::fs::remove_file(&socket).unwrap();
    reporter.tick(102).await;
    assert_eq!(payload(&manager, 1)["state"], "Unavailable");
    let listener = UnixListener::bind(&socket).unwrap();
    let restarted = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        handle(stream, local.reply.clone(), local.requests.clone()).await;
    });
    reporter.tick(104).await;
    restarted.await.unwrap();
    assert_eq!(payload(&manager, 2)["authUrl"], LOGIN_URL);
}

#[tokio::test]
async fn localapi_requests_have_a_deadline() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    local.reply.lock().unwrap().delay = Duration::from_secs(30);
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    timeout(Duration::from_secs(5), reporter.tick(100))
        .await
        .unwrap();
    assert_eq!(payload(&manager, 0)["state"], "Unavailable");
}

#[tokio::test]
async fn manager_redirects_never_receive_a_second_copy_of_the_link() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    manager.reply.lock().unwrap().status = 307;
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    reporter.tick(100).await;
    assert_eq!(manager.requests.lock().unwrap().len(), 1);
    manager.reply.lock().unwrap().status = 200;
    reporter.tick(102).await;
    assert_eq!(manager.requests.lock().unwrap().len(), 2);
    assert_eq!(payload(&manager, 1)["authUrl"], LOGIN_URL);
    assert_eq!(local.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn manager_requests_have_a_deadline() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    manager.reply.lock().unwrap().delay = Duration::from_secs(30);
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    timeout(Duration::from_secs(5), reporter.tick(100))
        .await
        .unwrap();
    assert_eq!(manager.requests.lock().unwrap().len(), 1);
    assert!(!local.requests.lock().unwrap().is_empty());
}

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn binary_clears_link_and_exits_on_sigterm_without_logging_it() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_tailscale-status-reporter"))
            .env("TS_SOCKET", dir.path().join("tailscaled.sock"))
            .env("DAPPMANAGER_URL", origin)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    timeout(Duration::from_secs(5), async {
        while manager.requests.lock().unwrap().is_empty() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(payload(&manager, 0)["authUrl"], LOGIN_URL);
    assert!(
        Command::new("kill")
            .args(["-TERM", &process.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    timeout(Duration::from_secs(4), async {
        while process.0.try_wait().unwrap().is_none() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(process.0.wait().unwrap().success());
    assert_eq!(payload(&manager, 1)["state"], "Unavailable");
    assert!(payload(&manager, 1)["authUrl"].is_null());
    let mut logs = String::new();
    std::io::Read::read_to_string(&mut process.0.stderr.take().unwrap(), &mut logs).unwrap();
    std::io::Read::read_to_string(&mut process.0.stdout.take().unwrap(), &mut logs).unwrap();
    assert!(!logs.contains("synthetic"));
    assert!(!local.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn heartbeat_republishes_unchanged_status() {
    let (local, dir) = local_server().await;
    let (manager, origin) = manager_server().await;
    let mut reporter =
        Reporter::new(&dir.path().join("tailscaled.sock"), origin.parse().unwrap()).unwrap();
    reporter.tick(100).await;
    // Move only the reporter's monotonic clock; no wall-clock timestamp can
    // accidentally suppress a heartbeat after a clock correction.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    reporter.tick(131).await;
    assert_eq!(payload(&manager, 1)["authUrl"], LOGIN_URL);
    assert_eq!(payload(&manager, 1)["updatedAt"], 131);
    assert_eq!(local.requests.lock().unwrap().len(), 2);
}
