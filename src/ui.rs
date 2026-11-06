//! Dark card grid. Refresh re-reads the catalog and checks GitHub again.

use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, Frame, RichText, Stroke, Vec2};

use crate::catalog::{self, AppEntry, Availability};
use crate::central::{self, CentralClient};
use crate::github::{self, FetchOutcome, RateSnapshot, SelectedAsset};
use crate::install::{self, InstallRequest, InstallState};
use crate::model::PackageKind;
use crate::theme::{self, ACCENT, BG, BORDER, MUTED, PANEL, TEXT};
use nexus_protocol::{CatalogApp, ReleaseMeta, Visibility};

#[path = "central_panel.rs"]
mod central_panel;

pub fn run(catalog_path: PathBuf) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([780.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Straconis Nexus Launcher",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(NexusApp::new(catalog_path)))
        }),
    )
}

struct NexusApp {
    catalog_path: PathBuf,
    show_supported: bool,
    cards: Vec<Card>,
    catalog_error: Option<String>,
    rate: Option<RateSnapshot>,
    limited_detail: Option<String>,
    banner: Option<String>,
    status: String,
    data_root: Result<PathBuf, String>,
    jobs: Sender<Job>,
    central_jobs: Sender<CentralJob>,
    events: Receiver<Event>,
    confirm: Option<PendingInstall>,
    dialog_error: Option<String>,
    children: Vec<Child>,
    central: central_panel::CentralPanel,
}

struct Card {
    entry: AppEntry,
    phase: Phase,
    private_download: bool,
    languages: Vec<String>,
    status_stamp: Option<nexus_protocol::StatusStamp>,
}

enum Phase {
    Loading,
    ComingSoon,
    NotOnPlatform,
    Failed(String),
    Live(Box<Live>),
}

struct Live {
    tag: String,
    html_url: String,
    asset: Option<SelectedAsset>,
    install: InstallState,
    busy: Option<Busy>,
}

struct Busy {
    message: String,
    fraction: Option<f32>,
}

struct PendingInstall {
    id: String,
    asset_name: String,
}

struct FetchTarget {
    id: String,
    repo: String,
}

enum Job {
    Fetch(Vec<FetchTarget>),
    Install(InstallRequest),
}

enum CentralJob {
    ProjectTypes {
        server_url: String,
        token: String,
        change: Option<nexus_protocol::CategoryChange>,
    },
    SetProjectStatus {
        server_url: String,
        token: String,
        id: String,
        status: Option<String>,
    },
    StatusStamps {
        server_url: String,
        token: String,
        change: Option<nexus_protocol::StatusStampChange>,
    },
    Languages {
        server_url: String,
        token: String,
        id: String,
    },
    Categories {
        server_url: String,
        token: String,
        change: Option<nexus_protocol::CategoryChange>,
    },
    RepoPreview {
        server_url: String,
        token: String,
        repo: String,
    },
    LoadSession {
        server_url: String,
    },
    StartLogin {
        server_url: String,
    },
    Poll {
        server_url: String,
        poll_id: String,
    },
    SignOut {
        server_url: String,
        token: String,
    },
    Catalog {
        server_url: String,
        token: Option<String>,
    },
    AdminList {
        server_url: String,
        token: String,
    },
    SaveApp {
        server_url: String,
        token: String,
        app: nexus_protocol::UpsertApp,
    },
    Publish {
        server_url: String,
        token: String,
        id: String,
        published: bool,
    },
    DeleteApp {
        server_url: String,
        token: String,
        id: String,
    },
    Grants {
        server_url: String,
        token: String,
        id: String,
    },
    Grant {
        server_url: String,
        token: String,
        id: String,
        login: String,
    },
    Revoke {
        server_url: String,
        token: String,
        id: String,
        github_id: u64,
    },
    Release {
        server_url: String,
        token: String,
        id: String,
    },
    InstallPrivate {
        server_url: String,
        token: String,
        id: String,
    },
}

enum Event {
    Languages {
        id: String,
        names: Vec<String>,
    },
    Rate(RateSnapshot),
    Fetched {
        id: String,
        outcome: FetchOutcome,
    },
    Progress {
        id: String,
        message: String,
        fraction: Option<f32>,
    },
    Installed {
        id: String,
        result: Result<(), String>,
    },
    Central(CentralEvent),
}

enum CentralEvent {
    ProjectTypes(Result<Vec<String>, String>),
    StatusStamps(Result<Vec<nexus_protocol::StatusStamp>, String>),
    Categories(Result<Vec<String>, String>),
    RepoPreview {
        repo: String,
        result: Result<nexus_protocol::RepoPreview, String>,
    },
    Session(Result<Option<(String, nexus_protocol::Account, Vec<CatalogApp>)>, String>),
    DeviceStarted(Result<nexus_protocol::DeviceStart, String>),
    Polled(Result<nexus_protocol::DevicePoll, String>),
    Catalog(Result<Vec<CatalogApp>, String>),
    Admin(Result<Vec<nexus_protocol::AdminApp>, String>),
    Grants(Result<Vec<nexus_protocol::Grant>, String>),
    Note(Result<String, String>),
    PrivateRelease {
        id: String,
        result: Result<ReleaseMeta, String>,
    },
}

enum UiAction {
    ShowPage(bool),
    Refresh,
    Retry(String),
    Install { id: String, confirmed: bool },
    Launch(String),
    OpenInstaller(String),
    OpenUrl(String),
    ConfirmInstall,
    CancelConfirm,
    DismissError,
}

enum CtaKind {
    Primary,
    Secondary,
    Disabled,
}

impl NexusApp {
    fn new(catalog_path: PathBuf) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let (central_jobs, central_rx) = mpsc::channel();
        let central_events = event_tx.clone();
        thread::Builder::new()
            .name("nexus-launcher-admin".into())
            .spawn(move || {
                while let Ok(job) = central_rx.recv() {
                    central_job(job, &central_events);
                }
            })
            .expect("spawn administration thread");
        thread::Builder::new()
            .name("nexus-launcher-net".into())
            .spawn(move || worker_loop(job_rx, event_tx))
            .expect("spawn network thread");
        let mut app = Self {
            catalog_path,
            show_supported: false,
            cards: Vec::new(),
            catalog_error: None,
            rate: None,
            limited_detail: None,
            banner: None,
            status: "Ready".into(),
            data_root: install::default_data_root().map_err(|err| err.to_string()),
            jobs,
            central_jobs,
            events,
            confirm: None,
            dialog_error: None,
            children: Vec::new(),
            central: central_panel::CentralPanel::new(
                central::config_path()
                    .and_then(|path| central::load_config(&path).ok().flatten())
                    .map(|config| config.server_url),
            ),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.limited_detail = None;
        if let Some(server_url) = self.central.server_url.clone() {
            self.catalog_error = None;
            self.status = "Checking central catalog…".into();
            self.send_central(CentralJob::LoadSession { server_url });
            return;
        }
        match catalog::load_catalog(&self.catalog_path) {
            Ok(catalog) => {
                self.catalog_error = None;
                self.cards = catalog.apps.into_iter().map(Card::from_entry).collect();
                self.queue_fetches();
            }
            Err(err) => self.catalog_error = Some(err.to_string()),
        }
        self.recompute_banner();
        self.refresh_status();
    }

