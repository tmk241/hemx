//! Interaction integrity: cross-site and stale-build requests must be rejected
//! before any application handler runs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{header, Request, Response, StatusCode};
use axum::routing::post;
use axum::{Extension, Router};
use hemx_axum::{
    interactions, AllowSameSite, DispatchRejection, EffectResponse, InteractionForm,
    InteractionRequest, MultipartLimits, HEMX_FINGERPRINT_HEADER,
};
use hemx_core::{BuildFingerprint, Handle, Slot};
use tower::ServiceExt;

const BUILD: BuildFingerprint = BuildFingerprint(77);
const FORM: &str = "application/x-www-form-urlencoded";

async fn mutate(
    State(calls): State<Arc<AtomicUsize>>,
    request: InteractionRequest,
) -> Result<EffectResponse, DispatchRejection> {
    request.dispatch(interactions(BUILD).on(Handle::<()>::new(7), move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Slot::<String>::new(3).text("done")
    }))
}

fn app(calls: &Arc<AtomicUsize>) -> Router {
    Router::new()
        .route("/mutate", post(mutate))
        .with_state(calls.clone())
}

fn submit(body: &str, site: Option<&str>) -> Request<Body> {
    let mut request = Request::post("/mutate").header(header::CONTENT_TYPE, FORM);
    if let Some(site) = site {
        request = request.header("sec-fetch-site", site);
    }
    request.body(Body::from(body.to_owned())).unwrap()
}

async fn text(response: Response<Body>) -> String {
    String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn matching_build_reaches_the_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = app(&calls)
        .oneshot(submit("__h=7&__b=77", Some("same-origin")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn missing_or_different_build_is_rejected_before_the_handler() {
    for body in [
        "__h=7",
        "__h=7&__b=76",
        "__h=7&__b=not-a-number",
        "__h=7&__b=",
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let response = app(&calls).oneshot(submit(body, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT, "{body}");

        // The current build travels back so the browser can tell this is a stale page.
        let build = &response.headers()[HEMX_FINGERPRINT_HEADER];
        assert_eq!(build, "77", "{body}");
        assert!(text(response).await.contains("reload"), "{body}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{body}");
    }
}

#[tokio::test]
async fn stale_build_wins_over_an_unknown_handle() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = app(&calls)
        .oneshot(submit("__h=999&__b=76", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn async_dispatch_checks_the_build_too() {
    let handle = Handle::<()>::new(7);
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = || {
        let calls = calls.clone();
        interactions(BUILD).on_async(handle, move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Slot::<String>::new(3).text("done")
            }
        })
    };
    let stale = InteractionForm::for_handle(handle, []).with_build(BuildFingerprint(1));
    assert!(matches!(
        registry().dispatch_async(stale).await,
        Err(DispatchRejection::StaleBuild { .. })
    ));
    let missing = InteractionForm::for_handle(handle, []);
    assert!(matches!(
        registry().dispatch_async(missing).await,
        Err(DispatchRejection::StaleBuild { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no async handler ran");
    let current = InteractionForm::for_handle(handle, []).with_build(BUILD);
    assert!(registry().dispatch_async(current).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_users_get_one_explicit_check() {
    let handle = Handle::<()>::new(7);
    let form = InteractionForm::for_handle(handle, []);
    assert!(form.require_build(BUILD).is_err());
    assert_eq!(form.build_fingerprint(), None);
    let stamped = form.with_build(BUILD);
    assert_eq!(stamped.build_fingerprint(), Some(BUILD));
    assert!(stamped.require_build(BUILD).is_ok());
    assert!(stamped.require_build(BuildFingerprint(78)).is_err());
}

#[tokio::test]
async fn cross_site_is_rejected_before_the_body_is_read() {
    let calls = Arc::new(AtomicUsize::new(0));
    // An undecodable body would be a 400 if extraction got that far.
    let response = app(&calls)
        .oneshot(submit("%ZZ", Some("cross-site")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let valid = app(&calls)
        .oneshot(submit("__h=7&__b=77", Some("cross-site")))
        .await
        .unwrap();
    assert_eq!(valid.status(), StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn same_site_needs_an_explicit_opt_in() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rejected = app(&calls)
        .oneshot(submit("__h=7&__b=77", Some("same-site")))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let allowed = app(&calls)
        .layer(Extension(AllowSameSite))
        .oneshot(submit("__h=7&__b=77", Some("same-site")))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // The opt-in never admits cross-site requests.
    let still_rejected = app(&calls)
        .layer(Extension(AllowSameSite))
        .oneshot(submit("__h=7&__b=77", Some("cross-site")))
        .await
        .unwrap();
    assert_eq!(still_rejected.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn same_origin_user_initiated_and_header_free_requests_pass() {
    for site in [Some("same-origin"), Some("none"), None] {
        let calls = Arc::new(AtomicUsize::new(0));
        let response = app(&calls)
            .oneshot(submit("__h=7&__b=77", site))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{site:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{site:?}");
    }
}

fn multipart_submit(build: Option<&str>, site: &str) -> Request<Body> {
    let mut body = String::from("--X\r\nContent-Disposition: form-data; name=\"__h\"\r\n\r\n7\r\n");
    if let Some(build) = build {
        body.push_str(&format!(
            "--X\r\nContent-Disposition: form-data; name=\"__b\"\r\n\r\n{build}\r\n"
        ));
    }
    body.push_str("--X\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"clip.glb\"\r\n\r\nfile\r\n--X--\r\n");
    Request::post("/mutate")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=X")
        .header("sec-fetch-site", site)
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn bounded_multipart_preserves_fetch_metadata_and_build_guards() {
    for (build, site, opt_in, expected) in [
        (Some("77"), "cross-site", false, StatusCode::FORBIDDEN),
        (Some("77"), "cross-site", true, StatusCode::FORBIDDEN),
        (Some("77"), "same-site", false, StatusCode::FORBIDDEN),
        (Some("77"), "same-site", true, StatusCode::OK),
        (Some("77"), "same-origin", false, StatusCode::OK),
        (Some("76"), "same-origin", false, StatusCode::CONFLICT),
        (None, "same-origin", false, StatusCode::CONFLICT),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut router = app(&calls).layer(Extension(MultipartLimits {
            text_bytes: 32,
            file_bytes: 4,
        }));
        if opt_in {
            router = router.layer(Extension(AllowSameSite));
        }
        let response = router.oneshot(multipart_submit(build, site)).await.unwrap();
        assert_eq!(response.status(), expected, "{build:?}/{site}/{opt_in}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(expected.is_success())
        );
    }
}

#[tokio::test]
async fn cross_site_multipart_is_rejected_without_polling_its_body() {
    let calls = Arc::new(AtomicUsize::new(0));
    let body = Body::from_stream(futures_util::stream::poll_fn(
        |_| -> std::task::Poll<Option<Result<Vec<u8>, std::convert::Infallible>>> {
            panic!("cross-site multipart body was polled");
        },
    ));
    let mut request = multipart_submit(Some("77"), "cross-site");
    *request.body_mut() = body;
    let response = app(&calls)
        .layer(Extension(MultipartLimits {
            text_bytes: 0,
            file_bytes: 0,
        }))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
