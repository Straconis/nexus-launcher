//! HTTP API. Every catalog, release, and download handler checks access itself.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use nexus_protocol::{
    Account, AdminApp, CatalogApp, DevicePoll as PollBody, DevicePollRequest, Grant, GrantRequest,
    Health, PublishRequest, ReleaseMeta, UpsertApp, Visibility, VERSION,
};
use serde::Deserialize;

use crate::auth::{self, Caller};
use crate::config::Config;
use crate::db::{AppRow, Database};
use crate::error::ApiFailure;
use crate::github::{self, Github};
use crate::policy;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub config: Arc<Config>,
    pub github: Arc<dyn Github>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route(
            "/v1/admin/project-types",
            get(crate::admin_tools::project_types).post(crate::admin_tools::change_project_type),
        )
        .route(
            "/v1/admin/status-stamps",
            get(crate::admin_tools::status_stamps).post(crate::admin_tools::change_status_stamp),
        )
        .route("/v1/auth/device/start", post(device_start))
        .route("/v1/auth/device/poll", post(device_poll))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/account", get(account))
        .route("/v1/catalog", get(catalog))
        .route("/v1/apps/{id}/release", get(release))
        .route("/v1/apps/{id}/languages", get(languages))
        .route("/v1/apps/{id}/download", get(download))
        .route(
            "/v1/admin/categories",
            get(crate::admin_tools::categories).post(crate::admin_tools::change_category),
        )
        .route(
            "/v1/admin/repo-preview",
            post(crate::admin_tools::repo_preview),
        )
        .route("/v1/admin/apps", get(admin_list).post(admin_upsert))
        .route("/v1/admin/apps/{id}", axum::routing::delete(admin_delete))
        .route("/v1/admin/apps/{id}/published", post(admin_publish))
        .route("/v1/admin/apps/{id}/status", post(admin_project_status))
        .route(
            "/v1/admin/apps/{id}/grants",
            get(admin_grants).post(admin_grant),
        )
        .route(
            "/v1/admin/apps/{id}/grants/{github_id}",
            axum::routing::delete(admin_revoke),
        )
        .with_state(state)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: VERSION.into(),
    })
}

async fn device_start(
    State(state): State<AppState>,
) -> Result<Json<nexus_protocol::DeviceStart>, ApiFailure> {
    Ok(Json(
        auth::start_device(&state.db, &state.config, state.github.as_ref()).await?,
    ))
}

async fn device_poll(
    State(state): State<AppState>,
    Json(body): Json<DevicePollRequest>,
) -> Result<Json<PollBody>, ApiFailure> {
    Ok(Json(
        auth::poll_device(
            &state.db,
            &state.config,
            state.github.as_ref(),
            &body.poll_id,
        )
        .await?,
    ))
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiFailure> {
    auth::logout(&state.db, &headers)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn account(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Account>, ApiFailure> {
    let caller = auth::authenticate(&state.db, &state.config, &headers)?;
    Ok(Json(account_of(&caller)))
}

async fn catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CatalogApp>>, ApiFailure> {
    let caller = optional_caller(&state, &headers)?;
    let apps = state.db.apps()?;
    let stamps = state.db.status_stamps()?;
    let mut visible = Vec::new();
    for app in apps {
        if !can_see(&state, caller.as_ref(), &app)? {
            continue;
        }
        visible.push(catalog_view(&app, &stamps));
    }
    Ok(Json(visible))
}

#[derive(Deserialize)]
struct PlatformQuery {
    os: String,
    arch: String,
}

async fn release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<PlatformQuery>,
) -> Result<Json<ReleaseMeta>, ApiFailure> {
    let caller = optional_caller(&state, &headers)?;
    let app = load_visible(&state, caller.as_ref(), &id).await?;
    let meta = metadata_for(&state, &app, &query.os, &query.arch).await?;
    Ok(Json(meta))
}

async fn languages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<String>>, ApiFailure> {
    let caller = optional_caller(&state, &headers)?;
    let app = load_visible(&state, caller.as_ref(), &id).await?;
    let repo = app.repo.as_deref().ok_or_else(ApiFailure::not_found)?;
    let repo = policy::parse_repo(repo).map_err(|_| ApiFailure::not_found())?;
    let client = reqwest::Client::builder()
        .user_agent("nexus-server")
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiFailure::internal())?;
    let mut request = client.get(format!("https://api.github.com/repos/{repo}/languages"));
    if let Some(token) = release_token(&state, &app) {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|_| ApiFailure::internal())?;
    if !response.status().is_success() {
        return Ok(Json(Vec::new()));
    }
    let counts = response.json().await.map_err(|_| ApiFailure::internal())?;
    Ok(Json(nexus_protocol::top_languages(counts)))
}

async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<PlatformQuery>,
) -> Result<axum::response::Response, ApiFailure> {
    let caller = optional_caller(&state, &headers)?;
    let app = load_visible(&state, caller.as_ref(), &id).await?;
    let (meta, api_url) = asset_for(&state, &app, &query.os, &query.arch).await?;
    let bytes = github::download_asset(
        state.github.as_ref(),
        &meta,
        &api_url,
        release_token(&state, &app),
    )
    .await?;
    into_download(&meta, bytes)
}

