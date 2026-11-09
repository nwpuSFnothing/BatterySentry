#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryValue {
    Percent(u8),
    Charging,
}

/// Device identity returned by the BitMouse protocol, independent of USB PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtkIdentity {
    pub company: u8,
    pub model: u32,
}

/// Builds a read-only identity request (0x89 via receiver, 0x4a via mouse).
pub fn atk_identity_request(command: u8) -> Vec<u8> {
    let mut frame = vec![0; 64];
    frame[0] = 8;
    frame[2..8].copy_from_slice(&[0x72, 8, 0, 1, command, 6]);
    frame[1] = frame[2..].iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
    frame
}

pub fn atk_identity_reply(r: &[u8], command: u8) -> Option<AtkIdentity> {
    // Byte positions include the report ID; discard request echoes and zero IDs.
    // Replies omit the request checksum before class 0x72 (same as atk_reply).
    if r.len() < 13
        || r[0] != 8
        || r[1] != 0x72
        || r[2] != 0
        || r[5] != command
        || r[6] != 6
        || r[8] == 0
    {
        return None;
    }
    Some(AtkIdentity {
        company: r[8],
        model: u32::from_le_bytes(r[9..13].try_into().ok()?),
    })
}

pub fn aula_identity_request() -> Vec<u8> {
    let mut frame = vec![0; 20];
    frame[0] = 0x13;
    frame[1] = 0x05;
    frame[19] = 0x18;
    frame
}

/// Returns the six identity bytes, never a guessed marketing name.
pub fn aula_identity_reply(r: &[u8]) -> Option<[u8; 6]> {
    if r.len() != 20
        || r[0] != 0x13
        || r[1] != 0x05
        || r[2] != 1
        || r[3] != 0
        || !(6..=14).contains(&r[4])
        || r[19] != r[..19].iter().fold(0u8, |sum, b| sum.wrapping_add(*b))
    {
        return None;
    }
    let psd: [u8; 6] = r[5..11].try_into().ok()?;
    psd.iter().any(|b| *b != 0).then_some(psd)
}
pub fn atk_request() -> Vec<u8> {
    let mut f = vec![0; 64];
    f[..8].copy_from_slice(&[8, 0x7d, 0x72, 2, 0, 1, 7, 1]);
    f
}
pub fn atk_reply(r: &[u8]) -> Option<BatteryValue> {
    if r.len() >= 8 && r[0] == 8 && r[1] == 0x72 && r[5] == 7 && r[7] <= 100 {
        Some(BatteryValue::Percent(r[7]))
    } else {
        None
    }
}
pub fn aula_request() -> Vec<u8> {
    let mut f = vec![0; 20];
    f[0] = 0x13;
    f[1] = 0x4a;
    f[19] = 0x5d;
    f
}
pub fn aula_reply(r: &[u8]) -> Option<BatteryValue> {
    if r.len() != 20 || r[0] != 0x13 || r[1] & 0x7f != 0x4a {
        return None;
    }
    let sum = r[..19].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != r[19] || r[2..5] != [1, 0, 2] {
        return None;
    }
    match r[6] {
        1 if (1..=100).contains(&r[5]) => Some(BatteryValue::Percent(r[5])),
        0x10 => Some(BatteryValue::Charging),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_decoders_reject_echo_truncation_and_bad_checksum() {
        assert_eq!(atk_identity_reply(&atk_identity_request(0x4a), 0x4a), None);
        let mut atk = [0u8; 64];
        // Captured from the local 373B:1216 receiver, not a request-shaped fixture.
        atk[..13].copy_from_slice(&[8, 0x72, 0, 0x0a, 0, 0x4a, 6, 0, 2, 4, 0, 0, 0]);
        assert_eq!(
            atk_identity_reply(&atk, 0x4a),
            Some(AtkIdentity {
                company: 2,
                model: 4
            })
        );
        assert_eq!(atk_identity_reply(&atk[..12], 0x4a), None);
        assert_eq!(atk_identity_reply(&atk, 0x89), None);
        assert_eq!(aula_identity_reply(&aula_identity_request()), None);
        let mut aula = [0u8; 20];
        aula[..11].copy_from_slice(&[0x13, 5, 1, 0, 10, 3, 0, 0, 0, 1, 0x58]);
        aula[19] = aula[..19].iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
        assert_eq!(aula_identity_reply(&aula), Some([3, 0, 0, 0, 1, 0x58]));
        aula[10] ^= 1;
        assert_eq!(aula_identity_reply(&aula), None);
    }
    #[test]
    fn atk_matches_original_request() {
        let r = atk_request();
        assert_eq!(r.len(), 64);
        assert_eq!(&r[..8], &[8, 0x7d, 0x72, 2, 0, 1, 7, 1]);
        assert!(r[8..].iter().all(|b| *b == 0));
    }
    #[test]
    fn atk_rejects_unrelated_truncated_and_invalid_reports() {
        let mut r = [0u8; 64];
        r[0] = 8;
        r[1] = 0x72;
        r[5] = 7;
        for n in [0, 1, 50, 100] {
            r[7] = n;
            assert_eq!(atk_reply(&r), Some(BatteryValue::Percent(n)));
        }
        r[7] = 101;
        assert_eq!(atk_reply(&r), None);
        r[7] = 50;
        r[1] = 0;
        assert_eq!(atk_reply(&r), None);
        assert_eq!(atk_reply(&r[..7]), None);
    }
    #[test]
    fn aula_captured_battery_and_cable_frames() {
        let r = [
            0x13, 0x4a, 1, 0, 2, 0x61, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xc2,
        ];
        assert_eq!(aula_reply(&r), Some(BatteryValue::Percent(97)));
        let mut c = r;
        c[5] = 100;
        c[6] = 0x10;
        c[19] = 0xd4;
        assert_eq!(aula_reply(&c), Some(BatteryValue::Charging));
    }
    #[test]
    fn aula_rejects_echo_checksum_state_and_invalid_level() {
        assert_eq!(aula_reply(&aula_request()), None);
        let mut r = [0u8; 20];
        r[..7].copy_from_slice(&[0x13, 0xca, 1, 0, 2, 50, 1]);
        r[19] = r[..19].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        assert_eq!(aula_reply(&r), Some(BatteryValue::Percent(50)));
        r[5] = 51;
        assert_eq!(aula_reply(&r), None);
        for (level, state) in [(0, 1), (101, 1), (50, 9)] {
            r[5] = level;
            r[6] = state;
            r[19] = r[..19].iter().fold(0u8, |a, b| a.wrapping_add(*b));
            assert_eq!(aula_reply(&r), None);
        }
        assert_eq!(aula_reply(&r[..19]), None);
    }
}
