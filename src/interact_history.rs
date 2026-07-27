//! Per-player Interact button history.
//!
//! Real game model (author-confirmed shape): a 64-bit shift register per player.
//! Each tick the buffer shifts left and the current Interact lands in the LSB.
//! Cleared on whistle / goal / roster reset (tie-break 3v3→2v2→1v1, etc.).
//!
//! - Claim / tackle: rising edge only — held this tick, not held last tick.
//! - Shot: release (falling edge) fires; charge comes from how many consecutive
//!   held ticks sit in the low bits (capped at full-charge length).

/// Bits needed to cover full shot charge at the sim's fixed dt.
/// `shot_charge_time_s / FIXED_DT` ≈ 0.38/0.019 ≈ 20; 64 leaves headroom.
pub const INTERACT_HISTORY_BITS: u32 = 64;

#[inline]
pub fn push_interact(bits: u64, held: bool) -> u64 {
    (bits << 1) | u64::from(held)
}

#[inline]
pub fn interact_now(bits: u64) -> bool {
    bits & 1 != 0
}

#[inline]
pub fn interact_prev(bits: u64) -> bool {
    bits & 2 != 0
}

/// Claim / tackle may fire: pressed now, was released last tick.
#[inline]
pub fn interact_rising_edge(bits_after_push: u64) -> bool {
    interact_now(bits_after_push) && !interact_prev(bits_after_push)
}

/// Carrier release-to-shoot: was held last tick, released this tick.
#[inline]
pub fn interact_falling_edge(bits_after_push: u64) -> bool {
    !interact_now(bits_after_push) && interact_prev(bits_after_push)
}

/// Consecutive held ticks ending at the LSB (0 if currently released).
#[inline]
pub fn consecutive_held_ticks(bits: u64) -> u32 {
    bits.trailing_ones()
}

/// Shot charge 0..1 from consecutive hold length, after optional warmup ticks.
#[inline]
pub fn charge_from_hold(bits: u64, warmup_ticks: u32, full_charge_ticks: u32) -> f32 {
    let streak = consecutive_held_ticks(bits);
    if streak == 0 || full_charge_ticks == 0 {
        return 0.0;
    }
    let charged = streak.saturating_sub(warmup_ticks);
    (charged as f32 / full_charge_ticks as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_edge_needs_release_then_press() {
        let mut b = 0u64;
        b = push_interact(b, true);
        assert!(interact_rising_edge(b));
        b = push_interact(b, true);
        assert!(!interact_rising_edge(b));
        b = push_interact(b, false);
        assert!(!interact_rising_edge(b));
        b = push_interact(b, true);
        assert!(interact_rising_edge(b));
    }

    #[test]
    fn falling_edge_and_charge_streak() {
        let mut b = 0u64;
        for _ in 0..5 {
            b = push_interact(b, true);
        }
        assert_eq!(consecutive_held_ticks(b), 5);
        assert!((charge_from_hold(b, 0, 10) - 0.5).abs() < 1e-5);
        b = push_interact(b, false);
        assert!(interact_falling_edge(b));
        assert_eq!(consecutive_held_ticks(b), 0);
    }
}
