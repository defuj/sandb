//! OpenAI-compatible HTTP client: `GET /models` and a streaming `POST /chat/completions`.
//! Every error leaves out the API key, also when the server echoes it back.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::core::error::{AppError, ErrorCode};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest wait for the response headers and for each stream chunk. Local servers can take
/// long to load a model before the first token.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MODELS_TIMEOUT: Duration = Duration::from_secs(15);
/// Characters of an error body kept in the `ai_upstream` detail.
const BODY_DETAIL_CHARS: usize = 500;
/// Bytes of an error body read at most.
const ERROR_BODY_BYTES: usize = 64 * 1024;
/// Longest SSE line; a longer one is treated as a broken stream.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Highest tool call index accepted in one answer.
const MAX_TOOL_CALLS: usize = 64;

/// One tool call of a model answer. `arguments` is the raw JSON text the model sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A whole streamed answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

pub struct AiClient {
    http: reqwest::Client,
    base: String,
    key: Option<String>,
    idle: Duration,
}

impl AiClient {
    /// `base_url`: the API root without a trailing `/`. `key`: sent as a bearer token when present.
    pub fn new(base_url: &str, key: Option<String>) -> Result<Self, AppError> {
        // reqwest is built without a bundled crypto provider; use ring, as sqlx and the updater
        // do. An error only means a provider is already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| {
                AppError::internal("Could not start the HTTP client").with_detail(chain(&e))
            })?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            key: key.filter(|k| !k.is_empty()),
            idle: DEFAULT_IDLE_TIMEOUT,
        })
    }

    /// Replaces the wait limit for the response headers and for each stream chunk.
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    /// Model ids from `GET {base}/models`.
    pub async fn list_models(&self) -> Result<Vec<String>, AppError> {
        #[derive(Deserialize)]
        struct ModelList {
            data: Vec<Model>,
        }
        #[derive(Deserialize)]
        struct Model {
            id: String,
        }
        let work = async {
            let req = self.auth(self.http.get(format!("{}/models", self.base)));
            let resp = req.send().await.map_err(|e| self.send_error(e))?;
            let resp = self.check_status(resp).await?;
            let bytes = resp.bytes().await.map_err(|e| self.send_error(e))?;
            let list: ModelList = serde_json::from_slice(&bytes).map_err(|e| {
                AppError::new(
                    ErrorCode::AiProtocol,
                    "The AI server sent an unexpected model list",
                )
                .with_detail(e.to_string())
            })?;
            Ok(list.data.into_iter().map(|m| m.id).collect())
        };
        let res = match tokio::time::timeout(MODELS_TIMEOUT, work).await {
            Ok(r) => r,
            Err(_) => Err(timeout_error(MODELS_TIMEOUT)),
        };
        res.map_err(|e| self.redact(e))
    }

    /// Streams `POST {base}/chat/completions` with `body` (which must ask for `stream: true`).
    /// `on_text` gets each text delta; returning false stops the stream as a cancel does.
    pub async fn chat_stream(
        &self,
        body: &Value,
        cancel: &CancellationToken,
        on_text: impl FnMut(&str) -> bool,
    ) -> Result<Completion, AppError> {
        self.stream_inner(body, cancel, on_text)
            .await
            .map_err(|e| self.redact(e))
    }

    async fn stream_inner(
        &self,
        body: &Value,
        cancel: &CancellationToken,
        mut on_text: impl FnMut(&str) -> bool,
    ) -> Result<Completion, AppError> {
        let req = self
            .auth(self.http.post(format!("{}/chat/completions", self.base)))
            .header(ACCEPT, "text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body)?);
        let send = async {
            let resp = req.send().await.map_err(|e| self.send_error(e))?;
            self.check_status(resp).await
        };
        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(cancelled()),
            r = tokio::time::timeout(self.idle, send) => r.map_err(|_| timeout_error(self.idle))??,
        };
        let mut stream = resp.bytes_stream();
        let mut lines = LineBuffer::default();
        let mut acc = Accumulator::default();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(cancelled()),
                n = tokio::time::timeout(self.idle, stream.next()) => n,
            };
            let chunk = match next {
                Err(_) => return Err(timeout_error(self.idle)),
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(AppError::new(
                        ErrorCode::AiProtocol,
                        "The AI server stream broke off",
                    )
                    .with_detail(chain(&e.without_url())));
                }
                Ok(Some(Ok(b))) => b,
            };
            for line in lines.push(&chunk)? {
                if acc.line(&line, &mut on_text)? {
                    return acc.finish();
                }
            }
        }
        if let Some(line) = lines.rest()?
            && acc.line(&line, &mut on_text)?
        {
            return acc.finish();
        }
        // Some servers close without `[DONE]`; a finish reason still marks a whole answer.
        if acc.out.finish_reason.is_some() {
            return acc.finish();
        }
        Err(AppError::new(
            ErrorCode::AiProtocol,
            "The AI server stream ended before the answer was complete",
        ))
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => req.header(AUTHORIZATION, format!("Bearer {k}")),
            None => req,
        }
    }

    fn send_error(&self, e: reqwest::Error) -> AppError {
        let e = e.without_url();
        if e.is_timeout() {
            return AppError::new(ErrorCode::Timeout, "The AI server did not answer in time")
                .with_detail(chain(&e));
        }
        let message = if e.is_connect() {
            format!("Cannot reach the AI server at {}", self.base)
        } else {
            "The connection to the AI server failed".to_string()
        };
        AppError::new(ErrorCode::ConnectFailed, message).with_detail(chain(&e))
    }

    async fn check_status(&self, resp: reqwest::Response) -> Result<reqwest::Response, AppError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(AppError::new(
                ErrorCode::AuthFailed,
                "The AI server refused the API key",
            ));
        }
        let body = tokio::time::timeout(self.idle.min(MODELS_TIMEOUT), read_capped(resp))
            .await
            .unwrap_or_default();
        let err = AppError::new(
            ErrorCode::AiUpstream,
            format!("The AI server answered HTTP {}", status.as_u16()),
        );
        let body = body.trim();
        Err(if body.is_empty() {
            err
        } else {
            err.with_detail(body)
        })
    }

    /// Replaces the key in the message and detail, for servers that echo it in an error, then
    /// cuts a server text detail to `BODY_DETAIL_CHARS`. The cut comes last so that it can
    /// never leave a part of the key.
    fn redact(&self, mut e: AppError) -> AppError {
        if let Some(k) = &self.key {
            e.message = e.message.replace(k.as_str(), "***");
            e.detail = e.detail.map(|d| d.replace(k.as_str(), "***"));
        }
        if e.code == ErrorCode::AiUpstream {
            e.detail = e
                .detail
                .map(|d| d.chars().take(BODY_DETAIL_CHARS).collect());
        }
        e
    }
}

