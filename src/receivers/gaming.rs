//! Windows battery reports for controllers using a wireless adapter (including Xbox).
//!
//! Adapted from MoFeng2223/BrimDeck GamingBatteries.cs at
//! 89b93972f4c0bf57e9b8a14ceff1515b30f9c1d6 and modified for BatterySentry's Rust
//! scanner, COM lifetime and device filtering. Apache-2.0; see THIRD_PARTY_NOTICES.md
//! and licenses/BrimDeck-Apache-2.0.txt.
use super::State;
use crate::{drivers::Reading, hid::Interface};
use windows::{
    Gaming::Input::RawGameController,
    System::Power::BatteryStatus,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful RoInitialize on this same worker thread.
        unsafe { RoUninitialize() };
    }
}
pub(super) fn read(interfaces: &[Interface], native: &[String]) -> Vec<Reading> {
    // SAFETY: initializes WinRT for this worker; a failed initialization is not uninitialized.
    let apartment = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    if apartment.is_err() {
        return Vec::new();
    }
    let _apartment = Apartment;
    let Ok(controllers) = RawGameController::RawGameControllers() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for index in 0..controllers.Size().unwrap_or(0) {
        let Ok(controller) = controllers.GetAt(index) else {
            continue;
        };
        if controller.IsWireless() != Ok(true) {
            continue;
        }
        let Ok(battery) = controller.TryGetBatteryReport() else {
            continue;
        };
        let status = battery.Status().unwrap_or(BatteryStatus::NotPresent);
        if status == BatteryStatus::NotPresent {
            continue;
        }
        let remaining = battery
            .RemainingCapacityInMilliwattHours()
            .and_then(|v| v.Value())
            .ok();
        let full = battery
            .FullChargeCapacityInMilliwattHours()
            .and_then(|v| v.Value())
            .ok();
        let percent = remaining.zip(full).filter(|(_, f)| *f > 0).map(|(r, f)| {
            (f64::from(r) * 100.0 / f64::from(f))
                .round_ties_even()
                .clamp(0.0, 100.0) as u8
        });
        let state = State {
            percent,
            charging: status == BatteryStatus::Charging,
            ..State::default()
        };
        let vid = controller.HardwareVendorId().unwrap_or(0);
        let pid = controller.HardwareProductId().unwrap_or(0);
        let hardware = format!("{vid:04X}:{pid:04X}");
        // WinRT does not expose the transport. Suppress known Bluetooth endpoints and
        // controllers already read through their native receiver protocol.
        if native.contains(&hardware)
            || interfaces.iter().any(|i| {
                i.vid == vid && i.pid == pid && i.path.to_ascii_lowercase().contains("bth")
            })
        {
            continue;
        }
        found.push(Reading {
            device_key: format!(
                "gaming:{}",
                controller
                    .NonRoamableId()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| format!("{vid:04x}:{pid:04x}:{index}"))
            ),
            name: controller
                .DisplayName()
                .map(|s| s.to_string())
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Wireless Controller".into()),
            identity: format!("{vid:04X}:{pid:04X}"),
            value: state.value(),
            last_known: None,
            detail: if state.charging {
                "Wireless controller · charging".into()
            } else {
                "Wireless controller".into()
            },
        });
    }
    found
}
