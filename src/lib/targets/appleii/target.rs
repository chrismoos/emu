use std::{
    collections::HashSet,
    fmt::Display,
    fs::File,
    io::{Read, Seek},
    pin::Pin,
    sync::{Arc, RwLock},
    time::Duration,
};

use eframe::egui::{Button, Color32, ComboBox, Ui};
use log::error;

use crate::{
    clock::Clock,
    cpu::{
        InterruptConnection, InterruptSource,
        mos6502::{
            self, ClockConfig, Mos6502,
            bus::{self, Interceptor, memory::MemoryBank},
        },
    },
    debug::{self, Debuggable},
    emulator::{
        AudioConfig, EmulatorProgramReader, EmulatorTarget, EmulatorTargetFactory, EmulatorTargetType, ResetType, Stats
    },
    errors::Error,
    peripherals::serial::{SerialDevice, echo::EchoSerialPort},
    targets::{
        ExecutionState,
        appleii::{
            args::{
                self, AppleIIArgs, AppleIICArgs, AppleIIEArgs, AppleIIEEnhancedArgs, RunArgs,
                Variant,
            },
            io::{
                disks::{
                    FloppyDiskReader,
                    dsk::{DskFile, SystemType},
                    woz::{WozDisk, WozDiskReader},
                },
                keyboard::Keyboard,
                memory::iic_bank::AppleIICBankSwitcher,
                misc::MiscIo,
                peripheral::PeripheralManager,
                peripherals::{
                    diskii::DiskIIPeripheral, extended_text::Extended80ColumnText,
                    language::LanguageCard, mouse::MouseCard, scratchpad::ScratchpadRam,
                    smartport::Smartport, superserial::SuperSerialCard,
                },
                soft_switches::{SoftSwitchListener, SoftSwitches, Switch},
                speaker::Speaker,
                video::{
                    MonitorType, Video, character_rom::CharacterROM, vbl::VblTickListener,
                    video7::Video7Listener,
                },
            },
        },
    },
    utils::time::sleep,
};
use log::debug;

pub const BUS_ROM_F8_BANK_START_ADDRESS: usize = 0xf800;
pub const BUS_RAM_START_ADDRESS: usize = 0x0;
pub const BUS_RAM_SIZE: usize = 48 * 1024;

const INTERRUPT_SOURCE_MOUSE: InterruptSource = 100;

const DISK_SLOT: usize = 6;
const SMARTPORT_SLOT: usize = 5;

pub struct Target {
    cpu: Arc<Mos6502>,
    video: Arc<Video>,
    keyboard: Arc<Keyboard>,
    ram: Arc<MemoryBank>,
    language_card: Arc<LanguageCard>,
    extended_text: Option<Arc<Extended80ColumnText>>,
    peripheral_manager: Arc<PeripheralManager>,
    diskii: Arc<DiskIIPeripheral>,
    misc_io: Arc<MiscIo>,
    mouse: Arc<MouseCard>,
    speaker: Arc<Speaker>,
    soft_switches: Arc<SoftSwitches>,
    memory_breakpoint_listener: Arc<MemoryBreakpointListener>,
    target_type_id: String,
    smartport: Arc<Smartport>,
}

impl Target {
    fn load_rom_file(path: &str) -> Result<Arc<MemoryBank>, Error> {
        Ok(Arc::new(MemoryBank::new_with_data(
            &std::fs::read(path)?,
            true,
        )))
    }

    fn load_rom_file_or_default(
        path: &Option<String>,
        data: &[u8],
    ) -> Result<Arc<MemoryBank>, Error> {
        Ok(path
            .as_ref()
            .map(|f| Self::load_rom_file(f))
            .transpose()?
            .unwrap_or_else(|| Arc::new(MemoryBank::new_with_data(data, true))))
    }

