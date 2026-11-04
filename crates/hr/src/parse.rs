//! Heart Rate Measurement characteristic (0x2A37), Bluetooth GATT spec.
//! Flags: bit 0 = 16-bit value, bits 1-2 = contact status, bit 3 = energy, bit 4 = RR intervals.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    NotSupported,
    Detected,
    NotDetected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub bpm: u16,
    pub contact: Contact,
    pub energy_kj: Option<u16>,
    /// Seconds between beats.
    pub rr: Vec<f32>,
}

impl Measurement {
    /// Straps report 0 or junk while not on skin.
    pub fn usable(&self) -> bool {
        self.bpm > 0 && self.contact != Contact::NotDetected
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Truncated,
}

pub fn parse(b: &[u8]) -> Result<Measurement, ParseError> {
    let (&flags, mut rest) = b.split_first().ok_or(ParseError::Empty)?;
    let u16_le = |rest: &mut &[u8]| -> Result<u16, ParseError> {
        let (v, tail) = rest.split_first_chunk::<2>().ok_or(ParseError::Truncated)?;
        *rest = tail;
        Ok(u16::from_le_bytes(*v))
    };
    let bpm = if flags & 0x01 != 0 {
        u16_le(&mut rest)?
    } else {
        let (&v, tail) = rest.split_first().ok_or(ParseError::Truncated)?;
        rest = tail;
        u16::from(v)
    };
    let contact = match (flags >> 1) & 0b11 {
        0b11 => Contact::Detected,
        0b10 => Contact::NotDetected,
        _ => Contact::NotSupported,
    };
    let energy_kj = if flags & 0x08 != 0 {
        Some(u16_le(&mut rest)?)
    } else {
        None
    };
    let mut rr = Vec::new();
    if flags & 0x10 != 0 {
        if rest.len() % 2 != 0 {
            return Err(ParseError::Truncated);
        }
        while !rest.is_empty() {
            rr.push(f32::from(u16_le(&mut rest)?) / 1024.0);
        }
    }
    Ok(Measurement {
        bpm,
        contact,
        energy_kj,
        rr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_no_contact_bits() {
        let m = parse(&[0x00, 72]).expect("parse");
        assert_eq!(
            (m.bpm, m.contact, m.energy_kj, m.rr.len()),
            (72, Contact::NotSupported, None, 0)
        );
        assert!(m.usable());
    }

    #[test]
    fn sixteen_bit_value() {
        assert_eq!(parse(&[0x01, 0x2C, 0x01]).expect("parse").bpm, 300);
        assert_eq!(parse(&[0x01, 0x8C, 0x00]).expect("parse").bpm, 140);
    }

    #[test]
    fn contact_flags() {
        assert_eq!(parse(&[0x06, 80]).expect("p").contact, Contact::Detected);
        let off = parse(&[0x04, 80]).expect("p");
        assert_eq!(off.contact, Contact::NotDetected);
        assert!(!off.usable());
    }

    #[test]
    fn energy_and_rr_intervals() {
        let m = parse(&[0x19, 0x8C, 0x00, 0x10, 0x00, 0x00, 0x02, 0x00, 0x04]).expect("parse");
        assert_eq!((m.bpm, m.energy_kj), (140, Some(16)));
        assert_eq!(m.rr, vec![0.5, 1.0]);
        assert_eq!(parse(&[0x10, 70, 0x00, 0x04]).expect("p").rr, vec![1.0]);
    }

    #[test]
    fn malformed() {
        assert_eq!(parse(&[]), Err(ParseError::Empty));
        assert_eq!(parse(&[0x00]), Err(ParseError::Truncated));
        assert_eq!(parse(&[0x01, 0x8C]), Err(ParseError::Truncated));
        assert_eq!(parse(&[0x10, 70, 0x00]), Err(ParseError::Truncated));
    }
}
