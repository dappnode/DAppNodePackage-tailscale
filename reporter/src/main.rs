use std::{env, path::PathBuf, process::ExitCode, time::SystemTime};

use tailscale_status_reporter::{
    DEFAULT_MANAGER, DEFAULT_SOCKET, ManagerEndpoint, POLL_INTERVAL, Reporter, Snapshot,
};
use tokio::{
    signal::unix::{SignalKind, signal},
    time::{Duration, MissedTickBehavior, interval, timeout},
};

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let socket = env::var_os("TS_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET));
    let manager = env::var("DAPPMANAGER_URL").unwrap_or_else(|_| DEFAULT_MANAGER.to_owned());
    let endpoint = match manager.parse::<ManagerEndpoint>() {
        Ok(endpoint) => endpoint,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok(mut reporter) = Reporter::new(&socket, endpoint) else {
        eprintln!("Cannot initialize reporter HTTP clients.");
        return ExitCode::FAILURE;
    };
    let (Ok(mut terminate), Ok(mut interrupt)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        eprintln!("Cannot initialize reporter shutdown handling.");
        return ExitCode::FAILURE;
    };
    let shutdown = async move {
        tokio::select! {
            _ = terminate.recv() => {},
            _ = interrupt.recv() => {},
        }
    };
    tokio::pin!(shutdown);
    let mut poll = interval(POLL_INTERVAL);
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            _ = poll.tick() => {},
        }
        tokio::select! {
            () = &mut shutdown => break,
            () = reporter.tick(unix_time()) => {},
        }
    }
    // A final clear is best effort. Freshness/container checks are still required
    // in Dappmanager because SIGKILL and network outages cannot deliver it.
    let _ = timeout(
        Duration::from_secs(2),
        reporter.publish(Snapshot::unavailable(), unix_time()),
    )
    .await;
    ExitCode::SUCCESS
}
