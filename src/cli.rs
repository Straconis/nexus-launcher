//! Command line: open the window, or `--check` the catalog and exit.

use std::path::PathBuf;

use crate::catalog::{self, Catalog};

pub const HELP: &str = "\
Nexus Launcher — Straconis desktop tools

Usage:
  nexus-launcher [--catalog PATH]
  nexus-launcher --check [PATH]
  nexus-launcher --help

  --check, -c       Parse the catalog and print it. Does not open a window
                    and does not call GitHub.
  --catalog PATH    Use this apps.toml instead of catalog/apps.toml.
  --help, -h        Show this text.

With no flags, the window reads catalog/apps.toml (or the file beside the
executable) and checks GitHub releases.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    Gui { catalog: PathBuf },
    Check { catalog: PathBuf },
    Help,
    Error(String),
}

pub fn parse_args<I, S>(args: I) -> CliAction
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut check = false;
    let mut catalog: Option<PathBuf> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--help" | "-h" => return CliAction::Help,
            "--check" | "-c" => check = true,
            "--catalog" => {
                let Some(path) = iter.next() else {
                    return CliAction::Error("--catalog needs a path".into());
                };
                if catalog.is_some() {
                    return CliAction::Error("catalog path was given twice".into());
                }
                catalog = Some(PathBuf::from(path.as_ref()));
            }
            other if other.starts_with('-') => {
                return CliAction::Error(format!("unknown flag {other}"));
            }
            other => {
                if catalog.is_some() {
                    return CliAction::Error(format!("unexpected argument {other}"));
                }
                catalog = Some(PathBuf::from(other));
            }
        }
    }
    let catalog = catalog::resolve_catalog_path(catalog);
    if check {
        CliAction::Check { catalog }
    } else {
        CliAction::Gui { catalog }
    }
}

pub fn execute(action: CliAction) -> i32 {
    match action {
        CliAction::Help => {
            print!("{HELP}");
            0
        }
        CliAction::Error(message) => {
            eprintln!("error: {message}\n");
            eprint!("{HELP}");
            2
        }
        CliAction::Check { catalog } => match check_catalog(&catalog) {
            Ok(report) => {
                println!("{report}");
                0
            }
            Err(err) => {
                eprintln!("{err}");
                1
            }
        },
        CliAction::Gui { catalog } => launch_gui(catalog),
    }
}

pub fn check_catalog(path: &std::path::Path) -> Result<String, catalog::CatalogError> {
    let catalog = catalog::load_catalog(path)?;
    Ok(render_check(&catalog))
}

pub fn render_check(catalog: &Catalog) -> String {
    let mut lines = vec![format!("catalog ok ({} apps)", catalog.apps.len())];
    for app in &catalog.apps {
        let repo = app.repo.as_deref().unwrap_or("no release repo");
        lines.push(format!("- {}: {} ({repo})", app.id, app.name));
    }
    lines.join("\n")
}

#[cfg(feature = "gui")]
fn launch_gui(catalog: PathBuf) -> i32 {
    match crate::ui::run(catalog) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("gui error: {err}");
            1
        }
    }
}

#[cfg(not(feature = "gui"))]
fn launch_gui(_catalog: PathBuf) -> i32 {
    eprintln!("this build does not include the gui feature");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        match parse_args(["--check", "catalog/apps.toml"]) {
            CliAction::Check { catalog } => {
                assert_eq!(catalog, PathBuf::from("catalog/apps.toml"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(parse_args(["--help"]), CliAction::Help));
        assert!(matches!(parse_args(["--nope"]), CliAction::Error(_)));
        assert!(matches!(parse_args(["--catalog"]), CliAction::Error(_)));
        match parse_args(["--catalog", "other.toml"]) {
            CliAction::Gui { catalog } => assert_eq!(catalog, PathBuf::from("other.toml")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn check_report_lists_seed_apps() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/apps.toml");
        let report = check_catalog(&path).unwrap();
        assert!(report.contains("catalog ok (4 apps)"));
        assert!(report.contains("stat-tracker: Stat Tracker"));
        assert!(report.contains("frameweaver: FrameWeaver"));
    }
}
