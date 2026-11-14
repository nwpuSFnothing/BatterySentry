//! Request/reply receiver transports. Ported and modified from BrimDeck; see licenses/.
use super::{
    AsusModel, F83Model, Protocol, RazerModel, State, SteelModel, catalog, collection, drain, find,
    frame, headsets, interface, logitech, name, reading, receive, send, transact,
};
use crate::{
    drivers::Reading,
    hid::{Interface, Transport},
};
use std::{
    io, thread,
    time::{Duration, Instant},
};

pub(super) fn group(infos: &[&Interface]) -> Vec<Reading> {
    if let Some(i) = infos.first() {
        if i.vid == 0x046d {
            return logitech::read(infos);
        }
        if i.vid == 0x1532 {
            return razer(infos);
        }
        if i.vid == 0x0b05 && i.pid == 0x1ace {
            return omni(infos);
        }
        if i.vid == 0x1b1c {
            return corsair(infos);
        }
    }
    let mut found = Vec::new();
    if let Some(read) = headsets::read(infos) {
        found.push(read);
        return found;
    }
    if let Some(read) = playstation(infos) {
        found.push(read);
        return found;
    }
    for i in infos {
        let Some(protocol) = find(i) else { continue };
        if let Ok(Some((state, label))) = query(i, protocol) {
            found.push(reading(i, "device", &label, state));
            break;
        }
    }
    if found.is_empty() {
        for i in infos.iter().filter(|i| !i.battery_features.is_empty()) {
            let Ok(t) = Transport::open_collection(i) else {
                continue;
            };
            for feature in &i.battery_features {
                if let Ok(Some(n)) = t.battery_feature(&i.path, feature, usize::from(i.feature_len))
                {
                    found.push(reading(
                        i,
                        "device",
                        name(i, "Wireless Device"),
                        State::new(n, false),
                    ));
                    return found;
                }
            }
        }
    }
    found
}

fn playstation(infos: &[&Interface]) -> Option<Reading> {
    let i = infos.iter().copied().find(|i| {
        i.vid == 0x054c
            && matches!(i.pid, 0x0ce6 | 0x0df2 | 0x05c4 | 0x09cc | 0x0ba0)
            && i.page == 1
            && i.usage == 5
    })?;
    let t = Transport::open_collection(i).ok()?;
    receive(
        &t,
        |r| {
            if r.len() < 64 || r[0] != 1 {
                return None;
            }
            let sense = matches!(i.pid, 0x0ce6 | 0x0df2);
            let b = r[if sense { 53 } else { 30 }];
            let n = b & 0xf;
            let s = if sense {
                match b >> 4 {
                    0 => State::new(n.saturating_mul(10).saturating_add(5).min(100), false),
                    1 => State::new(n.saturating_mul(10).saturating_add(5).min(100), true),
                    2 => State::new(100, false),
                    _ => return None,
                }
            } else if b & 0x10 != 0 {
                match n {
                    0..=9 => State::new(n * 10 + 5, true),
                    10 => State::new(100, true),
                    11 => State::new(100, false),
                    _ => return None,
                }
            } else {
                if n > 10 {
                    return None;
                }
                State::new((n * 10 + 5).min(100), false)
            };
            Some(reading(
                i,
                "device",
                name(i, if sense { "DualSense" } else { "DualShock 4" }),
                State { coarse: true, ..s },
            ))
        },
        Duration::from_millis(300),
    )
    .ok()
    .flatten()
}

