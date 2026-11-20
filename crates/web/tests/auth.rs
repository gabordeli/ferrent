//! Auth surface of the gateway: login, session cookie, logout and the limit.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use ferrent_web::{DaemonConfig, WebConfig, app};

fn router() -> Router {
    let config = WebConfig {
        password: "secret".to_owned(),
        daemon: DaemonConfig {
            ca: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
            token: "token".to_owned(),
            ..DaemonConfig::default()
        },
        ..WebConfig::default()
    };
    let (_, router) = app(config);
    router
}

fn login_request(password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!("{{\"password\":\"{password}\"}}")))
        .expect("request")
}

fn cookie_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned()
}

async fn body(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn health_reports_a_missing_daemon() {
    let response = router()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let text = body(response).await;
    assert!(text.contains("degraded"), "{text}");
    assert!(text.contains("read_only"), "{text}");
}

#[tokio::test]
async fn login_sets_a_session_and_logout_clears_it() {
    let app = router();
    let response = app
        .clone()
        .oneshot(login_request("wrong"))
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(login_request("secret"))
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = cookie_of(&response);
    assert!(cookie.starts_with("ferrent_session="), "{cookie}");
    assert!(
        response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("HttpOnly") && value.contains("SameSite=Strict")),
        "the cookie is HttpOnly and SameSite"
    );

    let authed = |uri: &str| {
        Request::builder()
            .uri(uri)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .expect("request")
    };
    let response = app
        .clone()
        .oneshot(authed("/api/session"))
        .await
        .expect("session");
    assert_eq!(response.status(), StatusCode::OK);
    let text = body(response).await;
    assert!(text.contains("\"read_only\":true"), "{text}");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("session");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("logout");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.oneshot(authed("/api/session")).await.expect("session");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "logged out");
}

#[tokio::test]
async fn the_login_limit_refuses_the_sixth_attempt() {
    let app = router();
    for attempt in 0..5 {
        let response = app
            .clone()
            .oneshot(login_request("wrong"))
            .await
            .expect("login");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {attempt}"
        );
    }
    let response = app.oneshot(login_request("secret")).await.expect("login");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let text = body(response).await;
    assert!(text.contains("too many"), "{text}");
}

#[tokio::test]
async fn events_need_a_session() {
    let response = router()
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("events");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