    fn queue_fetches(&mut self) {
        let targets: Vec<FetchTarget> = self
            .cards
            .iter()
            .filter_map(|card| {
                if card.entry.kind == "supported" {
                    return None;
                }
                let repo = card.entry.repo.clone()?;
                if matches!(card.phase, Phase::Loading)
                    && !card.private_download
                    && card.entry.repo.is_some()
                {
                    Some(FetchTarget {
                        id: card.entry.id.clone(),
                        repo,
                    })
                } else {
                    None
                }
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        self.status = "Checking releases…".into();
        if self.jobs.send(Job::Fetch(targets)).is_err() {
            self.dialog_error = Some("Background worker stopped.".into());
        }
    }

    fn poll(&mut self) {
        self.children
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        while let Ok(event) = self.events.try_recv() {
            self.apply(event);
        }
        self.refresh_status();
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Languages { id, names } => {
                if let Some(card) = self.cards.iter_mut().find(|card| card.entry.id == id) {
                    card.languages = names;
                }
            }
            Event::Rate(rate) => {
                self.rate = Some(rate);
                self.recompute_banner();
            }
            Event::Fetched { id, outcome } => self.apply_fetch(id, outcome),
            Event::Progress {
                id,
                message,
                fraction,
            } => {
                if let Some(live) = self.live_mut(&id) {
                    live.busy = Some(Busy { message, fraction });
                }
                self.status = format!("{id}: working");
            }
            Event::Central(event) => self.apply_central(event),
            Event::Installed { id, result } => {
                let state = self.install_state(&id);
                if let Some(live) = self.live_mut(&id) {
                    live.busy = None;
                    live.install = state;
                }
                match result {
                    Ok(()) => self.status = format!("Finished {id}"),
                    Err(err) => {
                        self.status = format!("{id} failed");
                        self.dialog_error = Some(err);
                    }
                }
            }
        }
    }

    fn apply_fetch(&mut self, id: String, outcome: FetchOutcome) {
        let install = self.install_state(&id);
        let Some(card) = self.cards.iter_mut().find(|card| card.entry.id == id) else {
            return;
        };
        card.phase = match outcome {
            FetchOutcome::Missing => Phase::ComingSoon,
            FetchOutcome::Failed(message) => Phase::Failed(message),
            FetchOutcome::RateLimited(message) => {
                self.limited_detail = Some(message.clone());
                Phase::Failed(message)
            }
            FetchOutcome::Release(release) => {
                let asset = github::select_asset(
                    &release.assets,
                    catalog::current_platform(),
                    catalog::current_arch(),
                    &card.entry.asset_preference,
                );
                Phase::Live(Box::new(Live {
                    tag: release.tag,
                    html_url: release.html_url,
                    asset,
                    install,
                    busy: None,
                }))
            }
        };
        if self.limited_detail.is_some() {
            self.recompute_banner();
        }
    }

    fn live_mut(&mut self, id: &str) -> Option<&mut Live> {
        self.cards.iter_mut().find_map(|card| {
            if card.entry.id == id {
                if let Phase::Live(live) = &mut card.phase {
                    return Some(live.as_mut());
                }
            }
            None
        })
    }

    fn install_state(&self, id: &str) -> InstallState {
        match &self.data_root {
            Ok(root) => install::detect(root, id),
            Err(_) => InstallState::NotInstalled,
        }
    }

    fn recompute_banner(&mut self) {
        let (remaining, reset) = self
            .rate
            .as_ref()
            .map(|rate| (rate.remaining, rate.reset_unix))
            .unwrap_or((None, None));
        self.banner = github::banner_message(remaining, reset, self.limited_detail.as_deref());
    }

    fn refresh_status(&mut self) {
        let busy = self.cards.iter().any(Card::is_busy);
        if busy && self.status == "Ready" {
            self.status = "Checking releases…".into();
        } else if !busy && self.status.starts_with("Checking") {
            self.status = "Ready".into();
        }
    }

    fn working(&self) -> bool {
        self.cards.iter().any(Card::is_busy)
    }

    fn handle(&mut self, action: UiAction) {
        match action {
            UiAction::ShowPage(supported) => self.show_supported = supported,
            UiAction::Refresh => self.reload(),
            UiAction::Retry(id) => self.retry(&id),
            UiAction::Install { id, confirmed } => self.begin_install(&id, confirmed),
            UiAction::Launch(id) => self.launch(&id),
            UiAction::OpenInstaller(id) => self.open_installer(&id),
            UiAction::OpenUrl(url) => {
                if let Err(err) = open::that(&url) {
                    self.dialog_error = Some(format!("Could not open {url}: {err}"));
                }
            }
            UiAction::ConfirmInstall => {
                if let Some(pending) = self.confirm.take() {
                    self.begin_install(&pending.id, true);
                }
            }
            UiAction::CancelConfirm => self.confirm = None,
            UiAction::DismissError => self.dialog_error = None,
        }
    }

    fn retry(&mut self, id: &str) {
        let Some(card) = self.cards.iter_mut().find(|card| card.entry.id == id) else {
            return;
        };
        let Some(repo) = card.entry.repo.clone() else {
            card.phase = Phase::ComingSoon;
            return;
        };
        card.phase = Phase::Loading;
        self.limited_detail = None;
        self.recompute_banner();
        self.status = format!("Checking {id}…");
        if self
            .jobs
            .send(Job::Fetch(vec![FetchTarget {
                id: id.to_string(),
                repo,
            }]))
            .is_err()
        {
            self.dialog_error = Some("Background worker stopped.".into());
        }
    }

    fn begin_install(&mut self, id: &str, confirmed: bool) {
        let Some(card) = self.cards.iter().find(|card| card.entry.id == id) else {
            return;
        };
        let Phase::Live(live) = &card.phase else {
            return;
        };
        if live.busy.is_some() {
            return;
        }
        if card.private_download {
            let Some(server_url) = self.central.server_url.clone() else {
                self.dialog_error = Some("Central catalog is not configured.".into());
                return;
            };
            let Some(token) = self.central.token.clone() else {
                self.dialog_error = Some("Sign in to download this app.".into());
                return;
            };
            if let Some(live) = self.live_mut(id) {
                live.busy = Some(Busy {
                    message: "Starting download".into(),
                    fraction: Some(0.0),
                });
            }
            self.send_central(CentralJob::InstallPrivate {
                server_url,
                token,
                id: id.to_string(),
            });
            return;
        }
        let Some(asset) = live.asset.clone() else {
            return;
        };
        if !confirmed && !asset.has_checksum() {
            self.confirm = Some(PendingInstall {
                id: id.to_string(),
                asset_name: asset.name.clone(),
            });
            return;
        }
        let Some(request) = self.install_request(id, confirmed) else {
            return;
        };
        if let Some(live) = self.live_mut(id) {
            live.busy = Some(Busy {
                message: "Starting download".into(),
                fraction: Some(0.0),
            });
        }
        self.status = format!("Installing {id}…");
        if self.jobs.send(Job::Install(request)).is_err() {
            self.dialog_error = Some("Background worker stopped.".into());
        }
    }

    fn install_request(&self, id: &str, confirmed: bool) -> Option<InstallRequest> {
        let card = self.cards.iter().find(|card| card.entry.id == id)?;
        let Phase::Live(live) = &card.phase else {
            return None;
        };
        let asset = live.asset.clone()?;
        let allow_unverified = confirmed && !asset.has_checksum();
        Some(InstallRequest {
            app_id: card.entry.id.clone(),
            version: live.tag.clone(),
            executable_hint: card.entry.executable.clone(),
            asset_name: asset.name,
            url: asset.url,
            kind: asset.kind,
            digest: asset.digest,
            checksum_asset: asset.checksum_asset,
            allow_unverified,
        })
    }

    fn launch(&mut self, id: &str) {
        let Some((name, program, cwd)) = self.launch_target(id) else {
            return;
        };
        match install::spawn_launch(&program, &cwd) {
            Ok(Some(child)) => {
                self.children.push(child);
                self.status = format!("Launched {name}");
            }
            Ok(None) => self.status = format!("Opened {name}"),
            Err(err) => {
                self.dialog_error = Some(format!("Could not launch {name}: {err}"));
            }
        }
    }

    fn launch_target(&self, id: &str) -> Option<(String, PathBuf, PathBuf)> {
        let card = self.cards.iter().find(|card| card.entry.id == id)?;
        let Phase::Live(live) = &card.phase else {
            return None;
        };
        let InstallState::Ready(record) = &live.install else {
            return None;
        };
        let program = record.launch_path.clone()?;
        let cwd = program.parent().unwrap_or(program.as_path()).to_path_buf();
        Some((card.entry.name.clone(), program, cwd))
    }

    fn open_installer(&mut self, id: &str) {
        let Some(path) = self.installer_path(id) else {
            return;
        };
        match install::hand_to_os(&path) {
            Ok(()) => self.status = format!("Handed {} to the OS", path.display()),
            Err(err) => {
                self.dialog_error = Some(format!("Could not open {}: {err}", path.display()));
            }
        }
    }

    fn installer_path(&self, id: &str) -> Option<PathBuf> {
        let card = self.cards.iter().find(|card| card.entry.id == id)?;
        let Phase::Live(live) = &card.phase else {
            return None;
        };
        match &live.install {
            InstallState::InstallerStaged(record) | InstallState::Broken { record, .. } => {
                record.installer_path.clone()
            }
            InstallState::Ready(record) => record.installer_path.clone(),
            InstallState::NotInstalled => None,
        }
    }
}

impl Card {
    fn from_entry(entry: AppEntry) -> Self {
        let phase = match catalog::availability(&entry, catalog::current_platform()) {
            Availability::ComingSoon => Phase::ComingSoon,
            Availability::NotOnPlatform => Phase::NotOnPlatform,
            Availability::Fetch => Phase::Loading,
        };
        let status_stamp = entry.project_status.as_ref().and_then(|id| {
            nexus_protocol::default_status_stamps()
                .into_iter()
                .find(|stamp| &stamp.id == id)
        });
        Self {
            entry,
            phase,
            status_stamp,
            private_download: false,
            languages: Vec::new(),
        }
    }

