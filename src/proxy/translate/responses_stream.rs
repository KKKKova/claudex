use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::pin::Pin;

use crate::proxy::util::{format_sse, ToolNameMap};

/// Translates an OpenAI Responses API SSE stream to Anthropic SSE format.
///
/// Responses API events: response.created, response.output_text.delta, etc.
/// Anthropic events: message_start, content_block_start, content_block_delta, etc.
pub fn translate_responses_stream<S>(
    input: S,
    tool_name_map: ToolNameMap,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let mut state = ResponsesStreamState::new(tool_name_map);

    let output = async_stream::stream! {
        // Send message_start immediately
        let msg_start = format_sse("message_start", &json!({
            "type": "message_start",
            "message": {
                "id": format!("msg_{}", uuid::Uuid::new_v4()),
                "type": "message",
                "role": "assistant",
                "model": "claudex-proxy",
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        }));
        yield Ok(Bytes::from(msg_start));

        let mut stream = std::pin::pin!(input);
        let mut buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));

                    // Process complete SSE events (separated by double newline or single newline)
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        for event in state.process_line(&line) {
                            yield Ok(Bytes::from(event));
                        }
                    }
                }
                Err(e) => {
                    // 传输层错误（连接中断等）：不再把 reqwest::Error 原样透传导致连接
                    // 被硬中断，而是收敛成 Anthropic 的 error 事件，让客户端至少能看到
                    // 「失败了」而不是一段莫名截断的空响应。
                    for event in state.emit_error(&format!("upstream transport error: {e}")) {
                        yield Ok(Bytes::from(event));
                    }
                    return;
                }
            }
        }

        // 终态收敛：
        // - 已经发过 error 事件（response.failed 或 emit_error 中途触发）：
        //   error 本身就是终态，不再补发 content_block_stop / message_delta / message_stop。
        // - 既没见到 response.completed 也没见到 response.failed，且没有任何输出：
        //   上游多半是中途断开或返回了没识别出来的失败，视为失败用 error 收尾，
        //   避免把它包装成一次「什么都没说」的正常结束。
        // - 其余情况（正常 completed，或虽未见 completed 但已有部分输出）：维持原有的
        //   content_block_stop → message_delta → message_stop 收尾。
        if state.errored {
            // no-op：error 事件已经作为终态发出
        } else if !state.completed && !state.has_output {
            for event in state.emit_error("upstream stream ended without completion") {
                yield Ok(Bytes::from(event));
            }
        } else {
            if state.block_started {
                yield Ok(Bytes::from(format_sse("content_block_stop", &json!({
                    "type": "content_block_stop",
                    "index": state.block_index,
                }))));
            }

            let stop_reason = if state.has_tool_use { "tool_use" } else { &state.stop_reason };
            yield Ok(Bytes::from(format_sse("message_delta", &json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                "usage": {"output_tokens": state.output_tokens}
            }))));
            yield Ok(Bytes::from(format_sse("message_stop", &json!({"type": "message_stop"}))));
        }
    };

    Box::pin(output)
}

struct ResponsesStreamState {
    tool_name_map: ToolNameMap,
    block_index: usize,
    block_started: bool,
    has_tool_use: bool,
    stop_reason: String,
    output_tokens: u64,
    /// 是否已经见过 `response.completed` 事件。
    completed: bool,
    /// 是否已经产出过任何实质输出（文本 delta 或工具调用），用于判断流
    /// 中途断开时能否保留已产出的部分内容。
    has_output: bool,
    /// 是否已经发出过 Anthropic 的 `error` 事件。一旦置位，外层收尾逻辑
    /// 不再补发 content_block_stop / message_delta / message_stop，
    /// error 事件本身就是终态。
    errored: bool,
}

impl ResponsesStreamState {
    fn new(tool_name_map: ToolNameMap) -> Self {
        Self {
            tool_name_map,
            block_index: 0,
            block_started: false,
            has_tool_use: false,
            stop_reason: "end_turn".to_string(),
            output_tokens: 0,
            completed: false,
            has_output: false,
            errored: false,
        }
    }

