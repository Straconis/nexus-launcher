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
    help_open: bool,
    help_query: String,
    editor_open: bool,
    pub admin_apps: Vec<AdminApp>,
    pub grants: Vec<Grant>,
    pub draft_id: String,
    pub draft_name: String,
    pub draft_summary: String,
    pub draft_repo: String,
    pub draft_category: String,
    pub draft_homepage: String,
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
            help_open: false,
            help_query: String::new(),
            editor_open: false,
            admin_apps: Vec::new(),
            grants: Vec::new(),
            draft_id: String::new(),
            draft_name: String::new(),
            draft_summary: String::new(),
            draft_repo: String::new(),
            draft_category: String::new(),
            draft_homepage: String::new(),
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
                if ui.button("HELP / HOW TO USE").clicked() {
                    self.help_open = true;
                }
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
            });
        if self.admin_open {
            let mut open = self.admin_open;
            egui::Window::new("MANAGE APPS")
                .id(egui::Id::new("central-admin-window"))
                .open(&mut open)
                .default_size([680.0, 460.0])
                .resizable(true)
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    if let Some(message) = &self.message {
                        ui.label(RichText::new(message).color(ACCENT));
                        ui.separator();
                    }
                    actions.extend(self.admin(ui));
                });
            self.admin_open = open;
        }
        if self.help_open {
            let mut open = self.help_open;
            egui::Window::new("NEXUS LAUNCHER HELP")
                .open(&mut open)
                .default_size([560.0, 440.0])
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.help_query).hint_text("Search help…"),
                    );
                    let query = self.help_query.to_lowercase();
                    let mut matches = 0;
                    for (title, body) in HELP_TOPICS {
                        if !format!("{title} {body}").to_lowercase().contains(&query) {
                            continue;
                        }
                        matches += 1;
                        ui.heading(*title);
                        ui.label(*body);
                        ui.add_space(12.0);
                    }
                    if matches == 0 {
                        ui.label("No matching help topics. Try listing, access, or install.");
                    }
                });
            self.help_open = open;
        }
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
        let mut show_editor = false;
        let mut show_grants = false;
        ui.heading("Launcher listings");
        ui.label("Choose which apps appear in the launcher. Public access and listing are separate settings.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("ADD APP").clicked() {
                self.editor_open = true;
                show_editor = true;
                self.draft_id.clear();
                self.draft_name.clear();
                self.draft_repo.clear();
                self.draft_summary.clear();
                self.draft_category.clear();
                self.draft_homepage.clear();
                self.draft_private = false;
            }
            if ui.button("RELOAD LIST").clicked() {
                actions.push(CentralAction::RefreshAdmin);
            }
            if ui.button("HELP WITH LISTINGS").clicked() {
                self.help_open = true;
            }
        });
        if self.admin_apps.is_empty() {
            ui.label("No apps loaded. Use RELOAD LIST, or ADD APP to create an entry.");
        }
        ui.separator();
        for app in self.admin_apps.clone() {
            ui.push_id(&app.id, |ui| {
                let mark = if app.published { "SHOWN IN LAUNCHER" } else { "HIDDEN FROM LAUNCHER" };
                ui.label(
                    RichText::new(format!(
                        "{} · {} · {} · {}",
                        app.name,
                        app.visibility.as_str(),
                        mark,
                        app.repo.as_deref().unwrap_or("no repo")
                    ))
                    .color(TEXT),
                );
                ui.horizontal_wrapped(|ui| {
                    if cta(ui, "EDIT", CtaKind::Secondary, 80.0).clicked() {
                        self.draft_id = app.id.clone();
                        self.draft_name = app.name.clone();
                        self.draft_summary = app.summary.clone();
                        self.draft_repo = app.repo.clone().unwrap_or_default();
                        self.draft_category = app.category.clone().unwrap_or_default();
                        self.draft_homepage = app.homepage.clone().unwrap_or_default();
                        self.draft_private = app.visibility == nexus_protocol::Visibility::Private;
                        self.editor_open = true;
                        show_editor = true;
                        self.message = Some(format!("Editing {}. Save keeps its current listing state.", app.name));
                    }
                    let label = if app.published {
                        "HIDE FROM LAUNCHER"
                    } else {
                        "SHOW IN LAUNCHER"
                    };
                    if cta(ui, label, CtaKind::Primary, 205.0)
                        .on_hover_text("Changes this app's listing for everyone using this central catalog. Does not change GitHub repository privacy or delete installed files.")
                        .clicked() {
                        actions.push(CentralAction::TogglePublished {
                            id: app.id.clone(),
                            published: !app.published,
                        });
                    }
                    if cta(ui, "DELETE", CtaKind::Secondary, 100.0).clicked() {
                        actions.push(CentralAction::DeleteApp { id: app.id.clone() });
                    }
                    if cta(ui, "USER ACCESS", CtaKind::Secondary, 130.0).on_hover_text("Choose who can access a restricted app.").clicked() {
                        self.selected = app.id.clone();
                        self.grants.clear();
                        show_grants = true;
                        actions.push(CentralAction::Grant {
                            id: app.id.clone(),
                            login: String::new(),
                        });
                    }
                });
                ui.add_space(8.0);
            });
        }
        if self.editor_open {
            ui.separator();
            let heading = ui.heading("App details");
            if show_editor {
                heading.scroll_to_me(Some(egui::Align::TOP));
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("App ID").color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.draft_id).desired_width(140.0));
                ui.label(RichText::new("Display name").color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.draft_name).desired_width(160.0));
            });
            ui.horizontal(|ui| {
            ui.label(RichText::new("GitHub repo").color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.draft_repo).desired_width(220.0));
            ui.checkbox(&mut self.draft_private, "Restricted to granted users").on_hover_text("Unchecked: public access. This does not change the repository privacy on GitHub.");
        });
            ui.add(egui::TextEdit::singleline(&mut self.draft_summary).hint_text("summary"));
            ui.horizontal(|ui| {
                ui.label(RichText::new("category").color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.draft_category).desired_width(120.0));
                ui.label(RichText::new("homepage").color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft_homepage)
                        .desired_width(240.0)
                        .hint_text("https://"),
                );
            });
            if cta(ui, "SAVE APP DETAILS", CtaKind::Primary, 190.0).clicked() {
                actions.push(CentralAction::SaveApp);
            }
            ui.label(
                "New apps start hidden. After saving, choose SHOW IN LAUNCHER in the list above.",
            );
        }
        if !self.selected.is_empty() {
            ui.separator();
            ui.label(RichText::new(format!("ACCESS FOR {}", self.selected)).color(ACCENT));
            if self.grants.is_empty() {
                ui.label("No user grants listed. Enter a GitHub username to grant access.");
            }
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
            if show_grants {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
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

const HELP_TOPICS: &[(&str, &str)] = &[
    ("Show or hide an app", "Open MANAGE. Each entry says SHOWN IN LAUNCHER or HIDDEN FROM LAUNCHER. Choose SHOW IN LAUNCHER to list it, or HIDE FROM LAUNCHER to remove it from the shared catalog. Changes apply to everyone using this server. Installed copies stay on disk."),
    ("Public access versus listing", "Public means anyone can see a listed app. Restricted means only granted GitHub users and admins can see it. An app must also be shown in the launcher: public does not automatically mean listed. These settings do not change the repository's privacy on GitHub."),
    ("Add or edit a repository", "In MANAGE, choose ADD APP or EDIT. Use owner/repository for GitHub repo, for example Straconis/stat_tracker. App ID is a separate lowercase slug such as stat-tracker. SAVE APP DETAILS saves metadata; new apps start hidden until you choose SHOW IN LAUNCHER."),
    ("Give a user access", "Choose USER ACCESS on a restricted app, enter a GitHub username, then GRANT. REVOKE blocks later private downloads; it cannot remove copies already downloaded. Public apps do not require individual grants."),
    ("Listed but cannot install", "Listing and download availability are separate. COMING SOON means no public release was found (a missing or inaccessible repo can look the same). NO BUILD FOR THIS PLATFORM means the release lacks a compatible download. Windows installers cannot be installed as Linux apps. REPOSITORY opens the project page."),
    ("Sign in and refresh", "SIGN IN uses GitHub to identify you. Only configured admins see MANAGE. Normal restarts should restore your session; sign-out, expiry, or clearing OS credentials requires another login. REFRESH reloads the shared catalog and release information. RELOAD LIST refreshes admin entries."),
    ("Local and central catalogs", "When connected to a central server, its catalog controls the app list. Without a central server configured, the launcher reads catalog/apps.toml. MANAGE changes the shared central catalog, not GitHub repositories or the local TOML file."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_and_hide_buttons_request_the_opposite_listing_state() {
        for published in [false, true] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut panel = CentralPanel::new(None);
            panel.admin_apps.push(AdminApp {
                id: "frameweaver".into(),
                name: "FrameWeaver".into(),
                summary: String::new(),
                category: None,
                homepage: None,
                repo: Some("Straconis/frameweaver".into()),
                visibility: nexus_protocol::Visibility::Public,
                published,
            });
            let mut actions = Vec::new();
            let mut draw = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            actions.extend(panel.admin(ui));
                        });
                    },
                )
            };
            let output = draw(Vec::new());
            let label = if published {
                "HIDE FROM LAUNCHER"
            } else {
                "SHOW IN LAUNCHER"
            };
            let point = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.job.text == label {
                            return Some(text.pos + text.galley.size() / 2.0);
                        }
                    }
                    None
                })
                .expect("listing button rendered");
            let _ = draw(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let _ = draw(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert!(actions.iter().any(|action| matches!(action, CentralAction::TogglePublished { id, published: target } if id == "frameweaver" && *target != published)));
        }
    }

    #[test]
    fn edit_button_populates_the_app_form() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut panel = CentralPanel::new(None);
        panel.admin_apps.push(AdminApp {
            id: "tool".into(),
            name: "Tool".into(),
            summary: "A tool".into(),
            category: Some("Tools".into()),
            homepage: None,
            repo: Some("Straconis/tool".into()),
            visibility: nexus_protocol::Visibility::Public,
            published: true,
        });
        let mut draw = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.admin(ui);
                    });
                },
            )
        };
        let output = draw(Vec::new());
        let point = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.job.text == "EDIT" {
                        return Some(text.pos + text.galley.size() / 2.0);
                    }
                }
                None
            })
            .expect("EDIT button rendered");
        let _ = draw(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let _ = draw(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(panel.draft_id, "tool");
        assert_eq!(panel.draft_repo, "Straconis/tool");
        assert!(!panel.draft_private);
    }
}
