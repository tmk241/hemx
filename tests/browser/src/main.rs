mod checks;
mod form_checks;
mod lifecycle_checks;
mod native_form_checks;
mod native_forms;
mod review_checks;
mod runtime_checks;
mod stream_checks;
mod streams;
mod wire_checks;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::oneshot;

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Probe {
    cases: Value,
    started: Mutex<Option<oneshot::Sender<()>>>,
    report: Mutex<Option<oneshot::Sender<Value>>>,
}

async fn cases(State(probe): State<Arc<Probe>>) -> Result<Json<Value>, StatusCode> {
    if let Some(started) = probe.started.lock().expect("startup lock").take() {
        started.send(()).map_err(|_| StatusCode::GONE)?;
        println!("{}", json!({"phase": "running"}));
    }
    Ok(Json(probe.cases.clone()))
}

async fn receive_report(
    started: oneshot::Receiver<()>,
    report: oneshot::Receiver<Value>,
    limit: Duration,
) -> Result<Value, String> {
    match tokio::time::timeout(limit, started).await {
        Ok(Ok(())) => {}
        other => {
            return Err(format!(
                "browser did not request cases within {limit:?}: {other:?}"
            ))
        }
    }
    match tokio::time::timeout(limit, report).await {
        Ok(Ok(report)) => Ok(report),
        other => Err(format!(
            "browser did not report within {limit:?} of starting: {other:?}"
        )),
    }
}

async fn report(State(probe): State<Arc<Probe>>, Json(value): Json<Value>) -> StatusCode {
    let Some(sender) = probe.report.lock().expect("report lock").take() else {
        return StatusCode::CONFLICT;
    };
    if sender.send(value).is_ok() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::GONE
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let (start_sender, start_receiver) = oneshot::channel();
    let (report_sender, report_receiver) = oneshot::channel();
    let probe = Arc::new(Probe {
        cases: hemx_browser_test::cases(),
        started: Mutex::new(Some(start_sender)),
        report: Mutex::new(Some(report_sender)),
    });
    let streams = streams::Streams::new();
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../probe.html")) }))
        .route("/frame", get(|| async {
            Html("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Isolated Hemx fixture</title><body></body></html>")
        }))
        .route("/probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../probe.js"))
        }))
        .route("/hemx.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], hemx_js::RUNTIME_JS)
        }))
        .route("/runtime-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../runtime-probe.js"))
        }))
        .route("/forms-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../forms-probe.js"))
        }))
        .route("/review-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../review-probe.js"))
        }))
        .route("/review/{name}", get(|axum::extract::Path(name): axum::extract::Path<String>| async move {
            let Some(batches) = hemx_browser_test::review_cases::sequence(&name) else { return Err(StatusCode::NOT_FOUND); };
            let mut body = String::new();
            for batch in batches {
                body.push_str(&format!("event: hemx\ndata: {}\n\n", hemx_axum::encode_sse_batch(&batch).unwrap()));
            }
            Ok(([(header::CONTENT_TYPE, "text/event-stream")], body))
        }))
        .route("/cases", get(cases))
        .route("/results", post(report))
        .route("/wire-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../wire-probe.js"))
        }))
        .route("/native-forms.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../native-forms.js"))
        }))
        .route("/lifecycle-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../lifecycle-probe.js"))
        }))
        .route("/streams-probe.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../streams-probe.js"))
        }))
        .with_state(probe)
        .merge(native_forms::routes())
        .merge(streams.routes());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let (stop_sender, stop_receiver) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_receiver.await;
            })
            .await
    });
    println!("{}", json!({"phase": "ready", "url": format!("{origin}/")}));
    let received = receive_report(start_receiver, report_receiver, Duration::from_secs(60)).await;
    streams.shutdown();
    let _ = stop_sender.send(());
    server.await??;
    let report = match received {
        Ok(report) => report,
        Err(failure) => {
            println!(
                "{}",
                json!({"phase": "result", "success": false, "failure": failure})
            );
            return Err(failure.into());
        }
    };
    // Keep assertions at the actual browser-result boundary. A panic is an
    // unsuccessful run, not a swallowed test failure; always close the server.
    let verified = std::panic::catch_unwind(|| checks::assert_report(&report, &origin));
    let failure = verified.err().map(|panic| {
        if let Some(message) = panic.downcast_ref::<String>() {
            message.clone()
        } else if let Some(message) = panic.downcast_ref::<&str>() {
            (*message).to_owned()
        } else {
            "browser assertion failed".to_owned()
        }
    });
    println!(
        "{}",
        json!({
            "phase": "result", "success": failure.is_none(), "failure": failure,
            "observations": report
        })
    );
    if failure.is_some() {
        return Err("native browser conformance failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn launch_delay_does_not_consume_execution_budget() {
        let (start_sender, started) = oneshot::channel();
        let (report_sender, report) = oneshot::channel();
        let waiting = tokio::spawn(receive_report(started, report, Duration::from_secs(60)));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        start_sender.send(()).unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        tokio::task::yield_now().await;
        assert!(
            !waiting.is_finished(),
            "startup consumed the execution budget"
        );
        let expected = json!({"received": true});
        report_sender.send(expected.clone()).unwrap();
        assert_eq!(waiting.await.unwrap().unwrap(), expected);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_browser_start_fails() {
        let (_start_sender, started) = oneshot::channel();
        let (_report_sender, report) = oneshot::channel();
        let error = receive_report(started, report, Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(error.contains("did not request cases"));
    }

    #[tokio::test(start_paused = true)]
    async fn missing_report_after_start_fails() {
        let (start_sender, started) = oneshot::channel();
        let (_report_sender, report) = oneshot::channel();
        start_sender.send(()).unwrap();
        let error = receive_report(started, report, Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(error.contains("did not report"));
    }
}