    fn is_busy(&self) -> bool {
        match &self.phase {
            Phase::Loading => true,
            Phase::Live(live) => live.busy.is_some(),
            _ => false,
        }
    }
}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        let modal_open = self.confirm.is_some() || self.dialog_error.is_some();
        let mut actions = Vec::new();
        egui::TopBottomPanel::bottom("status-bar")
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ctx, |ui| self.footer(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(20))
            .show(ctx, |ui| {
                if self.central.take_poll() {
                    if let (Some(server_url), Some(poll_id)) =
                        (self.central.server_url.clone(), self.central.poll_id())
                    {
                        self.send_central(CentralJob::Poll {
                            server_url,
                            poll_id,
                        });
                    }
                }
                ui.add_enabled_ui(!modal_open, |ui| {
                    actions.extend(self.header(ui));
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!self.show_supported, "TOOLS").clicked() {
                            actions.push(UiAction::ShowPage(false));
                        }
                        if ui
                            .selectable_label(self.show_supported, "SUPPORTED PROJECTS")
                            .clicked()
                        {
                            actions.push(UiAction::ShowPage(true));
                        }
                    });
                    ui.add_space(8.0);
                    actions.extend(self.central_actions(ui));
                    actions.extend(self.banners(ui));
                    actions.extend(self.grid(ui));
                });
            });
        actions.extend(self.dialogs(ctx));
        for action in actions {
            self.handle(action);
        }
        if self.working() || self.central.next_poll.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        } else if self.central.server_url.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else if !self.children.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