    pub fn new(args: RunArgs) -> Result<Target, Error> {
        let bus = Arc::new(bus::Bus::new());
        let cpu_variant = match &args.variant {
            crate::targets::appleii::args::Variant::AppleIIEEnhanced(_apple_iieenhanced_args) => {
                mos6502::Variant::Wdc65c02
            }
            crate::targets::appleii::args::Variant::AppleIIC(_) => mos6502::Variant::Wdc65c02,
            _ => mos6502::Variant::Original,
        };

        let cpu = Arc::new(mos6502::Mos6502::new(
            bus.clone(),
            ClockConfig::new(1022727, 1.0),
            cpu_variant,
        ));

        let memory_breakpoint_listener = Arc::new(MemoryBreakpointListener {
            cpu: cpu.clone(),
            breakpoints: RwLock::new(HashSet::new()),
        });
        bus.add_interceptor(memory_breakpoint_listener.clone());

        let misc_io = Arc::new(MiscIo::new());
        bus.connect(0xc060, 32, misc_io.clone());

        let soft_switches = Arc::new(SoftSwitches::new());
        let keyboard = Arc::new(Keyboard::new(soft_switches.clone(), misc_io.clone()));
        let speaker = Arc::new(Speaker::new(cpu.clone())?);

        bus.connect(0xc030, 1, speaker.clone());
        bus.connect(0xc000, 1, keyboard.clone());
        bus.connect_map(0xc010, 0xc000, 1, keyboard.clone());
        bus.connect_map(0xc000, 0x00, 0x60, soft_switches.clone());

        let language_card = Arc::new(LanguageCard::new(soft_switches.clone()));
        let mut extended_text = None;

        let peripheral = match args.variant {
            crate::targets::appleii::args::Variant::AppleII(ref args) => {
                bus.connect(
                    BUS_ROM_F8_BANK_START_ADDRESS,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_f8,
                        include_bytes!("../../../../resources/appleii/rom/autostart-monitor.bin"),
                    )?,
                );

                bus.connect(
                    0xd000,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_d0,
                        include_bytes!("../../../../resources/appleii/rom/d0-applesoft-basic.bin"),
                    )?,
                );

                bus.connect(
                    0xd800,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_d8,
                        include_bytes!("../../../../resources/appleii/rom/d8-applesoft-basic.bin"),
                    )?,
                );