async fn admin_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminApp>>, ApiFailure> {
    require_admin(&state, &headers)?;
    let apps = state.db.apps()?;
    Ok(Json(apps.iter().map(AdminApp::from).collect()))
}

async fn admin_upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertApp>,
) -> Result<(StatusCode, Json<AdminApp>), ApiFailure> {
    require_admin(&state, &headers)?;
    let app = validate_upsert(body)?;
    if state
        .db
        .app(&app.id)?
        .is_some_and(|existing| existing.kind != app.kind)
    {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "id_in_use",
            "that ID belongs to another listing type; choose a different ID",
        ));
    }
    if app.project_status.as_ref().is_some_and(|id| {
        !state
            .db
            .status_stamps()
            .unwrap_or_default()
            .iter()
            .any(|stamp| &stamp.id == id)
    }) {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_status",
            "choose a configured status stamp",
        ));
    }
    if app.kind == "supported"
        && app
            .category
            .as_ref()
            .is_some_and(|kind| !state.db.project_types().unwrap_or_default().contains(kind))
    {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_project_type",
            "choose a configured supported-project type",
        ));
    }
    state.db.upsert_app(&app, &auth::stamp(auth::now()))?;
    Ok((StatusCode::OK, Json(AdminApp::from(&app))))
}

#[derive(Deserialize)]
struct ProjectStatusBody {
    project_status: Option<String>,
}

async fn admin_project_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ProjectStatusBody>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    require_admin(&state, &headers)?;
    if let Some(status) = &body.project_status {
        if !state
            .db
            .status_stamps()?
            .iter()
            .any(|stamp| &stamp.id == status)
        {
            return Err(ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_status",
                "choose a configured status stamp",
            ));
        }
    }
    if !state.db.set_project_status(
        &id,
        body.project_status.as_deref(),
        &auth::stamp(auth::now()),
    )? {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(serde_json::json!({"saved":true})))
}

