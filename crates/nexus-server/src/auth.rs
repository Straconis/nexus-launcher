//! GitHub device flow, finished on the server, then a local session.
//!
//! The device code and OAuth client secret stay here. The desktop only receives
//! a poll id and, once, a session token. GitHub's user token is dropped after
//! the numeric user id is read.

use std::time::Duration;

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::{Database, NewFlow};
use crate::error::ApiFailure;
use crate::github::{DevicePoll, Github};
use nexus_protocol::{Account, DevicePoll as PollResponse, DeviceStart};

pub fn random_id() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).expect("os random");
    hex::encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

pub fn stamp(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn parse_stamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%SZ")
                .ok()
                .map(|time| time.and_utc())
        })
}

pub async fn start_device(
    db: &Database,
    config: &Config,
    github: &dyn Github,
) -> Result<DeviceStart, ApiFailure> {
    let (Some(client_id), Some(_)) = (
        config.github_client_id.as_deref(),
        config.github_client_secret.as_deref(),
    ) else {
        return Err(ApiFailure::auth_unconfigured());
    };
    let codes = github.start_device(client_id).await.map_err(|_| {
        ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "device_start_failed",
            "GitHub did not start a login",
        )
    })?;
    let poll_id = random_id();
    let created = now();
    let expires = created + Duration::from_secs(codes.expires_in.min(15 * 60));
    db.insert_flow(&NewFlow {
        poll_hash: &token_hash(&poll_id),
        device_code: &codes.device_code,
        interval_seconds: codes.interval.max(1) as i64,
        expires_at: &stamp(expires),
        next_poll_at: &stamp(created),
    })?;
    Ok(DeviceStart {
        poll_id,
        user_code: codes.user_code,
        verification_uri: codes.verification_uri,
        verification_uri_complete: codes.verification_uri_complete,
        interval_seconds: codes.interval.max(1),
        expires_in: codes.expires_in,
    })
}

pub async fn poll_device(
    db: &Database,
    config: &Config,
    github: &dyn Github,
    poll_id: &str,
) -> Result<PollResponse, ApiFailure> {
    if poll_id.len() < 32 || poll_id.len() > 128 {
        return Ok(PollResponse::Expired);
    }
    let (Some(client_id), Some(client_secret)) = (
        config.github_client_id.as_deref(),
        config.github_client_secret.as_deref(),
    ) else {
        return Err(ApiFailure::auth_unconfigured());
    };
    let hash = token_hash(poll_id);
    let Some(flow) = db.flow(&hash)? else {
        return Ok(PollResponse::Expired);
    };
    let current = now();
    let Some(expires) = parse_stamp(&flow.expires_at) else {
        db.delete_flow(&hash)?;
        return Ok(PollResponse::Expired);
    };
    if current >= expires {
        db.delete_flow(&hash)?;
        return Ok(PollResponse::Expired);
    }
    if let Some(next_poll) = parse_stamp(&flow.next_poll_at) {
        if current < next_poll {
            return Ok(PollResponse::Pending {
                interval_seconds: flow.interval_seconds.max(1) as u64,
            });
        }
    }
    let outcome = github
        .poll_device(client_id, client_secret, &flow.device_code)
        .await
        .map_err(|_| {
            ApiFailure::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "device_poll_failed",
                "GitHub did not answer the login check",
            )
        })?;
    match outcome {
        DevicePoll::Pending => {
            let interval = flow.interval_seconds.max(1);
            db.touch_flow(
                &hash,
                interval,
                &stamp(current + Duration::from_secs(interval as u64)),
            )?;
            Ok(PollResponse::Pending {
                interval_seconds: interval as u64,
            })
        }
        DevicePoll::SlowDown => {
            let interval = (flow.interval_seconds + 5).clamp(5, 60);
            db.touch_flow(
                &hash,
                interval,
                &stamp(current + Duration::from_secs(interval as u64)),
            )?;
            Ok(PollResponse::SlowDown {
                interval_seconds: interval as u64,
            })
        }
        DevicePoll::Denied => {
            db.delete_flow(&hash)?;
            Ok(PollResponse::Denied)
        }
        DevicePoll::Expired => {
            db.delete_flow(&hash)?;
            Ok(PollResponse::Expired)
        }
        DevicePoll::Approved { access_token } => {
            let identity = github.current_user(&access_token).await.map_err(|_| {
                ApiFailure::new(
                    axum::http::StatusCode::BAD_GATEWAY,
                    "identity_failed",
                    "GitHub did not return the signed-in user",
                )
            })?;
            drop(access_token);
            db.delete_flow(&hash)?;
            let account = finish_login(db, config, identity.id, &identity.login)?;
            let session_token = random_id();
            let created = stamp(current);
            let expires = stamp(current + config.session_ttl);
            db.insert_session(
                &token_hash(&session_token),
                account.github_id,
                &created,
                &expires,
            )?;
            Ok(PollResponse::Approved {
                session_token,
                account,
            })
        }
    }
}

fn finish_login(
    db: &Database,
    config: &Config,
    github_id: u64,
    login: &str,
) -> Result<Account, ApiFailure> {
    db.upsert_user(github_id, login, &stamp(now()))?;
    Ok(Account {
        github_id,
        login: login.to_string(),
        admin: config.is_admin(github_id),
    })
}

pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

pub struct Caller {
    pub github_id: u64,
    pub login: String,
    pub admin: bool,
}

pub fn authenticate(
    db: &Database,
    config: &Config,
    headers: &HeaderMap,
) -> Result<Caller, ApiFailure> {
    let Some(token) = bearer(headers) else {
        return Err(ApiFailure::unauthorized());
    };
    let Some(session) = db.session(&token_hash(token))? else {
        return Err(ApiFailure::unauthorized());
    };
    let Some(expires) = parse_stamp(&session.expires_at) else {
        db.delete_session(&token_hash(token))?;
        return Err(ApiFailure::unauthorized());
    };
    if now() >= expires {
        db.delete_session(&token_hash(token))?;
        return Err(ApiFailure::unauthorized());
    }
    Ok(Caller {
        github_id: session.github_id,
        login: session.login,
        admin: config.is_admin(session.github_id),
    })
}

pub fn logout(db: &Database, headers: &HeaderMap) -> Result<(), ApiFailure> {
    if let Some(token) = bearer(headers) {
        db.delete_session(&token_hash(token))?;
    }
    Ok(())
}
