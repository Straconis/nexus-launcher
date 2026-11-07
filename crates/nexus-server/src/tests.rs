//! HTTP access tests. GitHub is a fake, so this does not prove a live OAuth app.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nexus_protocol::{AdminApp, CatalogApp, DevicePoll, DeviceStart, UpsertApp, Visibility};
use serde_json::json;
use tower::ServiceExt;

use crate::config::Config;
use crate::db::Database;
use crate::github::{
    DeviceCodes, DevicePoll as GhPoll, GhAsset, GhRelease, Github, GithubError, Identity,
};
use crate::routes::{router, AppState};

const CLIENT_SECRET: &str = "client-secret-value";
const RELEASE_TOKEN: &str = "release-token-value";
const DEVICE_CODE: &str = "device-code-secret";
const USER_TOKEN: &str = "gh-user-token-value";

struct Fake {
    user: Mutex<Identity>,
    downloads: Mutex<Vec<String>>,
    polls: AtomicUsize,
    metadata_calls: AtomicUsize,
    bytes: Vec<u8>,
    sha: String,
}

impl Fake {
    fn new(bytes: Vec<u8>) -> Self {
        let sha = crate::github::sha256_hex(&bytes);
        Self {
            user: Mutex::new(Identity {
                id: 42,
                login: "octocat".into(),
            }),
            downloads: Mutex::new(Vec::new()),
            polls: AtomicUsize::new(0),
            metadata_calls: AtomicUsize::new(0),
            bytes,
            sha,
        }
    }

    fn set_user(&self, id: u64, login: &str) {
        *self.user.lock().unwrap() = Identity {
            id,
            login: login.into(),
        };
    }
}

#[async_trait]
impl Github for Fake {
    async fn public_metadata(
        &self,
        _repo: &str,
        _id: Option<&str>,
        _secret: Option<&str>,
        _token: Option<&str>,
    ) -> Result<nexus_protocol::PublicMetadata, GithubError> {
        self.metadata_calls.fetch_add(1, Ordering::SeqCst);
        Ok(nexus_protocol::PublicMetadata {
            release_json: None,
            languages: vec!["Python".into()],
            stale: false,
        })
    }

    async fn start_device(&self, _client_id: &str) -> Result<DeviceCodes, GithubError> {
        Ok(DeviceCodes {
            device_code: DEVICE_CODE.into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://github.com/login/device".into(),
            verification_uri_complete: "https://github.com/login/device?user_code=ABCD-EFGH".into(),
            expires_in: 900,
            interval: 1,
        })
    }

    async fn poll_device(
        &self,
        _client_id: &str,
        client_secret: &str,
        device_code: &str,
    ) -> Result<GhPoll, GithubError> {
        assert_eq!(client_secret, CLIENT_SECRET);
        assert_eq!(device_code, DEVICE_CODE);
        self.polls.fetch_add(1, Ordering::SeqCst);
        Ok(GhPoll::Approved {
            access_token: USER_TOKEN.into(),
        })
    }

    async fn current_user(&self, access_token: &str) -> Result<Identity, GithubError> {
        assert_eq!(access_token, USER_TOKEN);
        Ok(self.user.lock().unwrap().clone())
    }

    async fn user_by_login(&self, login: &str) -> Result<Identity, GithubError> {
        match login {
            "octocat" => Ok(Identity {
                id: 42,
                login: "octocat".into(),
            }),
            "other" => Ok(Identity {
                id: 99,
                login: "other".into(),
            }),
            _ => Err(GithubError::Failed),
        }
    }

    async fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
    ) -> Result<GhRelease, GithubError> {
        if repo == "Straconis/private-tool" {
            assert_eq!(token, Some(RELEASE_TOKEN));
        }
        let api_url = if repo == "Straconis/evil" {
            "http://169.254.169.254/latest".into()
        } else {
            format!("https://api.github.com/repos/{repo}/releases/assets/1")
        };
        let digest = if repo == "Straconis/badsum" {
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()
        } else {
            format!("sha256:{}", self.sha)
        };
        Ok(GhRelease {
            tag: "v1.2.3".into(),
            assets: vec![GhAsset {
                name: "tool-linux-x86_64.tar.gz".into(),
                api_url,
                digest: Some(digest),
                size: self.bytes.len() as u64,
            }],
        })
    }

    async fn fetch_bytes(
        &self,
        url: &str,
        token: Option<&str>,
        _accept: &str,
    ) -> Result<Vec<u8>, GithubError> {
        self.downloads.lock().unwrap().push(url.to_string());
        if url.contains("169.254.169.254") {
            return Err(GithubError::Failed);
        }
        if token.is_some() {
            assert_eq!(token, Some(RELEASE_TOKEN));
        }
        Ok(self.bytes.clone())
    }
}

