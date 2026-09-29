//! Test helper that runs a throwaway JetStream-enabled `nats-server`.
//!
//! The binary is `AVENA_TEST_NATS_SERVER` if set, otherwise `nats-server` from `PATH`.
//! When it cannot be started the tests that use it print a note and pass, so the suite
//! still runs on machines without NATS installed.

use std::{
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// A running `nats-server` with its own port and store directory.
pub struct TestNats {
    child: Child,
    store: PathBuf,
    /// Client URL, `nats://127.0.0.1:<port>`.
    pub url: String,
}

impl TestNats {
    /// Starts a server, or returns `None` (with a note on stderr) if it cannot run.
    pub fn start(test: &str) -> Option<Self> {
        let binary =
            std::env::var("AVENA_TEST_NATS_SERVER").unwrap_or_else(|_| "nats-server".into());
        let port = TcpListener::bind("127.0.0.1:0")
            .ok()?
            .local_addr()
            .ok()?
            .port();
        let store = std::env::temp_dir().join(format!("avena-test-nats-{}", uuid::Uuid::new_v4()));
        let child = Command::new(&binary)
            .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
            .arg(&store)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let child = match child {
            Ok(child) => child,
            Err(err) => {
                eprintln!("{test}: skipped, cannot run {binary}: {err}");
                return None;
            }
        };
        let server = Self {
            child,
            store,
            url: format!("nats://127.0.0.1:{port}"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if Instant::now() > deadline {
                eprintln!("{test}: skipped, {binary} did not start listening");
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(server)
    }

    /// Connects and returns a JetStream context once JetStream answers.
    pub async fn jetstream(&self) -> async_nats::jetstream::Context {
        let client = async_nats::connect(&self.url)
            .await
            .expect("connect to test NATS");
        let js = async_nats::jetstream::new(client);
        for _ in 0..100 {
            if js.query_account().await.is_ok() {
                return js;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("JetStream did not become ready");
    }
}

impl Drop for TestNats {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.store);
    }
}
