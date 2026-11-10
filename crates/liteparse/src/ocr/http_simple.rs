use std::{pin::Pin, time::Duration};

use reqwest::{
    Client,
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize};

use crate::ocr::{
    EncodedOcrImage, OcrEngine, OcrFuture, OcrImage, OcrImageFormat, OcrOptions, OcrResult,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpOcrResponseItem {
    text: String,
    bbox: [f32; 4],
    confidence: f32,
    /// Optional 4-point polygon [[x,y]×4] of the (possibly rotated) detection,
    /// ordered top-left → top-right → bottom-right → bottom-left in the
    /// glyphs' upright reading frame.
    #[serde(default)]
    polygon: Option<[[f32; 2]; 4]>,
}

impl HttpOcrResponseItem {
    fn into_ocr_result(self) -> OcrResult {
        OcrResult {
            text: self.text,
            bbox: self.bbox,
            confidence: self.confidence,
            polygon: self.polygon,
        }
    }
}

/// A single detection from an OCR worker, which emits
/// EasyOCR/PaddleOCR-style positional tuples: `[polygon, text, confidence]`,
/// where `polygon` is a list of `[x, y]` points (4 for both engines) ordered
/// top-left → top-right → bottom-right → bottom-left.
#[derive(Debug, Deserialize)]
struct ProdOcrItem(Vec<[f32; 2]>, String, f32);

impl ProdOcrItem {
    fn into_ocr_result(self) -> OcrResult {
        let ProdOcrItem(poly, text, confidence) = self;
        // Axis-aligned bbox from the polygon's min/max extents.
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for [x, y] in &poly {
            min_x = min_x.min(*x);
            min_y = min_y.min(*y);
            max_x = max_x.max(*x);
            max_y = max_y.max(*y);
        }
        // Forward the raw polygon only when it's exactly 4 points, so the
        // projector can recover rotation for sideways text.
        let polygon = match poly.as_slice() {
            [a, b, c, d] => Some([*a, *b, *c, *d]),
            _ => None,
        };
        OcrResult {
            text,
            bbox: [min_x, min_y, max_x, max_y],
            confidence,
            polygon,
        }
    }
}

/// Accepts either the LiteParse standard response or the prod OCR
/// worker response. Untagged: serde tries `Standard` first (keyed on
/// `results` with object items), then falls back to `Prod` (keyed on `result`
/// with positional-tuple items).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HttpOcrResponse {
    Standard { results: Vec<HttpOcrResponseItem> },
    Prod { result: Vec<ProdOcrItem> },
}

impl HttpOcrResponse {
    fn into_results(self) -> Vec<OcrResult> {
        match self {
            HttpOcrResponse::Standard { results } => {
                results.into_iter().map(|i| i.into_ocr_result()).collect()
            }
            HttpOcrResponse::Prod { result } => {
                result.into_iter().map(|i| i.into_ocr_result()).collect()
            }
        }
    }
}

/// HTTP-based OCR engine that conforms to LiteParse OCR API specification.
/// The server must implement the API defined in OCR_API_SPEC.md:
///     - POST /ocr endpoint
///     - Accepts multipart/form-data with 'file' and 'language' fields
///     - Returns JSON: { results: [{ text, bbox: [x1,y1,x2,y2], confidence }] }
/// See ocr/easyocr/ and ocr/paddleocr/ for example server implementations.
pub struct HttpOcrEngine {
    pub name: String,
    server_url: String,
    /// Extra headers (name, value) sent with every request, e.g. auth tokens.
    headers: Vec<(String, String)>,
    /// Retry/backoff policy. Defaults to worker-parity semantics.
    retry: OcrRetryConfig,
}

