//! A chromatic softening control, without spatial blur or lifting black.
pub fn soften(c: [f32; 3], softness: f32) -> [f32; 3] {
    if softness <= 0.0 {
        return c;
    }
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let peak = c[0].max(c[1]).max(c[2]);
    c.map(|v| v * (1.0 - 0.5 * softness) + peak * 0.14 * softness)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn soft_neon_keeps_black_and_hue_order_while_reducing_peaks() {
        assert_eq!(soften([0.0; 3], 1.0), [0.0; 3]);
        let c = [0.0, 0.4, 1.0];
        assert_eq!(soften(c, 0.0), c);
        let soft = soften(c, 0.6);
        assert!(soft[0] < soft[1] && soft[1] < soft[2]);
        assert!(soft[2] < c[2] && soft[0] > c[0]);
        assert!(soften(c, 1.0)[2] < soft[2]);
    }
}
