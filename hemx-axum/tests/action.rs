use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use hemx_axum::{action, interactions, Native, HEMX_CONTENT_TYPE};
use hemx_core::{BuildFingerprint, Handle, Html};
use tower::ServiceExt;

const BUILD: BuildFingerprint = BuildFingerprint(9);
const SAVE: Handle<()> = Handle::new(7);

fn feedback(_: &hemx_axum::InteractionForm, problem: &str) -> Html {
    Html::__rendered(format!("<p>{problem}</p>"), BUILD)
}

fn app() -> Router {
    Router::new().route(
        "/save",
        action(
            interactions(BUILD).on(SAVE, |_| ()),
            Native::new("/saved", feedback),
        ),
    )
}

fn post(accept: &str) -> Request<Body> {
    Request::post("/save")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, accept)
        .body(Body::from("__h=7&__b=9"))
        .unwrap()
}

#[tokio::test]
async fn the_requested_representation_selects_effects_or_a_redirect() {
    for accept in [
        "application/hemx, text/html",
        "text/html;q=0.9, APPLICATION/HEMX;q=1",
    ] {
        let response = app().oneshot(post(accept)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{accept}");
        let content_type = &response.headers()[header::CONTENT_TYPE];
        assert_eq!(content_type, HEMX_CONTENT_TYPE);
    }
    for accept in ["text/html", "*/*", "application/hemxml"] {
        let response = app().oneshot(post(accept)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{accept}");
        assert_eq!(response.headers()[header::LOCATION], "/saved");
    }
}

#[test]
fn native_locations_must_stay_on_this_origin() {
    for location in [
        "//elsewhere.example/",
        "https://elsewhere.example/",
        "saved",
        "/\\elsewhere",
        "/a\nb",
    ] {
        let result = std::panic::catch_unwind(|| Native::new(location, feedback));
        assert!(result.is_err(), "{location:?} accepted");
    }
    let _ = Native::new("/saved?done=1", feedback);
}
