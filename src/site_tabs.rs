use std::collections::HashMap;
use std::time::Duration;

use eframe::egui;
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::{NewWindowResponse, PermissionResponse, WebContext, WebView, WebViewBuilder};

const SITE: &str = "https://straconis.dev";
const EXTERNAL_LINKS: &str = r#"
document.addEventListener('click', (event) => {
    if (event.defaultPrevented || event.button !== 0) return;
    const link = event.target.closest?.('a[href]');
    if (!link || link.target === '_blank') return;
    const url = new URL(link.href);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return;
    const onSite = url.protocol === 'https:' &&
        (url.hostname === 'straconis.dev' || url.hostname === 'www.straconis.dev');
    const admin = url.pathname.startsWith('/wp-admin') || url.pathname === '/wp-login.php';
    if (!onSite || admin) {
        event.preventDefault();
        window.open(url.href, '_blank', 'noopener');
    }
}, true);
"#;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SiteTab {
    Updates,
    Request,
    Library,
    Website,
}

impl SiteTab {
    pub(super) const ALL: [Self; 4] = [Self::Updates, Self::Request, Self::Library, Self::Website];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Updates => "LATEST UPDATES",
            Self::Request => "REQUEST A PROJECT",
            Self::Library => "LANGUAGE LIBRARY",
            Self::Website => "STRACONIS.DEV",
        }
    }

    fn url(self) -> String {
        let path = match self {
            Self::Updates => "/?nexus_app=updates#nexus-building-now",
            Self::Request => "/?nexus_app=request#project-request",
            Self::Library => "/language-library/",
            Self::Website => "/",
        };
        format!("{SITE}{path}")
    }
}

fn can_embed(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.host_str() {
        Some("challenges.cloudflare.com") => true,
        Some("straconis.dev" | "www.straconis.dev") => {
            url.path() == "/wp-admin/admin-post.php"
                || (!url.path().starts_with("/wp-admin") && url.path() != "/wp-login.php")
        }
        _ => false,
    }
}

fn open_external(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid page address.".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only web links can open in your browser.".into());
    }
    open::that(parsed.as_str()).map_err(|err| format!("Could not open the browser: {err}"))
}

#[derive(Default)]
pub(super) struct SiteTabs {
    // Drop child views before their shared browser context.
    views: HashMap<SiteTab, WebView>,
    context: Option<WebContext>,
    active: Option<SiteTab>,
    bounds: Option<wry::Rect>,
    failure: Option<(SiteTab, String)>,
    message: Option<String>,
}

impl SiteTabs {
    pub(super) fn hide(&mut self) {
        if let Some(tab) = self.active.take() {
            if let Some(view) = self.views.get(&tab) {
                let _ = view.set_visible(false);
            }
        }
        self.bounds = None;
    }

    fn create(
        &mut self,
        frame: &eframe::Frame,
        tab: SiteTab,
        bounds: wry::Rect,
    ) -> Result<WebView, String> {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle = frame.window_handle().map_err(|err| err.to_string())?;
            if !matches!(
                handle.as_raw(),
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)
            ) {
                return Err("Embedded pages need X11 or XWayland on Linux.".into());
            }
            if !gtk::is_initialized() {
                gtk::gdk::set_allowed_backends("x11");
                gtk::init().map_err(|err| format!("Could not start WebKitGTK: {err}"))?;
            }
        }

        let context = self.context.get_or_insert_with(|| {
            let profile = crate::install::default_data_root()
                .ok()
                .map(|root| root.join("site-browser"));
            WebContext::new(profile)
        });
        WebViewBuilder::new_with_web_context(context)
            .with_url(tab.url())
            .with_bounds(bounds)
            .with_visible(false)
            .with_background_color((16, 17, 24, 255))
            .with_autoplay(false)
            .with_devtools(false)
            .with_navigation_handler(|url| can_embed(&url))
            .with_initialization_script(EXTERNAL_LINKS)
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_new_window_req_handler(|url, _| {
                let _ = open_external(&url);
                NewWindowResponse::Deny
            })
            .with_download_started_handler(|url, _| {
                let _ = open_external(&url);
                false
            })
            .build_as_child(frame)
            .map_err(|err| format!("Could not open the embedded page: {err}"))
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, tab: SiteTab) {
        if self.active != Some(tab) {
            self.hide();
        }
        let mut navigate = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Back").clicked() {
                navigate = Some("back");
            }
            if ui.button("Start of tab").clicked() {
                navigate = Some("home");
            }
            if ui.button("Reload page").clicked() {
                navigate = Some("reload");
            }
            if ui.button("Open in browser").clicked() {
                let url = self
                    .views
                    .get(&tab)
                    .and_then(|view| view.url().ok())
                    .filter(|url| can_embed(url) && url != "about:blank")
                    .unwrap_or_else(|| tab.url());
                self.message = open_external(&url).err();
            }
        });
        if let Some(message) = &self.message {
            ui.label(message);
        }
        if let Some((failed_tab, message)) = &self.failure {
            if *failed_tab == tab {
                ui.label(message);
                if ui.button("Try again").clicked() {
                    self.failure = None;
                    ui.ctx().request_repaint();
                }
                return;
            }
        }
        ui.add_space(8.0);
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 2.0 || rect.height() < 2.0 {
            self.hide();
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
        if !self.views.contains_key(&tab) {
            match self.create(frame, tab, bounds) {
                Ok(view) => {
                    self.views.insert(tab, view);
                }
                Err(err) => {
                    self.failure = Some((tab, err));
                    ui.ctx().request_repaint();
                    return;
                }
            }
        }
        let view = &self.views[&tab];
        if self.bounds != Some(bounds) {
            if let Err(err) = view.set_bounds(bounds) {
                self.hide();
                self.failure = Some((tab, format!("Could not size the embedded page: {err}")));
                ui.ctx().request_repaint();
                return;
            }
            self.bounds = Some(bounds);
        }
        if self.active != Some(tab) {
            if let Err(err) = view.set_visible(true) {
                self.failure = Some((tab, format!("Could not show the embedded page: {err}")));
                ui.ctx().request_repaint();
                return;
            }
            self.active = Some(tab);
        }
        if let Some(action) = navigate {
            let result = match action {
                "back" => view.evaluate_script("window.history.back()"),
                "home" => view.load_url(&tab.url()),
                _ => view.reload(),
            };
            self.message = result.err().map(|err| err.to_string());
        }
        ui.allocate_rect(rect, egui::Sense::hover());
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
            ctx.request_repaint_after(Duration::from_millis(if self.active.is_some() {
                16
            } else {
                250
            }));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (ctx, Duration::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_public_site_and_request_submission_inside_the_launcher() {
        for tab in SiteTab::ALL {
            assert!(can_embed(&tab.url()));
        }
        assert!(can_embed("https://straconis.dev/wp-admin/admin-post.php"));
        assert!(can_embed("https://challenges.cloudflare.com/turnstile/"));
        for url in [
            "https://straconis.dev/wp-admin/plugins.php",
            "https://straconis.dev/wp-login.php",
            "https://straconis.dev.example.org/",
            "https://name:password@straconis.dev/",
            "file:///tmp/private",
            "http://straconis.dev/",
        ] {
            assert!(!can_embed(url), "{url}");
        }
    }
}
