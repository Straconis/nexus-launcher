//! Admin-only catalog conveniences. Repository text is source material, never instructions.
use crate::{
    error::ApiFailure,
    routes::{require_admin, AppState},
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use nexus_protocol::{CategoryChange, RepoPreview, RepoPreviewRequest};

fn invalid(message: &'static str) -> ApiFailure {
    ApiFailure::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

pub async fn categories(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, ApiFailure> {
    require_admin(&state, &headers)?;
    Ok(Json(state.db.categories()?))
}

pub async fn change_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CategoryChange>,
) -> Result<Json<Vec<String>>, ApiFailure> {
    require_admin(&state, &headers)?;
    let name = body.name.trim();
    let replacement = body.replacement.as_deref().map(str::trim);
    if name.len() > 60
        || replacement.is_some_and(|v| {
            v.is_empty()
                || v.len() > 60
                || v.chars().any(char::is_control)
                || v.split(" / ").any(|part| part.trim().is_empty())
        })
        || name.chars().any(char::is_control)
        || (name.is_empty() && replacement.is_none())
    {
        return Err(invalid("category names must be 1 to 60 characters"));
    }
    state.db.change_category(name, replacement)?;
    Ok(Json(state.db.categories()?))
}

pub async fn repo_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RepoPreviewRequest>,
) -> Result<Json<RepoPreview>, ApiFailure> {
    require_admin(&state, &headers)?;
    let repo =
        crate::policy::parse_repo(&body.repo).map_err(|_| invalid("repo must be owner/name"))?;
    let client = reqwest::Client::builder()
        .user_agent("nexus-server")
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiFailure::internal())?;
    let request = |path: &str| {
        let mut req = client.get(format!("https://api.github.com/repos/{repo}{path}"));
        if let Some(token) = &state.config.github_release_token {
            req = req.bearer_auth(token);
        }
        req
    };
    let response = request("")
        .send()
        .await
        .map_err(|_| invalid("GitHub repository lookup failed"))?;
    if !response.status().is_success() {
        return Err(invalid("GitHub repository unavailable. Check spelling, rate limits, or server access for private repos."));
    }
    let meta: serde_json::Value = response.json().await.map_err(|_| ApiFailure::internal())?;
    let description = meta["description"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let mut readme = String::new();
    if let Ok(mut response) = request("/readme")
        .header("Accept", "application/vnd.github.raw+json")
        .send()
        .await
    {
        if response.status().is_success() {
            let mut bytes = Vec::new();
            while let Ok(Some(chunk)) = response.chunk().await {
                let remaining = 65536_usize.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                if bytes.len() >= 65536 {
                    break;
                }
            }
            readme = String::from_utf8_lossy(&bytes).into_owned();
        }
    }
    let readme_excerpt = introductory_paragraph(&readme);
    let summary = if description.chars().count() < 45 && !readme_excerpt.is_empty() {
        readme_excerpt.clone()
    } else {
        description.clone()
    };
    Ok(Json(RepoPreview {
        repo,
        description,
        readme_excerpt,
        summary,
    }))
}

fn introductory_paragraph(readme: &str) -> String {
    let mut in_code = false;
    let mut paragraph = String::new();
    for line in readme.lines().chain(std::iter::once("")) {
        let text = line.trim();
        if text.starts_with("```") || text.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if text.is_empty() {
            if paragraph.chars().count() >= 45 {
                return paragraph.chars().take(500).collect();
            }
            paragraph.clear();
            continue;
        }
        if text.starts_with(['#', '<', '!', '[', '|', '-', '*']) || text.contains("![") {
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(&text.replace(['`', '*'], ""));
    }
    String::new()
}

pub async fn status_stamps(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<nexus_protocol::StatusStamp>>, ApiFailure> {
    require_admin(&state, &headers)?;
    Ok(Json(state.db.status_stamps()?))
}

pub async fn change_status_stamp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<nexus_protocol::StatusStampChange>,
) -> Result<Json<Vec<nexus_protocol::StatusStamp>>, ApiFailure> {
    require_admin(&state, &headers)?;
    if body.id.is_empty()
        || body.id.len() > 64
        || !body
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid("status id must be a short slug"));
    }
    if let Some(name) = &mut body.name {
        *name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            return Err(invalid("status label must be 1 to 60 characters"));
        }
        let color = body.color.as_deref().unwrap_or("");
        if color.len() != 7
            || !color.starts_with('#')
            || !color[1..].chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(invalid("status color must be #RRGGBB"));
        }
    }
    state.db.change_status_stamp(&body)?;
    Ok(Json(state.db.status_stamps()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readme_summary_skips_badges_headings_and_commands() {
        assert_eq!(introductory_paragraph("# Tool\n\n![badge](url)\n\n```sh\ncargo run\n```\n\nCapture screenshots automatically and arrange them into a time lapse."), "Capture screenshots automatically and arrange them into a time lapse.");
    }
}
