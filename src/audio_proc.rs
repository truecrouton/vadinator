/// Calculates the Zero Crossing Rate.
/// High ZCR (>60) usually indicates white noise, clicks, or "pops".
/// In a 256-sample frame (16ms) @ 16kHz:
/// - Speech usually has 10–40 crossings.
/// - Static/Pops/Clicks usually have >60 crossings.
pub fn calculate_zcr(frame: &[f32]) -> usize {
    frame
        .windows(2)
        .filter(|win| (win[0] >= 0.0 && win[1] < 0.0) || (win[0] < 0.0 && win[1] >= 0.0))
        .count()
}

/// Calculates the Root Mean Square (Volume).
/// Use this as a "Noise Gate" to ignore the floor hum.
pub fn calculate_rms(frame: &[f32]) -> f32 {
    let sq_sum: f32 = frame.iter().map(|&s| s * s).sum();
    (sq_sum / frame.len() as f32).sqrt()
}

/// High-pass filter
/// Higher Alpha (0.99): More low-end is kept, but it's more likely to overshoot
/// Lower Alpha (0.80): More low-end is cut (thinner sound), but it's much more stable.
pub fn apply_high_pass(frame: &mut [f32], state: &mut f32, alpha: f32) {
    for sample in frame.iter_mut() {
        let current = *sample;

        // High-pass math: y[n] = x[n] - x[n-1] + alpha * y[n-1]
        let filtered = current - *state;
        *state = current;

        // Clamp the result to keep things from panicking
        *sample = (filtered * alpha).clamp(-1.0, 1.0);
    }
}

/// Clamps and sanitizes audio to prevent NN panics.
/// Sanitization: Ensure NO values exceed 1.0 or -1.0
/// and handle any weird NaN/Inf values from the driver.
pub fn sanitize_frame(frame: &mut [f32]) {
    for sample in frame.iter_mut() {
        // 1. Handle NaNs (sometimes happens on mic disconnect)
        if sample.is_nan() {
            *sample = 0.0;
        }

        // 2. Clamp strictly to [-1.0, 1.0]
        *sample = sample.clamp(-1.0, 1.0);
    }
}
