use std::sync::Arc;

use crate::{
    clock::ClockTickListener,
    targets::appleii::io::{
        peripherals::mouse::MouseCard, soft_switches::SoftSwitches, video::Video,
    },
};

pub struct VblTickListener {
    pub last_vbl: u64,
    pub vbl: bool,
    pub soft_switches: Arc<SoftSwitches>,
    pub mouse_card: Arc<MouseCard>,
    pub video: Arc<Video>,
}

impl ClockTickListener for VblTickListener {
    fn tick_updated(&mut self, instant: crate::clock::ClockInstant) {
        // about 32 cycles per scan line, 525 total, about 45 VBI
        // 32 cycles * 480 lines  means we are drawing visible lines for ~15360 cycles
        // 32 cycles * 45 lines = 1440

        if instant.instant > self.last_vbl {
            let period = instant.instant - self.last_vbl;

            // Enter VBL
            if period >= 15360 && !self.vbl {
                self.video.update_frame();
                self.soft_switches.set_vbl(true);
                self.vbl = true;
                //trace!("VBL ON @ {}", instant.instant);
                self.mouse_card.signal_vbl();
            }

            // Exit VBL
            if period >= 15360 + 1440 && self.vbl {
                self.soft_switches.set_vbl(false);
                self.vbl = false;
                self.last_vbl = instant.instant;
                //trace!("VBL OFF @ {}", instant.instant);
            }
        } else {
            self.last_vbl = instant.instant;
        }
    }
}