                bus.connect(
                    0xe000,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_e0,
                        include_bytes!("../../../../resources/appleii/rom/e0-applesoft-basic.bin"),
                    )?,
                );

                bus.connect(
                    0xe800,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_e8,
                        include_bytes!("../../../../resources/appleii/rom/e8-applesoft-basic.bin"),
                    )?,
                );

                bus.connect(
                    0xf000,
                    0x800,
                    Self::load_rom_file_or_default(
                        &args.rom_f0,
                        include_bytes!("../../../../resources/appleii/rom/f0-applesoft-basic.bin"),
                    )?,
                );

                let peripheral = Arc::new(PeripheralManager::new(soft_switches.clone(), None));
                bus.connect_map(0xc080, 0xc000, 0xcfff - 0xc080 + 1, peripheral.clone());
                peripheral
            }
            crate::targets::appleii::args::Variant::AppleIIE(ref args) => {
                let c0_rom = Self::load_rom_file_or_default(
                    &args.rom_c0,
                    include_bytes!("../../../../resources/appleiie/rom/c0-rom.bin"),
                )?;
                bus.connect_map(0xd000, 0xc000, 0x1000, c0_rom.clone());
                bus.connect(
                    0xe000,
                    0x2000,
                    Self::load_rom_file_or_default(
                        &args.rom_e0,
                        include_bytes!("../../../../resources/appleiie/rom/e0-rom.bin"),
                    )?,
                );
                let peripheral =
                    Arc::new(PeripheralManager::new(soft_switches.clone(), Some(c0_rom)));
                bus.connect_map(0xc080, 0xc000, 0xcfff - 0xc080 + 1, peripheral.clone());

                let text = Arc::new(Extended80ColumnText::new(
                    soft_switches.clone(),
                    language_card.clone(),
                ));
                bus.add_interceptor(text.clone());
                extended_text = Some(text);

                peripheral
            }
            crate::targets::appleii::args::Variant::AppleIIEEnhanced(ref args) => {
                let c0_rom = Self::load_rom_file_or_default(
                    &args.rom_c0,
                    include_bytes!("../../../../resources/appleiie-enhanced/rom/c0-rom.bin"),
                )?;
                bus.connect_map(0xd000, 0xc000, 0x1000, c0_rom.clone());
                bus.connect(
                    0xe000,
                    0x2000,
                    Self::load_rom_file_or_default(
                        &args.rom_e0,
                        include_bytes!("../../../../resources/appleiie-enhanced/rom/e0-rom.bin"),
                    )?,
                );
                let peripheral =
                    Arc::new(PeripheralManager::new(soft_switches.clone(), Some(c0_rom)));
                bus.connect_map(0xc080, 0xc000, 0xcfff - 0xc080 + 1, peripheral.clone());

                let text = Arc::new(Extended80ColumnText::new(
                    soft_switches.clone(),
                    language_card.clone(),
                ));
                bus.add_interceptor(text.clone());
                extended_text = Some(text);

                peripheral
            }
            crate::targets::appleii::args::Variant::AppleIIC(ref args) => {
                let c0_rom = Self::load_rom_file_or_default(
                    &args.rom_c0,
                    include_bytes!("../../../../resources/appleiic/rom/c0-rom-3.bin"),
                )?;

                let iic_bank_switcher =
                    Arc::new(AppleIICBankSwitcher::new(soft_switches.clone(), c0_rom));
                let peripheral = Arc::new(PeripheralManager::new(
                    soft_switches.clone(),
                    Some(iic_bank_switcher.clone()),
                ));

                bus.connect_map(0xc100, 0xc000, 0x4000 - 0x100, iic_bank_switcher.clone());
                bus.connect_map(0xc080, 0xc000, 0xcfff - 0xc080 + 1, peripheral.clone());

                soft_switches.add_listener(Box::new(IICBankSwitchListener::new(
                    iic_bank_switcher.clone(),
                )));

                let text = Arc::new(Extended80ColumnText::new(
                    soft_switches.clone(),
                    language_card.clone(),
                ));
                bus.add_interceptor(text.clone());
                extended_text = Some(text);

                peripheral
            }
        };

        // Add language card after so extended text's interceptor goes first
        peripheral.assign(0, language_card.clone())?;
        bus.add_interceptor(language_card.clone());

        #[allow(warnings)]
        let mut serial_device: Option<Arc<dyn SerialDevice>> = None;
        #[cfg(feature = "native")]
        {
            serial_device = Some(match &args.serial_port {
                Some(path) => {
                    use crate::peripherals::serial;

                    debug!("Attaching to serial port {}", path);
                    Arc::new(serial::port::SerialDevicePort::new(
                        path,
                        args.serial_force_zero_baud.unwrap_or(false),
                    ))
                }
                None => {
                    use crate::peripherals::serial::internet_modem::InternetModem;

                    debug!("Attaching internet modem to serial card");
                    Arc::new(InternetModem::new())
                }
            });
        }

        let super_serial = Arc::new(SuperSerialCard::new(
            include_bytes!("../../../../resources/appleii/rom/super-serial.bin"),
            serial_device
                .unwrap_or_else(|| Arc::new(EchoSerialPort::new()))
                .clone(),
            cpu.clone(),
        ));

        peripheral.assign(1, super_serial.clone())?;

        let ram = Arc::new(MemoryBank::new(BUS_RAM_SIZE, false));
        bus.connect(BUS_RAM_START_ADDRESS, BUS_RAM_SIZE, ram.clone());

        let mouse_slot = match &args.variant {
            args::Variant::AppleIIC(_) => 7,
            _ => 4,
        };

        let mouse_card = Arc::new(MouseCard::new(
            include_bytes!(concat!(env!("OUT_DIR"), "/appleii-mouse.bin")),
            bus.clone(),
            ScratchpadRam::new(bus.clone(), mouse_slot),
            InterruptConnection::new(cpu.clone(), INTERRUPT_SOURCE_MOUSE),
        ));
        peripheral.assign(mouse_slot, mouse_card.clone())?;

        let smartport = Arc::new(Smartport::new(
            include_bytes!(concat!(env!("OUT_DIR"), "/appleii-smartport.bin")),
            bus.clone(),
        ));

        let diskii = Arc::new(DiskIIPeripheral::new(
            cpu.clone(),
            Arc::new(MemoryBank::new_with_data(
                include_bytes!("../../../../resources/appleii/rom/diskii-16-sector.bin"),
                true,
            )),
        ));
        peripheral.assign(DISK_SLOT, diskii.clone())?;

        if let Some(program_rom) = &args.program_rom {
            let floppy_disk = Self::load_disk_file(program_rom, File::open(program_rom)?)?;
            diskii.attach(floppy_disk);
        }

        let video = match &args.variant {
            crate::targets::appleii::args::Variant::AppleII(_apple_iiargs) => {
                let rom = include_bytes!("../../../../resources/appleii/rom/character-rom.bin");
                Arc::new(Video::new(
                    soft_switches.clone(),
                    bus.clone(),
                    Box::new(CharacterROM::new(rom, false, false, false)),
                    ram.clone(),
                    None,
                    Some(mouse_card.clone()),
                ))
            }
            crate::targets::appleii::args::Variant::AppleIIE(_apple_iieargs) => {
                let rom = include_bytes!("../../../../resources/appleiie/rom/video-rom.bin");
                Arc::new(Video::new(
                    soft_switches.clone(),
                    bus.clone(),
                    Box::new(CharacterROM::new(rom, true, true, true)),
                    ram.clone(),
                    Some(
                        extended_text
                            .as_ref()
                            .expect("must have on Apple IIe")
                            .clone(),
                    ),
                    Some(mouse_card.clone()),
                ))
            }
            crate::targets::appleii::args::Variant::AppleIIEEnhanced(_args) => {
                let rom =
                    include_bytes!("../../../../resources/appleiie-enhanced/rom/video-rom.bin");
                Arc::new(Video::new(
                    soft_switches.clone(),
                    bus.clone(),
                    Box::new(CharacterROM::new(rom, true, true, true)),
                    ram.clone(),
                    Some(
                        extended_text
                            .as_ref()
                            .expect("must have on Apple IIe")
                            .clone(),
                    ),
                    Some(mouse_card.clone()),
                ))
            }
            args::Variant::AppleIIC(_args) => {
                let rom = include_bytes!("../../../../resources/appleiic/rom/video-rom.bin");
                Arc::new(Video::new(
                    soft_switches.clone(),
                    bus.clone(),
                    Box::new(CharacterROM::new(rom, true, true, true)),
                    ram.clone(),
                    Some(
                        extended_text
                            .as_ref()
                            .expect("must have on Apple IIe")
                            .clone(),
                    ),
                    Some(mouse_card.clone()),
                ))
            }
        };

        soft_switches.add_listener(Box::new(Video7Listener::new(video.clone())));

        debug!("bus: {}", bus);
        cpu.reset();

        cpu.add_tick_listener(Box::new(VblTickListener {
            soft_switches: soft_switches.clone(),
            video: video.clone(),
            mouse_card: mouse_card.clone(),
            last_vbl: 0,
            vbl: false,
        }));

        let target_type_id = match &args.variant {
            Variant::AppleII(_) => TARGET_APPLE_II,
            Variant::AppleIIE(_) => TARGET_APPLE_IIE,
            Variant::AppleIIEEnhanced(_) => TARGET_APPLE_IIE_ENHANCED,
            Variant::AppleIIC(_) => TARGET_APPLE_IIC,
        }
        .to_owned();

        Ok(Target {
            diskii,
            smartport,
            cpu,
            speaker,
            memory_breakpoint_listener,
            video,
            peripheral_manager: peripheral,
            keyboard,
            ram,
            extended_text,
            misc_io,
            mouse: mouse_card,
            language_card,
            soft_switches,
            target_type_id,
        })
    }

    fn load_disk_file<R>(
        file_name: &str,
        reader: R,
    ) -> Result<Box<dyn FloppyDiskReader + Send + Sync>, Error>
    where
        R: Read + Seek + Send + 'static,
    {
        if file_name.to_lowercase().ends_with(".woz") {
            let woz_disk = WozDisk::parse(reader)?;
            Ok(Box::new(WozDiskReader::new(woz_disk)))
        } else if file_name.to_lowercase().ends_with("dsk") {
            let woz_disk = WozDisk::try_from(DskFile::new_16_sector(reader, SystemType::Dos))?;
            Ok(Box::new(WozDiskReader::new(woz_disk)))
        } else if file_name.to_lowercase().ends_with("po") {
            let woz_disk = WozDisk::try_from(DskFile::new_16_sector(reader, SystemType::ProDos))?;
            Ok(Box::new(WozDiskReader::new(woz_disk)))
        } else {
            Err("unsupported program file type".into())
        }
    }
}

