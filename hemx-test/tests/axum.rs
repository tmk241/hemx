#![cfg(feature = "axum")]

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::Router;
use hemx_axum::{EffectResponse, InteractionForm, HEMX_CONTENT_TYPE};
use hemx_core::{BuildFingerprint, Handle, Slot};

async fn effects() -> EffectResponse {
    EffectResponse::new(
        Slot::<()>::new(1).text("ready"),
        BuildFingerprint::from_parts(&[7]),
    )
}

async fn document() -> Html<&'static str> {
    Html("<!doctype html><html><body><main id=app>Ready</main></body></html>")
}

async fn fragment() -> Html<&'static str> {
    Html("<li data-state=open>Write tests</li>")
}

async fn redirect() -> Redirect {
    Redirect::to("/next")
}

async fn rejected() -> impl IntoResponse {
    (StatusCode::UNPROCESSABLE_ENTITY, "invalid title")
}

async fn request_header(headers: HeaderMap) -> String {
    headers
        .get("x-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing")
        .to_owned()
}

async fn malformed_wire() -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, HEMX_CONTENT_TYPE)
        .body(Body::from(vec![0xff, 0x00, 0x01]))
        .unwrap()
}

async fn form(form: InteractionForm) -> String {
    format!("{}:{}", form.handle_id, form.value("title").unwrap_or(""))
}

fn router() -> Router {
    Router::new()
        .route("/effects", get(effects))
        .route("/document", get(document))
        .route("/fragment", get(fragment))
        .route("/redirect", get(redirect))
        .route("/rejected", get(rejected))
        .route("/request-header", get(request_header))
        .route("/malformed", get(malformed_wire))
        .route("/form", post(form))
}

#[tokio::test]
async fn inspects_effect_response_from_real_router() {
    let response = hemx_test::axum::get("/effects")
        .send(router())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.content_type(), Some(HEMX_CONTENT_TYPE));
    assert!(!response.body().is_empty());
    assert!(response
        .effects()
        .unwrap()
        .updates_text_containing(GeneratedSlot, "ready"));
}

#[tokio::test]
async fn preserves_status_headers_redirects_and_text_errors() {
    let request_header = hemx_test::axum::request(Method::GET, "/request-header")
        .header(
            "x-test".parse().unwrap(),
            HeaderValue::from_static("request"),
        )
        .send(router())
        .await
        .unwrap();
    assert_eq!(request_header.text().unwrap(), "request");

    let redirect = hemx_test::axum::get("/redirect")
        .send(router())
        .await
        .unwrap();
    assert_eq!(redirect.status(), StatusCode::SEE_OTHER);
    assert_eq!(redirect.headers()[header::LOCATION], "/next");
    assert!(redirect.body().is_empty());

    let rejected = hemx_test::axum::get("/rejected")
        .send(router())
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rejected.text().unwrap(), "invalid title");
}

#[tokio::test]
async fn structurally_inspects_document_and_fragment_responses() {
    let document = hemx_test::axum::get("/document")
        .send(router())
        .await
        .unwrap()
        .html_document()
        .unwrap();
    document.assert_text("main#app", "Ready");

    let fragment = hemx_test::axum::get("/fragment")
        .send(router())
        .await
        .unwrap()
        .html_fragment()
        .unwrap();
    fragment.assert_attribute("li", "data-state", "open");
    fragment.assert_text("li", "Write tests");
}

#[tokio::test]
async fn builds_urlencoded_interaction_forms_from_typed_handles() {
    let response = hemx_test::axum::post("/form")
        .form(Handle::<()>::new(17), &[("title", "one & two")])
        .send(router())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().unwrap(), "17:one & two");
}

#[tokio::test]
async fn rejects_unexpected_content_types_and_malformed_wire() {
    let html = hemx_test::axum::get("/document")
        .send(router())
        .await
        .unwrap();
    let wrong_type = html.effects().unwrap_err().to_string();
    assert!(wrong_type.contains("application/hemx"), "{wrong_type}");
    assert!(wrong_type.contains("text/html"), "{wrong_type}");

    let malformed = hemx_test::axum::get("/malformed")
        .send(router())
        .await
        .unwrap()
        .effects()
        .unwrap_err()
        .to_string();
    assert!(
        malformed.contains("invalid Hemx effect response"),
        "{malformed}"
    );
}

#[tokio::test]
async fn enforces_explicit_response_body_limit() {
    let error = hemx_test::axum::get("/document")
        .body_limit(8)
        .send(router())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("within 8 bytes"), "{error}");
}

#[derive(Clone, Copy)]
struct GeneratedSlot;

impl hemx_core::GeneratedTarget for GeneratedSlot {
    fn __hemx_resource_id(self) -> hemx_core::ResourceId {
        Slot::<()>::new(1).id()
    }
}