fn feature(
    t: &Transport,
    i: &Interface,
    request: &[u8],
    wait: u64,
    parse: impl Fn(&[u8]) -> Option<State>,
) -> io::Result<Option<State>> {
    t.set_feature(request, usize::from(i.feature_len))?;
    thread::sleep(Duration::from_millis(wait));
    Ok(parse(&t.get_feature(
        request[0],
        usize::from(i.feature_len).max(request.len()),
    )?))
}
fn compx_request() -> Vec<u8> {
    let mut f = frame(17, &[8, 4]);
    f[16] = 0x55u8.wrapping_sub(12);
    f
}
fn compx_reply(r: &[u8], strict: bool) -> Option<State> {
    if r.len() < 8 || r[0] != 8 || r[1] != 4 || r[6] > 100 {
        return None;
    }
    if strict
        && (r.len() < 17
            || r[..17].iter().fold(0u8, |s, b| s.wrapping_add(*b)) != 0x55
            || r[2..16].iter().all(|b| *b == 0))
    {
        return None;
    }
    Some(State::new(r[6], r[7] == 1))
}
fn bitmouse(command: u8, target: u8) -> Vec<u8> {
    let mut f = frame(64, &[8, 0, 0x72, 2, 0, target, command, 1]);
    f[1] = f[2..].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    f
}
fn query(i: &Interface, p: Protocol) -> io::Result<Option<(State, String)>> {
    use Protocol::*;
    let t = Transport::open_collection(i)?;
    let (state, label) = match p {
        BitMouse => {
            let mut state = None;
            for target in [1, 0] {
                state = transact(
                    &t,
                    i,
                    &bitmouse(7, target),
                    |r| {
                        if r.len() < 10
                            || r[0] != 8
                            || r[1] != 0x72
                            || r[2] != 0
                            || r[5] != 7
                            || r[7] > 100
                        {
                            return None;
                        }
                        let mv = u16::from_le_bytes([r[8], r[9]]);
                        (r[7] != 0 || mv >= 2000).then(|| State::new(r[7], false))
                    },
                    500,
                )?;
                if let Some(mut s) = state {
                    s.charging = transact(
                        &t,
                        i,
                        &bitmouse(0x0f, target),
                        |r| {
                            (r.len() >= 8 && r[0] == 8 && r[1] == 0x72 && r[2] == 0 && r[5] == 0x0f)
                                .then(|| r[7] == 1)
                        },
                        500,
                    )?
                    .unwrap_or(false);
                    state = Some(s);
                    break;
                }
            }
            (state, "ATK / VXE Device")
        }
        Compx | CompxFramed => {
            let request = compx_request();
            let mut state = None;
            if matches!(p, Compx) && t.set_feature(&request, usize::from(i.feature_len)).is_ok() {
                state = t
                    .get_feature(8, usize::from(i.feature_len).max(17))
                    .ok()
                    .and_then(|r| compx_reply(&r, false));
            }
            if state.is_none() {
                state = transact(
                    &t,
                    i,
                    &request,
                    |r| compx_reply(r, matches!(p, CompxFramed)),
                    600,
                )?;
            }
            (state, "ATK / VXE / Pulsar Device")
        }
        Vgn | Cherry | Hecate => {
            let len = if matches!(p, Vgn) {
                6
            } else if matches!(p, Cherry) {
                4
            } else {
                0
            };
            let s = transact(
                &t,
                i,
                &frame(64, &[4, 0, 0, 0x1a, len]),
                |r| {
                    (r.len() >= 10 && r[0] == 4 && r[3] == 0x1a && r[8] <= 100).then(|| {
                        State::new(
                            r[8],
                            if matches!(p, Hecate) {
                                r[9] == 1
                            } else if matches!(p, Cherry) {
                                r[9] & 1 != 0
                            } else {
                                r[9] != 0
                            },
                        )
                    })
                },
                700,
            )?;
            (
                s,
                if matches!(p, Cherry) {
                    "CHERRY PIXIU75"
                } else if matches!(p, Hecate) {
                    "HECATE G5M Pro"
                } else if i.vid == 0x373b {
                    "ATK Z87 Pro"
                } else {
                    "VGN Keyboard"
                },
            )
        }
        Endgame => (
            feature(
                &t,
                i,
                &frame(usize::from(i.feature_len), &[0xa1, 0xb4]),
                350,
                |r| {
                    (r.len() >= 17 && r[0] == 0xa1 && r[1] == 1 && r[16] <= 100)
                        .then(|| State::new(r[16], false))
                },
            )?,
            "Endgame Gear Device",
        ),
        Vaxee => (
            feature(
                &t,
                i,
                &frame(usize::from(i.feature_len), &[0x0e, 0xa5, 0x0b, 1, 1]),
                120,
                |r| {
                    (r.len() >= 6 && r[0] == 0x0e && r[5] <= 20).then(|| State {
                        coarse: true,
                        ..State::new(r[5] * 5, false)
                    })
                },
            )?,
            "VAXEE Device",
        ),
        Nordic => (
            feature(
                &t,
                i,
                &frame(usize::from(i.feature_len), &[5, 0x15, 0, 0, 1]),
                90,
                |r| {
                    if r.len() < 13 || r[0] != 5 || r[9] > 100 {
                        return None;
                    }
                    Some(if r[12] == 0 || r[9] == 0 {
                        State {
                            asleep: true,
                            ..State::default()
                        }
                    } else {
                        State::new(
                            if r[11] != 0 { 100 } else { r[9] },
                            r[10] != 0 && r[11] == 0,
                        )
                    })
                },
            )?,
            "Ninjutso Sora V2",
        ),
        Inphic => {
            let request = |cmd: u8, payload: &[u8]| {
                let mut f = frame(33, &[0x0c, 1, cmd, 0, 1]);
                f[5..5 + payload.len()].copy_from_slice(payload);
                f
            };
            t.set_feature(&request(0xf0, &[1, 1, 0xff]), usize::from(i.feature_len))?;
            thread::sleep(Duration::from_millis(10));
            (
                feature(&t, i, &request(0x20, &[]), 10, |r| {
                    (r.len() >= 20 && r[0] == 0x0c && r[2] == 0x20 && r[6] != 0 && r[18] <= 100)
                        .then(|| State::new(r[18], r[19] == 2))
                })?,
                "Inphic Device",
            )
        }
        Mchose => {
            let mut f = frame(64, &[0x4d, 1, 1, 0, 0, 9]);
            f[8] = f[2..8].iter().fold(0, |a, b| a ^ b);
            let mut state = None;
            for attempt in 0..3 {
                if attempt > 0 {
                    thread::sleep(Duration::from_millis(30));
                }
                state = transact(
                    &t,
                    i,
                    &f,
                    |r| {
                        (r.len() >= 21
                            && r[0] == 0x4d
                            && r[4] == 0
                            && r[5] == 9
                            && r[8] != 0xff
                            && (1..=100).contains(&r[20]))
                        .then(|| State::new(r[20], r[19] != 0))
                    },
                    400,
                )?;
                if state.is_some() {
                    break;
                }
            }
            (state, "MCHOSE Device")
        }
        Ajazz => {
            let s = transact(
                &t,
                i,
                &frame(33, &[0, 0x10]),
                |r| {
                    let status_offset = usize::from(r.len() > 2 && r[0] == 0 && r[1] == 0xc0);
                    if r.len() >= status_offset + 3
                        && r[status_offset] == 0xc0
                        && r[status_offset + 1] == 1
                        && r[status_offset + 2] <= 100
                    {
                        return Some((State::new(r[status_offset + 2], false), String::new()));
                    }
                    let o = usize::from(r.len() > 15 && r[0] == 0 && r[1] == 0x10);
                    if r.len() < o + 15
                        || r[o] != 0x10
                        || r[o + 13] > 100
                        || r[o + 14] == 0 && r[o + 13] == 0
                    {
                        return None;
                    }
                    let end = r[o + 4..o + 8]
                        .iter()
                        .position(|b| !(0x20..=0x7e).contains(b))
                        .unwrap_or(4);
                    let label = String::from_utf8_lossy(&r[o + 4..o + 4 + end]).into_owned();
                    Some((
                        State {
                            asleep: i.vid == 0x249a && r[o + 14] == 0,
                            ..State::new(r[o + 13], r[o + 12] != 0)
                        },
                        label,
                    ))
                },
                1500,
            )?;
            return Ok(s.map(|(s, id)| {
                (
                    s,
                    if id.is_empty() {
                        "AJAZZ Device".into()
                    } else {
                        format!("AJAZZ {id}")
                    },
                )
            }));
        }
        F83(m) => {
            let mut request = vec![0; 65];
            if m.old {
                request[2] = 2;
                request[3] = 0x8f;
                request[4] = u8::from(!m.cable);
            } else {
                request[3] = 2;
                request[4] = 2;
                request[6] = 0x83;
            }
            t.set_feature(&request, usize::from(i.feature_len))?;
            thread::sleep(Duration::from_millis(34));
            let mut s = None;
            for attempt in 0..15 {
                if attempt > 0 {
                    thread::sleep(Duration::from_millis(50));
                }
                s = f83_reply(m, &t.get_feature(0, usize::from(i.feature_len).max(65))?);
                if s.is_some() {
                    break;
                }
            }
            (s, m.name)
        }
        Realtek => {
            for long in [false, true] {
                let mut request = vec![0xff; if long { 65 } else { 21 }];
                request[0] = if long { 0x12 } else { 0x11 };
                request[1] = 0xf9;
                if t.set_feature(&request, usize::from(i.feature_len)).is_err() {
                    continue;
                }
                thread::sleep(Duration::from_millis(120));
                let r = t.get_feature(request[0], usize::from(i.feature_len).max(request.len()))?;
                if r.len() >= 13
                    && matches!(r[0], 0x11 | 0x12)
                    && r[1] == 0xf9
                    && matches!(u16::from_le_bytes([!r[2], !r[3]]), 0x5253 | 0x3837)
                    && !r[11] <= 100
                {
                    return Ok(Some((
                        State::new(!r[11], r[12] != 0xff),
                        if u16::from_le_bytes([!r[4], !r[5]]) == 0x0031 {
                            "MCHOSE M7 Ultra".into()
                        } else {
                            name(i, "MCHOSE Device").into()
                        },
                    )));
                }
            }
            (None, "MCHOSE Device")
        }
        Gmouse => (
            transact(
                &t,
                i,
                &frame(65, &[0, 0x55, 0x30, 0xa5, 0x0b, 0x2e, 1, 1, 1]),
                |r| {
                    let o = if r.starts_with(&[0xaa, 0x30]) {
                        0
                    } else if r.len() > 2 && r[1..].starts_with(&[0xaa, 0x30]) {
                        1
                    } else {
                        return None;
                    };
                    (r.len() >= o + 10 && r[o + 8] <= 100)
                        .then(|| State::new(r[o + 8], r[o + 9] != 0))
                },
                800,
            )?,
            "MCHOSE Device",
        ),
        Lingbao => (
            feature(&t, i, &frame(65, &[0, 0xf7]), 50, |r| {
                (r.len() >= 6 && r[0] == 0 && r[2] == 0 && r[4] == 1 && r[3] <= 100)
                    .then(|| State::new(r[3], r[5] == 1))
            })?,
            "LINGBAO M5PRO Max",
        ),
        Nzxt => (
            transact(
                &t,
                i,
                &frame(64, &[0x4e, 2, 0x81, 0, 0xb0]),
                |r| {
                    if r.len() < 27 || !r.starts_with(&[0x4e, 2, 0x97]) {
                        return None;
                    }
                    let n = u16::from_le_bytes([r[17], r[18]]);
                    (n <= 100).then(|| State::new(n as u8, r[22] & 1 != 0))
                },
                1500,
            )?,
            "NZXT Lift Elite",
        ),
        Asus(m) => (asus(&t, i, m)?, m.name),
        Steel(m) => {
            let request = if m.command == 0xaa {
                vec![0, 0xaa, 1]
            } else {
                vec![0, m.command]
            };
            (
                transact(&t, i, &request, |r| steel_reply(m, r), 600)?,
                m.name,
            )
        }
        Rapoo => {
            let end = Instant::now() + Duration::from_millis(1500);
            let mut state = None;
            for _ in 0..4 {
                if Instant::now() >= end {
                    break;
                }
                send(
                    &t,
                    i,
                    &frame(33, &[0xba, if i.pid == 0x4431 { 0xff } else { 0xa5 }, 0xaa]),
                )?;
                let sent = Instant::now();
                let mut busy = false;
                while Instant::now() < end {
                    thread::sleep(Duration::from_millis(5));
                    let Ok(r) = t.get_input(0xba, usize::from(i.input_len).max(33)) else {
                        break;
                    };
                    let o = usize::from(r.len() > 1 && r[0] == 0xba);
                    if r.len() < o + 3 {
                        continue;
                    }
                    if r[o] != 1 {
                        busy = true;
                        continue;
                    }
                    if busy && matches!(r[o + 1], 1 | 2) {
                        state = Some(State::new(r[o + 2].min(100), r[o + 1] == 2));
                        break;
                    }
                    if sent.elapsed() >= Duration::from_millis(100) {
                        break;
                    }
                }
                if state.is_some() {
                    break;
                }
            }
            (state, "Rapoo VT3 PRO")
        }
    };
    Ok(state.map(|s| {
        (
            s,
            match p {
                Asus(_) | F83(_) | Steel(_) | Nzxt | Lingbao | Cherry | Hecate => label.to_owned(),
                _ => name(i, label).to_owned(),
            },
        )
    }))
}

