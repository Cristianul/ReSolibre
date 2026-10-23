//! Portable, versioned named presets; names are labels, never paths or filter commands.

use super::Equalizer;

pub const MAX_BYTES: usize = 4096;
pub const MAX_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub settings: Equalizer,
}

impl Preset {
    /// Trims and validates a display name while retaining already validated EQ values.
    pub fn new(name: &str, settings: Equalizer) -> Result<Self, &'static str> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err("Use a preset name of 1–64 characters, without control characters");
        }
        Ok(Self {
            name: name.to_owned(),
            settings,
        })
    }

    /// Encodes only a name and numeric values; version one fixes the shared band order and Q.
    pub fn encode(&self) -> String {
        let mut text = format!(
            "ReSolibre-EQ-Preset-1\n{}\n{} {:e}",
            self.name,
            u8::from(self.settings.enabled()),
            self.settings.preamp()
        );
        for value in self.settings.bands() {
            text.push_str(&format!(" {value:e}"));
        }
        text.push('\n');
        text
    }

    /// Rejects oversized, malformed or unsupported data before constructing any audio settings.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let invalid = "Invalid or unsupported ReSolibre EQ preset";
        if bytes.len() > MAX_BYTES {
            return Err(invalid);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| invalid)?;
        let mut lines = text.lines();
        if lines.next() != Some("ReSolibre-EQ-Preset-1") {
            return Err(invalid);
        }
        let name = lines.next().ok_or(invalid)?;
        let mut fields = lines.next().ok_or(invalid)?.split_whitespace();
        let enabled = match fields.next() {
            Some("0") => false,
            Some("1") => true,
            _ => return Err(invalid),
        };
        let preamp = fields.next().ok_or(invalid)?.parse().map_err(|_| invalid)?;
        let mut bands = [0.0; 10];
        for band in &mut bands {
            *band = fields.next().ok_or(invalid)?.parse().map_err(|_| invalid)?;
        }
        if fields.next().is_some() || lines.next().is_some() {
            return Err(invalid);
        }
        Self::new(
            name,
            Equalizer::new(enabled, bands, preamp).map_err(|_| invalid)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unicode names and tiny finite gains round-trip; invalid names, versions and gains fail closed.
    #[test]
    fn portable_presets_validate_all_fields() {
        let preset = Preset::new(
            "  Căști · 夜  ",
            Equalizer::new(false, [1e-300; 10], -1.5).unwrap(),
        )
        .unwrap();
        assert_eq!(Preset::decode(preset.encode().as_bytes()).unwrap(), preset);
        for name in ["", " \n ", "bad\nname", "bad\0name", &"a".repeat(65)] {
            assert!(Preset::new(name, Equalizer::default()).is_err());
        }
        let valid = Preset::new("Desk", Equalizer::preset(1).unwrap())
            .unwrap()
            .encode();
        for invalid in [
            valid.replace("Preset-1", "Preset-9"),
            valid.replace("1 0e0", "2 0e0"),
            valid.replace("1 0e0", "1 NaN"),
            valid.replace("1 0e0", "1 3"),
            valid.replace("5e0", "13"),
            format!("{valid}extra"),
            "x".repeat(MAX_BYTES + 1),
        ] {
            assert!(Preset::decode(invalid.as_bytes()).is_err());
        }
        assert!(Preset::decode(&[0xff]).is_err());
        assert!(Preset::decode(b"ReSolibre-EQ-Preset-1\nDesk\n1 0 1\n").is_err());
    }
}
