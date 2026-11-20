// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Live strap over btleplug. Runs on its own thread with a single-threaded tokio runtime so the
//! render loop never blocks on Bluetooth. Reconnects forever while a strap is chosen.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{Device, HrSource, STALE, Status, parse};

const RETRY: Duration = Duration::from_secs(3);
const SCAN_TIME: Duration = Duration::from_secs(10);

enum Cmd {
    Scan,
    Use(Option<String>),
}

enum Wait {
    Cmd(Cmd),
    Retry,
    Closed,
}

#[derive(Default)]
struct Shared {
    status: Status,
    reading: Option<(u16, Instant)>,
    devices: Vec<Device>,
}

type State = Arc<Mutex<Shared>>;

fn lock(s: &State) -> std::sync::MutexGuard<'_, Shared> {
    // A panic in the BLE thread must not take the overlay down with it.
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct BleHr {
    state: State,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl BleHr {
    /// Starts the worker; connects right away if a strap was saved.
    pub fn start(saved: Option<String>) -> Self {
        let state = State::default();
        let (tx, rx) = mpsc::unbounded_channel();
        let worker = state.clone();
        let spawned = std::thread::Builder::new().name("ble-hr".into()).spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            {
                Ok(rt) => rt.block_on(run(worker, rx, saved)),
                Err(e) => {
                    tracing::warn!("ble runtime: {e}");
                    lock(&worker).status = Status::NoAdapter;
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("ble thread: {e}");
            lock(&state).status = Status::NoAdapter;
        }
        Self { state, cmd: tx }
    }

    pub fn scan(&self) {
        let _ = self.cmd.send(Cmd::Scan);
    }

    pub fn use_device(&self, id: Option<String>) {
        let _ = self.cmd.send(Cmd::Use(id));
    }

    pub fn devices(&self) -> Vec<Device> {
        lock(&self.state).devices.clone()
    }
}

impl HrSource for BleHr {
    fn bpm(&self) -> Option<u16> {
        lock(&self.state)
            .reading
            .filter(|(_, at)| at.elapsed() < STALE)
            .map(|(b, _)| b)
    }

    fn status(&self) -> Status {
        lock(&self.state).status
    }
}

fn set(state: &State, s: Status) {
    lock(state).status = s;
}

async fn run(state: State, mut rx: mpsc::UnboundedReceiver<Cmd>, mut target: Option<String>) {
    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("bluetooth unavailable: {e}");
            set(&state, Status::NoAdapter);
            return;
        }
    };
    let mut scan_requested = false;
    loop {
        let central = manager.adapters().await.ok().and_then(|a| a.into_iter().next());
        let wait = match (&central, &target) {
            (None, _) => {
                set(&state, Status::NoAdapter);
                next_cmd(&mut rx, Some(RETRY)).await
            }
            (Some(c), _) if std::mem::take(&mut scan_requested) => {
                scan(c, &state).await;
                Wait::Retry
            }
            (Some(c), Some(id)) => stream(c, id, &state, &mut rx).await,
            (Some(_), None) => {
                set(&state, Status::Off);
                next_cmd(&mut rx, None).await
            }
        };
        match wait {
            Wait::Closed => return,
            Wait::Cmd(Cmd::Scan) => scan_requested = true,
            Wait::Cmd(Cmd::Use(id)) => target = id,
            Wait::Retry if target.is_some() => {
                if let Wait::Cmd(c) = next_cmd(&mut rx, Some(RETRY)).await {
                    match c {
                        Cmd::Scan => scan_requested = true,
                        Cmd::Use(id) => target = id,
                    }
                }
            }
            Wait::Retry => {}
        }
    }
}

async fn next_cmd(rx: &mut mpsc::UnboundedReceiver<Cmd>, timeout: Option<Duration>) -> Wait {
    let recv = async { rx.recv().await.map_or(Wait::Closed, Wait::Cmd) };
    match timeout {
        Some(t) => tokio::time::timeout(t, recv).await.unwrap_or(Wait::Retry),
        None => recv.await,
    }
}

async fn heart_rate_peripherals(c: &Adapter) -> Vec<(Peripheral, Device)> {
    let mut out = Vec::new();
    for p in c.peripherals().await.unwrap_or_default() {
        if let Ok(Some(props)) = p.properties().await
            && props.services.contains(&uuid_from_u16(0x180D))
        {
            let id = p.id().to_string();
            let name = props.local_name.unwrap_or_else(|| id.clone());
            out.push((p, Device { id, name }));
        }
    }
    out
}

async fn scan(c: &Adapter, state: &State) {
    set(state, Status::Scanning);
    if let Err(e) = c
        .start_scan(ScanFilter {
            services: vec![uuid_from_u16(0x180D)],
        })
        .await
    {
        tracing::warn!("scan: {e}");
        return;
    }
    let until = Instant::now() + SCAN_TIME;
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let found = heart_rate_peripherals(c).await;
        lock(state).devices = found.into_iter().map(|(_, d)| d).collect();
    }
    let _ = c.stop_scan().await;
}

async fn stream(c: &Adapter, id: &str, state: &State, rx: &mut mpsc::UnboundedReceiver<Cmd>) -> Wait {
    set(state, Status::Connecting);
    let measurement = uuid_from_u16(0x2A37);
    let _ = c
        .start_scan(ScanFilter {
            services: vec![uuid_from_u16(0x180D)],
        })
        .await;
    let mut peripheral = None;
    for _ in 0..10 {
        peripheral = heart_rate_peripherals(c)
            .await
            .into_iter()
            .find(|(_, d)| d.id == id)
            .map(|(p, _)| p);
        if peripheral.is_some() {
            break;
        }
        if let Wait::Cmd(cmd) = next_cmd(rx, Some(Duration::from_secs(1))).await {
            let _ = c.stop_scan().await;
            return Wait::Cmd(cmd);
        }
    }
    let _ = c.stop_scan().await;
    let Some(p) = peripheral else { return Wait::Retry };

    let subscribed = async {
        if !p.is_connected().await? {
            p.connect().await?;
        }
        p.discover_services().await?;
        let ch = p
            .characteristics()
            .into_iter()
            .find(|ch| ch.uuid == measurement)
            .ok_or(btleplug::Error::NoSuchCharacteristic)?;
        p.subscribe(&ch).await?;
        p.notifications().await
    };
    let mut notes = match subscribed.await {
        Ok(n) => n,
        Err(e) => {
            tracing::info!("hr connect: {e}");
            return Wait::Retry;
        }
    };
    set(state, Status::Connected);
    loop {
        tokio::select! {
            n = notes.next() => match n {
                Some(n) if n.uuid == measurement => match parse(&n.value) {
                    Ok(m) if m.usable() => lock(state).reading = Some((m.bpm, Instant::now())),
                    Ok(_) => {}
                    Err(e) => tracing::debug!("hr packet: {e:?}"),
                },
                Some(_) => {}
                None => return Wait::Retry,
            },
            cmd = rx.recv() => {
                let _ = p.disconnect().await;
                return cmd.map_or(Wait::Closed, Wait::Cmd);
            }
            _ = tokio::time::sleep(STALE) => {
                if !p.is_connected().await.unwrap_or(false) {
                    return Wait::Retry;
                }
            }
        }
    }
}
