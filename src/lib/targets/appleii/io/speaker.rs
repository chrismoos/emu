#![allow(unused_mut)]
use std::{
    fmt::Display,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    time::Duration,
};

use fundsp::{
    hacker::{AudioNode, ButterLowpass, Frame},
    prelude::U1,
};
use ringbuf::{
    SharedRb,
    storage::Heap,
    traits::{Consumer, Observer, Producer},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    clock::{Clock, ClockInstant},
    cpu::mos6502::bus::Slave,
    errors::Error,
    utils::time::Instant,
};

struct State {
    //prod: CachingProd<Arc<SharedRb<Heap<f32>>>>,
    fractional_amount: f32,
    fractional_value: f32,
    current_value: bool,
    last_cycle: ClockInstant,
    rx: Option<UnboundedReceiver<Duration>>,
    sample_rate: f64,
}
pub struct Speaker {
    value: Arc<AtomicBool>,
    tx: UnboundedSender<Duration>,
    last_touched: Mutex<Instant>,
    last_cycle: AtomicU64,
    state: Mutex<State>,
    clock: Arc<dyn Clock + Send + Sync>,
    buf: Mutex<SharedRb<Heap<f32>>>,
    lpf: Mutex<ButterLowpass<f32, U1>>,
}

impl Speaker {
    fn toggle(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        let now = self.clock.elapsed();

        if now.as_duration() < state.last_cycle.as_duration() {
            state.last_cycle = now;
        }

        let dur = now.as_duration() - state.last_cycle.as_duration();
        state.last_cycle = now;

        self.tx.send(dur)?;
        self.last_cycle.store(
            now.as_duration().as_micros() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );

        let inst = dur;
        let duration = inst;

        let sample_period = 1.0 / state.sample_rate as f32;

        // This threshold makes a big difference on things like programmers aid audio test
        if duration > Duration::from_secs_f32(1.0 / 50.0) {
            state.fractional_amount = 0.0;
            state.fractional_value = 0.0;
            state.current_value = !state.current_value;
            let silence_samples = (state.sample_rate / 50.0) as usize;
            self.buf
                .lock()
                .unwrap()
                .push_slice(&vec![0.0; silence_samples]);
            return Ok(());
        }

        let mut samples_push = vec![];

        if state.fractional_amount >= 1.0 {
            let amount = state.fractional_value;
            samples_push.push(amount);
            state.fractional_amount -= 1.0;
        }

        let int_samples = (duration.as_secs_f32() / sample_period) as usize;
        let fract_samples = (duration.as_secs_f32() % sample_period) / sample_period;

        // write out any integer samples
        for x in 0..int_samples {
            // include current fractional sample
            let mut val = if state.current_value { 1.0 } else { -1.0 };

            if x == 0 && state.fractional_amount > 0.0 {
                let new_val = val;

                val *= 1.0 - state.fractional_amount;
                val += state.fractional_value * state.fractional_amount;

                state.fractional_value = new_val;
                samples_push.push(val);
            } else {
                samples_push.push(val);
            }
        }

        // push any new fractional samples
        if fract_samples > 0.0 {
            assert!(state.fractional_amount < 1.0);

            state.fractional_value = (state.fractional_value * state.fractional_amount)
                + (fract_samples * if state.current_value { 1.0 } else { -1.0 });
            state.fractional_amount += fract_samples;
            state.fractional_value /= state.fractional_amount;
        }

        // consume state.fractional if >= 1.0
        if state.fractional_amount >= 1.0 {
            let amount = state.fractional_value;
            samples_push.push(amount);
            state.fractional_amount -= 1.0;
        }

        self.buf.lock().unwrap().push_slice(&samples_push);

        state.current_value = !state.current_value;

        Ok(())
    }

    pub fn fill_audio_buffer(&self, data: &mut [f32], channels: usize) {
        let mut buf = self.buf.lock().unwrap();
        let mut lpf = self.lpf.lock().unwrap();
        let data_len = data.len();
        if buf.occupied_len() < data_len {
            //debug!("we're low");
            for b in data {
                *b = lpf.tick(&Frame::from([0.0]))[0];
            }
            return;
        }
        for x in (0..data.len()).step_by(channels) {
            match buf.try_pop() {
                Some(val) => {
                    let val = lpf.tick(&Frame::from([val]))[0];
                    for y in 0..channels {
                        data[x+y] = val;
                    }
                }
                None => {
                    for y in 0..channels {
                        data[x+y] = 0.0;
                    }
                }
            }
        }
    }

    pub fn new(clock_source: Arc<dyn Clock + Send + Sync>) -> Result<Speaker, Error> {
        let value = Arc::new(AtomicBool::new(false));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lpf = ButterLowpass::new(7000.0);

        let default_sample_rate = 44100.0;

        // default fs
        lpf.set_sample_rate(default_sample_rate);
        let speaker = Speaker {
            value: value.clone(),
            last_cycle: AtomicU64::new(0),
            tx,
            buf: Mutex::new(SharedRb::<Heap<f32>>::new(65536)),
            last_touched: Mutex::new(Instant::now()),
            lpf: Mutex::new(lpf),
            state: Mutex::new(State {
                rx: Some(rx),
                // prod,
                fractional_amount: 0.0,
                fractional_value: 0.0,
                current_value: false,
                last_cycle: clock_source.elapsed(),
                sample_rate: default_sample_rate,
            }),
            clock: clock_source,
        };
        Ok(speaker)
    }

    pub fn set_sample_rate(&self, rate: u32) {
        self.state.lock().unwrap().sample_rate = rate as f64;
        let mut lpf = self.lpf.lock().unwrap();
        lpf.set_sample_rate(rate as f64);
    }
}

impl Display for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Speaker")
    }
}

impl Slave for Speaker {
    fn read(&self, _address: usize) -> Result<u8, crate::errors::Error> {
        self.toggle()?;
        Ok(0)
    }

    fn write(&self, _address: usize, _data: u8) -> Result<(), crate::errors::Error> {
        self.toggle()
    }
}