impl NexusApp {
    fn header(&self, ui: &mut egui::Ui) -> Vec<UiAction> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            let logo = branding_texture(
                ui.ctx(),
                "straconis-nexus-launcher",
                include_bytes!("../assets/branding/straconis-nexus-launcher.png"),
            );
            ui.add(egui::Image::from_texture(&logo).max_size(Vec2::splat(144.0)))
                .on_hover_ui(|ui| {
                    ui.add(egui::Image::from_texture(&logo).max_size(Vec2::splat(340.0)));
                });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("STRACONIS")
                        .family(theme::orbitron())
                        .size(14.0)
                        .color(ACCENT),
                );
                ui.label(
                    RichText::new("NEXUS LAUNCHER")
                        .family(theme::orbitron())
                        .size(24.0)
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("Your desktop tools, connected.")
                        .size(13.0)
                        .color(MUTED),
                );
            });
            ui.add_space((ui.available_width() - 148.0 - ui.spacing().item_spacing.x).max(0.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_width(148.0);
                    let project_logo = branding_texture(
                        ui.ctx(),
                        "straconis-dev-project-nexus",
                        include_bytes!("../assets/branding/straconis-dev-project-nexus-logo.png"),
                    );
                    ui.add(egui::Image::from_texture(&project_logo).max_size(Vec2::splat(128.0)))
                        .on_hover_ui(|ui| {
                            ui.add(
                                egui::Image::from_texture(&project_logo)
                                    .max_size(Vec2::splat(340.0)),
                            );
                        });
                    let enabled = !self.working();
                    if cta(
                        ui,
                        "REFRESH",
                        if enabled {
                            CtaKind::Primary
                        } else {
                            CtaKind::Disabled
                        },
                        148.0,
                    )
                    .clicked()
                    {
                        actions.push(UiAction::Refresh);
                    }
                });
            });
        });
        ui.add_space(8.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 2.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(1), ACCENT);
        ui.add_space(12.0);
        actions
    }

    fn banners(&self, ui: &mut egui::Ui) -> Vec<UiAction> {
        if let Some(banner) = &self.banner {
            notice(ui, banner, true);
            ui.add_space(8.0);
        }
        if let Some(error) = &self.catalog_error {
            notice(ui, error, false);
            ui.add_space(8.0);
        }
        Vec::new()
    }

    fn grid(&self, ui: &mut egui::Ui) -> Vec<UiAction> {
        let mut actions = Vec::new();
        let cards: Vec<_> = self
            .cards
            .iter()
            .filter(|card| (card.entry.kind == "supported") == self.show_supported)
            .collect();
        if cards.is_empty() {
            ui.label(
                RichText::new(if self.show_supported {
                    "No supported projects are listed yet."
                } else {
                    "No apps in the catalog."
                })
                .color(MUTED),
            );
            return actions;
        }
        let gap = 14.0;
        let min_card = 270.0;
        let avail = ui.available_width().max(min_card);
        let cols = ((avail + gap) / (min_card + gap)).floor().max(1.0) as usize;
        let card_w = (avail - gap * (cols.saturating_sub(1) as f32)) / cols as f32;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("app-grid")
                    .num_columns(cols)
                    .max_col_width(card_w)
                    .spacing(Vec2::new(gap, gap))
                    .show(ui, |ui| {
                        for (index, card) in cards.iter().enumerate() {
                            actions.extend(self.card(ui, card, card_w));
                            if (index + 1) % cols == 0 || index + 1 == cards.len() {
                                ui.end_row();
                            }
                        }
                    });
            });
        actions
    }

    fn card(&self, ui: &mut egui::Ui, card: &Card, width: f32) -> Vec<UiAction> {
        let mut actions = Vec::new();
        ui.allocate_ui_with_layout(
            Vec2::new(width, 360.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(14)
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(width - 30.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.set_min_height(328.0);
                        if let Some(category) = &card.entry.category {
                            ui.label(
                                RichText::new(category.replace(" / ", " › ").to_uppercase())
                                    .family(theme::orbitron())
                                    .size(11.0)
                                    .color(ACCENT),
                            );
                        }
                        ui.label(
                            RichText::new(card.entry.name.to_uppercase())
                                .family(theme::orbitron())
                                .size(18.0)
                                .color(TEXT),
                        );
                        if let Some(stamp) = &card.status_stamp {
                            let color = stamp_color(&stamp.color);
                            Frame::new()
                                .fill(color.gamma_multiply(0.12))
                                .stroke(Stroke::new(1.5_f32, color))
                                .corner_radius(3)
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(stamp.name.to_uppercase())
                                            .strong()
                                            .size(11.0)
                                            .color(color),
                                    );
                                });
                        }
                        if !card.languages.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                for language in &card.languages {
                                    Frame::new()
                                        .fill(BG)
                                        .stroke(Stroke::new(1.0_f32, BORDER))
                                        .corner_radius(5)
                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(language).size(11.0).color(MUTED),
                                            );
                                        })
                                        .response
                                        .on_hover_text(
                                            "GitHub source languages, ordered by code size.",
                                        );
                                }
                            });
                        }
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(if card.entry.summary.is_empty() {
                                " ".to_string()
                            } else {
                                card.entry.summary.clone()
                            })
                            .color(MUTED)
                            .size(13.0),
                        );
                        ui.add_space(6.0);
                        actions.extend(self.card_body(ui, card));
                    });
            },
        );
        actions
    }

    fn card_body(&self, ui: &mut egui::Ui, card: &Card) -> Vec<UiAction> {
        let mut actions = Vec::new();
        if card.entry.kind == "supported" {
            if card.entry.id == "s3d-labs" {
                let logo = branding_texture(
                    ui.ctx(),
                    "supported-s3d-labs",
                    include_bytes!("../assets/branding/s3d-labs-logo.png"),
                );
                ui.add(egui::Image::from_texture(&logo).max_size(Vec2::splat(90.0)));
            }
            if let Some(url) = &card.entry.homepage {
                if cta(ui, "VISIT PROJECT", CtaKind::Primary, ui.available_width()).clicked() {
                    actions.push(UiAction::OpenUrl(url.clone()));
                }
            }
            return actions;
        }
        match &card.phase {
            Phase::Loading => {
                ui.label(
                    RichText::new("Checking GitHub releases…")
                        .color(MUTED)
                        .size(13.0),
                );
                ui.add_space(8.0);
                let _ = cta(ui, "CHECKING", CtaKind::Disabled, ui.available_width());
            }
            Phase::ComingSoon => {
                ui.label(
                    RichText::new("No published release.")
                        .color(MUTED)
                        .size(13.0),
                );
                ui.add_space(8.0);
                let _ = cta(ui, "COMING SOON", CtaKind::Disabled, ui.available_width());
            }
            Phase::NotOnPlatform => {
                ui.label(
                    RichText::new(format!(
                        "Not offered on {}.",
                        catalog::current_platform().label()
                    ))
                    .color(MUTED)
                    .size(13.0),
                );
                ui.add_space(8.0);
                let _ = cta(
                    ui,
                    "NOT ON THIS PLATFORM",
                    CtaKind::Disabled,
                    ui.available_width(),
                );
            }
            Phase::Failed(message) => {
                ui.label(RichText::new(message).color(TEXT).size(13.0));
                ui.add_space(8.0);
                if cta(ui, "RETRY", CtaKind::Primary, ui.available_width()).clicked() {
                    actions.push(UiAction::Retry(card.entry.id.clone()));
                }
            }
            Phase::Live(live) => actions.extend(self.live_card(ui, card, live)),
        }
        if let Some(url) = repository_url(card) {
            ui.add_space(4.0);
            if cta(ui, "REPOSITORY", CtaKind::Secondary, ui.available_width()).clicked() {
                actions.push(UiAction::OpenUrl(url));
            }
        }
        actions
    }

    fn live_card(&self, ui: &mut egui::Ui, card: &Card, live: &Live) -> Vec<UiAction> {
        let mut actions = Vec::new();
        let installed = installed_version(&live.install);
        let update = live.asset.is_some()
            && installed.is_some_and(|current| github::is_newer_release(&live.tag, current));
        ui.label(
            RichText::new(release_line(&live.tag, &live.install, update))
                .color(TEXT)
                .size(13.0),
        );
        match &live.asset {
            Some(asset) => {
                let checksum = if asset.has_checksum() {
                    "SHA-256 published"
                } else {
                    "no checksum"
                };
                ui.label(
                    RichText::new(format!(
                        "{} · {} · {checksum}",
                        asset.name,
                        kind_label(asset.kind)
                    ))
                    .color(MUTED)
                    .size(12.0),
                );
            }
            None => {
                ui.label(
                    RichText::new("No release asset for this platform.")
                        .color(MUTED)
                        .size(12.0),
                );
            }
        }
        ui.add_space(8.0);
        if let Some(busy) = &live.busy {
            ui.label(RichText::new(&busy.message).color(MUTED).size(12.0));
            let fraction = busy.fraction.unwrap_or(0.0);
            ui.add(egui::ProgressBar::new(fraction).fill(ACCENT).animate(true));
            return actions;
        }
        let width = ui.available_width();
        match &live.asset {
            None => match &live.install {
                InstallState::Ready(_) => {
                    if cta(ui, "LAUNCH", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::Launch(card.entry.id.clone()));
                    }
                }
                InstallState::InstallerStaged(_) => {
                    if cta(ui, "OPEN INSTALLER", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::OpenInstaller(card.entry.id.clone()));
                    }
                }
                _ => {
                    let _ = cta(ui, "NO BUILD FOR THIS PLATFORM", CtaKind::Disabled, width);
                }
            },
            Some(_) => match &live.install {
                InstallState::Ready(_) if !update => {
                    if cta(ui, "LAUNCH", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::Launch(card.entry.id.clone()));
                    }
                }
                InstallState::Ready(_) => {
                    if cta(ui, "UPDATE", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::Install {
                            id: card.entry.id.clone(),
                            confirmed: false,
                        });
                    }
                    if cta(ui, "LAUNCH", CtaKind::Secondary, width).clicked() {
                        actions.push(UiAction::Launch(card.entry.id.clone()));
                    }
                }
                InstallState::InstallerStaged(_) if !update => {
                    if cta(ui, "OPEN INSTALLER", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::OpenInstaller(card.entry.id.clone()));
                    }
                }
                InstallState::InstallerStaged(_) => {
                    if cta(ui, "UPDATE", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::Install {
                            id: card.entry.id.clone(),
                            confirmed: false,
                        });
                    }
                    if cta(ui, "OPEN INSTALLER", CtaKind::Secondary, width).clicked() {
                        actions.push(UiAction::OpenInstaller(card.entry.id.clone()));
                    }
                }
                InstallState::NotInstalled | InstallState::Broken { .. } => {
                    if let InstallState::Broken { reason, .. } = &live.install {
                        ui.label(RichText::new(reason).color(ACCENT).size(12.0));
                    }
                    if cta(ui, "INSTALL", CtaKind::Primary, width).clicked() {
                        actions.push(UiAction::Install {
                            id: card.entry.id.clone(),
                            confirmed: false,
                        });
                    }
                }
            },
        }
        actions
    }

    fn send_central(&mut self, job: CentralJob) {
        if self.central_jobs.send(job).is_err() {
            self.central.message =
                Some("Administration worker stopped. Restart the launcher.".into());
        }
    }

    fn central_actions(&mut self, ui: &mut egui::Ui) -> Vec<UiAction> {
        let server_url = self.central.server_url.clone();
        let token = self.central.token.clone();
        for action in self.central.show(ui) {
            let Some(server_url) = server_url.clone() else {
                continue;
            };
            let authed = |token: &Option<String>| token.clone().unwrap_or_default();
            self.central.message = Some("Working…".into());
            match action {
                central_panel::CentralAction::StartLogin => {
                    self.send_central(CentralJob::StartLogin { server_url });
                }
                central_panel::CentralAction::SignOut => {
                    let token = authed(&token);
                    self.central.token = None;
                    self.central.account = None;
                    self.central.catalog.clear();
                    self.central.admin_open = false;
                    self.cards.clear();
                    self.send_central(CentralJob::SignOut {
                        server_url: server_url.clone(),
                        token,
                    });
                    self.send_central(CentralJob::Catalog {
                        server_url,
                        token: None,
                    });
                }
                central_panel::CentralAction::RefreshAdmin => {
                    self.send_central(CentralJob::ProjectTypes {
                        server_url: server_url.clone(),
                        token: authed(&token),
                        change: None,
                    });
                    self.send_central(CentralJob::StatusStamps {
                        server_url: server_url.clone(),
                        token: authed(&token),
                        change: None,
                    });
                    self.send_central(CentralJob::Categories {
                        server_url: server_url.clone(),
                        token: authed(&token),
                        change: None,
                    });
                    self.send_central(CentralJob::AdminList {
                        server_url,
                        token: authed(&token),
                    });
                }
                central_panel::CentralAction::ChangeStatusStamp(change) => {
                    self.send_central(CentralJob::StatusStamps {
                        server_url,
                        token: authed(&token),
                        change: Some(change),
                    });
                }
                central_panel::CentralAction::ChangeProjectType(change) => {
                    self.send_central(CentralJob::ProjectTypes {
                        server_url,
                        token: authed(&token),
                        change: Some(change),
                    });
                }
                central_panel::CentralAction::ChangeCategory(change) => {
                    self.send_central(CentralJob::Categories {
                        server_url,
                        token: authed(&token),
                        change: Some(change),
                    });
                }
                central_panel::CentralAction::RepoPreview(repo) => {
                    self.send_central(CentralJob::RepoPreview {
                        server_url,
                        token: authed(&token),
                        repo,
                    });
                }
                central_panel::CentralAction::SetProjectStatus { id, status } => {
                    self.send_central(CentralJob::SetProjectStatus {
                        server_url,
                        token: authed(&token),
                        id,
                        status,
                    });
                }
                central_panel::CentralAction::SaveApp => {
                    let app = nexus_protocol::UpsertApp {
                        id: self.central.draft_id.clone(),
                        name: self.central.draft_name.clone(),
                        kind: if self.central.manage_supported {
                            "supported".into()
                        } else {
                            "app".into()
                        },
                        summary: self.central.draft_summary.clone(),
                        project_status: self.central.draft_project_status.clone(),
                        category: Some(self.central.draft_category.clone()),
                        homepage: Some(self.central.draft_homepage.clone()),
                        repo: Some(self.central.draft_repo.clone()),
                        visibility: if self.central.draft_private {
                            Visibility::Private
                        } else {
                            Visibility::Public
                        },
                        published: self
                            .central
                            .admin_apps
                            .iter()
                            .find(|app| app.id == self.central.draft_id)
                            .is_some_and(|app| app.published),
                    };
                    self.send_central(CentralJob::SaveApp {
                        server_url,
                        token: authed(&token),
                        app,
                    });
                }
                central_panel::CentralAction::TogglePublished { id, published } => {
                    self.send_central(CentralJob::Publish {
                        server_url,
                        token: authed(&token),
                        id,
                        published,
                    });
                }
                central_panel::CentralAction::DeleteApp { id } => {
                    self.send_central(CentralJob::DeleteApp {
                        server_url,
                        token: authed(&token),
                        id,
                    });
                }
                central_panel::CentralAction::Grant { id, login } => {
                    if login.trim().is_empty() {
                        self.send_central(CentralJob::Grants {
                            server_url,
                            token: authed(&token),
                            id,
                        });
                    } else {
                        self.send_central(CentralJob::Grant {
                            server_url,
                            token: authed(&token),
                            id,
                            login,
                        });
                    }
                }
                central_panel::CentralAction::Revoke { id, github_id } => {
                    self.send_central(CentralJob::Revoke {
                        server_url,
                        token: authed(&token),
                        id,
                        github_id,
                    });
                }
            }
        }
        Vec::new()
    }

    fn apply_central(&mut self, event: CentralEvent) {
        match event {
            CentralEvent::StatusStamps(result) => match result {
                Ok(stamps) => {
                    if self
                        .central
                        .draft_project_status
                        .as_ref()
                        .is_some_and(|id| !stamps.iter().any(|stamp| &stamp.id == id))
                    {
                        self.central.draft_project_status = None;
                    }
                    self.central.status_stamps = stamps;
                    self.central.message = Some("Status stamps updated.".into());
                    if let Some(server_url) = self.central.server_url.clone() {
                        self.send_central(CentralJob::AdminList {
                            server_url: server_url.clone(),
                            token: self.central.token.clone().unwrap_or_default(),
                        });
                        self.send_central(CentralJob::Catalog {
                            server_url,
                            token: self.central.token.clone(),
                        });
                    }
                }
                Err(message) => self.central.message = Some(message),
            },
            CentralEvent::ProjectTypes(result) => match result {
                Ok(types) => {
                    if self.central.manage_supported
                        && !self.central.draft_category.is_empty()
                        && !types.contains(&self.central.draft_category)
                    {
                        self.central.draft_category.clear();
                    }
                    self.central.project_types = types;
                    if let Some(server_url) = self.central.server_url.clone() {
                        self.send_central(CentralJob::AdminList {
                            server_url: server_url.clone(),
                            token: self.central.token.clone().unwrap_or_default(),
                        });
                        self.send_central(CentralJob::Catalog {
                            server_url,
                            token: self.central.token.clone(),
                        });
                    }
                }
                Err(message) => self.central.message = Some(message),
            },
            CentralEvent::Categories(result) => match result {
                Ok(categories) => {
                    if !self.central.manage_supported
                        && !self.central.draft_category.is_empty()
                        && !categories.contains(&self.central.draft_category)
                    {
                        self.central.draft_category.clear();
                    }
                    self.central.categories = categories;
                    self.central.message = Some("Category list updated.".into());
                    if let Some(server_url) = self.central.server_url.clone() {
                        self.send_central(CentralJob::AdminList {
                            server_url: server_url.clone(),
                            token: self.central.token.clone().unwrap_or_default(),
                        });
                        self.send_central(CentralJob::Catalog {
                            server_url,
                            token: self.central.token.clone(),
                        });
                    }
                }
                Err(message) => self.central.message = Some(message),
            },
            CentralEvent::RepoPreview { repo, result } => {
                if self.central.draft_repo == repo {
                    match result {
                        Ok(preview) => {
                            self.central.summary_preview = Some(preview);
                            self.central.message = Some("Repository text loaded. Review the suggested summary before using it.".into());
                        }
                        Err(message) => self.central.message = Some(message),
                    }
                }
            }
            CentralEvent::Session(Ok(Some((token, account, catalog)))) => {
                self.central.token = Some(token);
                self.central.account = Some(account);
                self.central.catalog = catalog.clone();
                self.merge_central(catalog);
            }
            CentralEvent::Session(Ok(None)) => {
                self.central.token = None;
                self.central.account = None;
                if let Some(server_url) = self.central.server_url.clone() {
                    self.send_central(CentralJob::Catalog {
                        server_url,
                        token: None,
                    });
                }
            }
            CentralEvent::Session(Err(message)) => self.central.message = Some(message),
            CentralEvent::DeviceStarted(Ok(start)) => {
                self.central.poll_id = Some(start.poll_id);
                self.central.user_code = Some(start.user_code);
                self.central.open_url = Some(start.verification_uri_complete.clone());
                self.central.waiting = true;
                self.central.message = Some(format!(
                    "GitHub code {}. Finish it in the browser.",
                    self.central.user_code.as_deref().unwrap_or("")
                ));
                let _ = open::that(&start.verification_uri_complete);
                self.central.schedule_poll(start.interval_seconds);
            }
            CentralEvent::DeviceStarted(Err(message)) => {
                self.central.waiting = false;
                self.central.message = Some(message);
            }
            CentralEvent::Polled(Ok(nexus_protocol::DevicePoll::Pending { interval_seconds }))
            | CentralEvent::Polled(Ok(nexus_protocol::DevicePoll::SlowDown { interval_seconds })) =>
            {
                self.central.schedule_poll(interval_seconds);
            }
            CentralEvent::Polled(Ok(nexus_protocol::DevicePoll::Approved {
                session_token,
                account,
            })) => {
                self.central.token = Some(session_token);
                self.central.account = Some(account);
                self.central.waiting = false;
                self.central.poll_id = None;
                self.central.user_code = None;
                self.central.next_poll = None;
                self.central.message = Some("Signed in.".into());
                if let Some(server_url) = self.central.server_url.clone() {
                    self.send_central(CentralJob::Catalog {
                        server_url,
                        token: self.central.token.clone(),
                    });
                }
            }
            CentralEvent::Polled(Ok(nexus_protocol::DevicePoll::Denied)) => {
                self.central.waiting = false;
                self.central.next_poll = None;
                self.central.message = Some("GitHub denied the login.".into());
            }
            CentralEvent::Polled(Ok(nexus_protocol::DevicePoll::Expired)) => {
                self.central.waiting = false;
                self.central.next_poll = None;
                self.central.message = Some("The login code expired. Sign in again.".into());
            }
            CentralEvent::Polled(Err(message)) => {
                self.central.waiting = false;
                self.central.message = Some(message);
            }
            CentralEvent::Catalog(Ok(apps)) => {
                self.central.catalog = apps.clone();
                self.merge_central(apps);
            }
            CentralEvent::Catalog(Err(message)) => self.central.message = Some(message),
            CentralEvent::Admin(Ok(apps)) => {
                self.central.message = Some(format!("Loaded {} apps.", apps.len()));
                self.central.admin_apps = apps;
            }
            CentralEvent::Admin(Err(message)) => self.central.message = Some(message),
            CentralEvent::Grants(Ok(grants)) => {
                self.central.message = Some(format!(
                    "Loaded {} grants for {}.",
                    grants.len(),
                    self.central.selected
                ));
                self.central.grants = grants;
            }
            CentralEvent::Grants(Err(message)) => self.central.message = Some(message),
            CentralEvent::Note(Ok(message)) => {
                self.central.message = Some(message);
                if self.central.account.is_some() {
                    if let Some(server_url) = self.central.server_url.clone() {
                        let token = self.central.token.clone().unwrap_or_default();
                        self.send_central(CentralJob::AdminList {
                            server_url: server_url.clone(),
                            token: token.clone(),
                        });
                        self.send_central(CentralJob::Catalog {
                            server_url,
                            token: self.central.token.clone(),
                        });
                        if !self.central.selected.is_empty() {
                            let id = self.central.selected.clone();
                            self.send_central(CentralJob::Grants {
                                server_url: self.central.server_url.clone().unwrap_or_default(),
                                token,
                                id,
                            });
                        }
                    }
                }
            }
            CentralEvent::Note(Err(message)) => self.central.message = Some(message),
            CentralEvent::PrivateRelease { id, result } => self.apply_private_release(id, result),
        }
    }

    fn merge_central(&mut self, apps: Vec<CatalogApp>) {
        // A configured broker controls the visible catalog, including removals.
        // Local seed entries must not shadow an admin's publish/unpublish changes.
        self.cards.clear();
        for app in apps {
            if self.cards.iter().any(|card| card.entry.id == app.id) {
                continue;
            }
            let private_download = app.visibility == Visibility::Private;
            let entry = AppEntry {
                id: app.id,
                name: app.name,
                kind: app.kind,
                summary: app.summary,
                project_status: app.project_status,
                repo: if private_download { None } else { app.repo },
                homepage: app.homepage,
                category: app.category,
                executable: None,
                asset_preference: Vec::new(),
                platforms: Vec::new(),
            };
            let phase = if private_download {
                Phase::Loading
            } else {
                match catalog::availability(&entry, catalog::current_platform()) {
                    Availability::ComingSoon => Phase::ComingSoon,
                    Availability::NotOnPlatform => Phase::NotOnPlatform,
                    Availability::Fetch => Phase::Loading,
                }
            };
            self.cards.push(Card {
                entry,
                phase,
                private_download,
                languages: Vec::new(),
                status_stamp: app.status_stamp,
            });
        }
        self.queue_fetches();
        self.queue_private();
    }

    fn queue_private(&mut self) {
        let Some(server_url) = self.central.server_url.clone() else {
            return;
        };
        let Some(token) = self.central.token.clone() else {
            return;
        };
        let ids: Vec<String> = self
            .cards
            .iter()
            .filter(|card| card.private_download && matches!(card.phase, Phase::Loading))
            .map(|card| card.entry.id.clone())
            .collect();
        for id in ids {
            self.send_central(CentralJob::Languages {
                server_url: server_url.clone(),
                token: token.clone(),
                id: id.clone(),
            });
            self.send_central(CentralJob::Release {
                server_url: server_url.clone(),
                token: token.clone(),
                id,
            });
        }
    }

    fn apply_private_release(&mut self, id: String, result: Result<ReleaseMeta, String>) {
        let install = self.install_state(&id);
        let Some(card) = self.cards.iter_mut().find(|card| card.entry.id == id) else {
            return;
        };
        card.phase = match result {
            Err(message) => Phase::Failed(message),
            Ok(meta) => {
                let kind = match meta.kind.as_str() {
                    "archive" => PackageKind::Archive,
                    "handoff" => PackageKind::Handoff,
                    _ => PackageKind::Portable,
                };
                Phase::Live(Box::new(Live {
                    tag: meta.tag,
                    html_url: String::new(),
                    asset: Some(SelectedAsset {
                        name: meta.asset_name,
                        url: String::new(),
                        size: meta.size,
                        kind,
                        digest: Some(format!("sha256:{}", meta.sha256)),
                        checksum_asset: None,
                    }),
                    install,
                    busy: None,
                }))
            }
        };
    }

    fn footer(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            brand_link(
                ui,
                "straconis-software-wordmark",
                include_bytes!("../assets/branding/straconis-software-wordmark.png"),
                Vec2::new(160.0, 50.0),
                "https://straconis.dev",
            );
            ui.add_space(12.0);
            brand_link(
                ui,
                "s3d-labs",
                include_bytes!("../assets/branding/s3d-labs-logo.png"),
                Vec2::splat(58.0),
                "https://s3dlabs.com",
            );
        });
        ui.add_space(4.0);
        let root = match &self.data_root {
            Ok(path) => path.display().to_string(),
            Err(err) => err.clone(),
        };
        ui.label(
            RichText::new(format!(
                "v{}  ·  {}",
                env!("CARGO_PKG_VERSION"),
                self.status,
            ))
            .color(MUTED)
            .size(11.0),
        )
        .on_hover_text(format!(
            "Catalog: {}\nInstall location: {root}",
            self.catalog_path.display()
        ));
    }

    fn dialogs(&self, ctx: &egui::Context) -> Vec<UiAction> {
        let mut actions = Vec::new();
        if let Some(pending) = &self.confirm {
            egui::Modal::new(egui::Id::new("checksum-confirmation"))
            .backdrop_color(Color32::from_black_alpha(170))
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, ACCENT))
                    .inner_margin(16)
                    .corner_radius(8),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.heading(RichText::new("CHECKSUM MISSING").family(theme::orbitron()));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "{} does not publish a SHA-256 checksum. Nexus Launcher cannot verify the download.",
                        pending.asset_name
                    ))
                    .color(TEXT),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Install the file anyway?")
                        .color(MUTED),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if cta(ui, "CANCEL", CtaKind::Secondary, 140.0).clicked() {
                        actions.push(UiAction::CancelConfirm);
                    }
                    if cta(ui, "INSTALL ANYWAY", CtaKind::Primary, 180.0).clicked() {
                        actions.push(UiAction::ConfirmInstall);
                    }
                });
            });
        }
        if let Some(message) = &self.dialog_error {
            egui::Modal::new(egui::Id::new("error-dialog"))
                .backdrop_color(Color32::from_black_alpha(170))
                .frame(
                    Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(16)
                        .corner_radius(8),
                )
                .show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.heading(RichText::new("SOMETHING WENT WRONG").family(theme::orbitron()));
                    ui.add_space(8.0);
                    ui.label(RichText::new(message).color(TEXT));
                    ui.add_space(10.0);
                    if cta(ui, "DISMISS", CtaKind::Primary, 140.0).clicked() {
                        actions.push(UiAction::DismissError);
                    }
                });
        }
        actions
    }
}

