pub mod character_rom;
pub mod signetics;
pub mod vbl;
pub mod video7;

use std::{
    fmt::Display,
    sync::{Arc, RwLock},
    time::Duration,
};

use eframe::egui::{Color32, ColorImage, Context, Image, Response, Ui, load::SizedTexture, vec2};

use crate::{
    cpu::mos6502::bus::{Bus, Slave},
    errors::Error,
    targets::appleii::io::{peripherals::mouse::MouseCard, soft_switches::SoftSwitches},
    utils::time::Instant,
};

const SCREEN_WIDTH: usize = 280 * 2;
const SCREEN_HEIGHT: usize = 192 * 2;
const MONITOR_GREEN: Color32 = Color32::from_rgb(51, 255, 51);
const MONITOR_AMBER: Color32 = Color32::from_rgb(0xFF, 0xB7, 0x00);

pub trait CharacterGenerator {
    fn get_character(&self, code: u8, alternate: bool) -> &[u8];
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MonitorType {
    Color,
    Monochrome,
    Green,
    Amber,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum VideoMode {
    Implied,
    Video7BlackWhite,
    Video716Color140,
    Video716Color160,
    Video7Mix,
}

struct State {
    fps: f32,
    last_draw: Instant,
    blink_state: BlinkState,
    video_mode: VideoMode,
    monitor_type: MonitorType,
    frame: Vec<Color32>,
}

pub struct Video {
    soft_switches: Arc<SoftSwitches>,
    character_generator: Box<dyn CharacterGenerator + Send + Sync>,
    main_ram: Arc<dyn Slave + Send + Sync>,
    text_card: Option<Arc<dyn Slave + Send + Sync>>,
    state: RwLock<State>,
}

struct BlinkState {
    inverted: bool,
    last_blink: Instant,
}

impl Video {
    pub fn new(
        soft_switches: Arc<SoftSwitches>,
        _bus: Arc<Bus>,
        character_generator: Box<dyn CharacterGenerator + Send + Sync>,
        main_ram: Arc<dyn Slave + Send + Sync>,
        text_card: Option<Arc<dyn Slave + Send + Sync>>,
        _mouse_card: Option<Arc<MouseCard>>,
    ) -> Video {
        Video {
            soft_switches,
            character_generator,
            text_card,
            main_ram,
            state: RwLock::new(State {
                fps: 0.0,
                last_draw: Instant::now(),
                video_mode: VideoMode::Implied,
                monitor_type: MonitorType::Color,
                frame: vec![Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT],
                blink_state: BlinkState {
                    inverted: false,
                    last_blink: Instant::now(),
                },
            }),
        }
    }

    pub fn reset(&self) {}

    pub fn set_monitor_type(&self, monitor: MonitorType) {
        self.state.write().unwrap().monitor_type = monitor;
    }

    pub fn get_monitor_type(&self) -> MonitorType {
        self.state.read().unwrap().monitor_type
    }

    pub fn set_video_mode(&self, mode: VideoMode) {
        self.state.write().unwrap().video_mode = mode;
    }

    pub fn get_video_mode(&self) -> VideoMode {
        self.state.read().unwrap().video_mode
    }

    pub fn get_fps(&self) -> f32 {
        self.state.read().unwrap().fps
    }

    fn get_color(&self, code: u8) -> Color32 {
        match code {
            0 => Color32::BLACK,
            1 => Color32::from_rgb(0x8a, 0x21, 0x40),
            2 => Color32::from_rgb(0x3c, 0x22, 0xa5),
            3 => Color32::from_rgb(0xc8, 0x47, 0xe4),
            4 => Color32::from_rgb(0x07, 0x65, 0x3e),
            5 => Color32::from_rgb(0x7b, 0x7e, 0x80),
            6 => Color32::from_rgb(0x30, 0x8f, 0xe3),
            7 => Color32::from_rgb(0xb9, 0xa9, 0xfd),
            8 => Color32::from_rgb(0x3b, 0x51, 0x07),
            9 => Color32::from_rgb(0xc7, 0x70, 0x28),
            10 => Color32::from_rgb(0x7b, 0x7e, 0x80),
            11 => Color32::from_rgb(0xf3, 0x9a, 0xc2),
            12 => Color32::from_rgb(0x2f, 0xb8, 0x1f),
            13 => Color32::from_rgb(0xb9, 0xd0, 0x60),
            14 => Color32::from_rgb(0x6e, 0xe1, 0xc0),
            15 => Color32::WHITE,
            _ => Color32::BLACK,
        }
    }

    #[allow(warnings)]
    fn get_text_character_row(
        &self,
        code: u8,
        blink_state_inverted: bool,
        row: usize,
    ) -> [Color32; 7] {
        let char_inverted = (code >> 6) == 0b00;
        let mut flashing = false;
        if !self.soft_switches.altchar() {
            flashing = code >> 6 == 1;
        }

        let character = self
            .character_generator
            .get_character(code, self.soft_switches.altchar());

        [
            (character[row] >> 6) & 1,
            (character[row] >> 5) & 1,
            (character[row] >> 4) & 1,
            (character[row] >> 3) & 1,
            (character[row] >> 2) & 1,
            (character[row] >> 1) & 1,
            character[row] & 1,
        ]
        .map(|on| {
            let mut white = on == 1;
            if flashing && blink_state_inverted {
                white = !white;
            }
            if white {
                Color32::from_rgb(255, 255, 255)
            } else {
                Color32::from_rgb(0, 0, 0)
            }
        })
    }

    fn get_double_hires_color(value: u8) -> Color32 {
        let val = value.reverse_bits() >> 4;
        match val {
            0 => Color32::BLACK,
            0b0001 => Color32::from_rgb(0xdd, 0x00, 0x33), // magenta
            0b0010 => Color32::from_rgb(0x88, 0x55, 0x00), // brown
            0b0011 => Color32::from_rgb(0xff, 0x66, 0x00), // orange
            0b0100 => Color32::from_rgb(0x00, 0x77, 0x22), // dark green
            0b0101 => Color32::from_rgb(0xaa, 0xaa, 0xaa), // gray 1
            0b0110 => Color32::from_rgb(0x00, 0x77, 0x22), // green
            0b0111 => Color32::from_rgb(0xff, 0xff, 0x00), // yellow
            0b1000 => Color32::from_rgb(0x00, 0x00, 0x99), // dark blue
            0b1001 => Color32::from_rgb(0xdd, 0x22, 0xdd), // purple
            0b1010 => Color32::from_rgb(0x55, 0x55, 0x55), // gray 2
            0b1011 => Color32::from_rgb(0xff, 0x99, 0x88), // pink
            0b1100 => Color32::from_rgb(0x22, 0x22, 0xff), // medium blue
            0b1101 => Color32::from_rgb(0x66, 0xaa, 0xff), // light blue
            0b1110 => Color32::from_rgb(0x44, 0xff, 0x99), // aqua
            0b1111 => Color32::WHITE,
            _ => Color32::BLACK,
        }
    }

    fn fill_video7_black_white_row(&self, row_addr: u16, row: &mut [Color32]) {
        let mut index = 0;
        for (offset, _) in (0..40).enumerate() {
            let aux_byte = self
                .text_card
                .as_ref()
                .map(|c| c.read((row_addr + offset as u16) as usize).unwrap_or(0))
                .unwrap_or(0);

            let main_byte = self
                .main_ram
                .read((row_addr + offset as u16) as usize)
                .unwrap_or(0);

            for pixel in [aux_byte, main_byte] {
                for x in 0..7 {
                    if (pixel & (1 << x)) != 0 {
                        row[index] = Color32::WHITE;
                    } else {
                        row[index] = Color32::BLACK;
                    }
                    index += 1;
                }
            }
        }
    }

    fn fill_double_hires_row(&self, row_addr: u16, row: &mut [Color32]) {
        let mut index = 0;

        let mut sr = 0;

        let mut bits = 0;
        for (offset, _) in (0..40).enumerate() {
            let aux_byte = self
                .text_card
                .as_ref()
                .map(|c| c.read((row_addr + offset as u16) as usize).unwrap_or(0))
                .unwrap_or(0);

            let main_byte = self
                .main_ram
                .read((row_addr + offset as u16) as usize)
                .unwrap_or(0);

            sr |= ((aux_byte & 0x7f) as u32) << bits;
            bits += 7;
            sr |= ((main_byte & 0x7f) as u32) << bits;
            bits += 7;

            while bits >= 4 {
                let mut value = (sr & 0b1111) as u8;

                // position in sliding window
                let window_offset = index % 4;

                // bring in next `window_offset` bit into value
                if window_offset != 0 {
                    value = ((((sr << window_offset) & !((1 << window_offset) - 1))
                        | ((sr >> (4 - window_offset)) & ((1 << window_offset) - 1)))
                        & 0b1111) as u8;
                }

                let color = Self::get_double_hires_color(value);
                bits -= 1;
                sr >>= 1;
                row[index] = color;
                index += 1;
            }
        }
    }

    fn fill_hires_row(&self, row_addr: u16, row: &mut [Color32]) {
        let mut column = 0;
        let mut previous_bit = 0;
        let mut previous_pixel = None;

        let mut index = 0;

        for group in 0..40 {
            let mut byte = self.main_ram.read((row_addr + group) as usize).unwrap_or(0);
            let alternate_color = (byte >> 7) & 1 == 1;
            for _ in 1..8 {
                let dot = byte & 1;
                byte >>= 1;

                let mut next_pixel = Color32::BLACK;
                if dot == 1 {
                    next_pixel = match (column % 2, alternate_color) {
                        (0, false) => Color32::from_rgb(0xbb, 0x36, 0xff), // violet
                        (1, false) => Color32::from_rgb(0x43, 0xc8, 0x00), // green
                        (0, true) => Color32::from_rgb(0x07, 0xa8, 0xe0),  // blue
                        (1, true) => Color32::from_rgb(0xf9, 0x56, 0x1d),  // red
                        _ => Color32::BLACK,
                    };
                }

                let white = dot == 1 && previous_bit == 1;
                if let Some(previous_pixel) = previous_pixel.take() {
                    row[index] = if white {
                        Color32::WHITE
                    } else {
                        previous_pixel
                    };
                    row[index + 1] = row[index];
                    index += 2;
                }
                previous_pixel = Some(if white { Color32::WHITE } else { next_pixel });

                column += 1;
                previous_bit = dot;
            }
        }
        if let Some(previous_pixel) = previous_pixel.take() {
            row[index] = previous_pixel;
            row[index + 1] = previous_pixel;
        }
    }

    fn draw_frame(&self, state: &mut State) -> Vec<Color32> {
        let mixed_mode = self.soft_switches.mix_mode();
        let hires_mode = self.soft_switches.hires_mode();
        let text_mode = self.soft_switches.text_mode();
        let lores_mode = !hires_mode && !text_mode;

        let mut framebuffer = vec![Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT];

        let text_page_base_address =
            if self.soft_switches.page_two() && !self.soft_switches.eightycol() {
                0x800
            } else {
                0x400
            };
        let hires_page_base_address = if self.soft_switches.page_two() && self.soft_switches.an3() {
            0x4000
        } else {
            0x2000
        };

        let mut framebuffer_index = 0;
        for section in 0..3 {
            for row in 0..8 {
                let text_row_offset = text_page_base_address + (section * 40) + (row * 0x80);
                let hires_row_offset = hires_page_base_address + (section * 40) + (row * 0x80);

                let mixed_mode_write = mixed_mode && section == 2 && row >= 4;

                for subrow in 0..8 {
                    if lores_mode || text_mode || mixed_mode_write {
                        for y in 0..40 {
                            let row = if text_mode || mixed_mode_write {
                                let code = self
                                    .main_ram
                                    .read((text_row_offset + y) as usize)
                                    .unwrap_or(0);

                                if self.soft_switches.eightycol() {
                                    let code2 = self
                                        .text_card
                                        .as_ref()
                                        .map(|card| {
                                            card.read((text_row_offset + y) as usize).unwrap_or(0)
                                        })
                                        .unwrap_or(0);
                                    vec![
                                        self.get_text_character_row(
                                            code2,
                                            state.blink_state.inverted,
                                            subrow,
                                        ),
                                        self.get_text_character_row(
                                            code,
                                            state.blink_state.inverted,
                                            subrow,
                                        ),
                                    ]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>()
                                } else {
                                    self.get_text_character_row(
                                        code,
                                        state.blink_state.inverted,
                                        subrow,
                                    )
                                    .iter()
                                    .flat_map(|p| [*p, *p])
                                    .collect::<Vec<_>>()
                                }
                            } else {
                                let code = self
                                    .main_ram
                                    .read((text_row_offset + y) as usize)
                                    .unwrap_or(0);
                                let color = if subrow > 3 {
                                    self.get_color((code >> 4) & 0b1111)
                                } else {
                                    self.get_color(code & 0b1111)
                                };
                                [color; 7].iter().flat_map(|p| [*p, *p]).collect::<Vec<_>>()
                            };

                            framebuffer[framebuffer_index..framebuffer_index + row.len()]
                                .copy_from_slice(&row);
                            framebuffer_index += row.len();
                        }
                    } else {
                        let subrow_offset = hires_row_offset + (subrow as u16 * 0x400);
                        if !self.soft_switches.an3() && self.soft_switches.eightycol() {
                            if state.video_mode == VideoMode::Video7BlackWhite
                                || (state.monitor_type == MonitorType::Monochrome
                                    || state.monitor_type == MonitorType::Green
                                    || state.monitor_type == MonitorType::Amber)
                            {
                                self.fill_video7_black_white_row(
                                    subrow_offset,
                                    &mut framebuffer
                                        [framebuffer_index..framebuffer_index + SCREEN_WIDTH],
                                )
                            } else {
                                self.fill_double_hires_row(
                                    subrow_offset,
                                    &mut framebuffer
                                        [framebuffer_index..framebuffer_index + SCREEN_WIDTH],
                                )
                            }
                        } else {
                            self.fill_hires_row(
                                subrow_offset,
                                &mut framebuffer
                                    [framebuffer_index..framebuffer_index + SCREEN_WIDTH],
                            );
                        }
                        framebuffer_index += SCREEN_WIDTH;
                    }

                    // double line
                    framebuffer.copy_within(
                        framebuffer_index - SCREEN_WIDTH..framebuffer_index,
                        framebuffer_index,
                    );
                    framebuffer_index += SCREEN_WIDTH;
                }
            }
        }
        framebuffer
    }

    pub fn update_frame(&self) {
        let mut state = self.state.write().unwrap();
        let mut frame = self.draw_frame(&mut state);

        // TODO - consider doing the mapping when filling a pixel row instead to avoid
        // iterating here
        if state.monitor_type == MonitorType::Green
            || state.monitor_type == MonitorType::Monochrome
            || state.monitor_type == MonitorType::Amber
        {
            for p in &mut frame {
                if *p != Color32::BLACK {
                    if state.monitor_type == MonitorType::Green {
                        *p = MONITOR_GREEN;
                    } else if state.monitor_type == MonitorType::Monochrome {
                        *p = Color32::WHITE;
                    } else if state.monitor_type == MonitorType::Amber {
                        *p = MONITOR_AMBER;
                    }
                }
            }
        }
        state.frame = frame;
    }

    pub fn update_display(&self, _ctx: &Context, ui: &mut Ui) -> Result<Response, Error> {
        let mut state = self.state.write().unwrap();

        let now = Instant::now();
        if now.duration_since(state.blink_state.last_blink) > Duration::from_millis(250) {
            state.blink_state.inverted = !state.blink_state.inverted;
            state.blink_state.last_blink = now;
        }

        let fb = state.frame.to_vec();

        let image = ColorImage::new([SCREEN_WIDTH, SCREEN_HEIGHT], fb);
        let texture = ui.ctx().load_texture("frame", image, Default::default());

        let ratio = SCREEN_HEIGHT as f32 / SCREEN_WIDTH as f32;
        let mut height = ui.available_width() * ratio;
        let mut width = ui.available_width();

        if ui.available_height() < height {
            height = ui.available_height();
            width = ui.available_height() / ratio;
        }
        let img = Image::new(SizedTexture::new(texture.id(), vec2(width, height)));

        state.fps = 1.0 / (Instant::now() - state.last_draw).as_secs_f32();
        state.last_draw = Instant::now();

        //self.soft_switches.set_vbl(false);
        Ok(ui.add(img))
    }
}

impl Display for Video {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Video")
    }
}
