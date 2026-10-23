//! Capability boundary: the sealed Linux LADSPA tap is not available on Windows.

pub(super) struct Block {
    pub first_frame: u64,
    pub instance: u32,
    pub rate: u32,
    pub channels: u32,
    pub frames: u32,
    pub samples: [f32; 0],
}

pub(super) struct Tap;

impl Tap {
    /// Reports an unavailable optional visualization without interrupting audio playback.
    pub fn new() -> Result<Self, &'static str> {
        Err("Live spectrum is unavailable on Windows")
    }

    /// No graph is installed when the platform cannot create a tap.
    pub fn graph(&self, _channels: u32) -> String {
        String::new()
    }

    /// No samples are published without a validated native producer.
    pub fn read(&mut self) -> Result<Option<&Block>, &'static str> {
        Ok(None)
    }

    /// The unavailable producer never allocates a graph identity.
    pub fn barrier(&self) -> u32 {
        0
    }
}