pub(super) fn f83_reply(m: &F83Model, r: &[u8]) -> Option<State> {
    for o in [1, 0] {
        if r.len() < o + if m.old { 6 } else { 8 } {
            continue;
        }
        let shaped = if m.old {
            r[o + 1] == 2 && r[o + 2] == 0x8f
        } else {
            r[o + 4] == 0 && r[o + 5] == 0x83
        };
        if !shaped || !matches!(r[o], 0xa1 | 0xa2) {
            continue;
        }
        if r[o] == 0xa2 && (m.strict || m.old) {
            return Some(State {
                asleep: true,
                ..State::default()
            });
        }
        if r[o] == 0xa2 && m.clamp {
            continue;
        }
        let (c, n) = if m.old {
            (r[o + 4], r[o + 5])
        } else {
            (r[o + 6], r[o + 7])
        };
        if n > 100 && !m.clamp {
            return None;
        }
        return Some(State::new(n.min(100), c != 0));
    }
    None
}
fn steel_reply(m: &SteelModel, r: &[u8]) -> Option<State> {
    let o = usize::from(r.len() > 1 && r[0] == 0);
    if m.command == 0xaa {
        if r.len() >= o + 4 && r[o] == 0xaa {
            return (r[o + 1] <= 100).then(|| State::new(r[o + 1], r[o + 3] != 0));
        }
        return (r.len() >= o + 3 && r[o] <= 100 && r[o + 2] <= 1)
            .then(|| State::new(r[o], r[o + 2] == 1));
    }
    if r.len() < o + 2 || r[o] != m.command {
        return None;
    }
    let n = r[o + 1] & 0x7f;
    if n == 0 {
        return Some(State {
            asleep: true,
            ..State::default()
        });
    }
    Some(State {
        coarse: n <= 21,
        ..State::new(
            if n > 21 { n.min(100) } else { (n - 1) * 5 },
            r[o + 1] & 0x80 != 0,
        )
    })
}