/// The start of an error body, at most `ERROR_BODY_BYTES`.
async fn read_capped(resp: reqwest::Response) -> String {
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(Ok(b)) = stream.next().await {
        out.extend_from_slice(&b);
        if out.len() >= ERROR_BODY_BYTES {
            out.truncate(ERROR_BODY_BYTES);
            break;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Stopped")
}

fn timeout_error(limit: Duration) -> AppError {
    AppError::new(
        ErrorCode::Timeout,
        format!(
            "The AI server sent nothing for {} seconds",
            limit.as_secs_f64()
        ),
    )
}

/// The error and its sources, joined with `: `.
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        out.push_str(": ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

fn protocol(message: &str, detail: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::AiProtocol, message).with_detail(detail)
}

/// Splits the byte stream into lines; a line can span chunks.
#[derive(Default)]
struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, AppError> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            out.push(utf8(&line[..line.len() - 1])?);
        }
        if self.buf.len() > MAX_LINE_BYTES {
            return Err(protocol(
                "The AI server sent a malformed stream",
                "line too long",
            ));
        }
        Ok(out)
    }

    /// The last line when the stream ended without a newline.
    fn rest(&mut self) -> Result<Option<String>, AppError> {
        if self.buf.is_empty() {
            return Ok(None);
        }
        let line = std::mem::take(&mut self.buf);
        utf8(&line).map(Some)
    }
}

fn utf8(line: &[u8]) -> Result<String, AppError> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8(line.to_vec())
        .map_err(|e| protocol("The AI server sent a malformed stream", e.to_string()))
}

