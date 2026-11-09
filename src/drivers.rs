//! Validated receiver protocols. Names are resolved independently from battery reads.
use crate::{
    hid::{self, Interface, Transport},
    protocol::{self, AtkIdentity, BatteryValue},
};
use std::{
    collections::HashMap,
    io, thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct Reading {
    pub device_key: String,
    pub last_known: Option<BatteryValue>,
    pub name: String,
    pub identity: String,
    pub value: Option<BatteryValue>,
    pub detail: String,
}
impl Reading {
    pub fn value_text(&self) -> String {
        match self.value {
            Some(BatteryValue::Percent(n)) => format!("{n}%"),
            Some(BatteryValue::Charging) => "Charging".into(),
            None => match self.last_known {
                Some(BatteryValue::Percent(n)) => format!("~{n}%"),
                Some(BatteryValue::Charging) => "~Charging".into(),
                None => "--".into(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    model: Option<&'static str>,
    code: String,
}
impl DeviceIdentity {
    fn display_name(&self, brand: &str) -> String {
        self.model
            .map_or_else(|| format!("{brand} Device"), str::to_owned)
    }
}

pub trait DeviceDriver: Sync {
    fn brand(&self) -> &'static str;
    fn matches(&self, iface: &Interface) -> bool;
    fn identify(&self, transport: &Transport) -> io::Result<Option<DeviceIdentity>>;
    fn query(&self, transport: &Transport) -> io::Result<Option<BatteryValue>>;
}
struct Atk;
impl DeviceDriver for Atk {
    fn brand(&self) -> &'static str {
        "ATK"
    }
    fn matches(&self, i: &Interface) -> bool {
        i.vid == 0x373b
            && i.pid == 0x1216
            && i.page == 0xff05
            && i.input_len == 64
            && i.output_len == 64
    }
    fn identify(&self, t: &Transport) -> io::Result<Option<DeviceIdentity>> {
        for command in [0x89, 0x4a] {
            if let Some(id) = t.exchange(
                &protocol::atk_identity_request(command),
                |r| protocol::atk_identity_reply(r, command),
                Duration::from_millis(400),
            )? {
                return Ok(Some(DeviceIdentity {
                    model: atk_model(id),
                    code: format!("CID={} MID={}", id.company, id.model),
                }));
            }
        }
        Ok(None)
    }
    fn query(&self, t: &Transport) -> io::Result<Option<BatteryValue>> {
        for _ in 0..3 {
            if let Some(v) = t.exchange(
                &protocol::atk_request(),
                protocol::atk_reply,
                Duration::from_millis(400),
            )? {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
}

// Read-only identity mapping documented by atkctl's Catalog.mouseIDs.
// Unknown IDs intentionally keep the generic brand name. No PID-only model guesses.
fn atk_model(id: AtkIdentity) -> Option<&'static str> {
    match (id.company, id.model) {
        (2, 4 | 13) => Some("ATK A9 Plus NK"),
        (2, 25) => Some("ATK U2 Plus NK"),
        (2, 5 | 21) => Some("VXE V3 NK"),
        (2, 6 | 30) => Some("VXE V3 PRO NK"),
        (2, 7) => Some("VXE V3 Plus"),
        (2, 15) => Some("VXE V3 PRO Max"),
        (2, 8) => Some("VXE R1 NK"),
        (2, 9 | 31) => Some("VXE R1 S NK"),
        (2, 17) => Some("VXE R1 PRO MAX 2.0 NK"),
        (2, 14) => Some("ATK A9 Pro Max 2.0 NK"),
        (1, 3) => Some("ATK Ghost Ultimate"),
        (8, 1) => Some("ATK ZERO +"),
        _ => None,
    }
}

struct Aula;
impl DeviceDriver for Aula {
    fn brand(&self) -> &'static str {
        "AULA"
    }
    fn matches(&self, i: &Interface) -> bool {
        i.vid == 0x3554
            && i.pid == 0xfa09
            && i.page == 0xff02
            && i.usage == 2
            && i.input_len == 20
            && i.output_len == 20
    }
    fn identify(&self, t: &Transport) -> io::Result<Option<DeviceIdentity>> {
        Ok(t.exchange(
            &protocol::aula_identity_request(),
            protocol::aula_identity_reply,
            Duration::from_secs(1),
        )?
        .map(|psd| DeviceIdentity {
            // Full wireless PSD captured locally; 0x0B uses the wired model table
            // by the user's preference. Keep the prefix and receiver match strict.
            model: aula_model(psd),
            code: format!(
                "PSD={}",
                psd.iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(":")
            ),
        }))
    }
    fn query(&self, t: &Transport) -> io::Result<Option<BatteryValue>> {
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_millis(200));
            }
            if let Some(v) = t.exchange(
                &protocol::aula_request(),
                protocol::aula_reply,
                Duration::from_secs(1),
            )? {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
}
pub fn registry() -> Vec<Box<dyn DeviceDriver>> {
    vec![Box::new(Atk), Box::new(Aula)]
}

fn aula_model(psd: [u8; 6]) -> Option<&'static str> {
    match psd {
        [3, 0, 0, 0, 1, 0x0b] => Some("AULA F87 Pro"),
        [3, 0, 0, 0, 1, 0x58] => Some("AULA F99 Pro"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CachedIdentity {
    identity: Option<DeviceIdentity>,
    checked: Instant,
}

/// Long-lived only in the battery worker. Each task owns its HID transport.
#[derive(Debug, Default)]
pub struct Scanner {
    identities: HashMap<String, CachedIdentity>,
}
impl Scanner {
    pub fn invalidate_identities(&mut self) {
        self.identities.clear();
    }

    pub fn scan(&mut self) -> io::Result<Vec<Reading>> {
        let mut interfaces = hid::enumerate()?;
        // Enumeration order can change on hotplug; stable keys keep list/scroll stable.
        interfaces.sort_by(|a, b| a.path.cmp(&b.path));
        interfaces.dedup_by(|a, b| a.path == b.path);
        self.identities
            .retain(|key, _| interfaces.iter().any(|i| &i.path == key));
        let drivers = registry();
        let now = Instant::now();
        let outcomes = thread::scope(|scope| {
            let mut jobs = Vec::new();
            let mut results = Vec::new();
            for iface in &interfaces {
                let Some(driver) = drivers.iter().find(|d| d.matches(iface)) else {
                    continue;
                };
                let cached = self.identities.get(&iface.path);
                let driver = driver.as_ref();
                match thread::Builder::new()
                    .name(format!("query-{}", driver.brand()))
                    .spawn_scoped(scope, move || scan_interface(driver, iface, cached, now))
                {
                    Ok(job) => jobs.push(job),
                    Err(_) => results.push(scan_interface(driver, iface, cached, now)),
                }
            }
            for job in jobs {
                results.push(
                    job.join()
                        .map_err(|_| io::Error::other("device query thread panicked"))?,
                );
            }
            Ok::<_, io::Error>(results)
        })?;
        let mut readings = Vec::with_capacity(outcomes.len());
        for (reading, cached) in outcomes {
            if let Some(cached) = cached {
                self.identities.insert(reading.device_key.clone(), cached);
            } else {
                self.identities.remove(&reading.device_key);
            }
            readings.push(reading);
        }
        readings.sort_by(|a, b| a.device_key.cmp(&b.device_key));
        Ok(readings)
    }
}

fn scan_interface(
    driver: &dyn DeviceDriver,
    iface: &Interface,
    cached: Option<&CachedIdentity>,
    now: Instant,
) -> (Reading, Option<CachedIdentity>) {
    let mut identity_cache = cached.cloned();
    let result = Transport::open(iface).and_then(|t| {
        // Battery first: identity timeouts/errors never discard an available reading.
        let battery = driver.query(&t)?;
        if battery.is_some() {
            let reusable = cached.filter(|c| {
                let ttl = if c.identity.as_ref().is_some_and(|id| id.model.is_some()) {
                    300
                } else {
                    60
                };
                now.saturating_duration_since(c.checked) < Duration::from_secs(ttl)
            });
            identity_cache = Some(reusable.cloned().unwrap_or_else(|| CachedIdentity {
                identity: driver.identify(&t).ok().flatten(),
                checked: now,
            }));
        }
        Ok(battery)
    });
    let resolved = identity_cache.as_ref().and_then(|c| c.identity.as_ref());
    let name = resolved.map_or_else(
        || format!("{} Device", driver.brand()),
        |id| id.display_name(driver.brand()),
    );
    let identity = format!(
        "{:04X}:{:04X}{}",
        iface.vid,
        iface.pid,
        resolved.map_or_else(String::new, |id| format!(" · {}", id.code))
    );
    let (value, detail) = match result {
        Ok(Some(v)) => (
            Some(v),
            match v {
                BatteryValue::Charging => "Cable connected · percentage unavailable".into(),
                BatteryValue::Percent(_) => "Connected via 2.4 GHz".into(),
            },
        ),
        Ok(None) => (None, "No reply · wake the device and refresh".into()),
        Err(e) => (None, e.to_string()),
    };
    (
        Reading {
            device_key: iface.path.clone(),
            last_known: None,
            name,
            identity,
            value,
            detail,
        },
        identity_cache,
    )
}

pub fn scan() -> io::Result<Vec<Reading>> {
    Scanner::default().scan()
}

#[derive(Debug, Default)]
pub struct ReadingCache(HashMap<String, (BatteryValue, Instant)>);
impl ReadingCache {
    pub fn apply(&mut self, readings: &mut [Reading], now: Instant) {
        self.0
            .retain(|key, _| readings.iter().any(|r| &r.device_key == key));
        for r in readings {
            r.last_known = None;
            if let Some(value) = r.value {
                self.0.insert(r.device_key.clone(), (value, now));
            } else if let Some(&(value, time)) = self.0.get(&r.device_key) {
                let age = now.saturating_duration_since(time);
                if age <= Duration::from_secs(1800) {
                    r.last_known = Some(value);
                    r.detail = format!(
                        "Last read {}s ago · wake the device and refresh",
                        age.as_secs()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_identity_is_generic_and_cache_isolated() {
        assert_eq!(
            atk_model(AtkIdentity {
                company: 2,
                model: 4
            }),
            Some("ATK A9 Plus NK")
        );
        assert_eq!(aula_model([3, 0, 0, 0, 1, 0x0b]), Some("AULA F87 Pro"));
        assert_eq!(aula_model([3, 0, 0, 0, 2, 0x0b]), None);
        let unknown = DeviceIdentity {
            model: None,
            code: "unmapped".into(),
        };
        assert_eq!(unknown.display_name("AULA"), "AULA Device");
        assert_eq!(
            atk_model(AtkIdentity {
                company: 2,
                model: 13
            }),
            Some("ATK A9 Plus NK")
        );
        assert_eq!(
            atk_model(AtkIdentity {
                company: 2,
                model: 0xffff
            }),
            None
        );
        let now = Instant::now();
        let mut cache = ReadingCache::default();
        let mut readings = vec![Reading {
            device_key: "a".into(),
            last_known: None,
            name: "ATK Device".into(),
            identity: String::new(),
            value: Some(BatteryValue::Percent(83)),
            detail: String::new(),
        }];
        cache.apply(&mut readings, now);
        readings[0].value = None;
        cache.apply(&mut readings, now + Duration::from_secs(60));
        assert_eq!(readings[0].value_text(), "~83%");
        cache.apply(&mut readings, now + Duration::from_secs(1801));
        assert_eq!(readings[0].value_text(), "--");
        readings[0].device_key = "b".into();
        cache.apply(&mut readings, now + Duration::from_secs(60));
        assert_eq!(readings[0].value_text(), "--");
    }
}