/// Retry/backoff policy for OCR HTTP requests. The default is up to 10
/// attempts, 1s base backoff doubling to a 10s cap, plus jitter, with a fast
/// path for dropped connections) so that liteparse-driven OCR is resilient
/// to a down / rate-limited `/ocr` endpoint.
/// Without this, a transient outage or a 429 burst exhausts the old 3-attempt /
/// sub-second-backoff budget and the page's OCR is lost.
#[derive(Debug, Clone)]
pub struct OcrRetryConfig {
    /// Total attempts (1 initial + retries) before giving up on a request.
    pub max_attempts: u32,
    /// Backoff before the first retry; doubles each subsequent retry.
    pub base_backoff_ms: u64,
    /// Upper bound the doubling backoff is clamped to.
    pub max_backoff_ms: u64,
    /// Maximum random jitter added to each backoff, to de-correlate the
    /// concurrent per-page retries (avoids a thundering herd when the server
    /// recovers and every parked page retries at the same instant).
    pub jitter_ms: u64,
    /// Short fixed backoff for a mid-stream connection drop ("socket hang up"),
    /// which is usually a single dropped keepalive rather than overload.
    pub fast_retry_ms: u64,
    /// Per-request timeout.
    pub request_timeout_ms: u64,
    /// Request-hedging schedule, in milliseconds. Empty or single-element =
    /// no hedging (one request per attempt — the default). With multiple
    /// delays (e.g. `[0, 5000, 10000]`), each attempt fires a *duplicate*
    /// request at every delay and takes the first to succeed, cancelling the
    /// rest — a tail-latency trick that trades extra OCR-server load for
    /// lower p99 latency when a request lands on a slow/stuck pod.
    /// Opt-in: callers enable it via config.
    pub hedge_delays_ms: Vec<u64>,
}

impl Default for OcrRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 10,
            base_backoff_ms: 1000,
            max_backoff_ms: 10_000,
            jitter_ms: 500,
            fast_retry_ms: 500,
            request_timeout_ms: 60_000,
            hedge_delays_ms: Vec::new(),
        }
    }
}

/// Pseudo-random jitter in `0..=max` milliseconds. Derived from the wall-clock
/// nanosecond fraction rather than a `rand` dependency; concurrent tasks sample
/// at slightly different instants, which is enough to spread out retries.
fn jitter_ms(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % (max + 1))
        .unwrap_or(0)
}

/// True for a mid-stream connection drop (peer reset / "socket hang up" /
/// broken pipe). Walks the error source chain because reqwest wraps the
/// underlying hyper/io cause. These get the short `fast_retry_ms` backoff.
fn is_connection_drop(err: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = source {
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("connection reset")
            || msg.contains("hang up")
            || msg.contains("broken pipe")
            || msg.contains("connection closed")
            || msg.contains("incompletemessage")
        {
            return true;
        }
        source = e.source();
    }
    false
}

/// Send a single OCR request and return the raw response body. Encodes the
/// multipart form fresh each call (a `Form` is consumed by `send`).
async fn send_one(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    image: &EncodedOcrImage,
    language: &str,
    timeout_ms: u64,
) -> Result<String, reqwest::Error> {
    let form = Form::new()
        .part(
            "file",
            Part::bytes(image.data.clone())
                .file_name(image.format.file_name())
                .mime_str(image.format.mime_type())?,
        )
        .text("language", language.to_string());
    let mut request = client
        .post(url)
        .multipart(form)
        .timeout(Duration::from_millis(timeout_ms));
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    match request.send().await.and_then(|r| r.error_for_status()) {
        Ok(resp) => resp.text().await,
        Err(e) => Err(e),
    }
}

/// Whether a failed request is worth retrying. Transient transport problems
/// (connection refused/reset, timeouts) and overload/5xx status codes are
/// retryable; a 4xx like 400/401/404 is a deterministic caller/config error
/// that a retry would only repeat.
fn is_retryable(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() {
        return true;
    }
    if let Some(status) = err.status() {
        return matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504);
    }
    // No status and not a clean connect/timeout classification (e.g. a body
    // read that died mid-stream): treat as transient and retry.
    err.is_body() || err.is_request()
}

impl HttpOcrEngine {
    pub fn new(server_url: String) -> Self {
        Self::with_headers(server_url, Vec::new())
    }

    pub fn with_headers(server_url: String, headers: Vec<(String, String)>) -> Self {
        Self {
            name: "http-ocr".to_string(),
            server_url,
            headers,
            retry: OcrRetryConfig::default(),
        }
    }

