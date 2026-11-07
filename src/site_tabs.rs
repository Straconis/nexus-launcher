use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::{NewWindowResponse, PermissionResponse, WebView, WebViewBuilder};

const CHALLENGE_URL: &str = "https://straconis.dev/?nexus_app=challenge";
const CHALLENGE_GUARD: &str = r#"
document.addEventListener('DOMContentLoaded', () => {
    if (window.top !== window || document.body.dataset.nexusChallenge === '1') return;
    document.body.replaceChildren();
    document.body.style.cssText = 'background:#101118;color:#f5f5fa;font:14px system-ui,sans-serif';
    document.body.textContent = 'The spam check needs Project Nexus 1.20.0 on Straconis.dev.';
});
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SiteTab {
    Updates,
    Request,
    Library,
}

impl SiteTab {
    pub(super) const ALL: [Self; 3] = [Self::Updates, Self::Request, Self::Library];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Updates => "LATEST POST",
            Self::Request => "CONTACT / PROJECT REQUEST",
            Self::Library => "LANGUAGE LIBRARY",
        }
    }
}

#[derive(Deserialize)]
pub(super) struct ChallengeToken {
    pub(super) token: Option<String>,
    pub(super) nexus_nonce: String,
    pub(super) nexus_issued: String,
    pub(super) nexus_signature: String,
}

fn challenge_page(url: &str) -> bool {
    matches!(
        url,
        CHALLENGE_URL | "https://www.straconis.dev/?nexus_app=challenge"
    )
}

fn challenge_navigation(url: &str) -> bool {
    challenge_page(url)
        || reqwest::Url::parse(url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("challenges.cloudflare.com")
                && url.username().is_empty()
                && url.password().is_none()
        })
}

#[derive(Default)]
pub(super) struct Challenge {
    view: Option<WebView>,
    events: Option<Receiver<ChallengeToken>>,
    bounds: Option<wry::Rect>,
    visible: bool,
    pub(super) error: Option<String>,
}

impl Challenge {
    pub(super) fn start(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        self.reset();
        match self.create(frame, ctx) {
            Ok((view, events)) => {
                self.view = Some(view);
                self.events = Some(events);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn create(
        &self,
        frame: &eframe::Frame,
        ctx: &egui::Context,
    ) -> Result<(WebView, Receiver<ChallengeToken>), String> {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle = frame.window_handle().map_err(|err| err.to_string())?;
            if !matches!(
                handle.as_raw(),
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)
            ) {
                return Err("The spam check needs X11 or XWayland on Linux.".into());
            }
            if !gtk::is_initialized() {
                gtk::gdk::set_allowed_backends("x11");
                gtk::init().map_err(|err| format!("Could not start the spam check: {err}"))?;
            }
        }
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        let view = WebViewBuilder::new()
            .with_url(CHALLENGE_URL)
            .with_incognito(true)
            .with_visible(false)
            .with_background_color((16, 17, 24, 255))
            .with_devtools(false)
            .with_navigation_handler(|url| challenge_navigation(&url))
            .with_initialization_script(CHALLENGE_GUARD)
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_ipc_handler(move |request| {
                if challenge_page(&request.uri().to_string()) && request.body().len() < 4096 {
                    if let Ok(token) = serde_json::from_str(request.body()) {
                        let _ = send.send(token);
                        ctx.request_repaint();
                    }
                }
            })
            .build_as_child(frame)
            .map_err(|err| format!("Could not open the spam check: {err}"))?;
        Ok((view, receive))
    }

    pub(super) fn active(&self) -> bool {
        self.view.is_some()
    }

    pub(super) fn poll(&self) -> Option<ChallengeToken> {
        self.events.as_ref()?.try_iter().last()
    }

    pub(super) fn reset(&mut self) {
        self.hide();
        *self = Self::default();
    }

    pub(super) fn hide(&mut self) {
        if self.visible {
            if let Some(view) = &self.view {
                let _ = view.set_visible(false);
            }
            self.visible = false;
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &self.view else { return };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 76.0), egui::Sense::hover());
        if !ui.clip_rect().contains_rect(rect) {
            self.hide();
            ui.label("Widen the launcher to show the spam check.");
            return;
        }
        let scale = ui.ctx().pixels_per_point();
        let bounds = wry::Rect {
            position: PhysicalPosition::new(
                (rect.left() * scale).round() as i32,
                (rect.top() * scale).round() as i32,
            )
            .into(),
            size: PhysicalSize::new(
                (rect.width() * scale).round() as u32,
                (rect.height() * scale).round() as u32,
            )
            .into(),
        };
        let result = (|| {
            if self.bounds != Some(bounds) {
                view.set_bounds(bounds)?;
                self.bounds = Some(bounds);
            }
            if !self.visible {
                view.set_visible(true)?;
                self.visible = true;
            }
            Ok::<(), wry::Error>(())
        })();
        if let Err(error) = result {
            self.reset();
            self.error = Some(format!("Could not show the spam check: {error}"));
        }
    }

    pub(super) fn pump(&self, ctx: &egui::Context) {
        #[cfg(target_os = "linux")]
        if gtk::is_initialized() {
            for _ in 0..32 {
                if !gtk::events_pending() {
                    break;
                }
                gtk::main_iteration_do(false);
            }
            if self.active() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (ctx, Duration::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_challenge_page_can_supply_submission_credentials() {
        assert!(challenge_page(CHALLENGE_URL));
        assert!(!challenge_page(
            "https://challenges.cloudflare.com/turnstile/widget"
        ));
        assert!(challenge_navigation(
            "https://challenges.cloudflare.com/turnstile/widget"
        ));
        for url in [
            "https://straconis.dev/",
            "https://straconis.dev/?nexus_app=contact",
            "https://straconis.dev.example.org/?nexus_app=challenge",
            "http://challenges.cloudflare.com/widget",
        ] {
            assert!(!challenge_navigation(url), "{url}");
        }
    }
}
