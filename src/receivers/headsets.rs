//! Receiver headset battery protocols, ported and modified from BrimDeck (Apache-2.0).
use super::{State, drain, frame, interface, reading, send};
use crate::{
    drivers::Reading,
    hid::{Interface, Transport},
};
use std::{
    io,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy)]
enum Kind {
    Arctis7,
    Arctis9,
    Nova7,
    NovaSteps,
    NovaPro,
    Alpha,
    CloudII,
    Flight,
    Mchose,
    CloudIII,
    CloudIIIS,
    Alpha2,
}
fn kind(v: u16, p: u16) -> Option<Kind> {
    use Kind::*;
    match (v, p) {
        (0x1038, 0x1260 | 0x12ad | 0x1252 | 0x1280) => Some(Arctis7),
        (0x1038, 0x12c2 | 0x12c4) => Some(Arctis9),
        (0x1038, 0x22a1 | 0x227e | 0x2258 | 0x229e | 0x22ad | 0x22a9 | 0x22a5) => Some(Nova7),
        (0x1038, 0x2202 | 0x2206 | 0x223a | 0x227a | 0x22a4) => Some(NovaSteps),
        (0x1038, 0x12e0 | 0x12e5) => Some(NovaPro),
        (0x03f0, 0x098d) => Some(Alpha),
        (0x03f0, 0x0696 | 0x018b) => Some(CloudII),
        (0x0951, 0x16c4 | 0x1723) => Some(Flight),
        (0x03f0, 0x05b7 | 0x0c9d | 0x0995 | 0x0795) => Some(CloudIII),
        (0x03f0, 0x02cc | 0x06be) => Some(CloudIIIS),
        (0x03f0, 0x08be) => Some(Alpha2),
        (0x3837, 0x6045) => Some(Mchose),
        _ => None,
    }
}
pub(super) fn supported(i: &Interface) -> bool {
    kind(i.vid, i.pid).is_some()
}
fn channel(k: Kind, i: &Interface) -> bool {
    use Kind::*;
    match k {
        Arctis7 => interface(i) == 5,
        Arctis9 => interface(i) == 0,
        Nova7 | NovaSteps => i.page == 0xffc0,
        NovaPro => interface(i) == 4,
        CloudII | Mchose => i.page == 0xff90,
        CloudIII | Alpha2 => i.page == 0xff13,
        CloudIIIS => interface(i) == 3 && i.page >= 0xff00,
        _ => i.page >= 0xff00,
    }
}
fn request(k: Kind) -> Vec<u8> {
    use Kind::*;
    match k {
        Arctis7 => vec![6, 0x18],
        Arctis9 => vec![0, 0x20],
        Nova7 | NovaSteps => vec![0, 0xb0],
        NovaPro => frame(31, &[6, 0xb0]),
        Alpha => frame(31, &[0x21, 0xbb, 0x0b]),
        CloudII => frame(52, &[6, 0xff, 0xbb, 2]),
        Flight => frame(20, &[0x21, 0xff, 5]),
        Mchose => frame(64, &[0x55, 0x65, 1]),
        CloudIII => frame(62, &[0x66, 0x89]),
        CloudIIIS => frame(64, &[0x0c, 2, 3, 1, 0, 6]),
        Alpha2 => frame(64, &[0x50, 2]),
    }
}
fn charge_request(k: Kind) -> Option<Vec<u8>> {
    use Kind::*;
    match k {
        Alpha => Some(frame(31, &[0x21, 0xbb, 0x0c])),
        CloudII => Some(frame(52, &[6, 0xff, 0xbb, 3])),
        CloudIII => Some(frame(62, &[0x66, 0x8a])),
        CloudIIIS => Some(frame(64, &[0x0c, 2, 3, 1, 0, 0x48])),
        _ => None,
    }
}
fn charge(k: Kind, r: &[u8]) -> Option<bool> {
    use Kind::*;
    match k {
        Alpha if r.len() >= 4 && r[..3] == [0x21, 0xbb, 0x0c] => Some(r[3] == 1),
        CloudII if r.len() >= 5 && r[..4] == [6, 0xff, 0xbb, 3] => Some(r[4] == 1),
        CloudIII if r.len() >= 3 && r[0] == 0x66 && matches!(r[1], 0x8a | 0x0c) && r[2] <= 2 => {
            Some(r[2] == 1)
        }
        CloudIIIS if r.len() >= 7 && r[0] == 0x0c && r[5] == 0x48 && r[6] <= 2 => Some(r[6] == 1),
        CloudIIIS if r.len() >= 6 && r[0] == 0x0d && r[4] == 0x0a && r[5] <= 2 => Some(r[5] == 1),
        _ => None,
    }
}
fn battery(k: Kind, r: &[u8]) -> Option<State> {
    use Kind::*;
    match k {
        Arctis7 if r.len() >= 3 && r[0] == 6 && r[2] <= 100 => Some(State::new(r[2], false)),
        Arctis9 if r.len() >= 5 && (0x64..=0x9a).contains(&r[3]) => Some(State::new(
            ((f64::from(r[3] - 0x64) * 100.0 / 54.0).round_ties_even()) as u8,
            r[4] == 1,
        )),
        Nova7 if r.len() >= 4 && r[3] != 0 && r[2] <= 100 => {
            Some(State::new(r[2], matches!(r[3], 1 | 2)))
        }
        NovaSteps if r.len() >= 4 && r[3] != 0 && r[2] <= 4 => Some(State {
            coarse: true,
            ..State::new(r[2] * 25, matches!(r[3], 1 | 2))
        }),
        NovaPro if r.len() >= 16 && r[0] == 6 && r[15] != 1 && r[6] <= 8 => Some(State {
            coarse: true,
            ..State::new((f64::from(r[6]) * 12.5).round_ties_even() as u8, r[15] == 2)
        }),
        Alpha if r.len() >= 4 && r[..3] == [0x21, 0xbb, 0x0b] && r[3] <= 100 => {
            Some(State::new(r[3], false))
        }
        CloudII if r.len() >= 8 && r[..4] == [6, 0xff, 0xbb, 2] && r[7] <= 100 => {
            Some(State::new(r[7], false))
        }
        Flight if r.len() >= 5 && r[..3] == [0x21, 0xff, 5] => {
            let mv = u16::from_be_bytes([r[3], r[4]]);
            if mv <= 3000 {
                return None;
            }
            Some(State {
                percent: if mv > 4107 {
                    None
                } else {
                    Some(
                        ((f64::from(mv) - 3648.0) * 100.0 / 327.0)
                            .round_ties_even()
                            .clamp(0.0, 100.0) as u8,
                    )
                },
                charging: mv > 4107,
                coarse: true,
                asleep: false,
            })
        }
        Mchose if r.len() >= 4 && r[..2] == [0x55, 0x65] && r[3] == 2 && r[2] <= 100 => {
            Some(State::new(r[2], false))
        }
        CloudIII
            if r.len() >= 5
                && r[0] == 0x66
                && matches!(r[1], 0x89 | 0x0d)
                && (r[2] != 0 || r[3] != 0)
                && r[4] <= 100 =>
        {
            Some(State::new(r[4], false))
        }
        CloudIIIS if r.len() >= 7 && r[0] == 0x0c && r[5] == 6 && r[6] <= 100 => {
            Some(State::new(r[6], false))
        }
        CloudIIIS if r.len() >= 6 && r[0] == 0x0d && r[4] == 1 && r[5] <= 100 => {
            Some(State::new(r[5], false))
        }
        Alpha2 if r.len() >= 7 && r[..2] == [0x51, 2] && r[2] <= 100 => {
            Some(State::new(r[2], r[6] & 0x80 != 0))
        }
        _ => None,
    }
}
fn label(k: Kind) -> &'static str {
    use Kind::*;
    match k {
        Arctis7 => "SteelSeries Arctis 7",
        Arctis9 => "SteelSeries Arctis 9",
        Nova7 | NovaSteps => "SteelSeries Arctis Nova 7",
        NovaPro => "SteelSeries Arctis Nova Pro Wireless",
        Alpha => "HyperX Cloud Alpha Wireless",
        CloudII => "HyperX Cloud II Wireless",
        Flight => "HyperX Cloud Flight",
        Mchose => "MCHOSE X9",
        CloudIII => "HyperX Cloud III Wireless",
        CloudIIIS => "HyperX Cloud III S Wireless",
        Alpha2 => "HyperX Cloud Alpha 2",
    }
}
fn exchange<T>(
    devices: &[Transport],
    i: &Interface,
    k: Kind,
    request: &[u8],
    parse: impl Fn(&[u8]) -> Option<T>,
    ms: u64,
) -> io::Result<Option<T>> {
    for t in devices {
        drain(t);
    }
    if let Err(e) = send(&devices[0], i, request) {
        if matches!(k, Kind::CloudIII) && e.raw_os_error() == Some(1) {
            devices[0].set_feature(request, usize::from(i.feature_len))?;
        } else {
            return Err(e);
        }
    }
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        for t in devices {
            if let Some(r) = t.read(
                end.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            )? {
                let r = if r.first() == Some(&0) { &r[1..] } else { &r };
                if let Some(s) = parse(r) {
                    return Ok(Some(s));
                }
            }
        }
    }
    Ok(None)
}
pub(super) fn read(infos: &[&Interface]) -> Option<Reading> {
    let first = infos.first()?;
    let k = kind(first.vid, first.pid)?;
    let wait = match k {
        Kind::Alpha2 => 1500,
        Kind::CloudIII | Kind::CloudIIIS => 1000,
        _ => 600,
    };
    for i in infos.iter().filter(|i| channel(k, i)) {
        let Ok(t) = Transport::open_collection(i) else {
            continue;
        };
        let mut devices = vec![t];
        if matches!(k, Kind::CloudIIIS) {
            devices.extend(
                infos
                    .iter()
                    .filter(|other| {
                        other.path != i.path
                            && interface(other) == interface(i)
                            && other.input_len > 0
                    })
                    .filter_map(|other| Transport::open_collection(other).ok()),
            );
        }
        if matches!(k, Kind::Alpha)
            && exchange(
                &devices,
                i,
                k,
                &frame(31, &[0x21, 0xbb, 3]),
                |r| (r.len() >= 4 && r[..3] == [0x21, 0xbb, 3]).then(|| r[3] != 1),
                wait,
            )
            .ok()
            .flatten()
                == Some(false)
        {
            continue;
        }
        let Ok(Some(mut s)) = exchange(&devices, i, k, &request(k), |r| battery(k, r), wait) else {
            continue;
        };
        if let Some(req) = charge_request(k) {
            s.charging |= exchange(&devices, i, k, &req, |r| charge(k, r), wait)
                .ok()
                .flatten()
                .unwrap_or(false);
        }
        return Some(reading(i, "device", label(k), s));
    }
    None
}