fn stamp_color(hex: &str) -> Color32 {
    let bytes = hex::decode(hex.trim_start_matches('#')).unwrap_or_default();
    if bytes.len() == 3 {
        Color32::from_rgb(bytes[0], bytes[1], bytes[2])
    } else {
        ACCENT
    }
}

fn brand_link(ui: &mut egui::Ui, name: &'static str, bytes: &[u8], size: Vec2, url: &str) {
    let texture = branding_texture(ui.ctx(), name, bytes);
    let response = ui.add(
        egui::Image::from_texture(&texture)
            .fit_to_exact_size(size)
            .sense(egui::Sense::click()),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    // Retain the full developer mark as a brand detail without competing with Nexus.
    if name == "straconis-software-wordmark" {
        response.clone().on_hover_ui(|ui| {
            let logo = branding_texture(
                ui.ctx(),
                "straconis-dev-logo",
                include_bytes!("../assets/branding/straconis-dev-logo.png"),
            );
            ui.add(egui::Image::from_texture(&logo).max_size(Vec2::splat(260.0)));
        });
    }
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

// Cache decoded embedded assets in the egui context; portable builds need no external files.
fn branding_texture(ctx: &egui::Context, name: &'static str, bytes: &[u8]) -> egui::TextureHandle {
    let id = egui::Id::new(name);
    if let Some(texture) = ctx.data_mut(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let decoded = image::load_from_memory(bytes)
        .expect("bundled branding PNG must decode")
        .into_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let texture = ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

fn notice(ui: &mut egui::Ui, text: &str, accent: bool) {
    let stroke = if accent { ACCENT } else { BORDER };
    Frame::new()
        .fill(Color32::from_rgb(0x2a, 0x10, 0x18))
        .stroke(Stroke::new(1.0_f32, stroke))
        .inner_margin(10)
        .corner_radius(6)
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(TEXT).size(13.0));
        });
}

fn cta(ui: &mut egui::Ui, label: &str, kind: CtaKind, width: f32) -> egui::Response {
    let (fill, color, stroke, enabled) = match kind {
        CtaKind::Primary => (ACCENT, Color32::WHITE, Stroke::new(1.0_f32, ACCENT), true),
        CtaKind::Secondary => (PANEL, ACCENT, Stroke::new(1.0_f32, ACCENT), true),
        CtaKind::Disabled => (
            Color32::from_rgb(0x1a, 0x1c, 0x24),
            MUTED,
            Stroke::new(1.0_f32, BORDER),
            false,
        ),
    };
    let button = egui::Button::new(
        RichText::new(label)
            .family(theme::orbitron())
            .size(13.0)
            .color(color),
    )
    .wrap()
    .fill(fill)
    .stroke(stroke)
    .min_size(Vec2::new(width, 34.0))
    .corner_radius(4);
    ui.add_enabled(enabled, button)
}

fn release_line(tag: &str, install: &InstallState, update: bool) -> String {
    match install {
        InstallState::NotInstalled => format!("{tag}  ·  not installed"),
        InstallState::Ready(record) if update => {
            format!(
                "update available  ·  installed {}  ·  latest {tag}",
                record.version
            )
        }
        InstallState::Ready(record) => format!("installed {}  ·  {tag}", record.version),
        InstallState::InstallerStaged(record) if update => {
            format!("installer downloaded ({})  ·  latest {tag}", record.version)
        }
        InstallState::InstallerStaged(record) => {
            format!("installer downloaded  ·  {}", record.version)
        }
        InstallState::Broken { record, .. } => {
            format!("install incomplete ({})  ·  latest {tag}", record.version)
        }
    }
}

fn installed_version(install: &InstallState) -> Option<&str> {
    match install {
        InstallState::NotInstalled => None,
        InstallState::Ready(record)
        | InstallState::InstallerStaged(record)
        | InstallState::Broken { record, .. } => Some(record.version.as_str()),
    }
}

fn repository_url(card: &Card) -> Option<String> {
    if let Some(homepage) = &card.entry.homepage {
        return Some(homepage.clone());
    }
    match &card.phase {
        Phase::Live(live) if !live.html_url.is_empty() => Some(live.html_url.clone()),
        _ => card
            .entry
            .repo
            .as_ref()
            .map(|repo| format!("https://github.com/{repo}")),
    }
}

fn kind_label(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Portable => "portable",
        PackageKind::Archive => "archive",
        PackageKind::Handoff => "opens with the OS",
    }
}

