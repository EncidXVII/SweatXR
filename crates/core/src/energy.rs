// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! kcal estimates. Heart rate when available (Keytel et al. 2005, prediction equation without
//! VO2max), otherwise motion intensity mapped to METs, optionally through a personal fit.

use crate::profile::{Calibration, Profile, Sex};

pub const KJ_PER_KCAL: f32 = 4.184;
pub const SEDENTARY_MET: f32 = 1.3;
/// Below this many paired samples (20 min at 1 Hz) a personal fit is too noisy to trust.
pub const MIN_CALIBRATION_SAMPLES: usize = 20 * 60;

/// (intensity, MET) knots: sedentary, light, moderate, vigorous. Linear between, flat outside.
const MET_CURVE: [(f32, f32); 4] = [(0.05, SEDENTARY_MET), (0.25, 2.5), (0.6, 4.5), (1.2, 7.0)];

/// kcal/min from heart rate. Zero up to resting + 10 bpm: the equation was fitted on exercise
/// data and returns nonsense near rest.
pub fn keytel_kcal_min(hr: f32, p: &Profile) -> f32 {
    if hr < (p.resting_hr + 10) as f32 {
        return 0.0;
    }
    let (w, a) = (p.weight_kg, p.age as f32);
    let kj = match p.sex {
        Sex::Male => -55.0969 + 0.6309 * hr + 0.1988 * w + 0.2017 * a,
        Sex::Female => -20.4022 + 0.4472 * hr - 0.1263 * w + 0.074 * a,
    };
    (kj / KJ_PER_KCAL).max(0.0)
}

pub fn met_from_intensity(i: f32) -> f32 {
    let (first, last) = (MET_CURVE[0], MET_CURVE[MET_CURVE.len() - 1]);
    if i <= first.0 {
        return first.1;
    }
    MET_CURVE.windows(2).find(|w| i <= w[1].0).map_or(last.1, |w| {
        w[0].1 + (i - w[0].0) / (w[1].0 - w[0].0) * (w[1].1 - w[0].1)
    })
}

pub fn kcal_min_from_met(met: f32, kg: f32) -> f32 {
    met * 3.5 * kg / 200.0
}

pub fn met_from_kcal_min(kcal_min: f32, kg: f32) -> f32 {
    kcal_min / kcal_min_from_met(1.0, kg)
}

/// (kcal/min, MET) for one second. HR wins; the personal fit beats the generic curve.
pub fn estimate(intensity: f32, hr: Option<u16>, p: &Profile) -> (f32, f32) {
    let kcal = match (hr, p.calibration) {
        (Some(hr), _) if hr > 0 => keytel_kcal_min(f32::from(hr), p),
        (_, Some(c)) => (c.slope * intensity + c.intercept).max(0.0),
        _ => kcal_min_from_met(met_from_intensity(intensity), p.weight_kg),
    };
    (kcal, met_from_kcal_min(kcal, p.weight_kg))
}

/// Huber-weighted least squares of kcal/min on intensity. Robust to strap dropouts and the
/// zero floor of the HR equation, which plain least squares would chase.
pub fn fit_calibration(points: &[(f32, f32)]) -> Option<Calibration> {
    if points.len() < MIN_CALIBRATION_SAMPLES {
        return None;
    }
    let mut w = vec![1.0f64; points.len()];
    let (mut slope, mut intercept) = (0.0f64, 0.0f64);
    for _ in 0..25 {
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (&(x, y), &wi) in points.iter().zip(&w) {
            let (x, y) = (f64::from(x), f64::from(y));
            sw += wi;
            sx += wi * x;
            sy += wi * y;
            sxx += wi * x * x;
            sxy += wi * x * y;
        }
        let det = sw * sxx - sx * sx;
        if det.abs() < 1e-12 {
            return None;
        }
        slope = (sw * sxy - sx * sy) / det;
        intercept = (sy - slope * sx) / sw;
        let res: Vec<f64> = points
            .iter()
            .map(|&(x, y)| (f64::from(y) - slope * f64::from(x) - intercept).abs())
            .collect();
        let mut sorted = res.clone();
        sorted.sort_by(f64::total_cmp);
        let k = 1.345 * 1.4826 * sorted[sorted.len() / 2];
        for (wi, r) in w.iter_mut().zip(res) {
            *wi = if r <= k || k == 0.0 { 1.0 } else { k / r };
        }
    }
    (slope.is_finite() && slope > 0.0).then_some(Calibration {
        slope: slope as f32,
        intercept: intercept as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(sex: Sex, kg: f32, age: u32) -> Profile {
        Profile {
            sex,
            weight_kg: kg,
            age,
            ..Profile::default()
        }
    }

    /// Reference values computed by hand from the published coefficients.
    #[test]
    fn keytel_reference_values() {
        let cases = [
            (person(Sex::Male, 80.0, 35), 150.0, 14.9382),
            (person(Sex::Male, 70.0, 25), 120.0, 9.4574),
            (person(Sex::Female, 60.0, 30), 140.0, 8.8068),
            (person(Sex::Female, 55.0, 45), 165.0, 11.8951),
        ];
        for (p, hr, want) in cases {
            let got = keytel_kcal_min(hr, &p);
            assert!((got - want).abs() / want < 0.01, "{p:?} hr {hr}: {got} vs {want}");
        }
    }

    #[test]
    fn keytel_zero_near_rest() {
        let p = Profile::default();
        assert_eq!(keytel_kcal_min(69.0, &p), 0.0);
        assert!(keytel_kcal_min(71.0, &p) >= 0.0);
    }

    #[test]
    fn met_curve_is_monotonic_and_hits_bands() {
        let mut prev = 0.0;
        for i in 0..=300 {
            let met = met_from_intensity(i as f32 / 200.0);
            assert!(met >= prev, "drops at {i}");
            prev = met;
        }
        assert_eq!(met_from_intensity(0.0), SEDENTARY_MET);
        assert_eq!(met_from_intensity(0.25), 2.5);
        assert_eq!(met_from_intensity(0.6), 4.5);
        assert_eq!(met_from_intensity(5.0), 7.0);
    }

    #[test]
    fn calibration_recovers_slope_despite_outliers() {
        let mut s = 12345u32;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        let points: Vec<(f32, f32)> = (0..1500)
            .map(|i| {
                let x = rnd() * 1.5;
                let y = if i % 20 == 0 {
                    0.0
                } else {
                    9.0 * x + 1.5 + (rnd() - 0.5) * 0.8
                };
                (x, y)
            })
            .collect();
        let c = fit_calibration(&points).expect("fit");
        assert!((c.slope - 9.0).abs() / 9.0 < 0.05, "slope {}", c.slope);
        assert!(fit_calibration(&points[..100]).is_none(), "needs 20 min of data");
    }
}