impl Debuggable for Target {
    fn run<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move { self.cpu.run().await })
    }

    fn get_state(&self) -> Result<crate::debug::State, Error> {
        self.cpu.get_state()
    }

    fn step(&self) -> Result<(), Error> {
        self.cpu.step()
    }

    fn add_breakpoint(&self, address: usize) -> Result<(), Error> {
        self.cpu.add_breakpoint(address)
    }

    fn list_breakpoints(&self) -> Result<Vec<usize>, Error> {
        self.cpu.list_breakpoints()
    }

    fn delete_breakpoints(&self, address: usize) -> Result<(), Error> {
        self.cpu.delete_breakpoints(address)
    }

    fn halt(&self) -> Result<(), Error> {
        self.cpu.halt()
    }

    fn get_instructions(
        &self,
        address: usize,
        num: usize,
    ) -> Result<Vec<debug::InstructionInfo>, Error> {
        self.cpu.get_instructions(address, num)
    }

    fn read_memory(&self, address: usize, num: usize) -> Result<Vec<u8>, Error> {
        self.cpu.read_memory(address, num)
    }

    fn backtrace(&self) -> Result<Vec<usize>, Error> {
        self.cpu.backtrace()
    }

    fn add_memory_breakpoint(&self, address: usize) -> Result<(), Error> {
        self.memory_breakpoint_listener
            .breakpoints
            .write()
            .unwrap()
            .insert(address);
        Ok(())
    }

    fn list_memory_breakpoints(&self) -> Result<Vec<usize>, Error> {
        Ok(self
            .memory_breakpoint_listener
            .breakpoints
            .read()
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>())
    }

    fn delete_memory_breakpoint(&self, address: usize) -> Result<(), Error> {
        self.memory_breakpoint_listener
            .breakpoints
            .write()
            .unwrap()
            .remove(&address);
        Ok(())
    }
}

