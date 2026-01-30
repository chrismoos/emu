use std::sync::Arc;

use eframe::egui::Widget;

use crate::emulator::Emulator;

pub struct DebugWindow {
    emulator: Arc<Emulator>,
}

impl DebugWindow {
    pub fn new(emulator: Arc<Emulator>) -> DebugWindow {
        DebugWindow { emulator }
    }
}

impl Widget for &DebugWindow {
    fn ui(self, ui: &mut eframe::egui::Ui) -> eframe::egui::Response {
        ui.set_min_width(200.0);
        if let Ok(state) = self.emulator.target.lock().unwrap().get_state() {
            ui.horizontal(|ui| {
                ui.strong("State: ");
                ui.label(format!("{:?}", state.execution_state));
            });

            ui.horizontal(|ui| {
                ui.strong("PC: ");
                ui.label(format!("{:08x}", state.pc.value));
            });

            ui.strong(
                state
                    .flags
                    .iter()
                    .map(|f| f.0.to_owned())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            ui.label(
                state
                    .flags
                    .iter()
                    .map(|f| if f.1 { "1" } else { "0" })
                    .collect::<Vec<_>>()
                    .join(" "),
            );

            for reg in state.registers {
                ui.horizontal(|ui| {
                    ui.strong(reg.name);
                    ui.label(format!("{:08x}", reg.value));
                });
            }
            ui.response()
        } else {
            ui.label("Failed to get target state.")
        }
    }
}
