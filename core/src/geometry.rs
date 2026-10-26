//! Ring hit-testing. The UI (app/src/Ring.tsx) draws with the same proportions.

use std::f64::consts::PI;

/// Centre hub radius as a fraction of the ring radius. Dropping on the hub cancels.
pub const HUB_RATIO: f64 = 0.34;

/// Segment under a point relative to the ring centre: 0..n clockwise from the top,
/// -1 for the hub or outside the ring.
pub fn wedge_index(dx: f64, dy: f64, radius: f64, n: usize) -> i32 {
    let r = (dx * dx + dy * dy).sqrt();
    if n == 0 || r < HUB_RATIO * radius || r > radius {
        return -1;
    }
    let mut a = dx.atan2(-dy);
    if a < 0.0 {
        a += 2.0 * PI;
    }
    let step = 2.0 * PI / n as f64;
    (((a + step / 2.0) / step) as usize % n) as i32
}

#[cfg(test)]
mod tests {
    use super::wedge_index;

    #[test]
    fn seven_tool_ring_angles() {
        let r = 180.0;
        let at = |deg: f64| {
            let a = deg.to_radians();
            wedge_index(120.0 * a.sin(), -120.0 * a.cos(), r, 7)
        };
        assert_eq!(at(0.0), 0); // Compress, top
        assert_eq!(at(103.0), 2); // Edit, right
        assert_eq!(at(257.0), 5); // Crop, left-ish
        assert_eq!(at(355.0), 0); // wraps back to the top
    }

    #[test]
    fn hub_and_outside_are_misses() {
        assert_eq!(wedge_index(0.0, -30.0, 180.0, 8), -1);
        assert_eq!(wedge_index(0.0, -190.0, 180.0, 8), -1);
        assert_eq!(wedge_index(0.0, -100.0, 180.0, 0), -1);
    }
}