fn asus(t: &Transport, i: &Interface, m: &AsusModel) -> io::Result<Option<State>> {
    let cmd = if m.family == 1 { 1 } else { 7 };
    let req = |cmd| frame(usize::from(i.output_len), &[m.report, 0x12, cmd]);
    for _ in 0..3 {
        let reply = transact(
            t,
            i,
            &req(cmd),
            |r| {
                if r.len() < 12 || r[0] != m.report {
                    return None;
                }
                if r[1..3] == [0xff, 0xaa] || r[1..4] == [0, 0, 0] {
                    return Some(None);
                }
                if r[1] != 0x12 || r[2] != cmd {
                    return None;
                }
                let n = r[m.level_at];
                if n > if m.steps { 4 } else { 100 } {
                    return Some(None);
                }
                Some(Some(State {
                    coarse: m.steps,
                    ..State::new(
                        if m.steps { n * 25 } else { n },
                        if m.family == 1 {
                            r[9] == 1
                        } else if m.family == 0 {
                            r[10] != 0
                        } else {
                            false
                        },
                    )
                }))
            },
            300,
        )?;
        if let Some(mut state) = reply {
            if m.family == 2
                && let Some(s) = &mut state
            {
                s.charging = transact(
                    t,
                    i,
                    &req(8),
                    |r| {
                        (r.len() >= 6 && r[0] == m.report && r[1] == 0x12 && r[2] == 8)
                            .then(|| r[5] == 1)
                    },
                    300,
                )?
                .unwrap_or(false);
            }
            return Ok(state);
        }
    }
    Ok(None)
}
fn omni(infos: &[&Interface]) -> Vec<Reading> {
    let channel = |col| {
        infos
            .iter()
            .copied()
            .find(|i| interface(i) == 2 && collection(i) == col && i.output_len >= 64)
    };
    let Some(i) = channel(1) else {
        return Vec::new();
    };
    let Ok(t) = Transport::open_collection(i) else {
        return Vec::new();
    };
    let table = transact(
        &t,
        i,
        &frame(usize::from(i.output_len), &[1, 0xa0]),
        |r| (r.len() >= 5 && r[..3] == [1, 0xa0, 0]).then(|| r.to_vec()),
        700,
    )
    .ok()
    .flatten();
    let mut found = Vec::new();
    if let Some(r) = table {
        for entry in r[5..].chunks_exact(4).take(usize::from(r[3])) {
            let pid = u16::from_le_bytes([entry[0], entry[1]]);
            if pid == 0 {
                break;
            }
            let Some(m) = catalog::OMNI.iter().find(|m| m.pid == pid) else {
                continue;
            };
            if !(1..16).contains(&entry[2]) {
                continue;
            }
            let Some(i) = channel(i32::from(entry[2])) else {
                continue;
            };
            let model = AsusModel {
                report: entry[2],
                collection: i32::from(entry[2]),
                ..*m
            };
            if let Ok(t) = Transport::open_collection(i)
                && let Ok(Some(s)) = asus(&t, i, &model)
            {
                found.push(reading(i, &format!("omni:{pid:04x}"), m.name, s));
            }
        }
    }
    found
}

