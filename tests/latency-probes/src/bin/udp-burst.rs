//! Screening benchmark only: actual Tokio UDP APIs, not a WebRTC substitute.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    io::ErrorKind,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

const BURSTS: usize = 256;
const PAYLOAD: usize = 1200;
const RECEIVE_BYTES: usize = 1024 * 1024;

fn distribution(values: &mut [u64]) -> Value {
    values.sort_unstable();
    if values.is_empty() {
        return Value::Null;
    }
    json!({"samples":values.len(), "p50_us":values[values.len()/2] as f64/1000.0,
        "p95_us":values[(values.len()-1)*95/100] as f64/1000.0,
        "p99_us":values[(values.len()-1)*99/100] as f64/1000.0,
        "max_us":values[values.len()-1] as f64/1000.0})
}

async fn trial(batch: bool, width: usize, round: usize) -> Result<Value> {
    let receive = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let socket = socket2::SockRef::from(&receive);
    socket.set_recv_buffer_size(RECEIVE_BYTES)?;
    let granted = socket.recv_buffer_size()?;
    let destination = receive.local_addr()?;
    receive.set_nonblocking(true)?;
    let receive = UdpSocket::from_std(receive)?;
    let begin = Instant::now();
    let barrier = Arc::new(Barrier::new(2));
    let gate = barrier.clone();
    let sending = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let sender = std::thread::spawn(move || -> Result<()> {
        let socket = sending;
        let mut bytes = [0u8; PAYLOAD];
        gate.wait();
        for burst in 0..BURSTS {
            for packet in 0..width {
                let seq = (burst * width + packet) as u64;
                bytes[..8].copy_from_slice(&seq.to_le_bytes());
                bytes[8..16].copy_from_slice(&(begin.elapsed().as_nanos() as u64).to_le_bytes());
                socket.send_to(&bytes, destination)?;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        bytes[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        socket.send_to(&bytes[..16], destination)?;
        Ok(())
    });
    let finished = Arc::new(AtomicBool::new(false));
    let stop = finished.clone();
    let heartbeat = tokio::spawn(async move {
        let mut samples = Vec::new();
        let mut timer = tokio::time::interval(Duration::from_millis(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !stop.load(Ordering::Relaxed) {
            let deadline = timer.tick().await;
            samples.push(deadline.elapsed().as_nanos() as u64);
        }
        samples
    });
    let mut bytes = [0u8; PAYLOAD];
    let mut latency = Vec::new();
    let mut control = Vec::new();
    let mut seen = vec![false; BURSTS * width];
    let mut duplicates = 0;
    barrier.wait();
    let wall = Instant::now();
    let receiving = async {
        loop {
            let mut drained = 0;
            if batch {
                receive.readable().await?;
            }
            loop {
                let length = if batch {
                    match receive.try_recv_from(&mut bytes) {
                        Ok((length, _)) => length,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    receive.recv_from(&mut bytes).await?.0
                };
                let seq = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                if seq == u64::MAX {
                    return Ok::<(), anyhow::Error>(());
                }
                ensure!(
                    length == PAYLOAD && (seq as usize) < seen.len(),
                    "invalid synthetic packet"
                );
                let seq = seq as usize;
                if seen[seq] {
                    duplicates += 1;
                }
                seen[seq] = true;
                let sent = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                let delay = (begin.elapsed().as_nanos() as u64).saturating_sub(sent);
                if seq >= 16 * width {
                    latency.push(delay);
                    if seq.is_multiple_of(width) {
                        control.push(delay);
                    }
                }
                drained += 1;
                if !batch || drained == 32 {
                    break;
                }
            }
            if batch {
                tokio::task::yield_now().await;
            }
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(30), receiving).await;
    finished.store(true, Ordering::Relaxed);
    let mut heartbeat = heartbeat.await?;
    sender
        .join()
        .map_err(|_| anyhow::anyhow!("sender panic"))??;
    result??;
    Ok(
        json!({"mode":if batch {"readable_try_recv_drain32"} else {"recv_from_await"},
        "round":round, "burst_packets":width,"packets_sent":seen.len(),
        "packets_missing":seen.iter().filter(|seen| !**seen).count(),"duplicates":duplicates,
        "receive_buffer_granted":granted,"wall_ms":wall.elapsed().as_millis(),
        "receive_latency":distribution(&mut latency),"control_packet_latency":distribution(&mut control),
        "competing_timer_lateness":distribution(&mut heartbeat)}),
    )
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut trials = Vec::new();
    runtime.block_on(async {
        for width in [16, 64, 256] {
            for round in 0..5 {
                for batch in if round % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    trials.push(trial(batch, width, round).await?);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"scope":"isolated-loopback-tokio-udp-screening",
        "includes_dtls_sctp_or_decoder":false,"changes_live_client":false,"trials":trials}))?
    );
    Ok(())
}
