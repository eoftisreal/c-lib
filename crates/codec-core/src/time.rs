use std::cmp::Ordering;

/// Exact 128-bit reduced rational Q = i128 / u64 to prevent cumulative IEEE-754
/// rounding drift over multi-hour streams when converting container mdhd timescales
/// and AV1 timing metadata (num_units_in_display_tick / time_scale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational128 {
    pub num: i128,
    pub den: u64,
}

impl Rational128 {
    pub fn new(num: i128, den: u64) -> crate::CodecResult<Self> {
        if den == 0 {
            return Err(crate::CodecError::InvalidData);
        }
        let g = gcd_u128(num.unsigned_abs(), u128::from(den));
        let reduced_num = num / (g as i128);
        let reduced_den =
            u64::try_from(u128::from(den) / g).map_err(|_| crate::CodecError::InvalidData)?;
        Ok(Self {
            num: reduced_num,
            den: reduced_den,
        })
    }

    pub fn to_seconds_f64(self) -> f64 {
        (self.num as f64) / (self.den as f64)
    }

    pub fn checked_sub(self, other: Self) -> crate::CodecResult<Self> {
        let lhs = self
            .num
            .checked_mul(i128::from(other.den))
            .ok_or(crate::CodecError::InvalidData)?;
        let rhs = other
            .num
            .checked_mul(i128::from(self.den))
            .ok_or(crate::CodecError::InvalidData)?;
        let num = lhs.checked_sub(rhs).ok_or(crate::CodecError::InvalidData)?;
        let den = self
            .den
            .checked_mul(other.den)
            .ok_or(crate::CodecError::InvalidData)?;
        Self::new(num, den)
    }
}

const fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    if a == 0 {
        1
    } else {
        a
    }
}

#[derive(Debug, Clone, Copy, Eq)]
pub struct Timestamp {
    pub pts: i64,
    pub time_base_num: u32,
    pub time_base_den: u32,
}

impl Timestamp {
    pub fn new(pts: i64, num: u32, den: u32) -> crate::CodecResult<Self> {
        if num == 0 || den == 0 {
            return Err(crate::CodecError::InvalidData);
        }
        Ok(Self {
            pts,
            time_base_num: num,
            time_base_den: den,
        })
    }

    /// Convert to an exact reduced 128-bit rational timestamp in seconds.
    pub fn to_rational128(&self) -> crate::CodecResult<Rational128> {
        if self.time_base_num == 0 || self.time_base_den == 0 {
            return Err(crate::CodecError::InvalidData);
        }
        let num = i128::from(self.pts) * i128::from(self.time_base_num);
        Rational128::new(num, u64::from(self.time_base_den))
    }

    /// Rescale with truncation toward zero; invalid bases and overflow are errors.
    pub fn rescale(&self, new_num: u32, new_den: u32) -> crate::CodecResult<i64> {
        if self.time_base_num == 0 || self.time_base_den == 0 || new_num == 0 || new_den == 0 {
            return Err(crate::CodecError::InvalidData);
        }
        let numerator = i128::from(self.pts)
            .checked_mul(i128::from(self.time_base_num))
            .and_then(|v| v.checked_mul(i128::from(new_den)))
            .ok_or(crate::CodecError::InvalidData)?;
        let denominator = i128::from(self.time_base_den)
            .checked_mul(i128::from(new_num))
            .ok_or(crate::CodecError::InvalidData)?;
        i64::try_from(numerator / denominator).map_err(|_| crate::CodecError::InvalidData)
    }

    fn get_exact_compare_values(&self, other: &Self) -> (i128, i128) {
        let val1 =
            (self.pts as i128) * (self.time_base_num as i128) * (other.time_base_den as i128);
        let val2 =
            (other.pts as i128) * (other.time_base_num as i128) * (self.time_base_den as i128);
        (val1, val2)
    }
}

impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        let (v1, v2) = self.get_exact_compare_values(other);
        v1 == v2
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        let (v1, v2) = self.get_exact_compare_values(other);
        v1.cmp(&v2)
    }
}

