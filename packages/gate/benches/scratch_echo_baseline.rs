//! Throwaway baseline: the real, un-tuned `examples/echo/server`
//! binary (plain axum `POST /echo`, two `tracing::info!` calls, no
//! crypto, no Db, no business logic at all) driven by the same
//! concurrent-client shape as `firehose.rs`, as a separate real OS
//! process over real HTTP. This is the hard ceiling: `subbit-gate`
//! can never exceed it, since it does strictly more work per request.
//! Not meant to stay in the tree.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[path = "support/mod.rs"]
mod support;

const RUN_DURATION: Duration = Duration::from_secs(5);

fn main() {
    let sweep: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sweep = if sweep.is_empty() {
        vec![1, 10, 20, 40, 80, 160]
    } else {
        sweep
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let child = spawn_echo_server().await;
        for &n in &sweep {
            run_level(&child.base_url, n).await;
        }
    });
}

struct EchoChild {
    base_url: String,
    _child: tokio::process::Child,
}

async fn spawn_echo_server() -> EchoChild {
    let addr: std::net::SocketAddr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let bin = support::sibling_bin("echo-server");
    let child = tokio::process::Command::new(bin)
        .arg("--addr")
        .arg(addr.ip().to_string())
        .arg("--port")
        .arg(addr.port().to_string())
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn echo-server");
    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();
    let ready_url = format!("{base_url}/echo");
    for _ in 0..200 {
        if client.post(&ready_url).body("x").send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    EchoChild {
        base_url,
        _child: child,
    }
}

async fn run_level(base_url: &str, n: usize) {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let total_ops = std::sync::Arc::new(AtomicU64::new(0));
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let stop = stop.clone();
        let total_ops = total_ops.clone();
        let url = format!("{base_url}/echo");
        handles.push(tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut local = 0u64;
            while !stop.load(Ordering::Relaxed) {
                if client.post(&url).body("hello").send().await.is_ok() {
                    local += 1;
                }
            }
            total_ops.fetch_add(local, Ordering::Relaxed);
        }));
    }
    let start = Instant::now();
    tokio::time::sleep(RUN_DURATION).await;
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.await;
    }
    let elapsed = start.elapsed();
    let ops = total_ops.load(Ordering::Relaxed);
    let rate = ops as f64 / elapsed.as_secs_f64();
    println!("echo baseline: concurrency={n:<5} ops={ops:<10} req/s={rate:.0}");
}
