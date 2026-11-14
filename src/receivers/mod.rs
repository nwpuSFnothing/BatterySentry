//! USB receiver protocols ported and modified from BrimDeck 89b9397 (Apache-2.0).
//! Original protocol credits and licenses are retained in THIRD_PARTY_NOTICES.md and licenses/.
mod catalog;
mod gaming;
mod headsets;
mod logitech;
mod push;
mod query;

use crate::{
    drivers::Reading,
    hid::{Interface, Transport},
    protocol::BatteryValue,
};
use std::{
    collections::HashMap,
    io, thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct State {
    percent: Option<u8>,
    charging: bool,
    coarse: bool,
    asleep: bool,
}
impl State {
    fn new(percent: u8, charging: bool) -> Self {
        Self {
            percent: Some(percent),
            charging,
            ..Self::default()
        }
    }
    fn value(self) -> Option<BatteryValue> {
        match (self.percent, self.charging) {
            (Some(n), true) => Some(BatteryValue::PercentCharging(n)),
            (Some(n), false) => Some(BatteryValue::Percent(n)),
            (None, true) => Some(BatteryValue::Charging),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RazerModel {
    pid: u16,
    name: &'static str,
    transaction: u8,
    interface: i32,
    wait_us: u64,
    charging: bool,
}
#[derive(Debug, Clone, Copy)]
struct AsusModel {
    pid: u16,
    name: &'static str,
    family: u8,
    interface: i32,
    report: u8,
    steps: bool,
    level_at: usize,
    collection: i32,
}
#[derive(Debug, Clone, Copy)]
struct SteelModel {
    pid: u16,
    name: &'static str,
    interface: i32,
    command: u8,
}
#[derive(Debug, Clone, Copy)]
struct F83Model {
    vid: u16,
    pid: u16,
    name: &'static str,
    cable: bool,
    old: bool,
    strict: bool,
    clamp: bool,
    interface: i32,
    receiver: i32,
}

#[derive(Debug, Clone, Copy)]
enum Protocol {
    BitMouse,
    Compx,
    CompxFramed,
    Vgn,
    Endgame,
    Vaxee,
    Inphic,
    Mchose,
    Ajazz,
    F83(&'static F83Model),
    Nordic,
    Realtek,
    Gmouse,
    Lingbao,
    Nzxt,
    Cherry,
    Hecate,
    Asus(&'static AsusModel),
    Steel(&'static SteelModel),
    Rapoo,
}

fn path_number(path: &str, tag: &str) -> i32 {
    let path = path.to_ascii_lowercase();
    path.find(tag)
        .and_then(|i| path.get(i + tag.len()..i + tag.len() + 2))
        .and_then(|n| i32::from_str_radix(n, 16).ok())
        .unwrap_or(-1)
}
fn interface(i: &Interface) -> i32 {
    path_number(&i.path, "&mi_")
}
fn collection(i: &Interface) -> i32 {
    path_number(&i.path, "&col")
}
fn group(i: &Interface) -> String {
    // Remove interface/collection suffixes, retaining the physical receiver instance.
    let mut p = i.path.to_ascii_lowercase();
    for tag in ["&mi_", "&col"] {
        while let Some(start) = p.find(tag) {
            let end = start
                + tag.len()
                + p[start + tag.len()..]
                    .bytes()
                    .take_while(u8::is_ascii_hexdigit)
                    .count();
            p.replace_range(start..end, "");
        }
    }
    if let Some(hash) = p.find("#{")
        && hash >= 5
        && p.as_bytes()[hash - 5] == b'&'
        && p[hash - 4..hash].bytes().all(|b| b.is_ascii_hexdigit())
    {
        p.replace_range(hash - 5..hash, "");
    }
    p
}

fn find(i: &Interface) -> Option<Protocol> {
    use Protocol::*;
    let (v, p, page, usage) = (i.vid, i.pid, i.page, i.usage);
    let (input, output, feature) = (i.input_len, i.output_len, i.feature_len);
    // Known products are resolved before vendor-wide rules, including incompatible collections.
    if (v == 0x249a && p == 0x5c2f) || (v == 0x248a && matches!(p, 0x5c2e | 0x5d2e | 0x5e2e)) {
        return ((input >= 32 && output >= 32)
            && (interface(i) == 2 || (page == 1 && usage == 0 && input >= 33 && output >= 33)))
            .then_some(Ajazz);
    }
    if catalog::COMPX.contains(&(v, p)) {
        return (page == 0xff02 && usage == 2 && input == 17 && output == 17)
            .then_some(CompxFramed);
    }
    if let Some(m) = catalog::F83.iter().find(|m| m.vid == v && m.pid == p) {
        return (if v == 0x33e4 {
            feature == 65
        } else {
            page == 0xffff && feature >= 65 && (m.interface < 0 || interface(i) == m.interface)
        })
        .then_some(F83(m));
    }
    if matches!((v, p), (0x3837, 0x4245) | (0xa8a5, 0x2255)) {
        return (page == 0xff01 && usage == 0x10 && output >= 65).then_some(Gmouse);
    }
    if v == 0x1915 && matches!(p, 0xae1c | 0xae11) {
        return (page == 0xffa0 && feature >= 32).then_some(Nordic);
    }
    if v == 0x5253 && matches!(p, 0x1020 | 0x0031) {
        return (page == 0xff01 && feature >= 21).then_some(Realtek);
    }
    if (v, p) == (0x3151, 0x402d) {
        return (interface(i) == 2 && page == 0xffff && usage == 2 && feature == 65)
            .then_some(Lingbao);
    }
    if v == 0x1e71 && matches!(p, 0x2101 | 0x2129) {
        return (page == 0xffca && usage == 1 && output >= 64).then_some(Nzxt);
    }
    if v == 0x046a && matches!(p, 0x01e4 | 0x01e2) {
        return (page == 0xff1c && usage == 0x92 && output >= 64).then_some(Cherry);
    }
    if (v, p) == (0x35bb, 0xd00b) {
        return (page == 0xff1c && usage == 0x92 && output >= 64).then_some(Hecate);
    }
    if v == 0x0b05
        && let Some(m) = catalog::ASUS.iter().find(|m| m.pid == p)
    {
        return (output >= 64
            && (m.interface < 0 || interface(i) == m.interface)
            && (m.collection < 0 || collection(i) == m.collection)
            && (m.family != 2 || page == 0xff00))
            .then_some(Asus(m));
    }
    if v == 0x1038
        && let Some(m) = catalog::STEEL.iter().find(|m| m.pid == p)
    {
        return (interface(i) == m.interface && page >= 0xff00 && output > 0).then_some(Steel(m));
    }
    if v == 0x24ae && matches!(p, 0x1231 | 0x4431) {
        return (page >= 0xff00 && output >= 33).then_some(Rapoo);
    }
    if v == 0x373b && p == 0x1012 {
        return (page == 0xff1c && usage == 0x92 && output >= 64).then_some(Vgn);
    }
    match (v, p, page) {
        (0x373b, _, 0xff05) => Some(BitMouse),
        (0x373b, 0x1119 | 0x111a, 0xff04) => Some(Compx),
        (0x320f, 0x5088, 0xff1c) => Some(Vgn),
        (0x3367, _, 0xff01) => Some(Endgame),
        (0x3057, _, 0xff05) => Some(Vaxee),
        (0x248a, 0x5b2e | 0x5b2f, _) if feature >= 33 => Some(Inphic),
        (0x249a, _, _) if page >= 0xff00 && feature >= 33 => Some(Inphic),
        (0x3837, _, 0xff01 | 0xff0b)
            if !matches!(p, 0x1012 | 0x6045)
                && (i.product.to_ascii_lowercase().contains("mouse")
                    || i.product.contains("鼠标")) =>
        {
            Some(Mchose)
        }
        _ => None,
    }
}

fn frame(length: usize, head: &[u8]) -> Vec<u8> {
    let mut f = vec![0; length.max(head.len())];
    f[..head.len()].copy_from_slice(head);
    f
}
fn reading(i: &Interface, suffix: &str, name: &str, state: State) -> Reading {
    let value = if state.asleep { None } else { state.value() };
    Reading {
        device_key: format!("receiver:{}:{suffix}", group(i)),
        name: name.to_owned(),
        identity: format!("{:04X}:{:04X}", i.vid, i.pid),
        value,
        last_known: if state.asleep {
            state.percent.map(BatteryValue::Percent)
        } else {
            None
        },
        detail: if state.asleep {
            "Device asleep · receiver's last reading".into()
        } else if state.coarse {
            "Connected via receiver · approximate battery level".into()
        } else if state.charging {
            "Connected via receiver · charging".into()
        } else {
            "Connected via 2.4 GHz".into()
        },
    }
}
fn name<'a>(i: &'a Interface, fallback: &'a str) -> &'a str {
    if i.product.is_empty()
        || [
            "receiver",
            "dongle",
            "2.4g",
            "usb device",
            "wireless gaming",
        ]
        .iter()
        .any(|s| i.product.to_ascii_lowercase().contains(s))
    {
        fallback
    } else {
        &i.product
    }
}

fn send(t: &Transport, i: &Interface, bytes: &[u8]) -> io::Result<()> {
    t.write_interrupt(&frame(usize::from(i.output_len), bytes))
}
fn transact<T>(
    t: &Transport,
    i: &Interface,
    request: &[u8],
    parse: impl Fn(&[u8]) -> Option<T>,
    ms: u64,
) -> io::Result<Option<T>> {
    drain(t);
    send(t, i, request)?;
    receive(t, parse, Duration::from_millis(ms))
}
fn drain(t: &Transport) {
    for _ in 0..32 {
        if !matches!(t.read(Duration::from_millis(1)), Ok(Some(_))) {
            break;
        }
    }
}
fn receive<T>(
    t: &Transport,
    parse: impl Fn(&[u8]) -> Option<T>,
    timeout: Duration,
) -> io::Result<Option<T>> {
    t.receive(timeout, parse)
}

/// Retains passive listeners; request/reply queries run concurrently per physical receiver.
#[derive(Default)]
pub struct Scanner {
    pushes: HashMap<String, push::Listener>,
    notify: Option<push::Notify>,
    known: HashMap<String, Vec<(Reading, Instant)>>,
}
impl std::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiverScanner")
            .field("listeners", &self.pushes.len())
            .finish()
    }
}
impl Scanner {
    pub fn set_notify(&mut self, notify: impl Fn() + Send + Sync + 'static) {
        self.notify = Some(std::sync::Arc::new(notify));
    }
    pub fn scan(&mut self, interfaces: &[Interface]) -> Vec<Reading> {
        self.pushes
            .retain(|path, _| interfaces.iter().any(|i| i.path == *path));
        let mut groups: HashMap<String, Vec<&Interface>> = HashMap::new();
        for i in interfaces {
            // Existing drivers retain ownership of the locally validated ATK and AULA endpoints.
            if (i.vid == 0x3554 && i.pid == 0xfa09)
                || (i.vid == 0x373b && i.pid == 0x1216 && i.page == 0xff05)
            {
                continue;
            }
            if let Some(kind) = push::find(i) {
                if !self.pushes.contains_key(&i.path)
                    && let Ok(listener) =
                        push::Listener::start(i.clone(), kind, self.notify.clone())
                {
                    self.pushes.insert(i.path.clone(), listener);
                }
            } else if !i.path.to_ascii_lowercase().contains("bth") && supported(i) {
                groups.entry(group(i)).or_default().push(i);
            }
        }
        let mut readings = thread::scope(|scope| {
            let mut jobs = Vec::new();
            for (key, group) in &groups {
                let group = group.as_slice();
                if let Ok(job) = thread::Builder::new()
                    .name("receiver-query".into())
                    .spawn_scoped(scope, move || (key.clone(), query::group(group)))
                {
                    jobs.push(job);
                }
            }
            let mut result = Vec::new();
            self.known.retain(|key, _| groups.contains_key(key));
            for (key, mut fresh) in jobs.into_iter().filter_map(|job| job.join().ok()) {
                let now = Instant::now();
                let previous = self.known.entry(key).or_default();
                for (old, at) in previous.iter() {
                    if now.saturating_duration_since(*at) < Duration::from_secs(1800)
                        && !fresh.iter().any(|r| r.device_key == old.device_key)
                    {
                        let mut cached = old.clone();
                        cached.value = None;
                        cached.last_known = old.value.or(old.last_known);
                        cached.detail = "No reply · last known battery".into();
                        fresh.push(cached);
                    }
                }
                let next = fresh
                    .iter()
                    .cloned()
                    .map(|r| {
                        let at = if r.value.is_some() {
                            now
                        } else {
                            previous
                                .iter()
                                .find(|(old, _)| old.device_key == r.device_key)
                                .map_or(now, |(_, at)| *at)
                        };
                        (r, at)
                    })
                    .collect();
                *previous = next;
                result.extend(fresh);
            }
            result
        });
        if self.notify.is_none() && !self.pushes.is_empty() {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline
                && self.pushes.values().any(|p| {
                    p.reading()
                        .is_none_or(|r| r.value.is_none() && r.last_known.is_none())
                })
            {
                thread::sleep(Duration::from_millis(25));
            }
        }
        readings.extend(self.pushes.values().filter_map(push::Listener::reading));
        let native = readings
            .iter()
            .filter(|r| r.value.is_some())
            .map(|r| r.identity.clone())
            .collect::<Vec<_>>();
        readings.extend(gaming::read(interfaces, &native));
        // Hide an asleep receiver only when its documented cable counterpart answered.
        readings.retain(|r| r.value.is_some() || !shadowed(&r.identity, &native));
        readings.sort_by(|a, b| {
            a.device_key
                .cmp(&b.device_key)
                .then_with(|| b.value.is_some().cmp(&a.value.is_some()))
        });
        readings.dedup_by(|a, b| a.device_key == b.device_key);
        readings
    }
}

fn shadowed(identity: &str, active: &[String]) -> bool {
    catalog::TWINS.iter().any(|(vid, receiver, cable)| {
        identity == format!("{vid:04X}:{receiver:04X}")
            && active.contains(&format!("{vid:04X}:{cable:04X}"))
    }) || catalog::F83.iter().any(|m| {
        m.receiver >= 0
            && identity == format!("{:04X}:{:04X}", m.vid, m.receiver)
            && active.contains(&format!("{:04X}:{:04X}", m.vid, m.pid))
    })
}

fn supported(i: &Interface) -> bool {
    find(i).is_some()
        || headsets::supported(i)
        || !i.battery_features.is_empty()
        || (i.vid == 0x046d && i.page >= 0xff00 && matches!(i.usage, 1 | 2))
        || (i.vid == 0x1532 && catalog::RAZER.iter().any(|m| m.pid == i.pid))
        || (i.vid == 0x0b05 && i.pid == 0x1ace)
        || (i.vid == 0x1b1c && matches!(i.pid, 0x1b7f | 0x0a40 | 0x0a3e | 0x0a3d | 0x0a64 | 0x0a62))
        || (i.vid == 0x054c
            && matches!(i.pid, 0x0ce6 | 0x0df2 | 0x05c4 | 0x09cc | 0x0ba0)
            && i.page == 1
            && i.usage == 5)
}