/// 1D Kalman filter estimating true display VSync refresh period T_vsync
/// and computing integer VSync cadence ticks and oversample blend weight alpha_mix.
#[derive(Debug, Clone)]
pub struct KalmanVsyncEstimator {
    pub period_sec: f64,
    pub variance: f64,
    pub process_noise: f64,
    pub measurement_noise: f64,
    pub last_vsync_sec: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VsyncCadenceStep {
    pub expected_vsync_sec: f64,
    pub vsync_ticks: u32,
    pub alpha_mix: f64,
}

impl KalmanVsyncEstimator {
    pub fn new(nominal_hz: f64) -> Self {
        let nominal_period = if nominal_hz.is_finite() && nominal_hz > 0.0 {
            1.0 / nominal_hz
        } else {
            0.0
        };
        let period_sec = if nominal_period.is_finite() && nominal_period > 0.0 {
            nominal_period
        } else {
            1.0 / 60.0
        };
        Self {
            period_sec,
            variance: 1e-6,
            process_noise: 1e-9,
            measurement_noise: 1e-7,
            last_vsync_sec: None,
        }
    }

    pub fn observe_vsync(&mut self, vsync_timestamp_sec: f64) {
        // An invalid display timestamp must not poison subsequent measurements.
        if !vsync_timestamp_sec.is_finite() {
            return;
        }
        if let Some(prev) = self.last_vsync_sec {
            let measured_dt = vsync_timestamp_sec - prev;
            if measured_dt > 0.001 && measured_dt < 0.1 {
                let prior_var = self.variance + self.process_noise;
                let kalman_gain = prior_var / (prior_var + self.measurement_noise);
                self.period_sec += kalman_gain * (measured_dt - self.period_sec);
                self.variance = (1.0 - kalman_gain) * prior_var;
            }
        }
        self.last_vsync_sec = Some(vsync_timestamp_sec);
    }

    pub fn map_frame_pts(&self, frame_pts_sec: f64, base_vsync_sec: f64) -> VsyncCadenceStep {
        let base = if base_vsync_sec.is_finite() {
            base_vsync_sec
        } else {
            0.0
        };
        let fallback = VsyncCadenceStep {
            expected_vsync_sec: base,
            vsync_ticks: 0,
            alpha_mix: 0.0,
        };
        if !frame_pts_sec.is_finite() {
            return fallback;
        }
        let period = if self.period_sec.is_finite() && self.period_sec > 0.0 {
            self.period_sec
        } else {
            1.0 / 60.0
        };
        let exact_ticks = (frame_pts_sec - base).max(0.0) / period;
        if !exact_ticks.is_finite() {
            return fallback;
        }
        // Avoid an inconsistent fractional blend after integer tick saturation.
        let saturated = exact_ticks >= f64::from(u32::MAX);
        let vsync_ticks = exact_ticks.floor() as u32;
        let alpha_mix = if saturated {
            0.0
        } else {
            exact_ticks - f64::from(vsync_ticks)
        };
        let expected_vsync_sec = base + f64::from(vsync_ticks) * period;
        if !expected_vsync_sec.is_finite() {
            return fallback;
        }
        VsyncCadenceStep {
            expected_vsync_sec,
            vsync_ticks,
            alpha_mix,
        }
    }
}

/// Closed-loop discrete-time Proportional-Integral-Derivative (PID) controller
/// locking audio resampling ratio `r[k]` to VSync-aligned video presentation cadence.
#[derive(Debug, Clone)]
pub struct PidClockController {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
    pub delta_max: f64,
    pub integral: f64,
    pub prev_error: Option<f64>,
    /// Content-aware audio delay register (e.g., +1 frame offset for RIFE lookahead).
    pub audio_delay_offset_sec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncStepOutput {
    pub phase_error_sec: f64,
    pub control_signal: f64,
    pub audio_resample_ratio: f64,
}

impl Default for PidClockController {
    fn default() -> Self {
        Self::new_critically_damped()
    }
}

impl PidClockController {
    /// Critically damped defaults from architectural specification:
    /// Kp = 0.015, Ki = 0.0005 s^-1, Kd = 0.002 s, delta_max = 0.005 (+/- 0.5%).
    pub fn new_critically_damped() -> Self {
        Self {
            kp: 0.015,
            ki: 0.0005,
            kd: 0.002,
            delta_max: 0.005,
            integral: 0.0,
            prev_error: None,
            audio_delay_offset_sec: 0.0,
        }
    }

