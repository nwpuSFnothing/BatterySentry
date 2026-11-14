//! Passive receiver reports, ported and modified from BrimDeck (Apache-2.0).
use super::{State, interface, reading};
use crate::{
    drivers::Reading,
    hid::{Interface, Transport},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{io, thread, time::Duration};

#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Beken,
    Delux,
    Inphic,
    Rapoo,
    Aula,
    Steam,
    Jbl,
    Pulse,
    Flight,
}
pub(super) fn find(i: &Interface) -> Option<Kind> {
    if i.path.to_ascii_lowercase().contains("bth") {
        return None;
    }
    match (i.vid, i.pid) {
        (0x1d57, 0xfa60 | 0xfa62) if interface(i) == 2 && i.page == 0xa => Some(Kind::Beken),
        (0x1d57, 0xfa65) if i.page == 0xa => Some(Kind::Delux),
        (0x1d57, 0xfa65) if i.page == 0xff00 && i.usage == 1 => Some(Kind::Inphic),
        (
            0x24ae,
            0x1412 | 0x4612 | 0x1b13 | 0x4b13 | 0x1460 | 0x4660 | 0x1406 | 0x4606 | 0x1410 | 0x1411
            | 0x1417,
        ) if i.page == 0xff00 && i.usage == 2 && i.input_len >= 19 => Some(Kind::Rapoo),
        (0x38a6, 0x0030) if i.page == 0xffab => Some(Kind::Aula),
        (0x28de, 0x1304 | 0x1305) if (2..=5).contains(&interface(i)) && i.page >= 0xff00 => {
            Some(Kind::Steam)
        }
        (0x0ecb, 0x2088) if i.page == 0xff13 && i.usage == 1 => Some(Kind::Jbl),
        (0x054c, 0x0d5e) if i.page == 0xff01 && i.usage == 0x20 => Some(Kind::Pulse),
        (0x03f0, 0x0ac1) if i.page == 0xff90 => Some(Kind::Flight),
        _ => None,
    }
}
fn label(k: Kind, r: &[u8]) -> &'static str {
    match k {
        Kind::Beken => match r.get(1) {
            Some(0xb1) => "ATTACK SHARK X1",
            Some(0x55) => "ATTACK SHARK X11",
            Some(0x10) => "ATTACK SHARK R1",
            Some(0x85) => "ATTACK SHARK X6",
            Some(0x4d) => "ATTACK SHARK X3",
            Some(0xbe) => "ATTACK SHARK X11 Pro",
            Some(7) => "ATTACK SHARK X11 SE",
            _ => "ATTACK SHARK Device",
        },
        Kind::Delux => "DELUX M900 Pro",
        Kind::Inphic => "Inphic ZHEN",
        Kind::Rapoo => "Rapoo Mouse",
        Kind::Aula => "AULA G9",
        Kind::Steam => "Steam Controller",
        Kind::Jbl => "JBL Quantum 910 Wireless",
        Kind::Pulse => "PULSE 3D Wireless Headset",
        Kind::Flight => "HyperX Cloud Flight 2",
    }
}
pub(super) fn apply(k: Kind, last: Option<State>, r: &[u8]) -> Option<State> {
    use Kind::*;
    match k {
        Beken if r.len() >= 5 && r[0] == 3 && r[2] == 0x40 => {
            let n = if r[1] == 0x85 && (1..=10).contains(&r[4]) {
                r[4] * 10
            } else {
                r[4]
            };
            (n <= 100).then(|| State {
                coarse: r[1] == 0x85,
                ..State::new(n, matches!(r[3], 2 | 3 | 0x80))
            })
        }
        Delux if r.len() >= 5 && r[..3] == [3, 0x30, 0x40] && r[4] <= 100 => {
            Some(State::new(if r[3] == 2 { 100 } else { r[4] }, r[3] == 3))
        }
        Inphic if r.len() >= 5 && r[..3] == [3, 0x76, 0x41] && r[4] <= 100 => {
            Some(State::new(r[4], r[3] == 3))
        }
        Rapoo if r.len() >= 19 && r[0] == 7 => {
            if r[1] & 0x0f == 0 && r[18] & 0x80 == 0 {
                return Some(State {
                    charging: false,
                    asleep: true,
                    ..last.unwrap_or_default()
                });
            }
            (2..=100)
                .contains(&r[8])
                .then(|| State::new(r[8], r[7] == 2 && r[8] < 100))
        }
        Aula if r.len() >= 8
            && r[..4] == [0xab, 0x47, 0x4d, 0x86]
            && r[6] <= 100
            && (r[6] != 0 || r[7] != 0) =>
        {
            Some(State::new(r[6], r[7] == 2))
        }
        Steam if r.len() >= 15 && r[0] == 0x43 && matches!(r[1], 1 | 2 | 4) && r[2] <= 100 => {
            Some(State::new(r[2], r[1] == 2))
        }
        Steam if r.len() >= 2 && matches!(r[0], 0x79 | 0x46) && matches!(r[1], 1 | 2) => {
            last.map(|s| State {
                charging: s.charging && r[1] == 2,
                asleep: r[1] == 1,
                ..s
            })
        }
        Jbl if r.len() >= 2 && r[0] == 8 && r[1] <= 100 => Some(State::new(r[1], false)),
        Jbl if r.len() >= 2 && r[0] == 9 && r[1] <= 1 => last.map(|s| State {
            asleep: r[1] == 0,
            ..s
        }),
        Pulse if r.len() >= 5 && r[0] == 0xb0 => {
            if r[4] & 8 == 0 {
                return Some(State {
                    charging: false,
                    asleep: true,
                    ..last.unwrap_or_default()
                });
            }
            if r[3] == 0x80 {
                return Some(State {
                    charging: true,
                    asleep: false,
                    ..last.unwrap_or(State {
                        coarse: true,
                        ..State::default()
                    })
                });
            }
            (r[3] <= 100).then(|| State {
                coarse: true,
                ..State::new(r[3], false)
            })
        }
        Flight if r.len() >= 4 && r[..3] == [0xfb, 0x13, 0] && r[3] <= 100 => Some(State {
            percent: Some(r[3]),
            asleep: false,
            ..last.unwrap_or_default()
        }),
        Flight if r.len() >= 3 && r[..2] == [0xfb, 9] && r[2] <= 1 => Some(State {
            charging: r[2] == 1,
            ..last.unwrap_or_default()
        }),
        Flight if r.len() >= 3 && r[..2] == [0xfb, 0xa] && r[2] <= 1 => last.map(|s| State {
            asleep: r[2] == 0,
            ..s
        }),
        Flight if r.len() >= 3 && r[..2] == [0x51, 2] && r[2] <= 100 => Some(State {
            percent: Some(r[2]),
            asleep: false,
            ..last.unwrap_or_default()
        }),
        _ => None,
    }
}

