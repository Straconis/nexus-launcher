use eframe::egui;
use serde::Deserialize;

const TASKS: [(&str, &str, egui::Color32); 8] = [
    (
        "web",
        "Web interfaces",
        egui::Color32::from_rgb(112, 195, 255),
    ),
    (
        "backend",
        "APIs & services",
        egui::Color32::from_rgb(196, 162, 255),
    ),
    ("bots", "Bots", egui::Color32::from_rgb(249, 145, 200)),
    (
        "automation",
        "Automation & CLI",
        egui::Color32::from_rgb(121, 220, 168),
    ),
    (
        "data",
        "Data & databases",
        egui::Color32::from_rgb(250, 210, 109),
    ),
    (
        "desktop",
        "Desktop apps",
        egui::Color32::from_rgb(107, 220, 219),
    ),
    (
        "systems",
        "Systems & hardware",
        egui::Color32::from_rgb(255, 160, 110),
    ),
    ("games", "Games", egui::Color32::from_rgb(255, 140, 145)),
];

#[derive(Deserialize)]
struct Language {
    name: String,
    tasks: Vec<String>,
    fit: String,
    why: String,
    tradeoff: String,
    example: String,
    docs: String,
}

#[derive(Default)]
pub(super) struct LanguageLibrary {
    search: String,
    task: Option<usize>,
}

impl LanguageLibrary {
    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        let languages = languages();
        ui.heading("What are you building?");
        ui.label("Choose a task, then compare languages and starter ideas. Each card uses the color of its first listed task.");
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("language-library-task")
                .selected_text(self.task.map_or("All tasks", |index| TASKS[index].1))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.task, None, "All tasks");
                    for (index, (_, label, _)) in TASKS.iter().enumerate() {
                        ui.selectable_value(&mut self.task, Some(index), *label);
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.search).hint_text("Find a language or task"),
            );
        });
        let query = self.search.trim().to_lowercase();
        let matches: Vec<_> = languages
            .iter()
            .filter(|row| {
                let tags = TASKS
                    .iter()
                    .filter(|(key, _, _)| row.tasks.iter().any(|task| task == key));
                self.task
                    .is_none_or(|index| row.tasks.iter().any(|task| task == TASKS[index].0))
                    && (query.is_empty()
                        || [&row.name, &row.fit, &row.why, &row.tradeoff, &row.example]
                            .iter()
                            .any(|text| text.to_lowercase().contains(&query))
                        || tags.clone().any(|(key, label, _)| {
                            key.contains(&query) || label.to_lowercase().contains(&query)
                        }))
            })
            .collect();

        ui.label(format!(
            "{} of {} languages",
            matches.len(),
            languages.len()
        ));
        egui::ScrollArea::vertical().id_salt("language-library-results").show(ui, |ui| {
            ui.collapsing(".NET and .NET Framework", |ui| {
                ui.label("C# and Visual Basic are languages. .NET is the runtime, libraries, and SDK that can run their code. Modern .NET is cross-platform, while .NET Framework is the older Windows-only implementation. Some modern .NET app frameworks still require Windows.");
                ui.hyperlink_to(
                    "Microsoft’s .NET platform guide",
                    "https://learn.microsoft.com/en-us/dotnet/standard/choosing-core-framework-server",
                );
            });
            for row in matches {
                let tags = row.tasks.iter().filter_map(|task| {
                    TASKS.iter().find(|(key, _, _)| task == key)
                });
                let accent = tags.clone().next().map_or(ui.visuals().text_color(), |(_, _, color)| *color);
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.55)))
                    .corner_radius(6)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.heading(egui::RichText::new(&row.name).color(accent));
                        ui.horizontal_wrapped(|ui| {
                            for (_, label, color) in tags {
                                ui.label(egui::RichText::new(*label).small().strong().color(*color));
                            }
                        });
                        ui.add_space(4.0);
                        for (label, text) in [
                            ("Good fit", &row.fit),
                            ("Why pick it", &row.why),
                            ("Tradeoff", &row.tradeoff),
                            ("Try building", &row.example),
                        ] {
                            ui.label(format!("{label}: {text}"));
                        }
                        ui.hyperlink_to(format!("{} documentation", row.name), &row.docs);
                    });
            }
        });
    }
}

fn languages() -> &'static [Language] {
    // Snapshot of the public Straconis.dev Language Library, available offline.
    static LANGUAGES: std::sync::OnceLock<Vec<Language>> = std::sync::OnceLock::new();
    LANGUAGES.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/languages.json"))
            .expect("bundled Language Library must be valid")
    })
}