    pub fn set_neural_lookahead_frames(&mut self, lookahead_frames: u32, frame_rate_hz: f64) {
        if frame_rate_hz.is_finite() && frame_rate_hz > 0.0 {
            let delay = f64::from(lookahead_frames) / frame_rate_hz;
            if delay.is_finite() {
                self.audio_delay_offset_sec = delay;
            }
        }
    }

    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.prev_error = None;
    }

    fn neutral_step(&mut self) -> SyncStepOutput {
        self.reset();
        SyncStepOutput {
            phase_error_sec: 0.0,
            control_signal: 0.0,
            audio_resample_ratio: 1.0,
        }
    }

    /// Evaluates:
    /// `e[k] = (PTS_v[k] - t_vsync_expected[k]) - (PTS_a[k] + delay_offset - t_dac_buffered[k])`
    /// `u[k] = Kp * e[k] + Ki * sum(e[j] * dt_j) + Kd * (e[k] - e[k-1]) / dt_k`
    /// `r[k] = clamp(1.0 + u[k], 1.0 - delta_max, 1.0 + delta_max)`
    pub fn step(
        &mut self,
        pts_video_sec: f64,
        vsync_expected_sec: f64,
        pts_audio_sec: f64,
        dac_buffered_sec: f64,
        dt_sec: f64,
    ) -> SyncStepOutput {
        // Invalid clock data must not corrupt the controller's integral/derivative state.
        if ![
            pts_video_sec,
            vsync_expected_sec,
            pts_audio_sec,
            dac_buffered_sec,
            self.audio_delay_offset_sec,
            self.kp,
            self.ki,
            self.kd,
            self.delta_max,
            self.integral,
        ]
        .iter()
        .all(|value| value.is_finite())
            || self.delta_max < 0.0
        {
            return self.neutral_step();
        }
        let dt = if dt_sec.is_finite() {
            dt_sec.max(1e-4)
        } else {
            1.0 / 60.0
        };
        let video_phase = pts_video_sec - vsync_expected_sec;
        let audio_phase = (pts_audio_sec + self.audio_delay_offset_sec) - dac_buffered_sec;
        let error = video_phase - audio_phase;
        if !error.is_finite() {
            return self.neutral_step();
        }

        // Anti-windup clamped integral accumulation
        let max_integral = if self.ki > 0.0 {
            self.delta_max / self.ki
        } else {
            1.0
        };
        self.integral = (self.integral + error * dt).clamp(-max_integral, max_integral);

        let derivative = match self.prev_error {
            Some(prev) => (error - prev) / dt,
            None => 0.0,
        };
        self.prev_error = Some(error);

        let u = self.kp * error + self.ki * self.integral + self.kd * derivative;
        if !u.is_finite() {
            return self.neutral_step();
        }
        let r = (1.0 + u).clamp(1.0 - self.delta_max, 1.0 + self.delta_max);

        SyncStepOutput {
            phase_error_sec: error,
            control_signal: u,
            audio_resample_ratio: r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_rescale_and_invalid_bases() {
        let t = Timestamp::new(i64::MAX, u32::MAX, u32::MAX).unwrap();
        assert_eq!(t.rescale(u32::MAX, u32::MAX).unwrap(), i64::MAX);
        assert!(Timestamp::new(1, 0, 1).is_err());
        assert!(Timestamp::new(1, 1, 0).is_err());
        assert!(t.rescale(0, 1).is_err());
        assert!(t.rescale(1, 0).is_err());
        assert!(t.rescale(1, 2).is_err());
        assert_eq!(Timestamp::new(-3, 1, 2).unwrap().rescale(1, 1).unwrap(), -1);
        assert!(Timestamp {
            pts: 1,
            time_base_num: 0,
            time_base_den: 0
        }
        .rescale(1, 1)
        .is_err());
    }
    #[test]
    fn exact_ordering() {
        assert_eq!(
            Timestamp::new(1, 1, 2).unwrap(),
            Timestamp::new(2, 1, 4).unwrap()
        );
        assert!(Timestamp::new(-1, 1, 2).unwrap() < Timestamp::new(0, 1, 1).unwrap());
    }
    #[test]
    fn rational128_and_pid_sync_controller() {
        let r1 = Rational128::new(24000, 1001).unwrap();
        let r2 = Rational128::new(48000, 2002).unwrap();
        assert_eq!(r1, r2);
        let diff = r1.checked_sub(r2).unwrap();
        assert_eq!(diff.num, 0);

        let mut pid = PidClockController::new_critically_damped();
        pid.set_neural_lookahead_frames(1, 24.0);
        assert!((pid.audio_delay_offset_sec - (1.0 / 24.0)).abs() < 1e-9);
        let out = pid.step(1.05, 1.00, 1.00, 1.00, 1.0 / 60.0);
        assert!(out.audio_resample_ratio >= 0.995 && out.audio_resample_ratio <= 1.005);

        let mut kalman = KalmanVsyncEstimator::new(60.0);
        kalman.observe_vsync(0.0);
        kalman.observe_vsync(1.0 / 60.0);
        let step = kalman.map_frame_pts(1.0 / 24.0, 0.0);
        assert_eq!(step.vsync_ticks, 2);
        assert!((step.alpha_mix - 0.5).abs() < 1e-2);
    }

    #[test]
    fn invalid_display_clock_samples_do_not_poison_estimator() {
        for bad_rate in [0.0, -60.0, f64::NAN, f64::INFINITY, f64::MAX] {
            let estimator = KalmanVsyncEstimator::new(bad_rate);
            assert!(estimator.period_sec.is_finite() && estimator.period_sec > 0.0);
            assert_eq!(estimator.map_frame_pts(0.0, 0.0).vsync_ticks, 0);
        }
        let mut estimator = KalmanVsyncEstimator::new(60.0);
        estimator.observe_vsync(0.0);
        estimator.observe_vsync(f64::NAN);
        estimator.observe_vsync(f64::INFINITY);
        assert_eq!(estimator.last_vsync_sec, Some(0.0));
        estimator.observe_vsync(1.0 / 60.0);
        assert!(estimator.period_sec.is_finite());
        let invalid = estimator.map_frame_pts(f64::NAN, 123.0);
        assert_eq!(invalid.vsync_ticks, 0);
        assert_eq!(invalid.alpha_mix, 0.0);
        assert_eq!(invalid.expected_vsync_sec, 123.0);
        let huge = estimator.map_frame_pts(1.0e12, 0.0);
        assert_eq!(huge.vsync_ticks, u32::MAX);
        assert_eq!(huge.alpha_mix, 0.0);
    }

    #[test]
    fn invalid_sync_inputs_recover_without_persistent_nan_state() {
        let mut pid = PidClockController::new_critically_damped();
        let valid = pid.step(1.05, 1.0, 1.0, 1.0, 1.0 / 60.0);
        assert!(valid.audio_resample_ratio.is_finite());
        pid.set_neural_lookahead_frames(1, f64::NAN);
        pid.set_neural_lookahead_frames(1, f64::INFINITY);
        assert_eq!(pid.audio_delay_offset_sec, 0.0);
        let invalid = pid.step(f64::NAN, 1.0, 1.0, 1.0, f64::NAN);
        assert_eq!(invalid.audio_resample_ratio, 1.0);
        assert_eq!(invalid.control_signal, 0.0);
        assert_eq!(pid.integral, 0.0);
        assert_eq!(pid.prev_error, None);
        let recovered = pid.step(1.05, 1.0, 1.0, 1.0, f64::NAN);
        assert!(recovered.audio_resample_ratio.is_finite());
        assert!(pid.integral.is_finite());
    }
}
