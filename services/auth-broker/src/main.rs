use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Registration {
    client_id: String,
    client_secret: String,
    authorize_url: String,
    token_url: String,
    pkce: bool,
}
#[derive(Clone)]
struct Broker {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    registrations: Arc<HashMap<String, Registration>>,
    public_url: String,
    http: reqwest::Client,
}
struct Session {
    provider: String,
    claim_hash: [u8; 32],
    verifier: String,
    expires: Instant,
    state: SessionState,
}
enum SessionState {
    Awaiting,
    Exchanging,
    Ready(Value),
    Denied,
}
struct Failure(StatusCode, &'static str);
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
type ApiResult<T> = std::result::Result<T, Failure>;
fn internal(_: impl std::fmt::Display) -> Failure {
    Failure(StatusCode::INTERNAL_SERVER_ERROR, "broker_error")
}
fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn app(broker: Broker) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/sessions", post(begin))
        .route("/sessions/{id}/claim", post(claim))
        .route("/callback/{provider}", get(callback))
        .route("/refresh", post(refresh))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    header::HeaderValue::from_static("no-store"),
                );
                response.headers_mut().insert(
                    header::REFERRER_POLICY,
                    header::HeaderValue::from_static("no-referrer"),
                );
                response
            },
        ))
        .with_state(broker)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Begin {
    provider: String,
    claim_secret: String,
}
async fn begin(State(b): State<Broker>, Json(input): Json<Begin>) -> ApiResult<Json<Value>> {
    let registration = b
        .registrations
        .get(&input.provider)
        .ok_or(Failure(StatusCode::NOT_FOUND, "provider_not_configured"))?;
    if input.claim_secret.len() < 32 || input.claim_secret.len() > 128 {
        return Err(Failure(StatusCode::BAD_REQUEST, "invalid_claim_secret"));
    }
    let id = random().map_err(internal)?;
    let verifier = random().map_err(internal)?;
    let mut url = url::Url::parse(&registration.authorize_url).map_err(internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &registration.client_id)
        .append_pair(
            "redirect_uri",
            &format!("{}/callback/{}", b.public_url, input.provider),
        )
        .append_pair("state", &id);
    if registration.pkce {
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            )
            .append_pair("code_challenge_method", "S256");
    }
    let mut sessions = b.sessions.lock().map_err(internal)?;
    sessions.retain(|_, s| s.expires > Instant::now());
    if sessions.len() >= 1000 {
        return Err(Failure(StatusCode::SERVICE_UNAVAILABLE, "session_capacity"));
    }
    sessions.insert(
        id.clone(),
        Session {
            provider: input.provider,
            claim_hash: Sha256::digest(input.claim_secret.as_bytes()).into(),
            verifier,
            expires: Instant::now() + Duration::from_secs(600),
            state: SessionState::Awaiting,
        },
    );
    Ok(Json(json!({"id":id,"authorize_url":url.as_str()})))
}
#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(b): State<Broker>,
    Path(provider): Path<String>,
    Query(input): Query<Callback>,
) -> ApiResult<Html<&'static str>> {
    let registration = b
        .registrations
        .get(&provider)
        .ok_or(Failure(StatusCode::NOT_FOUND, "provider_not_configured"))?;
    let verifier = {
        let mut sessions = b.sessions.lock().map_err(internal)?;
        let s = sessions
            .get_mut(&input.state)
            .filter(|s| s.expires > Instant::now() && s.provider == provider)
            .ok_or(Failure(StatusCode::BAD_REQUEST, "invalid_state"))?;
        if !matches!(s.state, SessionState::Awaiting) {
            return Err(Failure(StatusCode::CONFLICT, "callback_already_used"));
        }
        if input.error.is_some() {
            s.state = SessionState::Denied;
            return Ok(Html(
                "Authorization declined. You can close this window and return to GetKeys.",
            ));
        }
        if input.code.as_ref().is_none_or(|s| s.is_empty()) {
            return Err(Failure(StatusCode::BAD_REQUEST, "missing_code"));
        }
        s.state = SessionState::Exchanging;
        s.verifier.clone()
    };
    let mut form = vec![
        ("code", input.code.unwrap()),
        (
            "redirect_uri",
            format!("{}/callback/{provider}", b.public_url),
        ),
    ];
    if registration.pkce {
        form.push(("grant_type", "authorization_code".into()));
        form.push(("code_verifier", verifier));
    }
    let result = exchange(&b, registration, form).await;
    let mut sessions = b.sessions.lock().map_err(internal)?;
    if let Some(s) = sessions.get_mut(&input.state) {
        s.state = match result {
            Ok(tokens) => SessionState::Ready(tokens),
            Err(_) => SessionState::Denied,
        };
    }
    Ok(Html(
        "<!doctype html><meta name=viewport content='width=device-width'><title>GetKeys</title><body style='font:16px system-ui;max-width:440px;margin:15vh auto;padding:24px'><h1>Return to GetKeys</h1><p>The authorization result is ready in your app. You can close this window.</p></body>",
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    claim_secret: String,
}
async fn claim(
    State(b): State<Broker>,
    Path(id): Path<String>,
    Json(input): Json<Claim>,
) -> ApiResult<Response> {
    let sessions = b.sessions.lock().map_err(internal)?;
    let hash: [u8; 32] = Sha256::digest(input.claim_secret.as_bytes()).into();
    let s = sessions
        .get(&id)
        .filter(|s| s.expires > Instant::now() && s.claim_hash == hash)
        .ok_or(Failure(StatusCode::NOT_FOUND, "session_not_found"))?;
    Ok(match &s.state {
        SessionState::Ready(tokens) => Json(tokens.clone()).into_response(),
        SessionState::Denied => {
            return Err(Failure(StatusCode::BAD_REQUEST, "authorization_denied"));
        }
        _ => (StatusCode::ACCEPTED, Json(json!({"status":"pending"}))).into_response(),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refresh {
    provider: String,
    refresh_token: String,
}
async fn refresh(State(b): State<Broker>, Json(input): Json<Refresh>) -> ApiResult<Json<Value>> {
    let r = b
        .registrations
        .get(&input.provider)
        .filter(|r| r.pkce)
        .ok_or(Failure(StatusCode::NOT_FOUND, "refresh_not_supported"))?;
    if input.refresh_token.is_empty() {
        return Err(Failure(StatusCode::BAD_REQUEST, "missing_refresh_token"));
    }
    Ok(Json(
        exchange(
            &b,
            r,
            vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", input.refresh_token),
            ],
        )
        .await?,
    ))
}
async fn exchange(b: &Broker, r: &Registration, mut form: Vec<(&str, String)>) -> ApiResult<Value> {
    let mut request = b.http.post(&r.token_url);
    if r.pkce {
        request = request.basic_auth(&r.client_id, Some(&r.client_secret));
    } else {
        form.push(("client_id", r.client_id.clone()));
        form.push(("client_secret", r.client_secret.clone()));
    }
    let mut response = request
        .form(&form)
        .send()
        .await
        .map_err(|_| Failure(StatusCode::BAD_GATEWAY, "token_exchange_failed"))?;
    if !response.status().is_success() {
        return Err(Failure(StatusCode::BAD_GATEWAY, "token_exchange_rejected"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(internal)? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(Failure(StatusCode::BAD_GATEWAY, "invalid_token_response"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let tokens: Value = serde_json::from_slice(&bytes)
        .map_err(|_| Failure(StatusCode::BAD_GATEWAY, "invalid_token_response"))?;
    if tokens["access_token"].as_str().is_none_or(|s| s.is_empty()) {
        return Err(Failure(StatusCode::BAD_GATEWAY, "missing_access_token"));
    }
    Ok(tokens)
}

#[tokio::main]
async fn main() -> Result<()> {
    let public_url = std::env::var("GETKEYS_BROKER_PUBLIC_URL")
        .context("Set GETKEYS_BROKER_PUBLIC_URL")?
        .trim_end_matches('/')
        .to_owned();
    let url = url::Url::parse(&public_url)?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))),
        "Broker public URL must use HTTPS, or localhost for development"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Use an origin without credentials, path, query, or fragment for the broker URL"
    );
    let mut registrations = HashMap::new();
    for provider in ["supabase", "vercel"] {
        let prefix = provider.to_uppercase();
        if let Ok(client_id) = std::env::var(format!("{prefix}_CLIENT_ID"))
            && !client_id.trim().is_empty()
        {
            let client_secret = std::env::var(format!("{prefix}_CLIENT_SECRET"))
                .context("OAuth client secret is missing")?;
            ensure!(
                !client_id.is_empty() && !client_secret.is_empty(),
                "OAuth registration values must not be empty"
            );
            let (authorize_url, token_url, pkce) = if provider == "supabase" {
                (
                    "https://api.supabase.com/v1/oauth/authorize".into(),
                    "https://api.supabase.com/v1/oauth/token".into(),
                    true,
                )
            } else {
                let slug = std::env::var("VERCEL_INTEGRATION_SLUG")
                    .context("Set VERCEL_INTEGRATION_SLUG")?;
                ensure!(
                    !slug.is_empty()
                        && slug.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                    "Invalid Vercel integration slug"
                );
                (
                    format!("https://vercel.com/integrations/{slug}/new"),
                    "https://api.vercel.com/v2/oauth/access_token".into(),
                    false,
                )
            };
            registrations.insert(
                provider.into(),
                Registration {
                    client_id,
                    client_secret,
                    authorize_url,
                    token_url,
                    pkce,
                },
            );
        }
    }
    ensure!(
        !registrations.is_empty(),
        "Configure at least one OAuth registration. See docs/setup.md."
    );
    let broker = Broker {
        sessions: Arc::new(Mutex::new(HashMap::new())),
        registrations: Arc::new(registrations),
        public_url,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };
    let cleanup = broker.sessions.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Ok(mut sessions) = cleanup.lock() {
                sessions.retain(|_, s| s.expires > Instant::now());
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(
        std::env::var("GETKEYS_BROKER_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into()),
    )
    .await?;
    println!(
        "GetKeys OAuth broker listening on {}",
        listener.local_addr()?
    );
    axum::serve(listener, app(broker))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    fn fixture() -> Broker {
        Broker {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            registrations: Arc::new(HashMap::from([(
                "supabase".into(),
                Registration {
                    client_id: "public-id".into(),
                    client_secret: "server-only".into(),
                    authorize_url: "https://api.supabase.com/v1/oauth/authorize".into(),
                    token_url: "https://api.supabase.com/v1/oauth/token".into(),
                    pkce: true,
                },
            )])),
            public_url: "https://broker.example.com".into(),
            http: reqwest::Client::new(),
        }
    }
    async fn post_json(router: Router, path: &str, value: Value) -> Response {
        router
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn session_is_bound_to_device_and_authorization_uses_pkce() {
        let b = fixture();
        let router = app(b.clone());
        let response = post_json(
            router.clone(),
            "/sessions",
            json!({"provider":"supabase","claim_secret":"x".repeat(32)}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let value: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            value["authorize_url"]
                .as_str()
                .unwrap()
                .contains("code_challenge_method=S256")
        );
        assert!(!value.to_string().contains("server-only"));
        let path = format!("/sessions/{}/claim", value["id"].as_str().unwrap());
        assert_eq!(
            post_json(router.clone(), &path, json!({"claim_secret":"wrong"}))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post_json(
                router.clone(),
                &path,
                json!({"claim_secret":"x".repeat(32)})
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        b.sessions
            .lock()
            .unwrap()
            .get_mut(value["id"].as_str().unwrap())
            .unwrap()
            .state = SessionState::Ready(json!({"access_token":"test-secret"}));
        assert_eq!(
            post_json(router, &path, json!({"claim_secret":"x".repeat(32)}))
                .await
                .status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn callback_state_is_required_and_consumed_on_denial() {
        let b = fixture();
        b.sessions.lock().unwrap().insert(
            "state".into(),
            Session {
                provider: "supabase".into(),
                claim_hash: [0; 32],
                verifier: "v".into(),
                expires: Instant::now() + Duration::from_secs(10),
                state: SessionState::Awaiting,
            },
        );
        let router = app(b);
        for (query, status) in [
            ("state=unknown&error=denied", StatusCode::BAD_REQUEST),
            ("state=state&error=denied", StatusCode::OK),
            ("state=state&code=replay", StatusCode::CONFLICT),
        ] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(
                        Request::get(format!("/callback/supabase?{query}"))
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
    }
    #[tokio::test]
    async fn expired_sessions_cannot_be_claimed() {
        let b = fixture();
        b.sessions.lock().unwrap().insert(
            "expired".into(),
            Session {
                provider: "supabase".into(),
                claim_hash: Sha256::digest(b"secret").into(),
                verifier: "v".into(),
                expires: Instant::now() - Duration::from_secs(1),
                state: SessionState::Ready(json!({"access_token":"never-return"})),
            },
        );
        assert_eq!(
            post_json(
                app(b),
                "/sessions/expired/claim",
                json!({"claim_secret":"secret"})
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn callbacks_exchange_codes_server_side_and_only_return_tokens_to_the_device() {
        for pkce in [true, false] {
            let received = Arc::new(Mutex::new(None));
            let captured = received.clone();
            let token_server = Router::new().route("/token", post(move |
                headers: axum::http::HeaderMap,
                axum::Form(form): axum::Form<HashMap<String, String>>,
            | async move {
                *captured.lock().unwrap() = Some((headers, form));
                Json(json!({"access_token":"device-only-token", "refresh_token":"refresh-only"}))
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let token_url = format!("http://{}/token", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, token_server).await.unwrap();
            });
            let mut b = fixture();
            let registration = Arc::make_mut(&mut b.registrations)
                .get_mut("supabase")
                .unwrap();
            registration.pkce = pkce;
            registration.token_url = token_url;
            b.sessions.lock().unwrap().insert(
                "state".into(),
                Session {
                    provider: "supabase".into(),
                    claim_hash: Sha256::digest(b"device-secret").into(),
                    verifier: "proof-key".into(),
                    expires: Instant::now() + Duration::from_secs(60),
                    state: SessionState::Awaiting,
                },
            );
            let router = app(b);
            let response = router
                .clone()
                .oneshot(
                    Request::get("/callback/supabase?state=state&code=auth-code")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let html = String::from_utf8(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec(),
            )
            .unwrap();
            assert!(!html.contains("device-only-token"));
            let (headers, form) = received.lock().unwrap().take().unwrap();
            assert_eq!(form["code"], "auth-code");
            assert_eq!(
                form["redirect_uri"],
                "https://broker.example.com/callback/supabase"
            );
            if pkce {
                assert_eq!(form["code_verifier"], "proof-key");
                assert_eq!(form["grant_type"], "authorization_code");
                assert!(
                    headers[header::AUTHORIZATION]
                        .to_str()
                        .unwrap()
                        .starts_with("Basic ")
                );
                assert!(!form.contains_key("client_secret"));
            } else {
                assert_eq!(form["client_secret"], "server-only");
                assert_eq!(form["client_id"], "public-id");
                assert!(!form.contains_key("code_verifier"));
            }
            let response = post_json(
                router,
                "/sessions/state/claim",
                json!({"claim_secret":"device-secret"}),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let tokens: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(tokens["access_token"], "device-only-token");
            server.abort();
        }
    }
}