struct Harness {
    app: axum::Router,
    fake: Arc<Fake>,
}

fn harness() -> Harness {
    let root = std::env::temp_dir().join(format!(
        "nexus-server-test-{}-{}",
        std::process::id(),
        crate::auth::random_id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let db_path = root.join("test.sqlite");
    let fake = Arc::new(Fake::new(b"private-build".to_vec()));
    let config = Config {
        bind: "127.0.0.1:0".into(),
        database_path: db_path.clone(),
        admin_github_ids: [7].into_iter().collect(),
        github_client_id: Some("client-id".into()),
        github_client_secret: Some(CLIENT_SECRET.into()),
        github_release_token: Some(RELEASE_TOKEN.into()),
        session_ttl: Duration::from_secs(3600),
    };
    let rendered = format!("{config:?}");
    assert!(!rendered.contains(CLIENT_SECRET));
    assert!(!rendered.contains(RELEASE_TOKEN));
    let state = AppState {
        db: Arc::new(Database::open(&db_path).unwrap()),
        config: Arc::new(config),
        github: fake.clone(),
    };
    Harness {
        app: router(state),
        fake,
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_redacted(body: &str) {
    for secret in [CLIENT_SECRET, RELEASE_TOKEN, DEVICE_CODE, USER_TOKEN] {
        assert!(!body.contains(secret), "response leaked {secret}: {body}");
    }
}

async fn login(app: &axum::Router) -> (String, bool) {
    let (status, body) = send(app, "POST", "/v1/auth/device/start", None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let start: DeviceStart = serde_json::from_slice(&body).unwrap();
    assert_eq!(start.user_code, "ABCD-EFGH");
    assert_redacted(&text(&body));
    let (status, body) = send(
        app,
        "POST",
        "/v1/auth/device/poll",
        None,
        Some(json!({ "poll_id": start.poll_id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", text(&body));
    let poll: DevicePoll = serde_json::from_slice(&body).unwrap();
    let DevicePoll::Approved {
        session_token,
        account,
    } = poll
    else {
        panic!("expected approval: {}", text(&body));
    };
    let (again, again_body) = send(
        app,
        "POST",
        "/v1/auth/device/poll",
        None,
        Some(json!({ "poll_id": start.poll_id })),
    )
    .await;
    assert_eq!(again, StatusCode::OK);
    let replay: DevicePoll = serde_json::from_slice(&again_body).unwrap();
    assert_eq!(replay, DevicePoll::Expired);
    assert_redacted(&text(&again_body));
    (session_token, account.admin)
}

#[tokio::test]
async fn access_control_blocks_anonymous_and_cross_user_downloads() {
    let harness = harness();
    let app = &harness.app;
    let (status, body) = send(app, "GET", "/v1/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text(&body).contains(nexus_protocol::VERSION));

    let (status, _) = send(app, "GET", "/v1/admin/apps", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(app, "GET", "/v1/catalog", None, None).await;
    assert_eq!(status, StatusCode::OK);

    harness.fake.set_user(42, "octocat");
    let (user_token, admin) = login(app).await;
    assert!(!admin);
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps",
        Some(&user_token),
        Some(json!({
            "id": "tool",
            "name": "Tool",
            "visibility": "private",
            "repo": "Straconis/private-tool",
            "published": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    harness.fake.set_user(7, "steve");
    let (admin_token, admin) = login(app).await;
    assert!(admin);
    let (status, body) = send(
        app,
        "POST",
        "/v1/admin/apps",
        Some(&admin_token),
        Some(json!(UpsertApp {
            id: "tool".into(),
            name: "Private Tool".into(),
            kind: "app".into(),
            summary: "Entitled build".into(),
            project_status: None,
            category: Some("tools".into()),
            homepage: Some("https://example.com/tool".into()),
            banner_url: None,
            repo: Some("Straconis/private-tool".into()),
            visibility: Visibility::Private,
            published: false,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", text(&body));
    let created: AdminApp = serde_json::from_slice(&body).unwrap();
    assert_eq!(created.repo.as_deref(), Some("Straconis/private-tool"));

    let (status, body) = send(app, "GET", "/v1/catalog", Some(&user_token), None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(catalog.is_empty());

    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps/tool/published",
        Some(&admin_token),
        Some(json!({ "published": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(
        app,
        "GET",
        "/v1/apps/tool/download?os=linux&arch=x86_64",
        Some(&user_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        app,
        "GET",
        "/v1/apps/tool/download?os=linux&arch=x86_64",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = send(
        app,
        "POST",
        "/v1/admin/apps/tool/grants",
        Some(&admin_token),
        Some(json!({ "login": "octocat" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", text(&body));
    let (status, body) = send(app, "GET", "/v1/catalog", Some(&user_token), None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(catalog.len(), 1);
    assert!(catalog[0].repo.is_none());
    assert_eq!(catalog[0].visibility, Visibility::Private);
    assert_redacted(&text(&body));

    let (status, body) = send(app, "GET", "/v1/admin/apps", Some(&admin_token), None).await;
    let admin_apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        admin_apps[0].repo.as_deref(),
        Some("Straconis/private-tool")
    );

    let (status, body) = send(
        app,
        "GET",
        "/v1/apps/tool/download?os=linux&arch=x86_64",
        Some(&user_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", text(&body));
    assert_eq!(body, b"private-build");

    harness.fake.set_user(99, "other");
    let (other_token, _) = login(app).await;
    let (status, body) = send(
        app,
        "GET",
        "/v1/apps/tool/release?os=linux&arch=x86_64",
        Some(&other_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_redacted(&text(&body));
    let (status, _) = send(
        app,
        "GET",
        "/v1/apps/tool/download?os=linux&arch=x86_64",
        Some(&other_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = send(
        app,
        "DELETE",
        "/v1/admin/apps/tool/grants/42",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        app,
        "GET",
        "/v1/apps/tool/download?os=linux&arch=x86_64",
        Some(&user_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = send(app, "GET", "/v1/catalog", Some(&user_token), None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(catalog.is_empty());

    let (status, body) = send(
        app,
        "POST",
        "/v1/admin/apps",
        Some(&admin_token),
        Some(json!({
            "id": "../evil",
            "name": "Nope",
            "visibility": "public",
            "repo": "https://evil.example/x"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_redacted(&text(&body));

    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps",
        Some(&admin_token),
        Some(json!({
            "id": "evil",
            "name": "Evil",
            "visibility": "public",
            "repo": "Straconis/evil",
            "published": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        app,
        "GET",
        "/v1/apps/evil/download?os=linux&arch=x86_64",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(!text(&body).contains("169.254.169.254"));
    assert!(harness
        .fake
        .downloads
        .lock()
        .unwrap()
        .iter()
        .all(|url| !url.contains("169.254.169.254")));

    let (status, body) = send(
        app,
        "POST",
        "/v1/admin/apps",
        Some(&admin_token),
        Some(json!({
            "id": "badsum",
            "name": "Bad",
            "visibility": "public",
            "repo": "Straconis/badsum",
            "published": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", text(&body));
    let (status, body) = send(
        app,
        "GET",
        "/v1/apps/badsum/download?os=linux&arch=x86_64",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(text(&body).contains("checksum"));
    assert_redacted(&text(&body));
}

#[test]
fn selectors_and_urls_reject_non_github_hosts() {
    let asset = GhAsset {
        name: "tool-linux-x86_64.tar.gz".into(),
        api_url: "https://api.github.com/repos/acme/tool/releases/assets/1".into(),
        digest: None,
        size: 1,
    };
    let windows = GhAsset {
        name: "tool-windows-x86_64.zip".into(),
        api_url: "https://api.github.com/repos/acme/tool/releases/assets/2".into(),
        digest: None,
        size: 1,
    };
    let assets = [windows, asset.clone()];
    let picked = crate::github::select_asset(&assets, "linux", "x86_64");
    assert_eq!(picked.unwrap().name, asset.name);
    assert!(crate::github::select_asset(&[asset], "linux", "aarch64").is_none());
    assert!(crate::github::require_github_url("http://169.254.169.254/latest").is_err());
    assert!(crate::github::require_github_url("https://user:pass@github.com/a").is_err());
    assert!(crate::github::require_github_url(
        "https://api.github.com/repos/acme/tool/releases/assets/1"
    )
    .is_ok());
    assert_eq!(
        crate::github::parse_sums(
            "abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabca  tool-linux-x86_64.tar.gz\n",
            "tool-linux-x86_64.tar.gz"
        )
        .as_deref(),
        Some("abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabca")
    );
}

#[tokio::test]
async fn category_management_requires_admin_and_updates_assigned_apps() {
    let harness = harness();
    let app = &harness.app;
    for path in ["/v1/admin/categories", "/v1/admin/repo-preview"] {
        let (status, _) = send(
            app,
            "POST",
            path,
            None,
            Some(if path.ends_with("categories") {
                json!({"name":"", "replacement":"Utilities"})
            } else {
                json!({"repo":"Straconis/tool"})
            }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    harness.fake.set_user(42, "octocat");
    let (member, _) = login(app).await;
    let (status, _) = send(app, "GET", "/v1/admin/categories", Some(&member), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    harness.fake.set_user(7, "steve");
    let (admin, _) = login(app).await;
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/categories",
        Some(&admin),
        Some(json!({"name":"", "replacement":"Utilities"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(app, "POST", "/v1/admin/apps", Some(&admin), Some(json!({"id":"tool", "name":"Tool", "category":"Utilities", "repo":"Straconis/tool", "visibility":"public", "published":true}))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        app,
        "POST",
        "/v1/admin/categories",
        Some(&admin),
        Some(json!({"name":"Utilities", "replacement":"Tools"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&body).unwrap(),
        vec!["Tools"]
    );
    let (_, body) = send(app, "GET", "/v1/admin/apps", Some(&admin), None).await;
    let apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(apps[0].category.as_deref(), Some("Tools"));
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/categories",
        Some(&admin),
        Some(json!({"name":"Tools", "replacement":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/admin/apps", Some(&admin), None).await;
    let apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(apps[0].category, None);
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/repo-preview",
        Some(&admin),
        Some(json!({"repo":"https://evil.test/tool"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn custom_status_stamps_and_supported_project_round_trip() {
    let harness = harness();
    let app = &harness.app;
    let (status, _) = send(app, "GET", "/v1/admin/status-stamps", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    harness.fake.set_user(7, "steve");
    let (admin, _) = login(app).await;
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/status-stamps",
        Some(&admin),
        Some(json!({"id":"prototype", "name":"Seeking funding for prototype", "color":"#44bbdd"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(app, "POST", "/v1/admin/apps", Some(&admin), Some(json!({"id":"supported", "name":"Supported project", "kind":"supported", "summary":"A supported project.", "homepage":"https://example.com", "project_status":"prototype", "visibility":"public", "published":true}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/catalog", None, None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(catalog[0].kind, "supported");
    assert_eq!(catalog[0].status_stamp.as_ref().unwrap().color, "#44bbdd");
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps/supported/status",
        None,
        Some(json!({"project_status":"released"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps/supported/status",
        Some(&admin),
        Some(json!({"project_status":"released"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/catalog", None, None).await;
    let updated: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated[0].project_status.as_deref(), Some("released"));
    assert_eq!(updated[0].homepage.as_deref(), Some("https://example.com/"));
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/apps/supported/status",
        Some(&admin),
        Some(json!({"project_status":"prototype"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/status-stamps",
        Some(&admin),
        Some(json!({"id":"prototype", "name":"Prototype funding", "color":"#ffaa11"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/catalog", None, None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        catalog[0].status_stamp.as_ref().unwrap().name,
        "Prototype funding"
    );
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/status-stamps",
        Some(&admin),
        Some(json!({"id":"prototype", "name":null, "color":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/catalog", None, None).await;
    let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
    assert!(catalog[0].status_stamp.is_none());
    assert!(catalog[0].project_status.is_none());
    let (status, _) = send(app, "POST", "/v1/admin/apps", Some(&admin), Some(json!({"id":"invalid", "name":"No link", "kind":"supported", "visibility":"public", "published":true}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[test]
fn old_catalog_schema_migrates_without_losing_entries() {
    let path = std::env::temp_dir().join(format!(
        "nexus-schema-{}-{}.sqlite",
        std::process::id(),
        crate::auth::random_id()
    ));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE apps(id TEXT PRIMARY KEY,name TEXT NOT NULL,summary TEXT NOT NULL DEFAULT '',category TEXT,homepage TEXT,repo TEXT,visibility TEXT NOT NULL,published INTEGER NOT NULL DEFAULT 0,updated_at TEXT NOT NULL); INSERT INTO apps(id,name,visibility,updated_at) VALUES ('old','Existing','public','now');").unwrap();
    }
    let db = Database::open(&path).unwrap();
    let entry = db.app("old").unwrap().unwrap();
    assert_eq!(entry.kind, "app");
    assert!(entry.project_status.is_none());
    assert_eq!(db.status_stamps().unwrap().len(), 3);
    db.change_status_stamp(&nexus_protocol::StatusStampChange {
        id: "released".into(),
        name: None,
        color: None,
    })
    .unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(
        db.status_stamps().unwrap().len(),
        2,
        "removed defaults must not reappear"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn category_subtrees_rename_and_reject_cycles() {
    // Use a separate database because this test exercises persistence directly.
    let path = std::env::temp_dir().join(format!(
        "nexus-categories-{}.sqlite",
        crate::auth::random_id()
    ));
    let db = Database::open(&path).unwrap();
    db.change_category("", Some("Hardware / Electronics / Controllers"))
        .unwrap();
    assert_eq!(
        db.categories().unwrap(),
        [
            "Hardware",
            "Hardware / Electronics",
            "Hardware / Electronics / Controllers"
        ]
    );
    assert!(db
        .change_category("Hardware", Some("Hardware / Electronics / Hardware"))
        .is_err());
    db.change_category("Hardware", Some("Devices")).unwrap();
    assert_eq!(
        db.categories().unwrap(),
        [
            "Devices",
            "Devices / Electronics",
            "Devices / Electronics / Controllers"
        ]
    );
    db.change_category("Devices / Electronics", None).unwrap();
    assert_eq!(db.categories().unwrap(), ["Devices"]);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn supported_project_types_are_independent_of_tool_categories() {
    let harness = harness();
    let app = &harness.app;
    let (status, _) = send(app, "GET", "/v1/admin/project-types", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    harness.fake.set_user(7, "steve");
    let (admin, _) = login(app).await;
    for (id, kind) in [("tool", "app"), ("partner", "supported")] {
        let (status, _) = send(app, "POST", "/v1/admin/apps", Some(&admin), Some(json!({"id":id,"kind":kind,"name":id,"visibility":"public","homepage":"https://example.com","category":"Podcast","published":true}))).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/project-types",
        Some(&admin),
        Some(json!({"name":"Podcast","replacement":"Horror podcast"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/admin/apps", Some(&admin), None).await;
    let apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        apps.iter()
            .find(|app| app.id == "tool")
            .unwrap()
            .category
            .as_deref(),
        Some("Podcast")
    );
    assert_eq!(
        apps.iter()
            .find(|app| app.id == "partner")
            .unwrap()
            .category
            .as_deref(),
        Some("Horror podcast")
    );
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/categories",
        Some(&admin),
        Some(json!({"name":"Podcast","replacement":"Audio tools"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/admin/apps", Some(&admin), None).await;
    let apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        apps.iter()
            .find(|app| app.id == "partner")
            .unwrap()
            .category
            .as_deref(),
        Some("Horror podcast")
    );
    let (status, _) = send(
        app,
        "POST",
        "/v1/admin/project-types",
        Some(&admin),
        Some(json!({"name":"Horror podcast","replacement":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(app, "GET", "/v1/admin/apps", Some(&admin), None).await;
    let apps: Vec<AdminApp> = serde_json::from_slice(&body).unwrap();
    assert!(apps
        .iter()
        .find(|app| app.id == "partner")
        .unwrap()
        .category
        .is_none());
    assert_eq!(
        apps.iter()
            .find(|app| app.id == "tool")
            .unwrap()
            .category
            .as_deref(),
        Some("Audio tools")
    );
}

#[tokio::test]
async fn summaries_longer_than_400_chars_save_and_use_unicode_character_limits() {
    let harness = harness();
    let app = &harness.app;
    harness.fake.set_user(7, "steve");
    let (admin, _) = login(app).await;
    for summary in ["x".repeat(407), "🎯".repeat(1_001)] {
        let (status, body) = send(app,"POST","/v1/admin/apps",Some(&admin),Some(json!({"id":"summary-check","name":"Summary check","summary":summary,"visibility":"public","published":true}))).await;
        assert_eq!(status, StatusCode::OK, "{}", text(&body));
        let (_, body) = send(app, "GET", "/v1/catalog", None, None).await;
        let catalog: Vec<CatalogApp> = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            catalog
                .iter()
                .find(|app| app.id == "summary-check")
                .unwrap()
                .summary,
            summary
        );
    }
    let (status, body) = send(app,"POST","/v1/admin/apps",Some(&admin),Some(json!({"id":"summary-check","name":"Summary check","summary":"x".repeat(4_001),"visibility":"public"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text(&body).contains("4,000"));
}

#[tokio::test]
async fn public_metadata_is_cached_but_unpublishing_revokes_access() {
    let harness = harness();
    harness.fake.set_user(7, "steve");
    let (token, _) = login(&harness.app).await;
    let mut entry = json!({"id":"cached", "name":"Cached", "repo":"Straconis/cached", "visibility":"public", "published":true});
    assert_eq!(
        send(
            &harness.app,
            "POST",
            "/v1/admin/apps",
            Some(&token),
            Some(entry.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    for _ in 0..2 {
        let (status, body) = send(
            &harness.app,
            "GET",
            "/v1/apps/cached/public-metadata",
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(text(&body).contains("Python"));
        assert_redacted(&text(&body));
    }
    assert_eq!(harness.fake.metadata_calls.load(Ordering::SeqCst), 1);
    entry["published"] = json!(false);
    assert_eq!(
        send(
            &harness.app,
            "POST",
            "/v1/admin/apps",
            Some(&token),
            Some(entry)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_ne!(
        send(
            &harness.app,
            "GET",
            "/v1/apps/cached/public-metadata",
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(harness.fake.metadata_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tile_banners_round_trip_validate_and_preserve_older_client_saves() {
    let harness = harness();
    let app = &harness.app;
    harness.fake.set_user(7, "steve");
    let (admin, _) = login(app).await;
    for kind in ["app", "supported"] {
        let mut body = json!({
            "id": kind, "name": "Banner project", "kind": kind,
            "homepage": "https://example.com", "visibility": "public", "published": true,
            "banner_url": "https://example.com/wp-content/uploads/banner.png"
        });
        let (status, _) = send(
            app,
            "POST",
            "/v1/admin/apps",
            Some(&admin),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        body.as_object_mut().unwrap().remove("banner_url");
        let (status, _) = send(
            app,
            "POST",
            "/v1/admin/apps",
            Some(&admin),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for path in ["/v1/catalog", "/v1/admin/apps"] {
            let (_, saved) = send(app, "GET", path, Some(&admin), None).await;
            let saved: serde_json::Value = serde_json::from_slice(&saved).unwrap();
            let item = saved
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == kind)
                .unwrap();
            assert_eq!(
                item["banner_url"],
                "https://example.com/wp-content/uploads/banner.png"
            );
        }
        for invalid in [
            "http://example.com/banner.png",
            "https://user:password@example.com/banner.png",
            "file:///tmp/banner.png",
        ] {
            body["banner_url"] = json!(invalid);
            let (status, _) = send(
                app,
                "POST",
                "/v1/admin/apps",
                Some(&admin),
                Some(body.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        body["banner_url"] = json!("");
        let (status, _) = send(app, "POST", "/v1/admin/apps", Some(&admin), Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let (_, saved) = send(app, "GET", "/v1/catalog", None, None).await;
        let saved: Vec<CatalogApp> = serde_json::from_slice(&saved).unwrap();
        assert!(saved
            .iter()
            .find(|item| item.id == kind)
            .unwrap()
            .banner_url
            .is_none());
    }
}
