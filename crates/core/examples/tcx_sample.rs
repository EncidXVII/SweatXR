//! Prints a TCX file for schema validation in CI: `cargo run --example tcx_sample > s.tcx`
//! then `xmllint --noout --schema TrainingCenterDatabasev2.xsd s.tcx`.

use fitvr_core::export::tcx;
use fitvr_core::storage::Session;
use fitvr_core::tracker::Sample;

fn main() {
    let t0 = 1_800_000_000;
    let samples: Vec<Sample> = (0..120)
        .map(|i| Sample {
            ts: t0 + i,
            app_key: "steam.app.1".into(),
            intensity: 0.7,
            met: 5.0,
            hr: (i % 4 != 0).then_some(130),
            kcal: 0.1,
        })
        .collect();
    let with_hr = Session {
        id: 1,
        app_key: "steam.app.1".into(),
        app_name: "Rhythm & Blocks".into(),
        start: t0,
        end: t0 + 119,
        kcal: 12.0,
        active_min: 2,
        hr_avg: Some(130.0),
        hr_max: Some(130),
    };
    let no_samples = Session {
        id: 2,
        start: t0 + 600,
        end: t0 + 900,
        hr_avg: None,
        hr_max: None,
        ..with_hr.clone()
    };
    print!("{}", tcx(&[(with_hr, samples), (no_samples, Vec::new())]));
}
