//! AUTO GATE 2: synthetic traces give the expected MET bands, active minutes and session splits.

use fitvr_core::PoseSource;
use fitvr_core::intensity::IntensityMeter;
use fitvr_core::profile::Profile;
use fitvr_core::replay::{Pattern, RATE_HZ, Replay, synth};
use fitvr_core::tracker::{Event, Tracker};

struct Outcome {
    mean_met: f32,
    active_min: usize,
    sessions: usize,
    kcal: f32,
}

/// Feeds one or more (pattern, seconds, app) segments through meter + tracker at 1 Hz.
fn play(segments: &[(Pattern, f64, &str)]) -> Outcome {
    let p = Profile::default();
    let mut meter = IntensityMeter::new(p.height_cm);
    let mut tracker = Tracker::new();
    let mut events = Vec::new();
    let (mut t0, mut met_sum, mut met_n) = (0.0, 0.0, 0);
    for &(pattern, secs, app) in segments {
        let mut replay = Replay::new(synth(pattern, secs, t0));
        let mut i = 0usize;
        while let Some(f) = replay.next_frame() {
            let intensity = meter.update(&f);
            i += 1;
            if i.is_multiple_of(RATE_HZ as usize) {
                let ts = 1_800_000_000 + f.t as i64;
                let ev = tracker.tick(ts, intensity, None, app, &p);
                met_sum += fitvr_core::energy::met_from_intensity(intensity);
                met_n += 1;
                events.extend(ev);
            }
        }
        t0 += secs;
    }
    events.extend(tracker.finish());
    Outcome {
        mean_met: met_sum / met_n as f32,
        active_min: events
            .iter()
            .filter(|e| matches!(e, Event::ActiveMinute { .. }))
            .count(),
        sessions: events.iter().filter(|e| matches!(e, Event::Start { .. })).count(),
        kcal: events
            .iter()
            .map(|e| if let Event::Sample(s) = e { s.kcal } else { 0.0 })
            .sum(),
    }
}

#[test]
fn idle_is_sedentary_and_never_starts_a_session() {
    let o = play(&[(Pattern::Idle, 300.0, "g")]);
    assert!(o.mean_met < 1.5, "{}", o.mean_met);
    assert_eq!((o.sessions, o.active_min, o.kcal), (0, 0, 0.0));
}

#[test]
fn light_sway_is_light_and_not_active() {
    let o = play(&[(Pattern::Sway, 300.0, "g")]);
    assert!((1.5..3.0).contains(&o.mean_met), "{}", o.mean_met);
    assert_eq!((o.sessions, o.active_min), (0, 0));
}

#[test]
fn sustained_cardio_is_moderate_and_every_minute_active() {
    let o = play(&[(Pattern::SustainedCardio, 600.0, "g")]);
    assert!((3.0..6.0).contains(&o.mean_met), "{}", o.mean_met);
    assert_eq!(o.sessions, 1);
    assert!(o.active_min >= 9, "{}", o.active_min);
}

#[test]
fn boxing_bursts_are_vigorous_in_bursts() {
    let o = play(&[(Pattern::BoxingBursts, 600.0, "g")]);
    assert!(o.mean_met >= 4.5, "{}", o.mean_met);
    assert_eq!(o.sessions, 1, "10 s rests must not split the session");
    assert!(o.active_min >= 9, "{}", o.active_min);
}

#[test]
fn long_idle_and_app_change_split_sessions() {
    let o = play(&[
        (Pattern::SustainedCardio, 120.0, "a"),
        (Pattern::Idle, 360.0, "a"),
        (Pattern::SustainedCardio, 120.0, "a"),
        (Pattern::SustainedCardio, 120.0, "b"),
    ]);
    assert_eq!(o.sessions, 3);
}

#[test]
fn idle_tail_is_not_counted() {
    let short = play(&[(Pattern::SustainedCardio, 120.0, "a")]);
    let tail = play(&[
        (Pattern::SustainedCardio, 120.0, "a"),
        (Pattern::Idle, 360.0, "a"),
    ]);
    // A few seconds of EMA decay still count as motion; five idle minutes must not.
    assert!(tail.kcal - short.kcal < 1.0, "{} vs {}", tail.kcal, short.kcal);
}

#[test]
fn recorded_csv_replays() {
    let frames = synth(Pattern::Sway, 2.0, 0.0);
    let text: String = std::iter::once(fitvr_core::replay::CSV_HEADER.to_owned())
        .chain(frames.iter().map(fitvr_core::replay::csv_row))
        .collect::<Vec<_>>()
        .join("\n");
    let mut r = Replay::from_csv(&text).expect("parse");
    let mut n = 0;
    while r.next_frame().is_some() {
        n += 1;
    }
    assert_eq!(n, frames.len());
}
