//! Dark card grid. Refresh re-reads the catalog and checks GitHub again.

use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, Frame, RichText, Stroke, Vec2};

use crate::catalog::{self, AppEntry, Availability};
use crate::github::{self, FetchOutcome, RateSnapshot, SelectedAsset};
use crate::install::{self, InstallRequest, InstallState};
use crate::model::PackageKind;
use crate::theme::{self, ACCENT, BG, BORDER, MUTED, PANEL, TEXT};

pub fn run(catalog_path: PathBuf) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([780.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "NEXUS LAUNCHER",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(NexusApp::new(catalog_path)))
        }),
    )
}

struct NexusApp {
    catalog_path: PathBuf,
    cards: Vec<Card>,
    catalog_error: Option<String>,
    rate: Option<RateSnapshot>,
    limited_detail: Option<String>,
    banner: Option<String>,
    status: String,
    data_root: Result<PathBuf, String>,
    jobs: Sender<Job>,
    events: Receiver<Event>,
    confirm: Option<PendingInstall>,
    dialog_error: Option<String>,
    children: Vec<Child>,
}

struct Card {
    entry: AppEntry,
    phase: Phase,
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

enum Event {
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
}

enum UiAction {
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
        thread::Builder::new()
            .name("nexus-launcher-net".into())
            .spawn(move || worker_loop(job_rx, event_tx))
            .expect("spawn network thread");
        let mut app = Self {
            catalog_path,
            cards: Vec::new(),
            catalog_error: None,
            rate: None,
            limited_detail: None,
            banner: None,
            status: "Ready".into(),
            data_root: install::default_data_root().map_err(|err| err.to_string()),
            jobs,
            events,
            confirm: None,
            dialog_error: None,
            children: Vec::new(),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.limited_detail = None;
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
                let repo = card.entry.repo.clone()?;
                if matches!(card.phase, Phase::Loading) {
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
        Self { entry, phase }
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
                actions.extend(self.header(ui));
                actions.extend(self.banners(ui));
                actions.extend(self.grid(ui));
            });
        actions.extend(self.dialogs(ctx));
        for action in actions {
            self.handle(action);
        }
        if self.working() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
}

impl NexusApp {
    fn header(&self, ui: &mut egui::Ui) -> Vec<UiAction> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("NEXUS LAUNCHER")
                        .family(theme::orbitron())
                        .size(34.0)
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("STRACONIS DESKTOP TOOLS")
                        .family(theme::orbitron())
                        .size(12.0)
                        .color(MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        if self.cards.is_empty() {
            ui.label(RichText::new("No apps in the catalog.").color(MUTED));
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
                    .spacing(Vec2::new(gap, gap))
                    .show(ui, |ui| {
                        for (index, card) in self.cards.iter().enumerate() {
                            actions.extend(self.card(ui, card, card_w));
                            if (index + 1) % cols == 0 || index + 1 == self.cards.len() {
                                ui.end_row();
                            }
                        }
                    });
            });
        actions
    }

    fn card(&self, ui: &mut egui::Ui, card: &Card, width: f32) -> Vec<UiAction> {
        let mut actions = Vec::new();
        ui.allocate_ui(Vec2::new(width, 360.0), |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .inner_margin(14)
                .corner_radius(8)
                .show(ui, |ui| {
                    ui.set_width(width - 30.0);
                    ui.set_min_height(328.0);
                    if let Some(category) = &card.entry.category {
                        ui.label(
                            RichText::new(category.to_uppercase())
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
        });
        actions
    }

    fn card_body(&self, ui: &mut egui::Ui, card: &Card) -> Vec<UiAction> {
        let mut actions = Vec::new();
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
        let update = installed.is_some_and(|current| github::is_newer_release(&live.tag, current));
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
            None => {
                let _ = cta(ui, "NO BUILD FOR THIS PLATFORM", CtaKind::Disabled, width);
            }
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

    fn footer(&self, ui: &mut egui::Ui) {
        let root = match &self.data_root {
            Ok(path) => path.display().to_string(),
            Err(err) => err.clone(),
        };
        ui.label(
            RichText::new(format!(
                "v0.1.0  ·  {}  ·  {}  ·  {}",
                self.status,
                self.catalog_path.display(),
                root
            ))
            .color(MUTED)
            .size(11.0),
        );
    }

    fn dialogs(&self, ctx: &egui::Context) -> Vec<UiAction> {
        let mut actions = Vec::new();
        if self.confirm.is_some() || self.dialog_error.is_some() {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("nexus-scrim"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(170));
        }
        if let Some(pending) = &self.confirm {
            egui::Window::new(
                RichText::new("CHECKSUM MISSING")
                    .family(theme::orbitron())
                    .color(TEXT),
            )
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(Frame::new().fill(PANEL).stroke(Stroke::new(1.0_f32, ACCENT)).inner_margin(16).corner_radius(8))
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
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
            egui::Window::new(
                RichText::new("SOMETHING WENT WRONG")
                    .family(theme::orbitron())
                    .color(TEXT),
            )
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 40.0])
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(16)
                    .corner_radius(8),
            )
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
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
        _ => None,
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
