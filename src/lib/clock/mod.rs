use std::time::Duration;

pub trait ClockTickListener: Send + Sync {
    fn tick_updated(&mut self, instant: ClockInstant);
}

pub trait Clock {
    fn elapsed(&self) -> ClockInstant;
    fn add_tick_listener(&self, listener: Box<dyn ClockTickListener>);
}

#[derive(Debug, Clone, Copy)]
pub struct ClockInstant {
    pub instant: u64,
    pub tick_duration: Duration,
}

impl ClockInstant {
    pub fn as_duration(&self) -> Duration {
        self.tick_duration.mul_f32(self.instant as f32)
    }

    pub fn duration_since(&self, other: ClockInstant) -> Duration {
        self.tick_duration
            .mul_f32((self.instant - other.instant) as f32)
    }

    pub fn add_duration(&self, duration: Duration) -> ClockInstant {
        let scaled = duration.div_duration_f32(self.tick_duration) as u64;
        ClockInstant {
            instant: self.instant.wrapping_add(scaled),
            tick_duration: self.tick_duration,
        }
    }

    pub fn sub_duration(&self, duration: Duration) -> ClockInstant {
        let scaled = duration.div_duration_f32(self.tick_duration) as u64;
        ClockInstant {
            instant: self.instant.wrapping_sub(scaled),
            tick_duration: self.tick_duration,
        }
    }
}