impl EmulatorTarget for Target {
    fn start<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move { self.cpu.clone().start().await })
    }

    fn update_display(&self, ui: &mut Ui, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(Button::new("SW1").fill(Color32::RED))
                    .is_pointer_button_down_on()
                {
                    self.misc_io.update_game_switch(1, true);
                } else {
                    self.misc_io.update_game_switch(1, false);
                }
                if ui
                    .add(Button::new("SW2").fill(Color32::RED))
                    .is_pointer_button_down_on()
                {
                    self.misc_io.update_game_switch(2, false);
                } else {
                    self.misc_io.update_game_switch(2, true);
                }

                let caps_lock_enabled = self.keyboard.caps_lock_enabled();
                if ui
                    .add(Button::new("Caps Lock").fill(if caps_lock_enabled {
                        Color32::RED
                    } else {
                        Color32::BLACK
                    }))
                    .clicked()
                {
                    self.keyboard.set_caps_lock_enabled(!caps_lock_enabled);
                }

                let mut selected = self.video.get_monitor_type();
                let before = selected;
                ComboBox::from_id_salt("monitor")
                    .selected_text("Monitor")
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut selected,
                            MonitorType::Color,
                            "Standard (VGA Monitor)",
                        );
                        ui.selectable_value(&mut selected, MonitorType::Monochrome, "Monochrome");
                        ui.selectable_value(&mut selected, MonitorType::Green, "Green");
                        ui.selectable_value(&mut selected, MonitorType::Amber, "Amber");
                    });
                if selected != before {
                    self.video.set_monitor_type(selected);
                }

                let mut speed = self.cpu.get_execution_speed();
                let cpu_speed_before = speed;
                ComboBox::from_id_salt("cpu_speed")
                    .selected_text(format!("CPU Speed: {:.1}", speed))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut speed, 1.0, "1.0");
                        ui.selectable_value(&mut speed, 1.5, "1.5");
                        ui.selectable_value(&mut speed, 2.0, "2.0");
                        ui.selectable_value(&mut speed, 4.0, "4.0");
                        ui.selectable_value(&mut speed, 8.0, "8.0");
                    });
                if speed != cpu_speed_before {
                    self.cpu.set_execution_speed(speed);
                }
            });

            ui.vertical_centered(|ui| match self.video.update_display(ctx, ui) {
                Err(e) => error!("failed to update ui: {:?}", e),
                Ok(resp) => {
                    ui.input_mut(|i| {
                        self.mouse.process_mouse(i, &resp.rect);

                        /*if resp
                            .rect
                            .contains(i.pointer.latest_pos().unwrap_or_default())
                        {*/
                        self.keyboard.process_keys(i);
                        //}
                    });
                }
            });
        });
    }

    fn reset<'a>(
        &'a self,
        reset_type: ResetType,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            self.cpu.halt()?;

            while self.cpu.get_state()?.execution_state != ExecutionState::Stopped {
                sleep(Duration::from_millis(10)).await;
            }

            self.cpu.reset();

            if reset_type == ResetType::Cold {
                self.language_card.reset();
                self.video.reset();
                self.ram.clear();
                self.soft_switches.reset();
                self.peripheral_manager.reset();

                if let Some(text) = &self.extended_text {
                    text.reset();
                }
            }

            self.cpu.reset();
            self.cpu.run().await?;
            Ok(())
        })
    }

    fn stats(&self) -> crate::emulator::Stats {
        Stats {
            fps: Some(self.video.get_fps()),
            cpu_actual_freq: self.cpu.get_actual_freq(),
        }
    }

    fn load_program(
        &self,
        file_name: &str,
        reader: Box<dyn EmulatorProgramReader>,
    ) -> Result<(), Error> {
        if file_name.to_lowercase().ends_with(".hdv") {
            self.smartport.attach(reader)?;

            if !self.peripheral_manager.is_assigned(SMARTPORT_SLOT) {
                self.peripheral_manager
                    .assign(SMARTPORT_SLOT, self.smartport.clone())?;
            }
        } else {
            let disk = Self::load_disk_file(file_name, reader)?;
            self.diskii.attach(disk);
        }
        Ok(())
    }

    fn fill_audio_buffer(&self, data: &mut [f32], channels: usize) {
        self.speaker.fill_audio_buffer(data, channels);
    }

    fn configure_audio(&self, audio_config: &AudioConfig) {
        self.speaker.set_sample_rate(audio_config.sample_rate);
    }

    fn unload_program(&self) -> Result<(), Error> {
        self.diskii.detach();
        self.smartport.detach();
        Ok(())
    }

    fn stop<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move { self.cpu.stop() })
    }

    fn target_type_id(&self) -> &str {
        &self.target_type_id
    }
}