pub(super) type Notify = Arc<dyn Fn() + Send + Sync>;
pub(super) struct Listener {
    stop: Arc<AtomicBool>,
    last: Arc<Mutex<Option<Reading>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener").finish_non_exhaustive()
    }
}
impl Listener {
    pub(super) fn start(i: Interface, k: Kind, notify: Option<Notify>) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let suffix = if matches!(k, Kind::Steam) {
            format!("slot:{}", interface(&i))
        } else {
            "device".into()
        };
        let mut pending = reading(&i, &suffix, label(k, &[]), State::default());
        pending.detail = "Waiting for receiver battery report".into();
        // Steam receivers expose empty controller slots as well as occupied ones.
        let last = Arc::new(Mutex::new(if matches!(k, Kind::Steam) {
            None
        } else {
            Some(pending)
        }));
        let (done, output) = (Arc::clone(&stop), Arc::clone(&last));
        let thread = thread::Builder::new()
            .name("receiver-events".into())
            .spawn(move || {
                let mut state = None;
                let mut title = label(k, &[]);
                while !done.load(Ordering::Relaxed) {
                    let Ok(t) = Transport::open_collection(&i) else {
                        for _ in 0..4 {
                            if done.load(Ordering::Relaxed) {
                                return;
                            }
                            thread::sleep(Duration::from_millis(250));
                        }
                        continue;
                    };
                    while !done.load(Ordering::Relaxed) {
                        match t.read(Duration::from_millis(250)) {
                            Ok(Some(r)) => {
                                if let Some(next) = apply(k, state, &r) {
                                    let name = if matches!(k, Kind::Beken) {
                                        label(k, &r)
                                    } else {
                                        title
                                    };
                                    if state != Some(next) || name != title {
                                        state = Some(next);
                                        title = name;
                                        let suffix = if matches!(k, Kind::Steam) {
                                            format!("slot:{}", interface(&i))
                                        } else {
                                            "device".into()
                                        };
                                        if let Ok(mut last) = output.lock() {
                                            *last = Some(reading(&i, &suffix, title, next));
                                        }
                                        if let Some(notify) = &notify {
                                            notify();
                                        }
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(_) => {
                                // Access-denied/disconnected collections must not become a busy reopen loop.
                                thread::sleep(Duration::from_millis(250));
                                break;
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            last,
            thread: Some(thread),
        })
    }
    pub(super) fn reading(&self) -> Option<Reading> {
        self.last.lock().ok()?.clone()
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
