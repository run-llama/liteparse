use std::{io::Cursor, pin::Pin, time::Duration};

use image::ImageFormat;
use reqwest::{
    Client,
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize};

use crate::ocr::{OcrEngine, OcrOptions, OcrResult, OcrWord};

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
    /// Optional measured words of this result, returned when the request set
    /// `return_word_boxes`.
    #[serde(default)]
    words: Vec<OcrWord>,
}

impl HttpOcrResponseItem {
    fn into_ocr_result(self) -> OcrResult {
        OcrResult {
            text: self.text,
            bbox: self.bbox,
            confidence: self.confidence,
            polygon: self.polygon,
            words: self.words,
        }
    }
}

/// A single detection from an OCR worker, which emits
/// EasyOCR/PaddleOCR-style positional tuples: `[polygon, text, confidence]`,
/// where `polygon` is a list of `[x, y]` points (4 for both engines) ordered
/// top-left → top-right → bottom-right → bottom-left.
#[derive(Debug, Deserialize)]
struct ProdOcrItem(Vec<[f32; 2]>, String, f32);

/// One measured word from the prod OCR worker's `words` (`return_word_boxes`).
#[derive(Debug, Deserialize)]
struct ProdOcrWord {
    bbox: Vec<[f32; 2]>,
    text: String,
}

/// Axis-aligned `[x1, y1, x2, y2]` from a polygon's min/max extents.
fn polygon_bounds(poly: &[[f32; 2]]) -> [f32; 4] {
    poly.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |[x1, y1, x2, y2], [x, y]| [x1.min(*x), y1.min(*y), x2.max(*x), y2.max(*y)],
    )
}

impl ProdOcrItem {
    fn into_ocr_result(self, words: Vec<ProdOcrWord>) -> OcrResult {
        let ProdOcrItem(poly, text, confidence) = self;
        let [min_x, min_y, max_x, max_y] = polygon_bounds(&poly);
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
            words: words
                .into_iter()
                .map(|w| OcrWord {
                    bbox: polygon_bounds(&w.bbox),
                    polygon: match w.bbox.as_slice() {
                        [a, b, c, d] => Some([*a, *b, *c, *d]),
                        _ => None,
                    },
                    text: w.text,
                })
                .collect(),
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
    Standard {
        results: Vec<HttpOcrResponseItem>,
    },
    /// `words[i]` holds the measured words of `result[i]` when the request set
    /// `return_word_boxes`.
    Prod {
        result: Vec<ProdOcrItem>,
        #[serde(default)]
        words: Vec<Vec<ProdOcrWord>>,
    },
}

impl HttpOcrResponse {
    fn into_results(self) -> Vec<OcrResult> {
        match self {
            HttpOcrResponse::Standard { results } => {
                results.into_iter().map(|i| i.into_ocr_result()).collect()
            }
            HttpOcrResponse::Prod { result, mut words } => {
                // Words that do not pair one list per line cannot be attributed.
                if words.len() != result.len() {
                    words.clear();
                }
                let mut words = words.into_iter();
                result
                    .into_iter()
                    .map(|i| i.into_ocr_result(words.next().unwrap_or_default()))
                    .collect()
            }
        }
    }
}

/// HTTP-based OCR engine that conforms to LiteParse OCR API specification.
/// The server must implement the API defined in OCR_API_SPEC.md:
///     - POST /ocr endpoint
///     - Accepts multipart/form-data with 'file', 'language' and
///       'correct_rotation' (from `OcrOptions::correct_rotation`) fields, plus
///       'return_word_boxes', 'dense_ocr_detection' and 'return_bbox_angles'
///       when the engine is built `with_word_boxes(true)`
///     - Returns JSON: { results: [{ text, bbox: [x1,y1,x2,y2], confidence }] }
/// See ocr/easyocr/ and ocr/paddleocr/ for example server implementations.
pub struct HttpOcrEngine {
    pub name: String,
    server_url: String,
    /// Extra headers (name, value) sent with every request, e.g. auth tokens.
    headers: Vec<(String, String)>,
    /// Retry/backoff policy. Defaults to worker-parity semantics.
    retry: OcrRetryConfig,
    /// Ask the server for measured word boxes (see `with_word_boxes`).
    word_boxes: bool,
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

/// The text fields of one OCR request: `language`, `correct_rotation`, and
/// the fields every LlamaParse tier sends when it wants word boxes.
fn request_fields(
    language: &str,
    correct_rotation: bool,
    word_boxes: bool,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("language", language.to_string()),
        ("correct_rotation", correct_rotation.to_string()),
    ];
    if word_boxes {
        for name in [
            "return_word_boxes",
            "dense_ocr_detection",
            "return_bbox_angles",
        ] {
            fields.push((name, "true".to_string()));
        }
    }
    fields
}

/// Send a single OCR request and return the raw response body. Encodes the
/// multipart form fresh each call (a `Form` is consumed by `send`).
async fn send_one(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    png_bytes: &[u8],
    fields: &[(&'static str, String)],
    timeout_ms: u64,
) -> Result<String, reqwest::Error> {
    let mut form = Form::new().part(
        "file",
        Part::bytes(png_bytes.to_vec())
            .file_name("image.png")
            .mime_str("image/png")?,
    );
    for (name, value) in fields {
        form = form.text(*name, value.clone());
    }
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
            word_boxes: false,
        }
    }

    /// Request measured word boxes from the server (`return_word_boxes`,
    /// `dense_ocr_detection`, `return_bbox_angles`). Results then carry
    /// `words`; a server that ignores the fields returns none, and merging
    /// estimates the boxes instead.
    pub fn with_word_boxes(mut self, word_boxes: bool) -> Self {
        self.word_boxes = word_boxes;
        self
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
        png_bytes: &[u8],
        language: &str,
        correct_rotation: bool,
    ) -> Result<String, reqwest::Error> {
        let delays = &self.retry.hedge_delays_ms;
        let timeout_ms = self.retry.request_timeout_ms;
        let fields = request_fields(language, correct_rotation, self.word_boxes);

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
                png_bytes,
                &fields,
                timeout_ms,
            )
            .await;
        }