fn worker_loop(jobs: Receiver<Job>, events: Sender<Event>) {
    let client = match github::http_client() {
        Ok(client) => client,
        Err(err) => {
            let message = format!("http client: {err}");
            while let Ok(job) = jobs.recv() {
                report_client_failure(&events, job, &message);
            }
            return;
        }
    };
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Fetch(targets) => fetch_all(&client, &targets, &events),
            Job::Install(request) => install_job(&client, request, &events),
        }
    }
}

fn report_client_failure(events: &Sender<Event>, job: Job, message: &str) {
    match job {
        Job::Fetch(targets) => {
            for target in targets {
                let _ = events.send(Event::Fetched {
                    id: target.id,
                    outcome: FetchOutcome::Failed(message.to_string()),
                });
            }
        }
        Job::Install(request) => {
            let _ = events.send(Event::Installed {
                id: request.app_id,
                result: Err(message.to_string()),
            });
        }
    }
}

fn central_job(job: CentralJob, events: &Sender<Event>) {
    let event =
        match job {
            CentralJob::ProjectTypes {
                server_url,
                token,
                change,
            } => CentralEvent::ProjectTypes(central_call(&server_url, Some(token), |client| {
                client.project_types(change.as_ref())
            })),
            CentralJob::SetProjectStatus {
                server_url,
                token,
                id,
                status,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                client
                    .set_project_status(&id, status.as_deref())
                    .map(|_| "Project stamp saved.".into())
            })),
            CentralJob::StatusStamps {
                server_url,
                token,
                change,
            } => CentralEvent::StatusStamps(central_call(&server_url, Some(token), |client| {
                client.status_stamps(change.as_ref())
            })),
            CentralJob::Languages {
                server_url,
                token,
                id,
            } => {
                let names = central_call(&server_url, Some(token), |client| client.languages(&id))
                    .unwrap_or_default();
                let _ = events.send(Event::Languages { id, names });
                return;
            }
            CentralJob::Categories {
                server_url,
                token,
                change,
            } => CentralEvent::Categories(central_call(&server_url, Some(token), |client| {
                match change {
                    Some(change) => client.change_category(&change),
                    None => client.categories(),
                }
            })),
            CentralJob::RepoPreview {
                server_url,
                token,
                repo,
            } => {
                let result = central_call(&server_url, Some(token), |client| {
                    client.repo_preview(&repo)
                });
                CentralEvent::RepoPreview { repo, result }
            }
            CentralJob::LoadSession { server_url } => {
                CentralEvent::Session(load_central_session(&server_url))
            }
            CentralJob::StartLogin { server_url } => {
                CentralEvent::DeviceStarted(start_login(&server_url))
            }
            CentralJob::Poll {
                server_url,
                poll_id,
            } => CentralEvent::Polled(poll_login(&server_url, &poll_id)),
            CentralJob::SignOut { server_url, token } => {
                if let Ok(client) = CentralClient::new(&server_url, Some(token)) {
                    let _ = client.logout();
                }
                central::clear_session();
                CentralEvent::Note(Ok("Signed out. Installed copies stay on disk.".into()))
            }
            CentralJob::Catalog { server_url, token } => {
                CentralEvent::Catalog(central_call(&server_url, token, |client| client.catalog()))
            }
            CentralJob::AdminList { server_url, token } => {
                CentralEvent::Admin(central_call(&server_url, Some(token), |client| {
                    client.admin_apps()
                }))
            }
            CentralJob::SaveApp {
                server_url,
                token,
                app,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                client.upsert_app(&app).map(|_| "Saved the app.".into())
            })),
            CentralJob::Publish {
                server_url,
                token,
                id,
                published,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                client.set_published(&id, published)?;
                Ok(format!(
                    "{} is now {} in the launcher.",
                    id,
                    if published { "shown" } else { "hidden" }
                ))
            })),
            CentralJob::DeleteApp {
                server_url,
                token,
                id,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                client.delete_app(&id)?;
                Ok("Removed the app.".into())
            })),
            CentralJob::Grants {
                server_url,
                token,
                id,
            } => CentralEvent::Grants(central_call(&server_url, Some(token), |client| {
                client.grants(&id)
            })),
            CentralJob::Grant {
                server_url,
                token,
                id,
                login,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                let grant = client.grant(&id, &login)?;
                Ok(format!("Granted {} ({}).", grant.login, grant.github_id))
            })),
            CentralJob::Revoke {
                server_url,
                token,
                id,
                github_id,
            } => CentralEvent::Note(central_call(&server_url, Some(token), |client| {
                client.revoke(&id, github_id)?;
                Ok("Revoked the grant. Copies already downloaded stay installed.".into())
            })),
            CentralJob::Release {
                server_url,
                token,
                id,
            } => {
                let os = catalog::current_platform().label();
                let arch = catalog::current_arch();
                CentralEvent::PrivateRelease {
                    id: id.clone(),
                    result: central_call(&server_url, Some(token), |client| {
                        client.release(&id, os, arch)
                    }),
                }
            }
            CentralJob::InstallPrivate {
                server_url,
                token,
                id,
            } => {
                let _ = events.send(Event::Progress {
                    id: id.clone(),
                    message: "Downloading from the broker".into(),
                    fraction: Some(0.2),
                });
                let result = install_private(&server_url, &token, &id);
                let _ = events.send(Event::Installed {
                    id,
                    result: result.map(|_| ()),
                });
                return;
            }
        };
    let _ = events.send(Event::Central(event));
}