    /// 根据错误消息内容判断 Anthropic error 的 `type` 字段。
    /// 只做最朴素的关键字匹配，不做过度分类。
    fn classify_error(message: &str) -> &'static str {
        if message.to_lowercase().contains("overloaded") {
            "overloaded_error"
        } else {
            "api_error"
        }
    }

    /// 收敛出 Anthropic 的 `event: error` 帧：如果有已打开的 content block，
    /// 先补一个 content_block_stop 把它关掉，再发 error，避免留下一个
    /// 没有 stop 的悬空 block。发出后置位 `errored`，外层收尾逻辑据此
    /// 跳过后续的正常终态事件。
    fn emit_error(&mut self, message: &str) -> Vec<String> {
        let mut events = Vec::new();
        if self.block_started {
            events.push(format_sse(
                "content_block_stop",
                &json!({
                    "type": "content_block_stop",
                    "index": self.block_index,
                }),
            ));
            self.block_started = false;
        }

        let error_type = Self::classify_error(message);
        events.push(format_sse(
            "error",
            &json!({
                "type": "error",
                "error": {
                    "type": error_type,
                    "message": message,
                }
            }),
        ));
        self.errored = true;
        events
    }

    fn process_line(&mut self, line: &str) -> Vec<String> {
        // 一旦发过 error 事件，流已经终结，后续行（如果上游还有残留数据）一律忽略。
        if self.errored {
            return vec![];
        }

        // Responses API SSE format: "event: <type>\ndata: <json>" or just "data: <json>"
        // We may receive "event:" and "data:" lines separately
        if line.starts_with("event:") {
            // Event type line — we'll get the data in the next line
            return vec![];
        }

        let data = if let Some(stripped) = line.strip_prefix("data: ") {
            stripped
        } else if let Some(stripped) = line.strip_prefix("data:") {
            stripped
        } else {
            return vec![];
        };

        let json: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![],
        };

        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match event_type {
            "response.output_text.delta" => {
                let delta = json.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                if delta.is_empty() {
                    return vec![];
                }
                self.has_output = true;

                let mut events = Vec::new();

                // Start content block if not started
                if !self.block_started {
                    events.push(format_sse(
                        "content_block_start",
                        &json!({
                            "type": "content_block_start",
                            "index": self.block_index,
                            "content_block": {"type": "text", "text": ""},
                        }),
                    ));
                    self.block_started = true;
                }

                events.push(format_sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": self.block_index,
                        "delta": {"type": "text_delta", "text": delta},
                    }),
                ));

                events
            }
            "response.output_text.done" | "response.content_part.done" => {
                if self.block_started {
                    self.block_started = false;
                    let event = format_sse(
                        "content_block_stop",
                        &json!({
                            "type": "content_block_stop",
                            "index": self.block_index,
                        }),
                    );
                    self.block_index += 1;
                    return vec![event];
                }
                vec![]
            }
            "response.output_item.added" => {
                // Check if it's a function_call
                let empty = json!({});
                let item = json.get("item").unwrap_or(&empty);
                let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

                if item_type == "function_call" {
                    self.has_tool_use = true;
                    self.has_output = true;
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown");
                    let original_name = self
                        .tool_name_map
                        .get(name)
                        .cloned()
                        .unwrap_or(name.to_string());
                    let call_id = item
                        .get("call_id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("call_0");

                    // Close any previous block
                    let mut events = Vec::new();
                    if self.block_started {
                        events.push(format_sse(
                            "content_block_stop",
                            &json!({
                                "type": "content_block_stop",
                                "index": self.block_index,
                            }),
                        ));
                        self.block_index += 1;
                        self.block_started = false;
                    }

                    events.push(format_sse(
                        "content_block_start",
                        &json!({
                            "type": "content_block_start",
                            "index": self.block_index,
                            "content_block": {
                                "type": "tool_use",
                                "id": call_id,
                                "name": original_name,
                                "input": {},
                            },
                        }),
                    ));
                    self.block_started = true;

                    return events;
                }
                vec![]
            }
            "response.function_call_arguments.delta" => {
                let delta = json.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                if delta.is_empty() {
                    return vec![];
                }
                self.has_output = true;

                vec![format_sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": self.block_index,
                        "delta": {"type": "input_json_delta", "partial_json": delta},
                    }),
                )]
            }
            "response.function_call_arguments.done" => {
                if self.block_started {
                    self.block_started = false;
                    let event = format_sse(
                        "content_block_stop",
                        &json!({
                            "type": "content_block_stop",
                            "index": self.block_index,
                        }),
                    );
                    self.block_index += 1;
                    return vec![event];
                }
                vec![]
            }
            "response.completed" => {
                self.completed = true;
                // Extract usage from the completed response
                if let Some(resp) = json.get("response") {
                    if let Some(usage) = resp.get("usage") {
                        self.output_tokens = usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                    let status = resp
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("completed");
                    if status == "incomplete" {
                        self.stop_reason = "max_tokens".to_string();
                    }
                }
                // Don't emit anything here — finalization happens in the outer stream
                vec![]
            }
            "response.failed" => {
                // 上游明确宣告失败：不能当作正常结束静默吞掉，转成 Anthropic 的
                // error 事件让客户端知道发生了什么。
                let message = json
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("upstream stream failed")
                    .to_string();
                self.emit_error(&message)
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_delta() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        let events = state.process_line(
            r#"data: {"type":"response.output_text.delta","delta":"Hello","output_index":0,"content_index":0}"#,
        );
        // Should get content_block_start + content_block_delta
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("content_block_start"));
        assert!(events[1].contains("text_delta"));
        assert!(events[1].contains("Hello"));
    }

    #[test]
    fn test_function_call_flow() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());

        // function_call added
        let events = state.process_line(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"","status":"in_progress"}}"#,
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("tool_use"));
        assert!(events[0].contains("get_weather"));

        // argument delta
        let events = state.process_line(
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"loc\""}"#,
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("input_json_delta"));

        // arguments done
        let events = state.process_line(
            r#"data: {"type":"response.function_call_arguments.done","name":"get_weather","arguments":"{\"location\":\"Paris\"}"}"#,
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("content_block_stop"));
        assert!(state.has_tool_use);
    }

    #[test]
    fn test_completed_extracts_usage() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        state.process_line(
            r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150}}}"#,
        );
        assert_eq!(state.output_tokens, 50);
    }

    #[test]
    fn test_response_failed_emits_error_with_upstream_message() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        let events = state.process_line(
            r#"data: {"type":"response.failed","response":{"error":{"message":"Our servers are currently overloaded. Please try again later."}}}"#,
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("event: error"));
        assert!(events[0].contains("Our servers are currently overloaded"));
        assert!(state.errored);
    }

    #[test]
    fn test_response_failed_overloaded_message_is_overloaded_error() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        let events = state.process_line(
            r#"data: {"type":"response.failed","response":{"error":{"message":"server_is_overloaded: Our servers are currently overloaded."}}}"#,
        );
        assert!(events[0].contains("\"type\":\"overloaded_error\""));
    }

    #[test]
    fn test_response_failed_generic_message_is_api_error() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        let events = state.process_line(
            r#"data: {"type":"response.failed","response":{"error":{"message":"internal error"}}}"#,
        );
        assert!(events[0].contains("\"type\":\"api_error\""));
    }

    #[test]
    fn test_response_failed_without_message_uses_generic_text() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        let events = state.process_line(r#"data: {"type":"response.failed","response":{}}"#);
        assert!(events[0].contains("upstream stream failed"));
    }

    #[test]
    fn test_response_failed_after_open_block_closes_block_before_error() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        state.process_line(
            r#"data: {"type":"response.output_text.delta","delta":"Hello","output_index":0,"content_index":0}"#,
        );
        assert!(state.block_started);

        let events = state.process_line(
            r#"data: {"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        );
        // 先关闭已打开的 content block，再发 error
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("content_block_stop"));
        assert!(events[1].contains("event: error"));
        assert!(!state.block_started);
    }

    #[test]
    fn test_errored_state_ignores_further_lines() {
        let mut state = ResponsesStreamState::new(ToolNameMap::new());
        state.process_line(
            r#"data: {"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        );
        let events = state.process_line(
            r#"data: {"type":"response.output_text.delta","delta":"late","output_index":0,"content_index":0}"#,
        );
        assert!(events.is_empty());
    }

    // ---- 驱动完整 async 流的端到端测试 ----

    async fn drive(events: &[&str]) -> Vec<String> {
        let chunks: Vec<Result<Bytes, reqwest::Error>> = events
            .iter()
            .map(|e| Ok(Bytes::from(format!("data: {e}\n\n"))))
            .collect();
        let input = futures::stream::iter(chunks);
        let output = translate_responses_stream(input, ToolNameMap::new());
        output
            .map(|r| String::from_utf8(r.unwrap().to_vec()).unwrap())
            .collect::<Vec<_>>()
            .await
    }

    #[tokio::test]
    async fn test_stream_response_failed_emits_error_and_no_message_stop() {
        let frames = drive(&[
            r#"{"type":"response.failed","response":{"error":{"message":"Our servers are currently overloaded. Please try again later."}}}"#,
        ])
        .await;
        let combined = frames.join("");
        assert!(combined.contains("event: error"));
        assert!(combined.contains("Our servers are currently overloaded"));
        assert!(combined.contains("overloaded_error"));
        assert!(!combined.contains("message_stop"));
        assert!(!combined.contains("message_delta"));
    }

    #[tokio::test]
    async fn test_stream_open_block_closed_before_error_on_failure() {
        let frames = drive(&[
            r#"{"type":"response.output_text.delta","delta":"Hello","output_index":0,"content_index":0}"#,
            r#"{"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        ])
        .await;
        let stop_idx = frames
            .iter()
            .position(|f| f.contains("content_block_stop"))
            .expect("content_block_stop should be emitted");
        let error_idx = frames
            .iter()
            .position(|f| f.contains("event: error"))
            .expect("error event should be emitted");
        assert!(stop_idx < error_idx);
        assert!(!frames.iter().any(|f| f.contains("message_stop")));
    }

    #[tokio::test]
    async fn test_stream_ends_without_completed_or_output_emits_error() {
        // 上游既没发 response.completed 也没发 response.failed，什么输出都没有就断了连接
        let frames = drive(&[]).await;
        let combined = frames.join("");
        assert!(combined.contains("event: error"));
        assert!(!combined.contains("message_stop"));
    }

    #[tokio::test]
    async fn test_stream_ends_without_completed_but_with_output_ends_normally() {
        // 没见到 response.completed，但已经吐出过 output_text.delta —— 不能把已产出的内容丢掉
        let frames = drive(&[
            r#"{"type":"response.output_text.delta","delta":"partial answer","output_index":0,"content_index":0}"#,
        ])
        .await;
        let combined = frames.join("");
        assert!(!combined.contains("event: error"));
        assert!(combined.contains("message_stop"));
        assert!(combined.contains("message_delta"));
    }

    #[tokio::test]
    async fn test_stream_normal_completion_unaffected() {
        let frames = drive(&[
            r#"{"type":"response.output_text.delta","delta":"Hi","output_index":0,"content_index":0}"#,
            r#"{"type":"response.output_text.done"}"#,
            r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":1,"output_tokens":2}}}"#,
        ])
        .await;
        let combined = frames.join("");
        assert!(!combined.contains("event: error"));
        assert!(combined.contains("content_block_stop"));
        assert!(combined.contains("message_delta"));
        assert!(combined.contains("message_stop"));
    }
}