struct MemoryBreakpointListener {
    cpu: Arc<Mos6502>,
    breakpoints: RwLock<HashSet<usize>>,
}

impl Display for MemoryBreakpointListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Memory Breakpoint Listener")
    }
}

impl Interceptor for MemoryBreakpointListener {
    fn read(&self, address: usize) -> Result<Option<u8>, Error> {
        if self.breakpoints.read().unwrap().contains(&address) {
            println!("Memory read breakpoint hit @ 0x{:08x}", address);
            self.cpu.force_break()?;
        }
        Ok(None)
    }

    fn write(&self, address: usize, _data: u8) -> Result<Option<()>, Error> {
        if self.breakpoints.read().unwrap().contains(&address) {
            let cpu = self.cpu.clone();
            println!("Memory write breakpoint hit @ 0x{:08x}", address);
            cpu.halt()?;
        }
        Ok(None)
    }
}

struct IICBankSwitchListener {
    switcher: Arc<AppleIICBankSwitcher>,
}

impl IICBankSwitchListener {
    pub fn new(switcher: Arc<AppleIICBankSwitcher>) -> IICBankSwitchListener {
        IICBankSwitchListener { switcher }
    }
}

impl SoftSwitchListener for IICBankSwitchListener {
    fn on_updated(&mut self, switch: Switch, _previous_value: bool, _new_value: bool) {
        if switch == Switch::Iicrom {
            self.switcher.toggle_bank();
        }
    }
}