    /// Override the retry/backoff policy
    pub fn with_retry(mut self, retry: OcrRetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Send one OCR request, or — when `hedge_delays_ms` has more than one
    /// entry — a hedged group: fire a duplicate request at each delay and
    /// return the first success, aborting the slower in-flight duplicates.
    /// Falls back to a plain single request (the common case) when hedging is
    /// not configured.
    async fn send_hedged(
        &self,
        client: &Client,
        image: &EncodedOcrImage,
        language: &str,
    ) -> Result<String, reqwest::Error> {
        let delays = &self.retry.hedge_delays_ms;
        let timeout_ms = self.retry.request_timeout_ms;

        // Single-request fast path: no hedging, or a single (possibly delayed)
        // request. Borrows directly — no task spawn / cloning.
        if delays.len() <= 1 {
            if let Some(&d) = delays.first().filter(|&&d| d > 0) {
                tokio::time::sleep(Duration::from_millis(d)).await;
            }
            return send_one(
                client,
                &self.server_url,
                &self.headers,
                image,
                language,
                timeout_ms,
            )
            .await;
        }

        // Hedged path: spawn one task per delay (spawned tasks need owned data,
        // so clone the cheap bits and the image per hedge — the duplicate upload
        // is the cost hedging deliberately pays). The first Ok wins and the
        // remaining tasks are aborted (which cancels their in-flight requests);
        // if all fail, surface the last error so the retry loop can back off.
        let (tx, mut rx) = tokio::sync::mpsc::channel(delays.len());
        let mut handles = Vec::with_capacity(delays.len());
        for &delay in delays {
            let tx = tx.clone();
            let client = client.clone();
            let url = self.server_url.clone();
            let headers = self.headers.clone();
            let image = image.clone();
            let lang = language.to_string();
            handles.push(tokio::spawn(async move {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                let res = send_one(&client, &url, &headers, &image, &lang, timeout_ms).await;
                let _ = tx.send(res).await;
            }));
        }
        drop(tx);

        let mut last_err: Option<reqwest::Error> = None;
        while let Some(res) = rx.recv().await {
            match res {
                Ok(body) => {
                    for h in &handles {
                        h.abort();
                    }
                    return Ok(body);
                }
                Err(e) => last_err = Some(e),
            }
        }
        // The channel only closes after every hedge task has sent its result,
        // so at least one error is present when we reach here.
        Err(last_err.expect("hedge group always yields at least one result"))
    }

    async fn request_image(
        &self,
        image: &EncodedOcrImage,
        options: &OcrOptions,
    ) -> Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::new();

        // Retry loop: each attempt sends one OCR request (or a hedged group
        // of duplicates when configured) and backs off exponentially on
        // transient failures. The image bytes are prepared once and
        // cloned per request.
        let max_attempts = self.retry.max_attempts.max(1);
        let mut attempt: u32 = 0;
        let raw = loop {
            attempt += 1;
            match self.send_hedged(&client, image, &options.language).await {
                Ok(body) => break body,
                Err(e) => {
                    if attempt >= max_attempts || !is_retryable(&e) {
                        return Err(e.into());
                    }
                    // A dropped connection ("socket hang up") gets a short
                    // fixed backoff; everything else gets exponential
                    // backoff clamped to the cap. Jitter spreads concurrent
                    // page retries so they don't all hit a recovering
                    // server at once.
                    let base = if is_connection_drop(&e) {
                        self.retry.fast_retry_ms
                    } else {
                        (self
                            .retry
                            .base_backoff_ms
                            .saturating_mul(2u64.saturating_pow(attempt - 1)))
                        .min(self.retry.max_backoff_ms)
                    };
                    let delay = base + jitter_ms(self.retry.jitter_ms);
                    if std::env::var("LITEPARSE_DEBUG_OCR").is_ok() {
                        eprintln!(
                            "[ocr-http] attempt {attempt}/{max_attempts} failed ({e}); retrying in {delay}ms"
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
        };
        // Parse from the buffered body (rather than `.json()`) so a
        // malformed/unexpected response can surface a snippet of what the
        // server actually returned.
        let response: HttpOcrResponse = serde_json::from_str(&raw).map_err(|e| {
            let snippet: String = raw.chars().take(200).collect();
            format!("OCR server returned unparseable response: {e}; body starts: {snippet}")
        })?;
        let results = response.into_results();
        if std::env::var("LITEPARSE_DEBUG_OCR").is_ok() {
            eprintln!(
                "[ocr-http] {} bytes -> {} result(s)",
                raw.len(),
                results.len()
            );
        }
        Ok(results)
    }
}

impl OcrEngine for HttpOcrEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn recognize_image<'a>(
        &'a self,
        image: &'a OcrImage,
        options: &'a OcrOptions,
    ) -> OcrFuture<'a> {
        Box::pin(async move {
            let encoded = image.encoded()?;
            self.request_image(encoded.as_ref(), options).await
        })
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let png_bytes = crate::extract::encode_pixels_png(image_data, width, height)?;

            self.request_image(
                &EncodedOcrImage {
                    data: png_bytes,
                    format: OcrImageFormat::Png,
                },
                options,
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_sets_name_and_url() {
        let e = HttpOcrEngine::new("http://example.com/ocr".into());
        assert_eq!(e.name(), "http-ocr");
        assert_eq!(e.server_url, "http://example.com/ocr");
    }

    #[test]
    fn test_response_deserializes() {
        let raw = r#"{"results":[{"text":"hi","bbox":[1.0,2.0,3.0,4.0],"confidence":0.85}]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        let results = parsed.into_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "hi");
        assert_eq!(results[0].bbox, [1.0, 2.0, 3.0, 4.0]);
        assert!((results[0].confidence - 0.85).abs() < 1e-6);
    }

    #[test]
    fn test_response_deserializes_empty() {
        let raw = r#"{"results":[]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.into_results().is_empty());
    }

    #[test]
    fn test_prod_response_deserializes() {
        let raw = r#"{"document_angle":-90,"result":[[[[10.0,20.0],[60.0,20.0],[60.0,40.0],[10.0,40.0]],"hi",0.85]]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        let results = parsed.into_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "hi");
        // bbox is the polygon's min/max extents.
        assert_eq!(results[0].bbox, [10.0, 20.0, 60.0, 40.0]);
        assert!((results[0].confidence - 0.85).abs() < 1e-6);
        // 4-point polygon forwarded as-is (TL → TR → BR → BL).
        assert_eq!(
            results[0].polygon,
            Some([[10.0, 20.0], [60.0, 20.0], [60.0, 40.0], [10.0, 40.0]])
        );
    }

    #[test]
    fn test_prod_response_empty() {
        let raw = r#"{"document_angle":null,"result":[]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.into_results().is_empty());
    }

    #[tokio::test]
    async fn test_recognize_network_error() {
        // Single attempt so the connection-refused failure surfaces fast
        // instead of grinding through the default multi-attempt backoff.
        let e = HttpOcrEngine::new("http://127.0.0.1:1/ocr".into()).with_retry(OcrRetryConfig {
            max_attempts: 1,
            ..Default::default()
        });
        let opts = OcrOptions {
            language: "eng".into(),
            dpi: 150.0,
        };
        let r = e.recognize(&[0u8; 4], 1, 1, &opts).await;
        assert!(r.is_err());
    }

    #[test]
    fn test_default_retry_matches() {
        let c = OcrRetryConfig::default();
        assert_eq!(c.max_attempts, 10);
        assert_eq!(c.base_backoff_ms, 1000);
        assert_eq!(c.max_backoff_ms, 10_000);
    }

    #[test]
    fn test_jitter_within_bounds() {
        for _ in 0..1000 {
            assert!(jitter_ms(500) <= 500);
        }
        assert_eq!(jitter_ms(0), 0);
    }

    #[test]
    fn test_default_has_no_hedging() {
        assert!(OcrRetryConfig::default().hedge_delays_ms.is_empty());
    }

    // Exercises the hedged spawn/mpsc path: two duplicate requests against a
    // dead endpoint must both fail and the call returns Err (rather than
    // hanging or panicking). Single attempt + zero backoff keeps it fast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_hedged_all_fail_returns_error() {
        let e = HttpOcrEngine::new("http://127.0.0.1:1/ocr".into()).with_retry(OcrRetryConfig {
            max_attempts: 1,
            hedge_delays_ms: vec![0, 10],
            ..Default::default()
        });
        let opts = OcrOptions {
            language: "eng".into(),
            dpi: 150.0,
        };
        let r = e.recognize(&[0u8; 4], 1, 1, &opts).await;
        assert!(r.is_err());
    }
}
