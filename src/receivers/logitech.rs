//! Logitech HID++ 1.0/2.0 receiver battery reads, ported and modified from BrimDeck.
use super::{State, drain, frame, reading, send};
use crate::{
    drivers::Reading,
    hid::{Interface, Transport},
};
use std::{
    collections::HashMap,
    io,
    time::{Duration, Instant},
};
enum Answer {
    Data(Vec<u8>),
    Error(u8),
    Timeout,
}
struct Channel<'a> {
    infos: Vec<&'a Interface>,
    transports: Vec<Transport>,
}
impl Channel<'_> {
    fn exchange(
        &self,
        sender: usize,
        request: &[u8],
        parse: impl Fn(&[u8]) -> Option<Answer>,
        ms: u64,
    ) -> io::Result<Answer> {
        for t in &self.transports {
            drain(t);
        }
        send(&self.transports[sender], self.infos[sender], request)?;
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            for t in &self.transports {
                if let Some(r) = t.read(
                    end.saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(10)),
                )? && let Some(answer) = parse(&r)
                {
                    return Ok(answer);
                }
            }
        }
        Ok(Answer::Timeout)
    }
    fn request(
        &self,
        index: u8,
        feature: u8,
        function: u8,
        parameters: &[u8],
        ms: u64,
    ) -> io::Result<Answer> {
        let fsw = function << 4 | 0x0e;
        let mut f = frame(20, &[0x11, index, feature, fsw]);
        let count = parameters.len().min(16);
        f[4..4 + count].copy_from_slice(&parameters[..count]);
        self.exchange(
            0,
            &f,
            |r| {
                if r.len() < 6 || !matches!(r[0], 0x10 | 0x11) || r[1] != index {
                    return None;
                }
                if r[2] == feature && r[3] == fsw {
                    return Some(Answer::Data(r[4..].to_vec()));
                }
                if matches!((r[0], r[2]), (0x11, 0xff) | (0x10, 0x8f))
                    && r[3] == feature
                    && r[4] == fsw
                {
                    return Some(Answer::Error(r[5]));
                }
                None
            },
            ms,
        )
    }
    fn register(&self, index: u8, register: u8) -> Option<Vec<u8>> {
        let short = self.infos.iter().position(|i| i.usage == 1)?;
        match self
            .exchange(
                short,
                &[0x10, index, 0x81, register, 0, 0, 0],
                |r| {
                    if r.len() < 6 || r[1] != index {
                        return None;
                    }
                    if r[2] == 0x81 && r[3] == register {
                        Some(Answer::Data(r[4..].to_vec()))
                    } else if r[2] == 0x8f && r[3] == 0x81 && r[4] == register {
                        Some(Answer::Error(r[5]))
                    } else {
                        None
                    }
                },
                400,
            )
            .ok()?
        {
            Answer::Data(r) => Some(r),
            _ => None,
        }
    }
}
struct Device<'a, 'b> {
    channel: &'a Channel<'b>,
    index: u8,
    features: HashMap<u16, u8>,
}
impl Device<'_, '_> {
    fn call(&mut self, feature: u16, function: u8, parameters: &[u8]) -> Option<Vec<u8>> {
        let idx = if let Some(idx) = self.features.get(&feature) {
            *idx
        } else {
            let Answer::Data(r) = self
                .channel
                .request(self.index, 0, 0, &feature.to_be_bytes(), 400)
                .ok()?
            else {
                return None;
            };
            let idx = *r.first()?;
            self.features.insert(feature, idx);
            idx
        };
        if idx == 0 {
            return None;
        }
        match self
            .channel
            .request(self.index, idx, function, parameters, 400)
            .ok()?
        {
            Answer::Data(r) => Some(r),
            _ => None,
        }
    }
    fn name(&mut self) -> Option<String> {
        let count = usize::from(*self.call(5, 0, &[])?.first()?);
        let mut name = Vec::new();
        while name.len() < count {
            let bytes = self.call(5, 1, &[name.len() as u8])?;
            let part = bytes
                .into_iter()
                .take(count - name.len())
                .take_while(|b| *b != 0)
                .collect::<Vec<_>>();
            if part.is_empty() {
                break;
            }
            name.extend(part);
        }
        Some(String::from_utf8_lossy(&name).trim().into())
    }
    fn battery(&mut self) -> Option<State> {
        if let Some(r) = self.call(0x1004, 1, &[])
            && r.len() >= 3
        {
            let caps = self.call(0x1004, 0, &[]);
            let coarse = caps.as_ref().is_some_and(|c| c.len() > 1 && c[1] & 2 == 0);
            let n = if coarse {
                if r[1] & 8 != 0 {
                    100
                } else if r[1] & 4 != 0 {
                    50
                } else if r[1] & 2 != 0 {
                    20
                } else {
                    5
                }
            } else {
                r[0].min(100)
            };
            return Some(State {
                coarse,
                ..State::new(n, (1..=3).contains(&r[2]))
            });
        }
        if let Some(r) = self.call(0x1000, 0, &[])
            && r.len() >= 3
        {
            return Some(State {
                percent: if r[2] == 3 {
                    Some(100)
                } else {
                    (r[0] > 0).then_some(r[0].min(100))
                },
                charging: (1..=4).contains(&r[2]),
                ..State::default()
            });
        }
        if let Some(r) = self.call(0x1001, 0, &[])
            && r.len() >= 3
        {
            return Some(State {
                coarse: true,
                ..State::new(voltage(u16::from_be_bytes([r[0], r[1]])), r[2] & 0x80 != 0)
            });
        }
        if let Some(r) = self.call(0x1f20, 0, &[])
            && r.len() >= 3
            && r[2] & 1 != 0
        {
            return Some(State {
                coarse: true,
                ..State::new(
                    if r[2] == 7 {
                        100
                    } else {
                        voltage(u16::from_be_bytes([r[0], r[1]]))
                    },
                    r[2] & 2 != 0,
                )
            });
        }
        None
    }
}
fn voltage(mv: u16) -> u8 {
    const CURVE: &[(u16, u8)] = &[
        (4186, 100),
        (4067, 90),
        (3989, 80),
        (3922, 70),
        (3859, 60),
        (3811, 50),
        (3778, 40),
        (3751, 30),
        (3717, 20),
        (3671, 10),
        (3646, 5),
        (3579, 2),
        (3500, 0),
    ];
    if mv >= 4186 {
        return 100;
    }
    for w in CURVE.windows(2) {
        let [(high, hp), (low, lp)] = w else { continue };
        if mv >= *low {
            return (f64::from(*lp)
                + f64::from(hp - lp) * f64::from(mv - low) / f64::from(high - low))
            .round_ties_even() as u8;
        }
    }
    0
}
fn legacy(channel: &Channel<'_>, index: u8) -> Option<State> {
    if let Some(r) = channel.register(index, 0x0d)
        && r.len() >= 3
    {
        return Some(State::new(r[0].min(100), matches!(r[2] >> 6, 1 | 2)));
    }
    let r = channel.register(index, 7)?;
    if r.len() < 2 {
        return None;
    }
    let full = matches!(r[1], 0x22 | 0x26);
    Some(State {
        percent: if full {
            Some(100)
        } else {
            match r[0] {
                1 => Some(5),
                2 => Some(10),
                3 => Some(20),
                4 => Some(35),
                5 => Some(50),
                6 => Some(75),
                7 => Some(100),
                _ => None,
            }
        },
        charging: matches!(r[1], 0x21 | 0x22 | 0x24 | 0x25 | 0x26),
        coarse: true,
        asleep: false,
    })
}
pub(super) fn read(infos: &[&Interface]) -> Vec<Reading> {
    let mut found = Vec::new();
    for long in infos
        .iter()
        .copied()
        .filter(|i| i.page >= 0xff00 && i.usage == 2 && i.input_len >= 20 && i.output_len >= 20)
    {
        let Ok(t) = Transport::open_collection(long) else {
            continue;
        };
        let mut channel = Channel {
            infos: vec![long],
            transports: vec![t],
        };
        if let Some(short) = infos
            .iter()
            .copied()
            .find(|i| i.page >= 0xff00 && i.usage == 1)
            && let Ok(t) = Transport::open_collection(short)
        {
            channel.infos.push(short);
            channel.transports.push(t);
        }
        let receiver = (0xc500..=0xc5ff).contains(&long.pid)
            || long.product.to_ascii_lowercase().contains("receiver");
        let indices: &[u8] = if receiver {
            &[1, 2, 3, 4, 5, 6]
        } else {
            &[0xff]
        };
        for index in indices {
            match channel.request(*index, 0, 1, &[0, 0, 0x5a], 1500) {
                Ok(Answer::Error(1)) => {
                    if let Some(s) = legacy(&channel, *index) {
                        found.push(reading(
                            long,
                            &format!("slot:{index}"),
                            &format!("Logitech Device {index}"),
                            s,
                        ));
                    }
                }
                Ok(Answer::Data(version)) if version.first().is_some_and(|v| *v >= 2) => {
                    let mut device = Device {
                        channel: &channel,
                        index: *index,
                        features: HashMap::new(),
                    };
                    let Some(state) = device.battery() else {
                        continue;
                    };
                    let name = device.name().filter(|n| !n.is_empty()).unwrap_or_else(|| {
                        if !receiver && !long.product.is_empty() {
                            long.product.clone()
                        } else {
                            format!("Logitech Device {index}")
                        }
                    });
                    let mut value = reading(long, &format!("slot:{index}"), &name, state);
                    if let Some(unit) = device.call(3, 0, &[])
                        && unit.len() >= 5
                        && unit[1..5].iter().any(|b| *b != 0)
                    {
                        value.device_key = format!(
                            "logitech:{:02X}{:02X}{:02X}{:02X}",
                            unit[1], unit[2], unit[3], unit[4]
                        );
                    }
                    found.push(value);
                }
                _ => {}
            }
        }
    }
    found
}
