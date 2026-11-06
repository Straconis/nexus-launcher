//! Account and admin controls. Policy stays on the server; this only draws state.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use nexus_protocol::{Account, AdminApp, CatalogApp, Grant};

use super::{cta, CtaKind, ACCENT, BORDER, MUTED, PANEL, TEXT};
use crate::theme;

pub enum CentralAction {
    StartLogin,
    SignOut,
    RefreshAdmin,
    SaveApp,
    TogglePublished { id: String, published: bool },
    DeleteApp { id: String },
    Grant { id: String, login: String },
    Revoke { id: String, github_id: u64 },
}

pub struct CentralPanel {
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub account: Option<Account>,
    pub message: Option<String>,
    pub user_code: Option<String>,
    pub poll_id: Option<String>,
    pub open_url: Option<String>,
    pub waiting: bool,
    pub next_poll: Option<Instant>,
    pub catalog: Vec<CatalogApp>,
    pub admin_open: bool,
    pub admin_apps: Vec<AdminApp>,
    pub grants: Vec<Grant>,
    pub draft_id: String,
    pub draft_name: String,
    pub draft_summary: String,
    pub draft_repo: String,
    pub draft_private: bool,
    pub draft_login: String,
    pub selected: String,
}

impl CentralPanel {
    pub fn new(server_url: Option<String>) -> Self {
        Self {
            server_url,
            token: None,
            account: None,
            message: None,
            user_code: None,
            poll_id: None,
            open_url: None,
            waiting: false,
            next_poll: None,
            catalog: Vec::new(),
            admin_open: false,
            admin_apps: Vec::new(),
            grants: Vec::new(),
            draft_id: String::new(),
            draft_name: String::new(),
            draft_summary: String::new(),
            draft_repo: String::new(),
            draft_private: true,
            draft_login: String::new(),
            selected: String::new(),
        }
    }

    pub fn poll_id(&self) -> Option<String> {
        self.poll_id.clone()
    }

    pub fn take_poll(&mut self) -> bool {
        match self.next_poll {
            Some(when) if Instant::now() >= when => {
                self.next_poll = None;
                true
            }
            _ => false,
        }
    }

    pub fn schedule_poll(&mut self, seconds: u64) {
        self.waiting = true;
        self.next_poll = Some(Instant::now() + Duration::from_secs(seconds.max(1)));
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<CentralAction> {
        let mut actions = Vec::new();
        egui::Frame::new()
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .corner_radius(8)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("ACCOUNT")
                        .family(theme::orbitron())
                        .size(11.0)
                        .color(ACCENT),
                );
                match &self.server_url {
                    None => {
                        ui.label(
                            RichText::new(
                                "Central catalog is off. Set server_url in central.toml. The local catalog still works.",
                            )
                            .color(MUTED)
                            .size(12.0),
                        );
                    }
                    Some(_) => actions.extend(self.signed_in_row(ui)),
                }
                if let Some(message) = &self.message {
                    ui.label(RichText::new(message).color(TEXT).size(12.0));
                }
                if self.admin_open {
                    ui.add_space(8.0);
                    actions.extend(self.admin(ui));
                }
            });
        ui.add_space(12.0);
        actions
    }

    fn signed_in_row(&mut self, ui: &mut egui::Ui) -> Vec<CentralAction> {
        let mut actions = Vec::new();
        if let Some(account) = &self.account {
            let role = if account.admin { "admin" } else { "member" };
            ui.label(
                RichText::new(format!(
                    "{} · github id {} · {role}",
                    account.login, account.github_id
                ))
                .color(TEXT)
                .size(13.0),
            );
            ui.horizontal(|ui| {
                if cta(ui, "SIGN OUT", CtaKind::Secondary, 120.0).clicked() {
                    actions.push(CentralAction::SignOut);
                }
                if account.admin && cta(ui, "MANAGE", CtaKind::Primary, 120.0).clicked() {
                    self.admin_open = !self.admin_open;
                    if self.admin_open {
                        actions.push(CentralAction::RefreshAdmin);
                    }
                }
            });
        } else if self.waiting {
            if let Some(code) = &self.user_code {
                ui.label(
                    RichText::new(format!(
                        "Enter {code} at GitHub, then this window finishes signing in."
                    ))
                    .color(TEXT)
                    .size(13.0),
                );
            }
        } else if cta(ui, "SIGN IN", CtaKind::Primary, 140.0).clicked() {
            actions.push(CentralAction::StartLogin);
        }
        actions
    }

    fn admin(&mut self, ui: &mut egui::Ui) -> Vec<CentralAction> {
        let mut actions = Vec::new();
        ui.label(
            RichText::new("Apps are added here one at a time. Repos are not scanned.")
                .color(MUTED)
                .size(12.0),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("id").color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.draft_id).desired_width(140.0));
            ui.label(RichText::new("name").color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.draft_name).desired_width(160.0));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("repo").color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.draft_repo).desired_width(220.0));
            ui.checkbox(&mut self.draft_private, "private");
            if cta(ui, "SAVE APP", CtaKind::Primary, 130.0).clicked() {
                actions.push(CentralAction::SaveApp);
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.draft_summary).hint_text("summary"));
        for app in self.admin_apps.clone() {
            ui.horizontal(|ui| {
                let mark = if app.published { "published" } else { "draft" };
                ui.label(
                    RichText::new(format!(
                        "{} · {} · {} · {}",
                        app.id,
                        app.visibility.as_str(),
                        mark,
                        app.repo.as_deref().unwrap_or("no repo")
                    ))
                    .color(TEXT)
                    .size(12.0),
                );
                let label = if app.published {
                    "UNPUBLISH"
                } else {
                    "PUBLISH"
                };
                if cta(ui, label, CtaKind::Secondary, 130.0).clicked() {
                    actions.push(CentralAction::TogglePublished {
                        id: app.id.clone(),
                        published: !app.published,
                    });
                }
                if cta(ui, "DELETE", CtaKind::Secondary, 100.0).clicked() {
                    actions.push(CentralAction::DeleteApp { id: app.id.clone() });
                }
                if cta(ui, "GRANTS", CtaKind::Secondary, 100.0).clicked() {
                    self.selected = app.id.clone();
                    actions.push(CentralAction::Grant {
                        id: app.id.clone(),
                        login: String::new(),
                    });
                }
            });
        }
        if !self.selected.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("grant {}", self.selected)).color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.draft_login).desired_width(160.0));
                if cta(ui, "GRANT", CtaKind::Primary, 110.0).clicked() {
                    actions.push(CentralAction::Grant {
                        id: self.selected.clone(),
                        login: self.draft_login.clone(),
                    });
                }
            });
            for grant in &self.grants {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} · {}", grant.login, grant.github_id))
                            .color(TEXT)
                            .size(12.0),
                    );
                    if cta(ui, "REVOKE", CtaKind::Secondary, 110.0).clicked() {
                        actions.push(CentralAction::Revoke {
                            id: self.selected.clone(),
                            github_id: grant.github_id,
                        });
                    }
                });
            }
        }
        actions
    }
}