        // Hedged path: spawn one task per delay (spawned tasks need owned data,
        // so clone the cheap bits and the PNG per hedge — the duplicate upload
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
            let png = png_bytes.to_vec();
            let fields = fields.clone();
            handles.push(tokio::spawn(async move {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                let res = send_one(&client, &url, &headers, &png, &fields, timeout_ms).await;
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
}

impl OcrEngine for HttpOcrEngine {
    fn name(&self) -> &str {
        &self.name
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
            // Encode raw RGB bytes as PNG for the server
            let img: image::RgbImage =
                image::ImageBuffer::from_raw(width, height, image_data.to_vec())
                    .ok_or("failed to create image buffer from raw RGB data")?;
            let mut png_bytes = Vec::new();
            img.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)?;

            let client = Client::new();

            // Retry loop: each attempt sends one OCR request (or a hedged group
            // of duplicates when configured) and backs off exponentially on
            // transient failures. The PNG bytes above are encoded once and
            // cloned per request.
            let max_attempts = self.retry.max_attempts.max(1);
            let mut attempt: u32 = 0;
            let raw = loop {
                attempt += 1;
                match self
                    .send_hedged(
                        &client,
                        &png_bytes,
                        &options.language,
                        options.correct_rotation,
                    )
                    .await
                {
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

    /// `return_word_boxes` adds `words`, one list per `result` line; each word's
    /// quad becomes an axis-aligned box on its own line.
    #[test]
    fn test_prod_response_attaches_measured_words() {
        let raw = r#"{"document_angle":0,"result":[
            [[[10,20],[200,20],[200,40],[10,40]],"Invoice total",0.9],
            [[[10,50],[60,50],[60,70],[10,70]],"42.50",0.8]],
          "words":[
            [{"bbox":[[10,20],[90,21],[90,40],[10,39]],"text":"Invoice","confidence":0.9},
             {"bbox":[[100,20],[200,20],[200,40],[100,40]],"text":"total","confidence":0.9}],
            [{"bbox":[[12,50],[58,50],[58,70],[12,70]],"text":"42.50","confidence":0.8}]]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        let results = parsed.into_results();
        let words: Vec<Vec<(&str, [f32; 4])>> = results
            .iter()
            .map(|r| r.words.iter().map(|w| (w.text.as_str(), w.bbox)).collect())
            .collect();
        assert_eq!(
            words,
            vec![
                vec![
                    ("Invoice", [10.0, 20.0, 90.0, 40.0]),
                    ("total", [100.0, 20.0, 200.0, 40.0])
                ],
                vec![("42.50", [12.0, 50.0, 58.0, 70.0])],
            ]
        );
        // The quad is kept so merging can take its bounds after a deskew.
        assert_eq!(
            results[0].words[0].polygon,
            Some([[10.0, 20.0], [90.0, 21.0], [90.0, 40.0], [10.0, 39.0]])
        );
    }

    /// Word lists that do not pair one per line are dropped, not shifted onto
    /// the wrong lines.
    #[test]
    fn test_prod_words_not_paired_per_line_are_dropped() {
        let raw = r#"{"result":[
            [[[0,0],[10,0],[10,10],[0,10]],"a",0.9],
            [[[0,20],[10,20],[10,30],[0,30]],"b",0.9]],
          "words":[[{"bbox":[[0,20],[10,20],[10,30],[0,30]],"text":"b","confidence":0.9}]]}"#;
        let parsed: HttpOcrResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.into_results().iter().all(|r| r.words.is_empty()));
    }

    /// Serve one OCR request on a local socket and return the request bytes.
    /// `engine` builds the engine under test from the socket's URL.
    async fn capture_request(
        correct_rotation: bool,
        engine: impl FnOnce(String) -> std::sync::Arc<dyn OcrEngine>,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/ocr", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            // Read until the multipart closing boundary arrives.
            while !String::from_utf8_lossy(&req).trim_end().ends_with("--") {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let body = r#"{"results":[]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&req).into_owned()
        });
        let engine = engine(url);
        let opts = OcrOptions {
            language: "en".into(),
            dpi: 150.0,
            correct_rotation,
        };
        engine.recognize(&[255u8; 3], 1, 1, &opts).await.unwrap();
        server.await.unwrap()
    }

