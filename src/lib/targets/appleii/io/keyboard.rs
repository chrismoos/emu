use eframe::egui::Key;
use std::{
    collections::HashSet,
    fmt::Display,
    sync::{Arc, RwLock, atomic::AtomicBool},
};

use eframe::egui::{Event, InputState};

use crate::{
    cpu::mos6502::bus::Slave,
    targets::appleii::io::{
        misc::{GAME_SWITCH_APPLE_KEY, MiscIo},
        soft_switches::SoftSwitches,
    },
};

pub struct Keyboard {
    key: RwLock<u8>,
    pressed_keys: RwLock<Vec<u8>>,
    caps_lock_enabled: AtomicBool,
    soft_switches: Arc<SoftSwitches>,
    misc_io: Arc<MiscIo>,
}

impl Keyboard {
    pub fn new(soft_switches: Arc<SoftSwitches>, misc_io: Arc<MiscIo>) -> Keyboard {
        Keyboard {
            key: RwLock::new(0),
            misc_io,
            caps_lock_enabled: AtomicBool::new(true),
            pressed_keys: RwLock::new(vec![]),
            soft_switches,
        }
    }

    fn alpha_modifier(key_code: u8, caps_lock: bool) -> (u8, u8, u8, u8) {
        (
            if caps_lock { key_code } else { key_code + 0x20 },
            key_code - 0x40,
            key_code,
            key_code - 0x40,
        )
    }

    fn paste_clipbard_char(&self, c: char) {
        let mut val = c as u8;
        if val < 128 {
            if val == 0x0a {
                val = 0x0d;
            }
            self.push_next_key(val | 0x80);
        }
    }

