use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;

use super::site_tabs::{Challenge, ChallengeToken};

const LATEST_URL: &str = "https://straconis.dev/wp-json/nexus/v1/launcher/latest-post";
const SUBMIT_URL: &str = "https://straconis.dev/wp-admin/admin-post.php";

#[derive(Deserialize)]
struct Post {
    title: String,
    date: String,
    url: String,
    body: String,
}

#[derive(Deserialize)]
struct PostPayload {
    post: Option<Post>,
}

#[derive(Default)]
pub(super) struct LatestPost {
    result: Option<Result<Option<Post>, String>>,
    pending: Option<Receiver<Result<Option<Post>, String>>>,
}

impl LatestPost {
    pub(super) fn refresh(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            self.pending = Some(background(ctx, || {
                let payload: PostPayload = client()?
                    .get(LATEST_URL)
                    .send()
                    .and_then(reqwest::blocking::Response::error_for_status)
                    .and_then(reqwest::blocking::Response::json)
                    .map_err(|_| "Could not load the latest post.".to_owned())?;
                Ok(payload.post)
            }));
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        if self.result.is_none() {
            self.refresh(ui.ctx());
        }
        if let Some(result) = poll(&mut self.pending) {
            self.result = Some(result);
        }
        ui.horizontal(|ui| {
            ui.heading("Latest post");
            if ui
                .add_enabled(self.pending.is_none(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh(ui.ctx());
            }
        });
        if self.pending.is_some() {
            ui.small("Checking for the latest post…");
        }
        match &self.result {
            Some(Ok(Some(post))) => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(&post.title);
                    ui.small(&post.date);
                    ui.add_space(12.0);
                    ui.label(&post.body);
                    if reqwest::Url::parse(&post.url).is_ok_and(|url| {
                        url.scheme() == "https"
                            && matches!(url.host_str(), Some("straconis.dev" | "www.straconis.dev"))
                            && url.username().is_empty()
                            && url.password().is_none()
                    }) {
                        ui.hyperlink_to("Read on Straconis.dev ↗", &post.url);
                    }
                });
            }
            Some(Ok(None)) => {
                ui.label("No public posts yet.");
            }
            Some(Err(error)) => {
                ui.label(error);
            }
            None => {}
        }
    }
}

#[derive(Clone, Default)]
struct Draft {
    name: String,
    contact: String,
    subject: String,
    message: String,
    consent: bool,
}

impl Draft {
    fn complete(&self, contact_mode: bool) -> bool {
        !self.name.trim().is_empty()
            && !self.contact.trim().is_empty()
            && !self.subject.trim().is_empty()
            && self.message.trim().chars().count() >= if contact_mode { 1 } else { 20 }
            && self.consent
    }
}

#[derive(Default)]
pub(super) struct ContactForm {
    contact_mode: bool,
    drafts: [Draft; 2],
    token: Option<ChallengeToken>,
    pending: Option<Receiver<Result<(), String>>>,
    result: Option<Result<(), String>>,
}

