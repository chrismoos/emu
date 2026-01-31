use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, RwLock},
    time::Duration,
};

use cpal::{
    SampleRate, Stream, StreamConfig, SupportedStreamConfigRange,
    traits::{DeviceTrait, HostTrait as _},
};
use eframe::{
    App,
    egui::{self, Button, ComboBox, Ui},
};
use log::{debug, error};
use rfd::{AsyncFileDialog, AsyncMessageDialog};

use crate::{
    emulator::{AudioConfig, Emulator, ResetType, Stats, TARGET_FACTORIES, debug::DebugWindow},
    errors::Error,
    targets::ExecutionState,
    utils::{
        futures::spawn,
        time::{Instant, sleep},
    },
};

struct State {
    file_dialog_open: bool,
    file_loaded: Option<String>,
    debugger_visible: bool,
    selected_machine: String,
}

pub struct EmulatorApplication {
    emulator: Arc<Emulator>,
    audio_stream: Option<Stream>,
    last_stats_update: Instant,
    last_stats: Stats,
    state: Arc<RwLock<State>>,
    debug_window: DebugWindow,
}

impl EmulatorApplication {
    fn detect_audio_config(
        configs: impl Iterator<Item = SupportedStreamConfigRange>,
    ) -> Option<AudioConfig> {
        let supported_rates = [44100u32, 48000u32];

        for config in configs {
            let min = config.min_sample_rate().0;
            let max = config.max_sample_rate().0;
            let channels = config.channels();

            for &rate in &supported_rates {
                if rate >= min && rate <= max {
                    return Some(AudioConfig {
                        sample_rate: rate,
                        channels,
                    });
                }
            }
        }

        None
    }

    pub fn new(emulator: Emulator) -> EmulatorApplication {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no default audio device")
            .unwrap();

        let audio_stream = match device.supported_output_configs() {
            Ok(supported_configs_range) => {
                let audio_config = Self::detect_audio_config(supported_configs_range)
                    .expect("couldn't find supported audio config");

                debug!(
                    "audio config: {} Hz, {} channels",
                    audio_config.sample_rate, audio_config.channels
                );

                emulator
                    .target
                    .lock()
                    .unwrap()
                    .configure_audio(&audio_config);

                let stream_config = StreamConfig {
                    channels: audio_config.channels,
                    sample_rate: SampleRate(audio_config.sample_rate),
                    buffer_size: cpal::BufferSize::Default,
                };
                let at = emulator.target.clone();
                let channels = audio_config.channels as usize;

                Some(
                    device
                        .build_output_stream(
                            &stream_config,
                            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                                at.lock().unwrap().fill_audio_buffer(data, channels);
                            },
                            move |_err| {},
                            None,
                        )
                        .unwrap(),
                )
            }
            Err(e) => {
                error!("failed to setup audio: {:?}", e);
                None
            }
        };

        let selected_machine = emulator.target.lock().unwrap().target_type_id().to_owned();
        let emulator = Arc::new(emulator);

        EmulatorApplication {
            last_stats: emulator.target.lock().unwrap().stats(),
            last_stats_update: Instant::now(),
            emulator: emulator.clone(),
            audio_stream,
            state: Arc::new(RwLock::new(State {
                file_dialog_open: false,
                file_loaded: None,
                debugger_visible: false,
                selected_machine,
            })),
            debug_window: DebugWindow::new(emulator.clone()),
        }
    }

    pub fn start(self) -> Result<(), Error> {
        let emu = self.emulator.clone();

        #[cfg(feature = "native")]
        {
            use eframe::egui;

            std::thread::spawn(move || {
                let rt = tokio::runtime::LocalRuntime::new().unwrap();
                rt.block_on(async move {
                    emu.run().await.unwrap();
                });
            });
            let options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
                ..Default::default()
            };
            eframe::run_native("Emulator", options, Box::new(|_cc| Ok(Box::new(self)))).unwrap();
        }

        #[cfg(target_arch = "wasm32")]
        {
            use eframe::wasm_bindgen::JsCast as _;
            eframe::WebLogger::init(log::LevelFilter::Debug).ok();
            let web_options = eframe::WebOptions::default();

            wasm_bindgen_futures::spawn_local(async move {
                emu.run().await.unwrap();
            });

            wasm_bindgen_futures::spawn_local(async {
                let document = web_sys::window()
                    .expect("No window")
                    .document()
                    .expect("No document");

                let canvas = document
                    .get_element_by_id("the_canvas_id")
                    .expect("Failed to find the_canvas_id")
                    .dyn_into::<web_sys::HtmlCanvasElement>()
                    .expect("the_canvas_id was not a HtmlCanvasElement");

                let start_result = eframe::WebRunner::new()
                    .start(canvas, web_options, Box::new(|_cc| Ok(Box::new(self))))
                    .await;

                // Remove the loading text and spinner:
                if let Some(loading_text) = document.get_element_by_id("loading_text") {
                    match start_result {
                        Ok(_) => {
                            loading_text.remove();
                        }
                        Err(e) => {
                            loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                            panic!("Failed to start eframe: {e:?}");
                        }
                    }
                }
            });
        }
        Ok(())
    }
    fn show_open_file_dialog(&self) {
        let st = self.state.clone();
        let emulator = self.emulator.clone();
        spawn(async move {
            st.write().unwrap().file_dialog_open = true;
            let file = AsyncFileDialog::new()
                .add_filter("ProDOS", &["po"])
                .add_filter("DOS", &["dsk"])
                .add_filter("HDV", &["hdv"])
                .add_filter("Woz", &["woz"])
                .pick_file();

            let mut err = None;
            if let Some(file) = file.await {
                let reader = Cursor::new(file.read().await);
                if let Err(e) = emulator
                    .target
                    .lock()
                    .unwrap()
                    .load_program(&file.file_name(), Box::new(reader))
                {
                    error!("Failed to load program: {:?}", e);
                    err = Some(e);
                } else {
                    st.write().unwrap().file_loaded = Some(file.file_name());
                }
            }

            if let Some(e) = err {
                AsyncMessageDialog::new()
                    .set_title("Failed to load")
                    .set_description(format!("Unable to load program: {:?}", e))
                    .show()
                    .await;
            }
            st.write().unwrap().file_dialog_open = false;
        });
    }

    fn display_machine_selector(&self, ui: &mut Ui, state: &mut State) {
        let mut current: Option<_> = None;

        let all_types = TARGET_FACTORIES
            .into_iter()
            .flat_map(|f| f.get_types())
            .map(|t| (t.id.to_owned(), t))
            .collect::<HashMap<_, _>>();

        ComboBox::from_id_salt("machine-select")
            .selected_text(
                all_types
                    .get(&state.selected_machine)
                    .map(|t| t.name)
                    .unwrap_or_else(|| "Unknown"),
            )
            .show_ui(ui, |ui| {
                for factory in TARGET_FACTORIES {
                    let types = factory.get_types();

                    for t in types {
                        ui.selectable_value(&mut current, Some(*t), t.name);
                    }
                }
            });
        if let Some(target_type) = current {
            let mut target = None;
            for factory in TARGET_FACTORIES {
                if let Some(t) = factory.create(&target_type.id) {
                    target = Some(t);
                    break;
                }
            }

            if let Some(target) = target {
                state.selected_machine = target_type.id.to_owned();
                let e = self.emulator.clone();
                spawn(async move {
                    let old_target = e.target.lock().unwrap().clone();
                    *e.target.lock().unwrap() = target;

                    old_target.stop().await.unwrap();
                    while old_target.get_state().unwrap().execution_state != ExecutionState::Stopped
                    {
                        sleep(Duration::from_millis(100)).await;
                    }

                    let new_target = e.target.lock().unwrap().clone();
                    new_target.run().await.unwrap();
                });
            }
        }
    }
}

