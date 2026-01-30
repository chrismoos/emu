use crate::cpu::mos6502::bus::Slave;
use crate::errors::Error;
use log::{error, trace};
use paste::{item, paste};
use std::fmt::Formatter;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, PartialEq, Eq)]
enum AccessType {
    Read,
    Write,
    Both,
}

pub trait SoftSwitchListener: Send + Sync {
    fn on_updated(&mut self, switch: Switch, previous_value: bool, new_value: bool);
}

macro_rules! soft_switches {
    (
        $(($field:ident,$off:expr,$on:expr,$read:expr,$access:expr)),*
    ) => {


        #[derive(Default)]
        pub struct SoftSwitches {
            // TODO - Remove locking
            listeners: std::sync::Mutex<Vec<Box<dyn SoftSwitchListener>>>,
            $(
                $field: AtomicBool,
            )*
        }

        impl std::fmt::Debug for SoftSwitches {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut s = f.debug_struct("SoftSwitches");
                paste! {
                    $(
                        s.field(stringify!($field), &self.[<$field>]);
                    )*
                }
                s.finish()
            }
        }

        paste! {
            #[derive(Debug, PartialEq, Eq)]
            pub enum Switch {
                $(
                    [<$field:camel>],
                )*
            }
        }

        impl SoftSwitches {
            pub fn new() -> SoftSwitches {
                let s = Self::default();
                s.reset();
                s
            }

            pub fn add_listener(&self, f: Box<dyn SoftSwitchListener>) {
                self.listeners.lock().unwrap().push(f);
            }

            pub fn reset(&self) {
                $(
                    self.$field.store(false, Ordering::SeqCst);
                )*

                self.set_ioudis(true);
            }

            $(
                pub fn $field(&self) -> bool {
                    self.$field.load(Ordering::Acquire)
                }

                paste! {
                    pub fn [<set_ $field>](&self, value: bool) {
                        if value {
                            //trace!("{} ON", stringify!($field));
                        }
                        else {
                            //trace!("{} OFF", stringify!($field));
                        }
                        self.$field.store(value, Ordering::SeqCst);
                    }
                }
            )*

            fn update_switches(&self, address: usize, write: bool) -> Result<u8, Error> {
                item! {
                    match address {
                        $(
                            $on if $on != 0 => {
                                if ((($access == AccessType::Both) || ($access == AccessType::Write && write)) || ($access == AccessType::Read && !write)) {
                                    let previous = self.$field.swap(true, Ordering::SeqCst);
                                    self.listeners.lock().unwrap().iter_mut().for_each(|l| l.on_updated(Switch::[<$field:camel>], previous, true));
                                    trace!("{} ON", stringify!($field));
                                    return Ok(1 << 7);
                                }
                            }
                            $off if $off != 0 => {
                                if ((($access == AccessType::Both) || ($access == AccessType::Write && write)) || ($access == AccessType::Read && !write)) {
                                    let previous = self.$field.swap(false, Ordering::SeqCst);
                                    trace!("{} OFF", stringify!($field));
                                    self.listeners.lock().unwrap().iter_mut().for_each(|l| l.on_updated(Switch::[<$field:camel>], previous, false));
                                }
                            },
                            val if Some(val) == $read => {
                                trace!("{} READ", stringify!($field));
                                return Ok(if self.$field.load(Ordering::Acquire) { 1 << 7 } else { 0 });
                            },
                        )*
                        _ => {
                            error!("unknown soft switch access 0x{:x}", address);
                        }
                    };
                }
                Ok(0)
            }
        }

        impl std::fmt::Display for SoftSwitches {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_fmt(format_args!("{:?}", self))
            }
        }

        impl Slave for SoftSwitches {
            fn read(&self, address: usize) -> Result<u8, Error> {
                Ok(self.update_switches(address, false)?)
            }

            fn write(&self, address: usize, _data: u8) -> Result<(), Error> {
                self.update_switches(address, true)?;
                Ok(())
            }
        }
    };
}

soft_switches!(
    (eightystore, 0xc000, 0xc001, Some(0xc018), AccessType::Write),
    (altzp, 0xc008, 0xc009, Some(0xc016), AccessType::Write),
    (eightycol, 0xc00c, 0xc00d, Some(0xc01f), AccessType::Write),
    (altchar, 0xc00e, 0xc00f, Some(0xc01e), AccessType::Write),
    (ramrd, 0xc002, 0xc003, Some(0xc013), AccessType::Write),
    (ramwrt, 0xc004, 0xc005, Some(0xc014), AccessType::Write),
    (text_mode, 0xc050, 0xc051, Some(0xc01a), AccessType::Both),
    (mix_mode, 0xc052, 0xc053, Some(0xc01b), AccessType::Both),
    (page_two, 0xc054, 0xc055, Some(0xc01c), AccessType::Both),
    (hires_mode, 0xc056, 0xc057, Some(0xc01d), AccessType::Both),
    (an0, 0xc058, 0xc059, None, AccessType::Both),
    (an1, 0xc05a, 0xc05b, None, AccessType::Both),
    (an2, 0xc05c, 0xc05d, None, AccessType::Both),
    (an3, 0xc05e, 0xc05f, None, AccessType::Both),
    (ioudis, 0xc07e, 0xc07f, Some(0xc07e), AccessType::Both),
    (c3rom_slot, 0xc00a, 0xc00b, Some(0xc017), AccessType::Write),
    (intcxrom, 0xc006, 0xc007, Some(0xc015), AccessType::Write),
    (rdlcram, 0, 0, Some(0xc012), AccessType::Both),
    (rdlbnk2, 0, 0, Some(0xc011), AccessType::Both),
    (vbl, 0, 0, Some(0xc019), AccessType::Both),
    (iicrom, 0, 0xc028, None, AccessType::Both)
);
