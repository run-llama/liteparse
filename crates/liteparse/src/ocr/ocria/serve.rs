//! `lit ocr-serve`: ocria-nano behind the HTTP OCR protocol (`OCR_API_SPEC.md`), so parsing
//! processes pointed at it with `--ocr-server-url` share one engine: one set of model sessions
//! (one GPU's memory), loaded once instead of per process.
//!
//! Requests run concurrently up to `max_concurrent`; on a GPU the session pool bounds how many
//! model calls overlap, and the rest of a request (PNG decode, crops, page heuristics) runs on
//! the CPU meanwhile.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde_json::json;
use tokio::sync::Semaphore;

use super::OcriaEngine;

/// A 300 dpi A3 page as RGB PNG stays well under this.
const MAX_BODY: usize = 64 << 20;

struct Server {
    engine: Arc<OcriaEngine>,
    slots: Semaphore,
    pages: AtomicU64,
    busy_us: AtomicU64,
}

type Reply = (StatusCode, Json<serde_json::Value>);

fn error(status: StatusCode, msg: impl std::fmt::Display) -> Reply {
    (status, Json(json!({ "error": msg.to_string() })))
}

async fn ocr(State(srv): State<Arc<Server>>, mut form: Multipart) -> Reply {
    let mut file = None;
    loop {
        match form.next_field().await {
            Ok(Some(field)) => {
                // `language` is accepted and ignored: the router picks each line's script
                if field.name() == Some("file") {
                    match field.bytes().await {
                        Ok(b) => file = Some(b),
                        Err(e) => return error(StatusCode::BAD_REQUEST, e),
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    }
    let Some(file) = file else {
        return error(StatusCode::BAD_REQUEST, "missing `file` field");
    };
    let _slot = srv.slots.acquire().await.expect("semaphore never closes");
    let engine = srv.engine.clone();
    let t = Instant::now();
    let res = tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&file)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
            .to_rgb8();
        let (w, h) = img.dimensions();
        engine
            .run(img.as_raw(), w, h)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await;
    srv.busy_us
        .fetch_add(t.elapsed().as_micros() as u64, Ordering::Relaxed);
    match res {
        Ok(Ok(results)) => {
            srv.pages.fetch_add(1, Ordering::Relaxed);
            (StatusCode::OK, Json(json!({ "results": results })))
        }
        Ok(Err((status, msg))) => error(status, msg),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn health(State(srv): State<Arc<Server>>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "pages": srv.pages.load(Ordering::Relaxed),
        "busy_s": srv.busy_us.load(Ordering::Relaxed) as f64 / 1e6,
        "engine": srv.engine.stats(),
    }))
}

/// Serve `engine` on `addr` until Ctrl-C / SIGTERM.
pub async fn serve(
    engine: Arc<OcriaEngine>,
    addr: SocketAddr,
    max_concurrent: usize,
) -> std::io::Result<()> {
    let srv = Arc::new(Server {
        engine,
        slots: Semaphore::new(max_concurrent.max(1)),
        pages: AtomicU64::new(0),
        busy_us: AtomicU64::new(0),
    });
    let app = Router::new()
        .route("/ocr", post(ocr))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(srv);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "[ocria-nano] serving on http://{}/ocr ({} concurrent)",
        listener.local_addr()?,
        max_concurrent.max(1)
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}
