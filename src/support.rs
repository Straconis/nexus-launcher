use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

const KOFI_STATUS: &str = "https://straconis.dev/wp-json/nexus/v1/kofi/status";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct TipStatus {
    tips: u64,
    filled: u8,
    cycles: u64,
}

#[derive(Default)]
pub(super) struct Support {
    status: Option<TipStatus>,
    pending: Option<Receiver<Option<TipStatus>>>,
    next_refresh: Option<Instant>,
    failed: bool,
}

impl Support {
    pub(super) fn update(&mut self, ctx: &egui::Context) {
        if let Some(pending) = &self.pending {
            let result = match pending.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => None,
            };
            self.pending = None;
            self.failed = result.is_none();
            if result.is_some() {
                self.status = result;
            }
            self.next_refresh = Some(Instant::now() + REFRESH_INTERVAL);
        }

        let now = Instant::now();
        if let Some(delay) = self
            .next_refresh
            .and_then(|next| next.checked_duration_since(now))
        {
            if !delay.is_zero() {
                ctx.request_repaint_after(delay);
                return;
            }
        }
        let (send, receive) = mpsc::channel();
        self.pending = Some(receive);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let status = fetch_status()
                .ok()
                .filter(|status| (1..=12).contains(&status.filled));
            let _ = send.send(status);
            ctx.request_repaint();
        });
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to("Support on Ko-fi", "https://ko-fi.com/straconis");
            ui.label("If you find my tools useful, feel free to help keep the projects alive.");
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(status) = &self.status {
                ui.add(
                    egui::ProgressBar::new(f32::from(status.filled) / 12.0)
                        .desired_width(96.0)
                        .text(format!("{}/12", status.filled)),
                )
                .on_hover_text("One starter segment, then one segment per confirmed tip.");
                let offline = if self.failed { " (last checked)" } else { "" };
                ui.small(format!(
                    "{} lifetime tips · {} full cores{offline}",
                    status.tips, status.cycles
                ));
            } else if self.failed {
                ui.small("Donation counter unavailable.");
            } else {
                ui.small("Checking donation counter…");
            }
        });
    }
}

fn fetch_status() -> Result<TipStatus, reqwest::Error> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?
        .get(KOFI_STATUS)
        .send()?
        .error_for_status()?;
    response.json()
}