    /// `ocr_word_boxes` sends the three fields the LlamaParse tiers send for
    /// word boxes; off, the request carries none of them.
    #[tokio::test]
    async fn test_ocr_word_boxes_option_sends_the_word_box_fields() {
        use crate::{config::LiteParseConfig, parser::LiteParse};
        let request = |on: bool| {
            capture_request(true, move |url| {
                LiteParse::new(LiteParseConfig {
                    ocr_enabled: true,
                    ocr_server_url: Some(url),
                    ocr_word_boxes: on,
                    ..Default::default()
                })
                .ocr_engine()
                .unwrap()
                .unwrap()
            })
        };
        let fields = [
            "return_word_boxes",
            "dense_ocr_detection",
            "return_bbox_angles",
        ];
        let on = request(true).await;
        for f in fields {
            assert!(
                on.contains(&format!("name=\"{f}\"\r\n\r\ntrue")),
                "{f} not sent"
            );
        }
        let off = request(false).await;
        assert!(fields.iter().all(|f| !off.contains(f)), "{off}");
    }

    /// Every request states `correct_rotation`, with or without word boxes,
    /// so the server never falls back to its own default.
    #[tokio::test]
    async fn test_every_request_sends_correct_rotation() {
        for (value, word_boxes) in [(true, false), (false, false), (false, true)] {
            let req = capture_request(value, move |url| -> std::sync::Arc<dyn OcrEngine> {
                std::sync::Arc::new(HttpOcrEngine::new(url).with_word_boxes(word_boxes))
            })
            .await;
            assert!(
                req.contains(&format!("name=\"correct_rotation\"\r\n\r\n{value}\r\n")),
                "correct_rotation={value} not sent: {req}"
            );
        }
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
            correct_rotation: true,
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
            correct_rotation: true,
        };
        let r = e.recognize(&[0u8; 4], 1, 1, &opts).await;
        assert!(r.is_err());
    }
}
