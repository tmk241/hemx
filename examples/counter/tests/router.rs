use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use hemx::advanced::{Effect, EffectBatch, PatchMode};
use hemx_counter_example::{app, heml};
use scraper::{Html, Selector};
use tower::ServiceExt;

async fn bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}

async fn page(app: &Router) -> Html {
    let response = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    Html::parse_document(&String::from_utf8(bytes(response).await).unwrap())
}

fn attr(document: &Html, selector: &str, name: &str) -> String {
    document
        .select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap()
        .value()
        .attr(name)
        .unwrap()
        .to_owned()
}

// Submit the actual generated fields a browser receives, not copied protocol IDs.
fn form_body(document: &Html, amount: &str) -> String {
    format!(
        "__h={}&__b={}&amount={amount}",
        attr(document, "form input[name=__h]", "value"),
        attr(document, "form input[name=__b]", "value"),
    )
}

fn submit(body: String, enhanced: bool, site: &str) -> Request<Body> {
    Request::post("/increment")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(
            header::ACCEPT,
            if enhanced {
                "application/hemx, text/html"
            } else {
                "text/html"
            },
        )
        .header("sec-fetch-site", site)
        .body(Body::from(body))
        .unwrap()
}

fn count(document: &Html) -> String {
    document
        .select(&Selector::parse("output").unwrap())
        .next()
        .unwrap()
        .text()
        .collect()
}

#[tokio::test]
async fn page_has_lowered_resources_native_fields_and_the_real_runtime() {
    let app = app();
    let document = page(&app).await;
    let build = heml::BUILD_FINGERPRINT.0.to_string();
    assert_eq!(
        attr(&document, "[data-hemx-root]", "data-hemx-build"),
        build
    );
    assert_eq!(attr(&document, "form input[name=__b]", "value"), build);
    assert_eq!(
        attr(&document, "form input[name=__h]", "value"),
        heml::page::increment.id().id.to_string()
    );
    assert_eq!(
        attr(&document, "form", "data-hemx-action"),
        attr(&document, "form input[name=__h]", "value")
    );
    assert_eq!(
        attr(&document, "form", "data-hemx-resource"),
        heml::page::increment_form.id().id.to_string()
    );
    assert_eq!(attr(&document, "form", "action"), "/increment");
    assert_eq!(attr(&document, "form", "method"), "post");
    assert_eq!(attr(&document, "input[name=amount]", "value"), "1");
    assert_eq!(count(&document), "0");
    for authoring in ["[data-hemx-slot]", "[data-hemx-handle]", "[data-hemx-form]"] {
        assert_eq!(
            document
                .select(&Selector::parse(authoring).unwrap())
                .count(),
            0
        );
    }

    let script = attr(&document, "script", "src");
    assert_eq!(script, hemx_axum::runtime_js_route_path());
    let response = app
        .oneshot(Request::get(script).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        hemx_axum::HEMX_RUNTIME_CONTENT_TYPE
    );
    assert_eq!(
        bytes(response).await,
        hemx_axum::runtime_js_source().as_bytes()
    );
}

#[tokio::test]
async fn enhanced_and_native_submissions_mutate_the_same_counter() {
    let app = app();
    let document = page(&app).await;
    let response = app
        .clone()
        .oneshot(submit(form_body(&document, "2"), true, "same-origin"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/hemx");
    assert_eq!(
        response.headers()[hemx_axum::HEMX_FINGERPRINT_HEADER],
        heml::BUILD_FINGERPRINT.0.to_string()
    );
    let batch = EffectBatch::from_wire(&bytes(response).await).unwrap();
    assert_eq!(batch.fingerprint, heml::BUILD_FINGERPRINT);
    let [Effect::Patch { target, mode, html }] = batch.ops.as_slice() else {
        panic!("expected one counter patch, got {:?}", batch.ops);
    };
    // The wire effect addresses the same resource that the complete page rendered.
    assert_eq!(
        target.resource.id.to_string(),
        attr(&document, "output", "data-hemx-resource")
    );
    assert!(target.scope.is_none());
    assert_eq!(*mode, PatchMode::Morph);
    assert_eq!(html.as_str(), "2");
    assert_eq!(count(&page(&app).await), "2");

    let native = app
        .clone()
        .oneshot(submit(form_body(&document, "3"), false, "same-origin"))
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::SEE_OTHER);
    assert_eq!(native.headers()[header::LOCATION], "/");
    assert_eq!(count(&page(&app).await), "5");
}

#[tokio::test]
async fn invalid_stale_and_cross_site_forms_never_mutate_either_representation() {
    let app = app();
    let document = page(&app).await;
    let handle = attr(&document, "form input[name=__h]", "value");
    let stale = heml::BUILD_FINGERPRINT.0.wrapping_add(1);
    for enhanced in [true, false] {
        for (body, site, status) in [
            (
                format!("__h={handle}&amount=1"),
                "same-origin",
                StatusCode::CONFLICT,
            ),
            (
                format!("__h={handle}&__b={stale}&amount=1"),
                "same-origin",
                StatusCode::CONFLICT,
            ),
            (
                form_body(&document, "1"),
                "cross-site",
                StatusCode::FORBIDDEN,
            ),
            (
                form_body(&document, "1"),
                "same-site",
                StatusCode::FORBIDDEN,
            ),
            (
                form_body(&document, "256"),
                "same-origin",
                StatusCode::BAD_REQUEST,
            ),
            (
                form_body(&document, "-1"),
                "same-origin",
                StatusCode::BAD_REQUEST,
            ),
            (
                form_body(&document, "no"),
                "same-origin",
                StatusCode::BAD_REQUEST,
            ),
            (
                form_body(&document, &"1".repeat(1025)),
                "same-origin",
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(submit(body, enhanced, site))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                status,
                "enhanced={enhanced}, site={site}"
            );
            if status == StatusCode::CONFLICT {
                assert_eq!(
                    response.headers()[hemx_axum::HEMX_FINGERPRINT_HEADER],
                    heml::BUILD_FINGERPRINT.0.to_string()
                );
            }
            assert_eq!(count(&page(&app).await), "0");
        }
    }
}