async fn admin_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiFailure> {
    require_admin(&state, &headers)?;
    let id = policy::parse_app_id(&id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    if !state.db.delete_app(&id)? {
        return Err(ApiFailure::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PublishRequest>,
) -> Result<StatusCode, ApiFailure> {
    require_admin(&state, &headers)?;
    let id = policy::parse_app_id(&id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    if !state
        .db
        .set_published(&id, body.published, &auth::stamp(auth::now()))?
    {
        return Err(ApiFailure::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_grants(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<Grant>>, ApiFailure> {
    require_admin(&state, &headers)?;
    let id = policy::parse_app_id(&id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    if state.db.app(&id)?.is_none() {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(state.db.grants_for_app(&id)?))
}

async fn admin_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<GrantRequest>,
) -> Result<(StatusCode, Json<Grant>), ApiFailure> {
    require_admin(&state, &headers)?;
    let id = policy::parse_app_id(&id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    if state.db.app(&id)?.is_none() {
        return Err(ApiFailure::not_found());
    }
    let login = policy::parse_login(&body.login).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_login",
            "login must be a single GitHub username",
        )
    })?;
    let identity = state.github.user_by_login(&login).await.map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "user_lookup_failed",
            "that GitHub username could not be resolved",
        )
    })?;
    state
        .db
        .grant(identity.id, &identity.login, &id, &auth::stamp(auth::now()))?;
    Ok((
        StatusCode::OK,
        Json(Grant {
            github_id: identity.id,
            login: identity.login,
        }),
    ))
}

async fn admin_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, github_id)): Path<(String, u64)>,
) -> Result<StatusCode, ApiFailure> {
    require_admin(&state, &headers)?;
    let id = policy::parse_app_id(&id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    if !state.db.revoke(github_id, &id)? {
        return Err(ApiFailure::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<Caller, ApiFailure> {
    let caller = auth::authenticate(&state.db, &state.config, headers)?;
    if !caller.admin {
        return Err(ApiFailure::forbidden());
    }
    Ok(caller)
}

fn optional_caller(state: &AppState, headers: &HeaderMap) -> Result<Option<Caller>, ApiFailure> {
    if auth::bearer(headers).is_none() {
        return Ok(None);
    }
    auth::authenticate(&state.db, &state.config, headers).map(Some)
}

fn can_see(state: &AppState, caller: Option<&Caller>, app: &AppRow) -> Result<bool, ApiFailure> {
    if !app.published {
        return Ok(false);
    }
    match app.visibility {
        Visibility::Public => Ok(true),
        Visibility::Private => match caller {
            Some(caller) if caller.admin || state.db.has_grant(caller.github_id, &app.id)? => {
                Ok(true)
            }
            _ => Ok(false),
        },
    }
}

async fn load_visible(
    state: &AppState,
    caller: Option<&Caller>,
    id: &str,
) -> Result<AppRow, ApiFailure> {
    let id = policy::parse_app_id(id).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    let Some(app) = state.db.app(&id)? else {
        return Err(ApiFailure::not_found());
    };
    if !app.published {
        return Err(ApiFailure::not_found());
    }
    match app.visibility {
        Visibility::Public => Ok(app),
        Visibility::Private => match caller {
            Some(caller) if caller.admin || state.db.has_grant(caller.github_id, &app.id)? => {
                Ok(app)
            }
            Some(_) => Err(ApiFailure::forbidden()),
            None => Err(ApiFailure::unauthorized()),
        },
    }
}

fn catalog_view(app: &AppRow, stamps: &[nexus_protocol::StatusStamp]) -> CatalogApp {
    CatalogApp {
        id: app.id.clone(),
        name: app.name.clone(),
        kind: app.kind.clone(),
        summary: app.summary.clone(),
        project_status: app.project_status.clone(),
        status_stamp: app
            .project_status
            .as_ref()
            .and_then(|id| stamps.iter().find(|stamp| &stamp.id == id))
            .cloned(),
        category: app.category.clone(),
        homepage: app.homepage.clone(),
        repo: if app.visibility == Visibility::Public {
            app.repo.clone()
        } else {
            None
        },
        visibility: app.visibility,
    }
}

fn account_of(caller: &Caller) -> Account {
    Account {
        github_id: caller.github_id,
        login: caller.login.clone(),
        admin: caller.admin,
    }
}

fn validate_upsert(body: UpsertApp) -> Result<AppRow, ApiFailure> {
    let id = policy::parse_app_id(body.id.trim()).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "app id is not a slug",
        )
    })?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "name must be 1 to 80 characters",
        ));
    }
    if body.summary.len() > 400 {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_app",
            "summary is too long",
        ));
    }
    let repo = match policy::clean_optional(body.repo) {
        Some(repo) => Some(policy::parse_repo(&repo).map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_repo",
                "repo must be owner/name on GitHub",
            )
        })?),
        None => None,
    };
    if body.visibility == Visibility::Private && repo.is_none() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_repo",
            "a private app needs an explicit GitHub repo",
        ));
    }
    let homepage = match policy::clean_optional(body.homepage) {
        Some(url) => Some(policy::parse_homepage(&url).map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_homepage",
                "homepage must be an https URL without credentials",
            )
        })?),
        None => None,
    };
    if !matches!(body.kind.as_str(), "app" | "supported") {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_kind",
            "project kind must be app or supported",
        ));
    }
    if body.kind == "supported" && (homepage.is_none() || body.visibility != Visibility::Public) {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_project",
            "supported projects need a public listing and an https project link",
        ));
    }
    let category = policy::clean_optional(body.category).filter(|value| value.len() <= 40);
    Ok(AppRow {
        id,
        name: name.to_string(),
        kind: body.kind,
        summary: body.summary,
        project_status: body.project_status,
        category,
        homepage,
        repo,
        visibility: body.visibility,
        published: body.published,
    })
}