/// Builds the answer from SSE lines.
#[derive(Default)]
struct Accumulator {
    out: Completion,
}

impl Accumulator {
    /// Handles one line. `Ok(true)` at `[DONE]`.
    fn line(
        &mut self,
        line: &str,
        on_text: &mut impl FnMut(&str) -> bool,
    ) -> Result<bool, AppError> {
        // Other SSE fields (`event:`, `id:`, `retry:`) and `:` comments carry nothing we use.
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(false);
        };
        let data = data.trim();
        if data.is_empty() {
            return Ok(false);
        }
        if data == "[DONE]" {
            return Ok(true);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| protocol("The AI server sent a malformed stream chunk", e.to_string()))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            return Err(AppError::new(
                ErrorCode::AiUpstream,
                "The AI server reported an error during the answer",
            )
            .with_detail(msg));
        }
        let Some(choice) = v
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            // Usage-only and keep-alive chunks have no choices.
            return Ok(false);
        };
        let delta = choice.get("delta");
        if let Some(text) = delta.and_then(|d| d.get("content")).and_then(Value::as_str)
            && !text.is_empty()
        {
            self.out.text.push_str(text);
            if !on_text(text) {
                return Err(cancelled());
            }
        }
        if let Some(calls) = delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for (pos, call) in calls.iter().enumerate() {
                self.tool_call(pos, call)?;
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.out.finish_reason = Some(reason.to_string());
        }
        Ok(false)
    }

    /// Merges one tool call fragment. Fragments of one call share its `index`. A server that
    /// omits `index` sends each call whole with its `id`, so a new `id` starts a new call.
    fn tool_call(&mut self, pos: usize, call: &Value) -> Result<(), AppError> {
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let calls = &mut self.out.tool_calls;
        let index = match (call.get("index").and_then(Value::as_u64), id) {
            (Some(i), _) => i as usize,
            (None, Some(id)) => match calls.iter().position(|c| c.id == id) {
                Some(i) => i,
                None if calls.last().is_some_and(|c| !c.id.is_empty()) => calls.len(),
                None => pos,
            },
            (None, None) => pos,
        };
        if index >= MAX_TOOL_CALLS {
            return Err(protocol(
                "The AI server sent a malformed tool call",
                format!("tool call index {index}"),
            ));
        }
        if calls.len() <= index {
            calls.resize_with(index + 1, ToolCall::default);
        }
        let slot = &mut calls[index];
        if let Some(id) = id {
            slot.id = id.to_string();
        }
        let func = call.get("function");
        if let Some(name) = func.and_then(|f| f.get("name")).and_then(Value::as_str)
            && slot.name != name
        {
            // Most servers send the name once; a few repeat it whole or split it.
            if slot.name.is_empty() || !name.starts_with(slot.name.as_str()) {
                slot.name.push_str(name);
            } else {
                slot.name = name.to_string();
            }
        }
        if let Some(args) = func
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
        {
            slot.arguments.push_str(args);
        }
        Ok(())
    }

    fn finish(self) -> Result<Completion, AppError> {
        let mut out = self.out;
        for (i, call) in out.tool_calls.iter_mut().enumerate() {
            if call.name.is_empty() {
                return Err(protocol(
                    "The AI server sent a malformed tool call",
                    format!("tool call {i} has no name"),
                ));
            }
            if call.id.is_empty() {
                call.id = format!("call_{i}");
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// One part of a scripted response: wait `ms`, then write `bytes`.
    pub(crate) type Part = (u64, String);

    /// A tiny HTTP server on a free port. Every connection gets `script`, then the socket
    /// closes. The handle yields the raw requests received.
    pub(crate) async fn serve(
        script: Vec<Part>,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let script = script.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let req = read_request(&mut sock).await;
                    let _ = tx.send(req);
                    for (ms, bytes) in script {
                        if ms > 0 {
                            tokio::time::sleep(Duration::from_millis(ms)).await;
                        }
                        if sock.write_all(bytes.as_bytes()).await.is_err() {
                            return;
                        }
                        let _ = sock.flush().await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        (format!("http://{addr}/v1"), rx)
    }

    async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some(end) = text.find("\r\n\r\n") {
                let len = text[..end]
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if buf.len() >= end + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    pub(crate) fn head(status: &str, content_type: &str) -> Part {
        (
            0,
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n"
            ),
        )
    }

    pub(crate) fn sse(v: Value) -> Part {
        (0, format!("data: {v}\n\n"))
    }

    pub(crate) fn text_chunk(s: &str) -> Part {
        sse(json!({"choices": [{"index": 0, "delta": {"content": s}, "finish_reason": null}]}))
    }

    pub(crate) fn finish_chunk(reason: &str) -> Part {
        sse(json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]}))
    }

    pub(crate) fn done() -> Part {
        (0, "data: [DONE]\n\n".into())
    }

    fn body() -> Value {
        json!({"model": "m", "stream": true, "messages": [{"role": "user", "content": "hi"}]})
    }

    async fn run(base: &str, key: Option<&str>) -> (Result<Completion, AppError>, Vec<String>) {
        let client = AiClient::new(base, key.map(String::from))
            .unwrap()
            .with_idle_timeout(Duration::from_millis(500));
        let mut deltas = Vec::new();
        let res = client
            .chat_stream(&body(), &CancellationToken::new(), |d| {
                deltas.push(d.to_string());
                true
            })
            .await;
        (res, deltas)
    }

    #[tokio::test]
    async fn streams_text_across_split_chunks() {
        let whole = format!(
            ": keep-alive\n\nevent: message\r\n{}{}{}{}",
            text_chunk("Hel").1.replace("\n\n", "\r\n\r\n"),
            text_chunk("lo").1,
            finish_chunk("stop").1,
            done().1,
        );
        // Cut the bytes at odd places so that lines span TCP writes.
        let cut: Vec<Part> = whole
            .as_bytes()
            .chunks(7)
            .map(|c| (1, String::from_utf8(c.to_vec()).unwrap()))
            .collect();
        let mut script = vec![head("200 OK", "text/event-stream")];
        script.extend(cut);
        let (base, mut reqs) = serve(script).await;
        let (res, deltas) = run(&base, Some("sk-test")).await;
        let c = res.unwrap();
        assert_eq!(c.text, "Hello");
        assert_eq!(c.finish_reason.as_deref(), Some("stop"));
        assert!(c.tool_calls.is_empty());
        assert_eq!(deltas.concat(), "Hello");
        let req = reqs.recv().await.unwrap();
        assert!(req.starts_with("POST /v1/chat/completions "), "{req}");
        assert!(req.contains("authorization: Bearer sk-test"), "{req}");
        assert!(req.contains(r#""stream":true"#), "{req}");
    }

    #[tokio::test]
    async fn assembles_tool_call_fragments_by_index() {
        let call = |index: u64, id: Option<&str>, name: Option<&str>, args: &str| {
            let mut f = json!({"arguments": args});
            if let Some(n) = name {
                f["name"] = json!(n);
            }
            let mut c = json!({"index": index, "function": f});
            if let Some(i) = id {
                c["id"] = json!(i);
                c["type"] = json!("function");
            }
            sse(json!({"choices": [{"index": 0, "delta": {"tool_calls": [c]}}]}))
        };
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("Let me check. "),
            call(0, Some("call_a"), Some("run_read_query"), ""),
            call(0, None, None, "{\"sql\": \"SELECT"),
            call(1, Some("call_b"), Some("list_tables"), "{"),
            call(0, None, None, " 1\"}"),
            call(1, None, None, "}"),
            finish_chunk("tool_calls"),
            done(),
        ];
        let (base, _reqs) = serve(script).await;
        let c = run(&base, None).await.0.unwrap();
        assert_eq!(c.text, "Let me check. ");
        assert_eq!(c.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            c.tool_calls,
            vec![
                ToolCall {
                    id: "call_a".into(),
                    name: "run_read_query".into(),
                    arguments: "{\"sql\": \"SELECT 1\"}".into(),
                },
                ToolCall {
                    id: "call_b".into(),
                    name: "list_tables".into(),
                    arguments: "{}".into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn missing_tool_call_id_gets_one() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            sse(json!({"choices": [{"delta": {"tool_calls": [
                {"function": {"name": "list_tables", "arguments": "{}"}}
            ]}, "finish_reason": "tool_calls"}]})),
        ];
        let (base, _r) = serve(script).await;
        let c = run(&base, None).await.0.unwrap();
        assert_eq!(c.tool_calls[0].id, "call_0");
        assert_eq!(c.tool_calls[0].name, "list_tables");
    }

    #[tokio::test]
    async fn unindexed_tool_calls_split_on_a_new_id() {
        let call = |id: &str, name: &str, args: &str| {
            sse(json!({"choices": [{"delta": {"tool_calls": [
                {"id": id, "type": "function", "function": {"name": name, "arguments": args}}
            ]}}]}))
        };
        let script = vec![
            head("200 OK", "text/event-stream"),
            call("call_a", "list_tables", "{}"),
            call("call_b", "describe_table", "{\"table\": \"orders\"}"),
            finish_chunk("tool_calls"),
            done(),
        ];
        let (base, _r) = serve(script).await;
        let c = run(&base, None).await.0.unwrap();
        assert_eq!(
            c.tool_calls,
            vec![
                ToolCall {
                    id: "call_a".into(),
                    name: "list_tables".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: "call_b".into(),
                    name: "describe_table".into(),
                    arguments: "{\"table\": \"orders\"}".into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn http_500_is_ai_upstream_with_short_redacted_body() {
        let body = format!("{{\"error\": \"boom for sk-secret\"}}{}", "x".repeat(2000));
        let script = vec![
            head("500 Internal Server Error", "application/json"),
            (0, body),
        ];
        let (base, _r) = serve(script).await;
        let e = run(&base, Some("sk-secret")).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::AiUpstream);
        assert_eq!(e.message, "The AI server answered HTTP 500");
        let detail = e.detail.unwrap();
        assert!(
            detail.starts_with("{\"error\": \"boom for ***\"}"),
            "{detail}"
        );
        assert_eq!(detail.chars().count(), BODY_DETAIL_CHARS);
        assert!(!detail.contains("sk-secret"));
    }

    #[tokio::test]
    async fn http_401_and_403_are_auth_failed() {
        for status in ["401 Unauthorized", "403 Forbidden"] {
            let script = vec![
                head(status, "application/json"),
                (0, "{\"error\": \"bad key sk-secret\"}".into()),
            ];
            let (base, _r) = serve(script).await;
            let e = run(&base, Some("sk-secret")).await.0.unwrap_err();
            assert_eq!(e.code, ErrorCode::AuthFailed, "{status}");
            assert_eq!(e.message, "The AI server refused the API key");
            assert!(e.detail.is_none());
        }
    }

    #[tokio::test]
    async fn malformed_chunk_is_ai_protocol() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("ok"),
            (0, "data: {\"choices\": [\n\n".into()),
            done(),
        ];
        let (base, _r) = serve(script).await;
        let e = run(&base, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::AiProtocol);
        assert_eq!(e.message, "The AI server sent a malformed stream chunk");
    }

    #[tokio::test]
    async fn error_object_in_stream_is_ai_upstream() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            sse(json!({"error": {"message": "context too long"}})),
        ];
        let (base, _r) = serve(script).await;
        let e = run(&base, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::AiUpstream);
        assert_eq!(e.detail.as_deref(), Some("context too long"));
    }

    #[tokio::test]
    async fn early_end_is_ai_protocol() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("half an ans"),
        ];
        let (base, _r) = serve(script).await;
        let (res, deltas) = run(&base, None).await;
        let e = res.unwrap_err();
        assert_eq!(e.code, ErrorCode::AiProtocol);
        assert_eq!(
            e.message,
            "The AI server stream ended before the answer was complete"
        );
        assert_eq!(deltas, vec!["half an ans"]);
    }

    #[tokio::test]
    async fn finish_without_done_is_complete() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("hi"),
            finish_chunk("stop"),
        ];
        let (base, _r) = serve(script).await;
        assert_eq!(run(&base, None).await.0.unwrap().text, "hi");
    }

    #[tokio::test]
    async fn idle_stream_times_out() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("first"),
            (3_000, text_chunk("late").1),
        ];
        let (base, _r) = serve(script).await;
        let started = std::time::Instant::now();
        let e = run(&base, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::Timeout);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn no_response_headers_times_out() {
        let (base, _r) = serve(vec![(3_000, String::new())]).await;
        let e = run(&base, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::Timeout);
    }

    #[tokio::test]
    async fn cancel_stops_the_stream() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("first"),
            (3_000, done().1),
        ];
        let (base, _r) = serve(script).await;
        let client = AiClient::new(&base, None).unwrap();
        let token = CancellationToken::new();
        let t = token.clone();
        let started = std::time::Instant::now();
        let e = client
            .chat_stream(&body(), &token, move |_| {
                t.cancel();
                true
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn receiver_gone_stops_like_cancel() {
        let script = vec![
            head("200 OK", "text/event-stream"),
            text_chunk("a"),
            text_chunk("b"),
            done(),
        ];
        let (base, _r) = serve(script).await;
        let client = AiClient::new(&base, None).unwrap();
        let mut n = 0;
        let e = client
            .chat_stream(&body(), &CancellationToken::new(), |_| {
                n += 1;
                false
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn refused_connection_is_connect_failed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let e = run(&format!("http://{addr}/v1"), None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::ConnectFailed);
        assert!(
            e.message
                .starts_with("Cannot reach the AI server at http://")
        );
    }

    #[tokio::test]
    async fn https_client_builds_without_a_provider_panic() {
        // A plain-text answer to the TLS hello fails the handshake; there must be no panic on a
        // missing crypto provider before that.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
                let _ = sock.shutdown().await;
            }
        });
        let client = AiClient::new(&format!("https://{addr}/v1"), None).unwrap();
        let e = client.list_models().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ConnectFailed, "{e:?}");
    }

    #[tokio::test]
    async fn lists_models() {
        let script = vec![
            head("200 OK", "application/json"),
            (
                0,
                json!({"object": "list", "data": [{"id": "a"}, {"id": "b", "owned_by": "x"}]})
                    .to_string(),
            ),
        ];
        let (base, mut reqs) = serve(script).await;
        let client = AiClient::new(&format!("{base}/"), Some("k1".into())).unwrap();
        assert_eq!(client.list_models().await.unwrap(), vec!["a", "b"]);
        let req = reqs.recv().await.unwrap();
        assert!(req.starts_with("GET /v1/models "), "{req}");
        assert!(req.contains("authorization: Bearer k1"), "{req}");
    }

    #[tokio::test]
    async fn bad_model_list_is_ai_protocol() {
        let script = vec![head("200 OK", "application/json"), (0, "[1, 2]".into())];
        let (base, _r) = serve(script).await;
        let e = AiClient::new(&base, None)
            .unwrap()
            .list_models()
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::AiProtocol);
    }

    #[tokio::test]
    async fn settings_test_checks_the_model() {
        use crate::core::ai::settings::{self, AiSettings};
        use crate::core::secrets::MemoryStore;
        use crate::core::state::AppState;

        let script = vec![
            head("200 OK", "application/json"),
            (0, json!({"data": [{"id": "mock-gpt"}]}).to_string()),
        ];
        let (base, mut reqs) = serve(script).await;
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::new(dir.path(), Box::new(MemoryStore::new())).unwrap();
        settings::set_key(&st, Some("stored-key".into())).unwrap();
        let s = |model: &str| AiSettings {
            base_url: base.clone(),
            model: model.into(),
            ..Default::default()
        };

        let ok = settings::test(&st, s("mock-gpt"), None).await.unwrap();
        assert_eq!(ok.model_count, 1);
        assert!(reqs.recv().await.unwrap().contains("Bearer stored-key"));

        let e = settings::test(&st, s("wrong-model"), Some(String::new()))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::ModelNotFound);
        assert_eq!(e.detail.as_deref(), Some("mock-gpt"));
        assert!(!reqs.recv().await.unwrap().contains("authorization"));

        settings::test(&st, s("mock-gpt"), Some("typed".into()))
            .await
            .unwrap();
        assert!(reqs.recv().await.unwrap().contains("Bearer typed"));
    }
}