fn razer(infos: &[&Interface]) -> Vec<Reading> {
    let Some(first) = infos.first() else {
        return Vec::new();
    };
    let Some(m) = catalog::RAZER.iter().find(|m| m.pid == first.pid) else {
        return Vec::new();
    };
    for i in infos
        .iter()
        .filter(|i| interface(i) < 0 || interface(i) == m.interface)
    {
        let Ok(t) = Transport::open_collection(i) else {
            continue;
        };
        if let Ok(Some(n)) = razer_query(&t, i, m, 0x80) {
            let charging = m.charging
                && razer_query(&t, i, m, 0x84)
                    .ok()
                    .flatten()
                    .is_some_and(|v| v != 0);
            return vec![reading(
                i,
                "device",
                m.name,
                State::new(((u16::from(n) * 100 + 127) / 255) as u8, charging),
            )];
        }
    }
    Vec::new()
}
fn razer_query(t: &Transport, i: &Interface, m: &RazerModel, cmd: u8) -> io::Result<Option<u8>> {
    let mut r = vec![0; 91];
    r[2] = m.transaction;
    r[6] = 2;
    r[7] = 7;
    r[8] = cmd;
    r[89] = r[3..89].iter().fold(0, |a, b| a ^ b);
    for attempt in 0..5 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(10));
        }
        t.set_feature(&r, usize::from(i.feature_len))?;
        thread::sleep(Duration::from_micros(m.wait_us.max(1000)));
        let r = t.get_feature(0, usize::from(i.feature_len).max(91))?;
        if r.len() < 91
            || r[3..89].iter().fold(0u8, |a, b| a ^ b) != r[89]
            || r[7] != 7
            || r[8] != cmd
        {
            continue;
        }
        if r[1] == 2 {
            return Ok(Some(r[10]));
        }
        if !matches!(r[1], 1 | 4) {
            return Ok(None);
        }
    }
    Ok(None)
}