const TARGET_APPLE_II: &str = "apple-ii";
const TARGET_APPLE_IIE: &str = "apple-iie";
const TARGET_APPLE_IIE_ENHANCED: &str = "apple-iie-enhanced";
const TARGET_APPLE_IIC: &str = "apple-iic";

const TARGET_TYPES: &[EmulatorTargetType] = &[
    EmulatorTargetType {
        name: "Apple II",
        id: TARGET_APPLE_II,
    },
    EmulatorTargetType {
        name: "Apple IIe",
        id: TARGET_APPLE_IIE,
    },
    EmulatorTargetType {
        name: "Apple IIe Enhanced",
        id: TARGET_APPLE_IIE_ENHANCED,
    },
    // TODO - WIP
    /*EmulatorTargetType {
        name: "Apple IIc",
        id: TARGET_APPLE_IIC,
    },*/
];

#[derive(Default)]
pub struct TargetFactory {}

impl TargetFactory {
    pub const fn new() -> TargetFactory {
        TargetFactory {}
    }
}

impl EmulatorTargetFactory for TargetFactory {
    fn name(&self) -> &str {
        "Apple II"
    }

    fn get_types(&self) -> &[crate::emulator::EmulatorTargetType] {
        TARGET_TYPES
    }

    fn create(&self, id: &str) -> Option<Arc<dyn EmulatorTarget>> {
        match id {
            TARGET_APPLE_II => Some(Variant::AppleII(AppleIIArgs::default())),
            TARGET_APPLE_IIC => Some(Variant::AppleIIC(AppleIICArgs::default())),
            TARGET_APPLE_IIE => Some(Variant::AppleIIE(AppleIIEArgs::default())),
            TARGET_APPLE_IIE_ENHANCED => {
                Some(Variant::AppleIIEEnhanced(AppleIIEEnhancedArgs::default()))
            }
            _ => None,
        }
        .map(|variant| {
            let ra = RunArgs {
                variant,
                ..Default::default()
            };
            let v: Arc<dyn EmulatorTarget> = Arc::new(Target::new(ra).unwrap());
            v
        })
    }
}