impl App for EmulatorApplication {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let mut state = self.state.write().unwrap();

        if Instant::now().duration_since(self.last_stats_update) > Duration::from_secs_f32(0.5) {
            self.last_stats = self.emulator.target.lock().unwrap().stats();
            self.last_stats_update = Instant::now();
        }

        if state.debugger_visible {
            egui::SidePanel::new(egui::panel::Side::Right, "debugger").show(ctx, |ui| {
                ui.add(&self.debug_window);
            });
        }

        egui::TopBottomPanel::top("my_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let target_state = self.emulator.target.lock().unwrap().get_state().unwrap();
                //ui.label(format!("{}", state.pc));

                let is_running = target_state.execution_state == ExecutionState::Running;
                if ui
                    .button(if is_running { "Stop" } else { "Start" })
                    .clicked()
                {
                    let t = self.emulator.target.clone();
                    if is_running {
                        t.lock().unwrap().halt().unwrap();
                    } else {
                        #[cfg(feature = "wasm")]
                        {
                            use cpal::traits::StreamTrait;
                            if let Some(stream) = self.audio_stream.as_ref() {
                                stream.play().unwrap();
                            }
                        }
                        spawn(async move {
                            let t1 = t.lock().unwrap().clone();
                            t1.run().await.unwrap();
                        });
                    }
                };
                if ui.button("Reset").clicked() {
                    let t = self.emulator.target.clone().lock().unwrap().clone();
                    spawn(async move {
                        t.reset(ResetType::Warm).await.unwrap();
                    });
                };
                if ui.button("Hard Reset").clicked() {
                    let t = self.emulator.target.clone().lock().unwrap().clone();
                    spawn(async move {
                        t.reset(ResetType::Cold).await.unwrap();
                    });
                };

                ui.label(format!("FPS: {:.0}", self.last_stats.fps.unwrap_or(0.0)));

                if self.last_stats.cpu_actual_freq > 0.0 {
                    ui.label(format!(
                        "CPU: {:.04} Mhz",
                        self.last_stats.cpu_actual_freq / 1_000_000.0
                    ));
                } else {
                    ui.label("CPU: N/A");
                }

                if ui.button("Debugger").clicked() {
                    state.debugger_visible = !state.debugger_visible;
                }
            });

            ui.horizontal(|ui| {
                let open_file = ui.add(Button::new("Open File"));
                if open_file.clicked() {
                    state.file_dialog_open = true;
                    self.show_open_file_dialog();
                }
                if let Some(file_loaded) = &state.file_loaded {
                    let mut file_name = file_loaded.to_owned();
                    file_name.truncate(100);
                    ui.label(file_name);
                    if ui.button("Detach").clicked() {
                        if let Ok(_) = self.emulator.target.lock().unwrap().unload_program() {
                            state.file_loaded = None;
                        }
                    }
                }

                self.display_machine_selector(ui, &mut state);
            });
            ui.add_space(4.0);
        });

        if state.debugger_visible {
            egui::TopBottomPanel::bottom("bottom")
                .max_height(200.0)
                .min_height(200.0)
                .show(ctx, |ui| {
                    ui.add(self.emulator.as_ref());
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.emulator
                .target
                .lock()
                .unwrap()
                .update_display(ui, ctx, frame);
        });

        // 30fps for now, egui's request_repaint_after ends up scheduling a repaint immediately if you are at the 60fps
        // mark
        ctx.request_repaint_after(Duration::from_secs_f32((1.0 / 30.0) + (1.0 / 60.0)));
    }
}