fn corsair(infos: &[&Interface]) -> Vec<Reading> {
    let Some(first) = infos.first() else {
        return Vec::new();
    };
    let (label, bare, session) = match first.pid {
        0x1b7f => ("Corsair Dark Core RGB Pro SE", false, false),
        0x0a40 => ("Corsair Virtuoso RGB Wireless SE", true, true),
        0x0a3e | 0x0a3d => ("Corsair Virtuoso RGB Wireless SE", true, false),
        0x0a64 | 0x0a62 => ("Corsair Virtuoso RGB Wireless XT", true, false),
        _ => return Vec::new(),
    };
    let deadline = Instant::now() + Duration::from_secs(7);
    let remaining = |max: u64| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(max))
    };
    let request = |prop, route| {
        frame(
            if bare { 64 } else { 65 },
            &[if bare { 2 } else { 0 }, route, 2, prop],
        )
    };
    let percent = |r: &[u8]| {
        corsair_value(r)
            .filter(|v| *v > 0 && *v <= 1000)
            .map(|v| (v / 10) as u8)
    };
    let mut candidates = infos
        .iter()
        .copied()
        .filter(|i| i.page == 0xff42)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates.extend(infos.iter().copied().filter(|i| i.page >= 0xff00));
    }
    candidates.sort_by_key(|i| i.usage != 1);
    for i in candidates {
        if remaining(1).is_zero() {
            break;
        }
        let Ok(t) = Transport::open_collection(i) else {
            continue;
        };
        drain(&t);
        if session {
            for (prop, route) in [(0x13, 8), (0x12, 8), (0x12, 9)] {
                if send(&t, i, &request(prop, route)).is_err() {
                    break;
                }
                let _ = receive(&t, |_: &[u8]| None::<()>, remaining(800));
            }
        }
        let routes: &[u8] = if matches!(i.pid, 0x0a3d | 0x0a62) {
            &[8, 9]
        } else if matches!(i.pid, 0x0a3e | 0x0a64) {
            &[9, 8]
        } else {
            &[9]
        };
        for route in routes {
            if send(&t, i, &request(0x0f, *route)).is_err() {
                break;
            }
            let Ok(Some(n)) = receive(&t, percent, remaining(6000)) else {
                continue;
            };
            if send(&t, i, &request(0x0f, *route)).is_ok()
                && let Ok(Some(second)) = receive(&t, percent, remaining(3000))
                && n != second
            {
                return Vec::new();
            }
            let charging = bare
                && send(&t, i, &request(0x10, *route)).is_ok()
                && receive(
                    &t,
                    |r| {
                        corsair_value(r).and_then(|v| match v {
                            1 => Some(true),
                            2 => Some(false),
                            _ => None,
                        })
                    },
                    remaining(3000),
                )
                .ok()
                .flatten()
                .unwrap_or(false);
            return vec![reading(i, "device", label, State::new(n, charging))];
        }
    }
    Vec::new()
}
fn corsair_value(r: &[u8]) -> Option<u16> {
    let r = if r.len() >= 65 && r[0] == 0 {
        &r[1..]
    } else {
        r
    };
    if r.len() < 5 || r[0] != 1 {
        return None;
    }
    if r[1..3] == [2, 0] {
        return Some(u16::from_le_bytes([r[3], r[4]]));
    }
    if r.len() >= 6 && r[1..4] == [1, 2, 0] {
        return Some(u16::from_le_bytes([r[4], r[5]]));
    }
    None
}