impl ContactForm {
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        challenge: &mut Challenge,
    ) {
        if let Some(token) = challenge.poll() {
            self.token = token.token.is_some().then_some(token);
        }
        if let Some(result) = poll(&mut self.pending) {
            if result.is_ok() {
                self.drafts[usize::from(self.contact_mode)] = Draft::default();
            }
            self.result = Some(result);
        }

        let sending = self.pending.is_some();
        ui.add_enabled_ui(!sending, |ui| {
            ui.horizontal(|ui| {
                let before = self.contact_mode;
                ui.selectable_value(&mut self.contact_mode, false, "Request a project");
                ui.selectable_value(&mut self.contact_mode, true, "Contact");
                if before != self.contact_mode {
                    self.result = None;
                }
            });
        });

        let mode = self.contact_mode;
        let draft = &mut self.drafts[usize::from(mode)];
        egui::TopBottomPanel::bottom("contact-submit").show_inside(ui, |ui| {
            if let Some(result) = &self.result {
                match result {
                    Ok(()) => {
                        ui.label(if mode {
                            "Your message was saved."
                        } else {
                            "Your project request was saved."
                        });
                    }
                    Err(error) => {
                        ui.label(error);
                    }
                }
            }
            if let Some(error) = &challenge.error {
                ui.label(error);
            }
            ui.horizontal_wrapped(|ui| {
                if !sending
                    && ui
                        .button(if challenge.active() {
                            "Restart spam check"
                        } else {
                            "Start spam check"
                        })
                        .clicked()
                {
                    self.token = None;
                    challenge.start(frame, ui.ctx());
                }
                let ready = draft.complete(mode) && self.token.is_some();
                if ui
                    .add_enabled(
                        !sending && ready,
                        egui::Button::new(if mode {
                            "Send message"
                        } else {
                            "Send project request"
                        }),
                    )
                    .clicked()
                {
                    if let Some(token) = self.token.take() {
                        let draft = draft.clone();
                        challenge.reset();
                        self.result = None;
                        self.pending = Some(background(ui.ctx(), move || send(mode, draft, token)));
                    }
                }
                if sending {
                    ui.spinner();
                    ui.label("Sending…");
                }
            });
            challenge.show(ui);
        });

        egui::ScrollArea::vertical().id_salt("native-contact-fields").show(ui, |ui| {
            ui.add_enabled_ui(!sending, |ui| {
                ui.heading(if mode { "Contact" } else { "Request a project" });
                field(ui, "Your name", &mut draft.name, 100);
                field(ui, "Email or Discord username", &mut draft.contact, 160);
                field(ui, if mode { "Subject" } else { "Project idea" }, &mut draft.subject, 160);
                ui.label(if mode { "Message" } else { "What should it do? (at least 20 characters)" });
                ui.add(egui::TextEdit::multiline(&mut draft.message).char_limit(3000).desired_rows(5).desired_width(520.0));
                ui.checkbox(&mut draft.consent, "I agree to share these details privately with the site owner through Syd on Discord.");
            });
        });
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, limit: usize) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(360.0),
    );
}

#[derive(Deserialize)]
struct SubmitReply {
    #[serde(default)]
    saved: bool,
    message: Option<String>,
}

fn send(contact_mode: bool, draft: Draft, token: ChallengeToken) -> Result<(), String> {
    let fields = [
        ("action", "nexus_project_request"),
        ("nexus_app_native", "1"),
        (
            "nexus_app_request",
            if contact_mode { "contact" } else { "1" },
        ),
        ("nexus_nonce", &token.nexus_nonce),
        ("nexus_issued", &token.nexus_issued),
        ("nexus_signature", &token.nexus_signature),
        (
            "cf-turnstile-response",
            token.token.as_deref().unwrap_or_default(),
        ),
        ("request_name", draft.name.trim()),
        ("request_contact", draft.contact.trim()),
        ("request_title", draft.subject.trim()),
        ("request_details", draft.message.trim()),
        ("request_consent", "1"),
    ];
    let response = client()?
        .post(SUBMIT_URL)
        .form(&fields)
        .send()
        .map_err(|_| {
            "Could not reach Straconis.dev. Start a new spam check to retry.".to_owned()
        })?;
    let success = response.status().is_success();
    let reply: SubmitReply = response
        .json()
        .map_err(|_| "The site did not confirm that your message was saved.".to_owned())?;
    if success && reply.saved {
        Ok(())
    } else {
        Err(reply
            .message
            .unwrap_or_else(|| "The site could not save your message.".into()))
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Could not connect to Straconis.dev.".into())
}

fn background<T: Send + 'static>(
    ctx: &egui::Context,
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (send, receive) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = send.send(job());
        ctx.request_repaint();
    });
    receive
}

fn poll<T>(pending: &mut Option<Receiver<Result<T, String>>>) -> Option<Result<T, String>> {
    let result = match pending.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(mpsc::TryRecvError::Disconnected) => Err("The request ended unexpectedly.".into()),
    };
    *pending = None;
    Some(result)
}
