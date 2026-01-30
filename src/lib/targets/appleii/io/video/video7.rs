use std::sync::Arc;

use log::debug;

use crate::targets::appleii::io::{
    self,
    soft_switches::{SoftSwitchListener, Switch},
    video::{Video, VideoMode},
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Video7State {
    HiresSet,
    MixClear,
    EightyColClear,
    An3Clear,
    An3Set,
    An3Clear2,
    An3Set2,
    An3Clear3,
}

struct Video7StateMachine {
    mode: VideoMode,
    states: Vec<(Switch, bool)>,
    state: usize,
}

pub struct Video7Listener {
    video: Arc<Video>,
    state: Video7State,

    state_machines: Vec<Video7StateMachine>,
}

impl Video7Listener {
    pub fn new(video: Arc<Video>) -> Video7Listener {
        Video7Listener {
            video,
            state: Video7State::HiresSet,
            state_machines: vec![Video7StateMachine {
                mode: VideoMode::Video7BlackWhite,
                states: vec![
                    (Switch::HiresMode, true),
                    (Switch::MixMode, false),
                    (Switch::Eightycol, false),
                    (Switch::An3, false),
                    (Switch::An3, true),
                    (Switch::An3, false),
                    (Switch::An3, true),
                    (Switch::An3, false),
                ],
                state: 0,
            }],
        }
    }
}

impl SoftSwitchListener for Video7Listener {
    fn on_updated(
        &mut self,
        switch: io::soft_switches::Switch,
        _previous_value: bool,
        new_value: bool,
    ) {
        for fsm in &mut self.state_machines {
            // Setting HIRES resets the state
            if switch == Switch::HiresMode && new_value {
                fsm.state = 1;
            } else {
                let init_state = fsm.state;

                if switch == fsm.states[fsm.state].0 && new_value == fsm.states[fsm.state].1 {
                    fsm.state += 1;
                    if fsm.state == fsm.states.len() {
                        debug!(
                            "Sequence for {:?} detected, setting video mode...",
                            fsm.mode,
                        );
                        self.video.set_video_mode(fsm.mode);
                        fsm.state = 0;
                    }
                }
                if init_state != fsm.state {
                    debug!(
                        "Video7 state {:?} -> {:?}",
                        fsm.states[init_state], fsm.states[fsm.state]
                    );
                }
            }
        }
    }
}
