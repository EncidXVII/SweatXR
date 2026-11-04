//! TCX (TrainingCenterDatabase v2) and CSV. Both are written by hand: the formats are tiny and
//! a serializer would add more code than it saves. quick-xml does the escaping.

use std::fmt::Write;

use quick_xml::escape::escape;

use crate::storage::Session;
use crate::time::{iso_utc, parse_iso_utc};
use crate::tracker::Sample;

const TCX_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<TrainingCenterDatabase xmlns="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2 http://www.garmin.com/xmlschemas/TrainingCenterDatabasev2.xsd">
  <Activities>
"#;

/// One activity per session, element order as the XSD sequence requires.
pub fn tcx(sessions: &[(Session, Vec<Sample>)]) -> String {
    let mut x = String::from(TCX_HEAD);
    for (s, samples) in sessions {
        let start = iso_utc(s.start);
        let hr = |v: Option<u16>| v.filter(|&b| (1..=255).contains(&b));
        let _ = write!(
            x,
            "    <Activity Sport=\"Other\">\n      <Id>{start}</Id>\n      <Lap StartTime=\"{start}\">\n        \
             <TotalTimeSeconds>{}</TotalTimeSeconds>\n        <DistanceMeters>0</DistanceMeters>\n        \
             <Calories>{}</Calories>\n",
            (s.end - s.start).max(1),
            s.kcal.round().clamp(0.0, 65_535.0) as u16,
        );
        if let Some(avg) = hr(s.hr_avg.map(|a| a.round() as u16)) {
            let _ = writeln!(
                x,
                "        <AverageHeartRateBpm><Value>{avg}</Value></AverageHeartRateBpm>"
            );
        }
        if let Some(max) = hr(s.hr_max) {
            let _ = writeln!(
                x,
                "        <MaximumHeartRateBpm><Value>{max}</Value></MaximumHeartRateBpm>"
            );
        }
        x.push_str("        <Intensity>Active</Intensity>\n        <TriggerMethod>Manual</TriggerMethod>\n");
        if !samples.is_empty() {
            x.push_str("        <Track>\n");
            for p in samples {
                let _ = write!(x, "          <Trackpoint><Time>{}</Time>", iso_utc(p.ts));
                if let Some(b) = hr(p.hr) {
                    let _ = write!(x, "<HeartRateBpm><Value>{b}</Value></HeartRateBpm>");
                }
                x.push_str("</Trackpoint>\n");
            }
            x.push_str("        </Track>\n");
        }
        let _ = write!(
            x,
            "      </Lap>\n      <Notes>{} (VR, calories are an estimate)</Notes>\n    </Activity>\n",
            escape(s.app_name.as_str())
        );
    }
    x.push_str("  </Activities>\n</TrainingCenterDatabase>\n");
    x
}

pub const CSV_HEADER: &str = "time_utc,app,intensity,met,heart_rate,kcal";

pub fn csv(samples: &[Sample]) -> String {
    let mut out = format!("{CSV_HEADER}\n");
    for s in samples {
        let hr = s.hr.map(|h| h.to_string()).unwrap_or_default();
        let _ = writeln!(
            out,
            "{},{},{},{},{hr},{}",
            iso_utc(s.ts),
            csv_field(&s.app_key),
            s.intensity,
            s.met,
            s.kcal
        );
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[derive(Debug, PartialEq)]
pub struct CsvError(pub usize);

impl std::fmt::Display for CsvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad CSV row {}", self.0)
    }
}

impl std::error::Error for CsvError {}

pub fn parse_csv(text: &str) -> Result<Vec<Sample>, CsvError> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, h)) if h == CSV_HEADER => {}
        _ => return Err(CsvError(1)),
    }
    lines
        .filter(|(_, l)| !l.is_empty())
        .map(|(i, line)| {
            let f = split_csv(line);
            let err = || CsvError(i + 1);
            if f.len() != 6 {
                return Err(err());
            }
            Ok(Sample {
                ts: parse_iso_utc(&f[0]).ok_or_else(err)?,
                app_key: f[1].clone(),
                intensity: f[2].parse().map_err(|_| err())?,
                met: f[3].parse().map_err(|_| err())?,
                hr: if f[4].is_empty() {
                    None
                } else {
                    Some(f[4].parse().map_err(|_| err())?)
                },
                kcal: f[5].parse().map_err(|_| err())?,
            })
        })
        .collect()
}

fn split_csv(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::{Reader, events::Event};

    fn fixture() -> (Session, Vec<Sample>) {
        let samples: Vec<Sample> = (0..90)
            .map(|i| Sample {
                ts: 1_800_000_000 + i,
                app_key: "steam.app.1, \"beat\"".into(),
                intensity: 0.5 + i as f32 / 1000.0,
                met: 4.2,
                hr: (i % 3 != 0).then_some(120 + i as u16 % 30),
                kcal: 0.123_456_7,
            })
            .collect();
        let session = Session {
            id: 1,
            app_key: samples[0].app_key.clone(),
            app_name: "Rhythm <Test> & Co".into(),
            start: samples[0].ts,
            end: samples[89].ts,
            kcal: 11.1,
            active_min: 1,
            hr_avg: Some(134.5),
            hr_max: Some(149),
        };
        (session, samples)
    }

    #[test]
    fn csv_round_trips() {
        let (_, samples) = fixture();
        assert_eq!(parse_csv(&csv(&samples)), Ok(samples));
        assert_eq!(parse_csv("nope"), Err(CsvError(1)));
    }

    #[test]
    fn tcx_is_well_formed_with_expected_content() {
        let x = tcx(&[fixture()]);
        let mut r = Reader::from_str(&x);
        let (mut points, mut hr, mut depth) = (0, 0, 0i32);
        loop {
            match r.read_event().expect("xml") {
                Event::Start(e) => {
                    depth += 1;
                    let name = e.name();
                    points += i32::from(name.as_ref() == "Trackpoint");
                    hr += i32::from(name.as_ref() == "HeartRateBpm");
                }
                Event::End(_) => depth -= 1,
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!((points, hr, depth), (90, 60, 0));
        assert!(x.contains("<Calories>11</Calories>") && x.contains("Rhythm &lt;Test&gt; &amp; Co"));
    }
}