fn load_central_session(
    server_url: &str,
) -> Result<Option<(String, nexus_protocol::Account, Vec<CatalogApp>)>, String> {
    let Some(token) = central::load_session() else {
        return Ok(None);
    };
    let client = CentralClient::new(server_url, Some(token.clone()))?;
    match client.account() {
        Ok(account) => {
            let catalog = client.catalog().unwrap_or_default();
            Ok(Some((token, account, catalog)))
        }
        Err(err) if err.contains("sign in") || err.contains("401") => {
            central::clear_session();
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn start_login(server_url: &str) -> Result<nexus_protocol::DeviceStart, String> {
    CentralClient::new(server_url, None)?.start_device()
}

fn poll_login(server_url: &str, poll_id: &str) -> Result<nexus_protocol::DevicePoll, String> {
    let poll = CentralClient::new(server_url, None)?.poll_device(poll_id)?;
    if let nexus_protocol::DevicePoll::Approved { session_token, .. } = &poll {
        central::save_session(session_token)?;
    }
    Ok(poll)
}

fn central_call<T>(
    server_url: &str,
    token: Option<String>,
    call: impl FnOnce(&CentralClient) -> Result<T, String>,
) -> Result<T, String> {
    let client = CentralClient::new(server_url, token)?;
    call(&client)
}

fn install_private(server_url: &str, token: &str, id: &str) -> Result<(), String> {
    let client = CentralClient::new(server_url, Some(token.to_string()))?;
    let os = catalog::current_platform().label();
    let arch = catalog::current_arch();
    let dir = std::env::temp_dir().join(format!("nexus-central-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let dest = dir.join("download");
    let meta = client.download_verified(id, os, arch, &dest)?;
    let kind = match meta.kind.as_str() {
        "archive" => PackageKind::Archive,
        "handoff" => PackageKind::Handoff,
        _ => PackageKind::Portable,
    };
    let root = install::default_data_root().map_err(|err| err.to_string())?;
    install::install_verified_file(
        &root,
        id,
        &meta.tag,
        &meta.asset_name,
        kind,
        None,
        &dest,
        &meta.sha256,
        |_, _| {},
    )
    .map_err(|err| err.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn fetch_all(client: &reqwest::blocking::Client, targets: &[FetchTarget], events: &Sender<Event>) {
    let mut blocked: Option<String> = None;
    for target in targets {
        if let Some(message) = &blocked {
            let _ = events.send(Event::Fetched {
                id: target.id.clone(),
                outcome: FetchOutcome::RateLimited(message.clone()),
            });
            continue;
        }
        let (outcome, rate) = github::fetch_latest(client, &target.repo);
        let _ = events.send(Event::Rate(rate));
        if let FetchOutcome::RateLimited(message) = &outcome {
            blocked = Some(message.clone());
        }
        if blocked.is_none() {
            let (names, rate) = github::fetch_languages(client, &target.repo);
            if rate.remaining == Some(0) {
                blocked = Some("GitHub API rate limit reached. Try again after the reset.".into());
            }
            let _ = events.send(Event::Rate(rate));
            let _ = events.send(Event::Languages {
                id: target.id.clone(),
                names,
            });
        }
        let _ = events.send(Event::Fetched {
            id: target.id.clone(),
            outcome,
        });
    }
}

fn install_job(
    client: &reqwest::blocking::Client,
    request: InstallRequest,
    events: &Sender<Event>,
) {
    let id = request.app_id.clone();
    let root = match install::default_data_root() {
        Ok(root) => root,
        Err(err) => {
            let _ = events.send(Event::Installed {
                id,
                result: Err(err.to_string()),
            });
            return;
        }
    };
    let progress_id = id.clone();
    let tx = events.clone();
    let result = install::install_package(&root, &request, client, |message, fraction| {
        let _ = tx.send(Event::Progress {
            id: progress_id.clone(),
            message: message.to_string(),
            fraction,
        });
    });
    let _ = events.send(Event::Installed {
        id,
        result: result.map(|_| ()).map_err(|err| err.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> NexusApp {
        let (jobs, _) = mpsc::channel();
        let (central_jobs, _) = mpsc::channel();
        let (_, events) = mpsc::channel();
        NexusApp {
            catalog_path: PathBuf::from("catalog/apps.toml"),
            show_supported: false,
            cards: Vec::new(),
            catalog_error: None,
            rate: None,
            limited_detail: None,
            banner: None,
            status: "Checking tool…".into(),
            data_root: Ok(PathBuf::from("data")),
            jobs,
            central_jobs,
            events,
            confirm: None,
            dialog_error: None,
            children: Vec::new(),
            central: central_panel::CentralPanel::new(None),
        }
    }

    #[test]
    fn supported_projects_render_as_links_on_their_own_page() {
        let mut app = app();
        let catalog = catalog::parse_catalog("[[app]]\nid='support'\nname='Supported Test'\nkind='supported'\nhomepage='https://example.com'\nsummary='A project supported by Straconis Software.'").unwrap();
        app.cards = catalog.apps.into_iter().map(Card::from_entry).collect();
        app.show_supported = true;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.grid(ui);
            });
        });
        let has = |label: &str| {
            output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
        };
        assert!(has("SUPPORTED TEST"));
        assert!(has("VISIT PROJECT"));
        assert!(!has("COMING SOON"));
        assert!(!has("INSTALL"));
    }

    #[test]
    fn grid_cards_stack_text_vertically_and_stay_inside_window() {
        for width in [780.0, 1120.0] {
            let mut app = app();
            let catalog = catalog::parse_catalog("[[app]]\nid='one'\nname='Tool One'\nsummary='Capture and organize screenshots for your projects.'\n[[app]]\nid='two'\nname='Tool Two'\nsummary='Review sequences of captured images.'").unwrap();
            app.cards = catalog.apps.into_iter().map(Card::from_entry).collect();
            app.cards[0].languages = vec!["Python".into(), "Shell".into()];
            app.cards[0].status_stamp = Some(nexus_protocol::StatusStamp {
                id: "development".into(),
                name: "In development".into(),
                color: "#f2bc56".into(),
            });
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut output = None;
            for _ in 0..2 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.grid(ui);
                        });
                    },
                ));
            }
            let output = output.unwrap();
            let text = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                        _ => None,
                    })
                    .unwrap()
            };
            let name = text("TOOL ONE");
            assert!(text("IN DEVELOPMENT").pos.y > name.pos.y);
            assert!(text("Python").pos.y > name.pos.y);
            assert!(text("Shell").pos.y > name.pos.y);
            let summary = text("Capture and organize screenshots for your projects.");
            assert!(summary.pos.y > name.pos.y);
            assert!((summary.pos.x - name.pos.x).abs() < 2.0);
            assert!(text("TOOL TWO").pos.x > name.pos.x + 200.0);
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape {
                    assert!(
                        text.pos.x + text.galley.size().x <= width + 1.0,
                        "text escaped the window"
                    );
                }
            }
        }
    }

    #[test]
    fn central_catalog_replaces_local_entries_and_respects_unpublish() {
        let mut app = app();
        let catalog = catalog::parse_catalog("[[app]]\nid = 'tool'\nname = 'Local Tool'").unwrap();
        app.cards.push(Card::from_entry(catalog.apps[0].clone()));
        app.merge_central(vec![CatalogApp {
            id: "tool".into(),
            name: "Central Tool".into(),
            kind: "app".into(),
            summary: String::new(),
            project_status: None,
            status_stamp: None,
            category: None,
            homepage: None,
            repo: None,
            visibility: Visibility::Public,
        }]);
        assert_eq!(app.cards.len(), 1);
        assert_eq!(app.cards[0].entry.name, "Central Tool");
        app.merge_central(Vec::new());
        assert!(app.cards.is_empty());
    }

    #[test]
    fn repository_link_exists_before_a_release() {
        let catalog =
            catalog::parse_catalog("[[app]]\nid = 'tool'\nname = 'Tool'\nrepo = 'Straconis/tool'")
                .unwrap();
        let card = Card::from_entry(catalog.apps[0].clone());
        assert_eq!(
            repository_url(&card).as_deref(),
            Some("https://github.com/Straconis/tool")
        );
    }

    #[test]
    fn installed_build_keeps_launch_button_without_latest_asset() {
        let app = app();
        let catalog = catalog::parse_catalog("[[app]]\nid = 'tool'\nname = 'Tool'").unwrap();
        let card = Card::from_entry(catalog.apps[0].clone());
        let live = Live {
            tag: "v2.0.0".into(),
            html_url: String::new(),
            asset: None,
            install: InstallState::Ready(install::InstallRecord {
                schema: install::RECORD_SCHEMA,
                app_id: "tool".into(),
                version: "v1.0.0".into(),
                asset_name: "tool".into(),
                sha256: None,
                kind: PackageKind::Portable,
                install_dir: PathBuf::from("data/tool"),
                launch_path: Some(PathBuf::from("data/tool/run")),
                installer_path: None,
                installed_at: String::new(),
            }),
            busy: None,
        };
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.live_card(ui, &card, &live);
            });
        });
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "LAUNCH")
        }));
        assert!(!output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "UPDATE")
        }));
    }
}