    // TODO - finish making sure all keys work
    pub fn process_keys(&self, input_state: &mut InputState) {
        if let Some(data) = input_state
            .events
            .iter()
            .flat_map(|evt| match evt {
                Event::Paste(data) => vec![data.to_owned()],
                _ => vec![],
            })
            .next()
        {
            data.chars().for_each(|c| self.paste_clipbard_char(c));
        }

        self.misc_io
            .update_game_switch(GAME_SWITCH_APPLE_KEY, input_state.modifiers.alt);

        let caps_lock = self
            .caps_lock_enabled
            .load(std::sync::atomic::Ordering::Acquire);

        #[allow(unused_mut)]
        let mut keys_handled = HashSet::<u8>::new();
        // This handles an issue on web targets where we don't get key events for most
        // keys with modifiers (shift, etc.)
        #[cfg(target_arch = "wasm32")]
        {
            for evt in &input_state.events {
                if let Event::Text(txt) = evt {
                    let chars = txt.chars().collect::<Vec<_>>();
                    if chars.len() == 1 {
                        let chr = chars[0] as u8;
                        let mut key = 0x80 | chr;
                        if caps_lock && chr >= 0x61 && chr <= 0x7a {
                            key -= 0x20;
                        }
                        keys_handled.insert(key);
                        self.push_next_key(key);
                    }
                }
            }
        }

        let keys = input_state
            .keys_down
            .iter()
            .map(|k| k.clone())
            .collect::<Vec<_>>();

        for key in keys {
            if !input_state.consume_key(input_state.modifiers, key) {
                continue;
            }

            let (alone, ctrl, shift, both) = match key {
                Key::A => Self::alpha_modifier(0xc1, caps_lock),
                Key::B => Self::alpha_modifier(0xc2, caps_lock),
                Key::C => Self::alpha_modifier(0xc3, caps_lock),
                Key::D => Self::alpha_modifier(0xc4, caps_lock),
                Key::E => Self::alpha_modifier(0xc5, caps_lock),
                Key::F => Self::alpha_modifier(0xc6, caps_lock),
                Key::G => Self::alpha_modifier(0xc7, caps_lock),
                Key::H => Self::alpha_modifier(0xc8, caps_lock),
                Key::I => Self::alpha_modifier(0xc9, caps_lock),
                Key::J => Self::alpha_modifier(0xca, caps_lock),
                Key::K => Self::alpha_modifier(0xcb, caps_lock),
                Key::L => Self::alpha_modifier(0xcc, caps_lock),
                Key::M => Self::alpha_modifier(0xcd, caps_lock),
                Key::N => Self::alpha_modifier(0xce, caps_lock),
                Key::O => Self::alpha_modifier(0xcf, caps_lock),
                Key::P => Self::alpha_modifier(0xd0, caps_lock),
                Key::Q => Self::alpha_modifier(0xd1, caps_lock),
                Key::R => Self::alpha_modifier(0xd2, caps_lock),
                Key::S => Self::alpha_modifier(0xd3, caps_lock),
                Key::T => Self::alpha_modifier(0xd4, caps_lock),
                Key::U => Self::alpha_modifier(0xd5, caps_lock),
                Key::V => Self::alpha_modifier(0xd6, caps_lock),
                Key::W => Self::alpha_modifier(0xd7, caps_lock),
                Key::X => Self::alpha_modifier(0xd8, caps_lock),
                Key::Y => Self::alpha_modifier(0xd9, caps_lock),
                Key::Z => Self::alpha_modifier(0xda, caps_lock),
                _ => match key {
                    Key::Escape => (0x9b, 0x9b, 0x9b, 0x9b),
                    Key::Enter => (0x8d, 0x8d, 0x8d, 0x8d),
                    Key::ArrowDown => (0x8a, 0x8a, 0x8a, 0x8a),
                    Key::ArrowUp => (0x8b, 0x8b, 0x8b, 0x8b),
                    Key::ArrowRight => (0x95, 0x95, 0x95, 0x95),
                    Key::ArrowLeft => (0x88, 0x88, 0x88, 0x88),
                    Key::Space => (0xa0, 0xa0, 0xa0, 0xa0),
                    Key::Num0 => (0xb0, 0xb0, 0xa9, 0xa9),
                    Key::Num1 => (0xb1, 0xb1, 0xa1, 0xa1),
                    Key::Num2 => (0xb2, 0xb2, 0xc0, 0xc0),
                    Key::Num3 => (0xb3, 0xb3, 0xa3, 0xa3),
                    Key::Num4 => (0xb4, 0xb4, 0xa4, 0xa4),
                    Key::Num5 => (0xb5, 0xb5, 0xa5, 0xa5),
                    Key::Num6 => (0xb6, 0xb6, 0xa6, 0xa6),
                    Key::Num7 => (0xb7, 0xb7, 0xa6, 0xa6),
                    Key::Num8 => (0xb8, 0xb8, 0xaa, 0xaa),
                    Key::Num9 => (0xb9, 0xb9, 0xa8, 0xa8),
                    Key::Semicolon => (0xbb, 0xbb, 0xab, 0xab),
                    Key::Comma => (0xac, 0xac, 0xbc, 0xbc),
                    Key::Colon => (0xba, 0xba, 0xba, 0xba),
                    Key::Equals => (0xbd, 0xbd, 0xbd, 0xbd),
                    Key::Minus => (0xad, 0xad, 0xdf, 0xad),
                    Key::Quote if input_state.modifiers.shift => (0xa2, 0xa2, 0xa2, 0xa2),
                    Key::Quote => (0xa7, 0xa7, 0xa7, 0xa7),
                    Key::Period => (0xae, 0xae, 0xbe, 0xbe),
                    Key::Questionmark => (0xbf, 0xbf, 0xbf, 0xbf),
                    Key::Plus => (0xab, 0xab, 0xab, 0xab),
                    Key::Slash => (0xaf, 0xaf, 0xbf, 0xbf),
                    Key::Exclamationmark => (0xa1, 0xa1, 0xa1, 0xa1),
                    Key::Tab => (0x89, 0x89, 0x89, 0x89),
                    Key::Delete => (0xff, 0xff, 0xff, 0xff),
                    _ => (0x00, 0x00, 0x00, 0x00),
                },
            };

            if alone != 0 {
                let key = match (input_state.modifiers.ctrl, input_state.modifiers.shift) {
                    (true, true) => both,
                    (true, false) => ctrl,
                    (false, true) => shift,
                    (false, false) => alone,
                };

                if !keys_handled.contains(&key) {
                    self.push_next_key(key);
                }
            }
        }
    }

    pub fn set_caps_lock_enabled(&self, enabled: bool) {
        self.caps_lock_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn caps_lock_enabled(&self) -> bool {
        self.caps_lock_enabled
            .load(std::sync::atomic::Ordering::Acquire)
    }

    fn push_next_key(&self, key: u8) {
        let mut keys = self.pressed_keys.write().unwrap();
        let mut current_key = self.key.write().unwrap();

        if *current_key & 128 == 0 {
            *current_key = key;
        } else {
            keys.push(key);
        }
    }

    fn clear_strobe(&self) {
        let mut keys = self.pressed_keys.write().unwrap();
        let mut key = self.key.write().unwrap();
        *key = *key & 127;

        if keys.len() > 0 {
            if let Some(next) = keys.drain(0..1).next() {
                *key = next;
            }
        }
    }
}

impl Display for Keyboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("keyboard")
    }
}

impl Slave for Keyboard {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        match address {
            0 => Ok(*self.key.read().unwrap()),
            0x10 => {
                self.clear_strobe();
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    fn write(&self, address: usize, _data: u8) -> Result<(), crate::errors::Error> {
        match address {
            0 => {
                self.soft_switches.set_eightystore(false);
            }
            0x10 => {
                self.clear_strobe();
            }
            _ => {}
        }
        Ok(())
    }
}