fn release_token<'a>(state: &'a AppState, _app: &AppRow) -> Option<&'a str> {
    state.config.github_release_token.as_deref()
}

async fn metadata_for(
    state: &AppState,
    app: &AppRow,
    os: &str,
    arch: &str,
) -> Result<ReleaseMeta, ApiFailure> {
    let (meta, _) = asset_for(state, app, os, arch).await?;
    Ok(meta)
}

async fn asset_for(
    state: &AppState,
    app: &AppRow,
    os: &str,
    arch: &str,
) -> Result<(ReleaseMeta, String), ApiFailure> {
    let Some(repo) = app.repo.as_deref() else {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "no_repo",
            "this app has no GitHub repo",
        ));
    };
    let repo = policy::parse_repo(repo).map_err(|_| ApiFailure::internal())?;
    let token = release_token(state, app);
    if app.visibility == Visibility::Private && token.is_none() {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "release_token_missing",
            "the server has no release credential for private apps",
        ));
    }
    let release = state
        .github
        .latest_release(&repo, token)
        .await
        .map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_GATEWAY,
                "release_lookup_failed",
                "the release could not be read",
            )
        })?;
    let Some(asset) = github::select_asset(&release.assets, os, arch) else {
        return Err(ApiFailure::new(
            StatusCode::NOT_FOUND,
            "no_asset",
            "no release asset for this platform",
        ));
    };
    let api_url = asset.api_url.clone();
    github::require_github_url(&api_url)?;
    let sha = match github::parse_digest(asset.digest.as_deref()) {
        Some(hex) => hex,
        None => {
            github::checksum_for_asset(state.github.as_ref(), &release.assets, &asset.name, token)
                .await?
        }
    };
    let meta = ReleaseMeta {
        tag: release.tag,
        asset_name: asset.name.clone(),
        size: asset.size,
        sha256: sha,
        kind: if asset.name.to_ascii_lowercase().ends_with(".zip")
            || asset.name.to_ascii_lowercase().ends_with(".tar.gz")
        {
            "archive".into()
        } else {
            "portable".into()
        },
    };
    Ok((meta, api_url))
}

fn into_download(
    meta: &ReleaseMeta,
    bytes: Vec<u8>,
) -> Result<axum::response::Response, ApiFailure> {
    if !meta.asset_name.is_ascii() || !meta.sha256.is_ascii() || !meta.tag.is_ascii() {
        return Err(ApiFailure::internal());
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    headers.insert(
        "x-nexus-asset-name",
        meta.asset_name
            .parse()
            .map_err(|_| ApiFailure::internal())?,
    );
    headers.insert(
        "x-nexus-sha256",
        meta.sha256.parse().map_err(|_| ApiFailure::internal())?,
    );
    headers.insert(
        "x-nexus-tag",
        meta.tag.parse().map_err(|_| ApiFailure::internal())?,
    );
    Ok((headers, bytes).into_response())
}
