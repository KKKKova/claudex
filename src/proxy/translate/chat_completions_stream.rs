use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::pin::Pin;

use crate::proxy::util::{format_sse, ToolNameMap};

/// Translates an OpenAI SSE stream to Anthropic SSE format.
///
/// OpenAI format:  `data: {"choices":[{"delta":{"content":"..."}}]}`
/// Anthropic format: multiple event types (message_start, content_block_start, content_block_delta, etc.)
pub fn translate_sse_stream<S>(
    input: S,
    tool_name_map: ToolNameMap,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let mut state = StreamState::new(tool_name_map);

    let output = async_stream::stream! {
        // Send message_start
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

                    // Process complete SSE lines
                    while let Some(pos) = buffer.find("\n\n") {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if let Some(events) = state.process_openai_line(&line) {
                            for event in events {
                                yield Ok(Bytes::from(event));
                            }
                        }
                    }
                    // Also handle single newline delimited chunks
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if let Some(events) = state.process_openai_line(&line) {
                            for event in events {
                                yield Ok(Bytes::from(event));
                            }
                        }
                    }
                }
                Err(e) => {
                    // 传输层错误：不再把 reqwest::Error 原样透传导致连接被硬中断，
                    // 收敛成 Anthropic 的 error 事件，让客户端至少能看到失败原因。
                    for event in state.emit_error(&format!("upstream transport error: {e}")) {
                        yield Ok(Bytes::from(event));
                    }
                    return;
                }
            }
        }

        // 终态收敛，policy 与 Responses API 流一致：
        // - 已经发过 error 事件：error 本身就是终态，不再补发后续事件。
        // - 既没见到 [DONE] 也没有任何输出：上游多半中途断开或返回了没识别出来的
        //   失败，视为失败用 error 收尾，避免包装成一次「什么都没说」的正常结束。
        // - 其余情况（正常收到 [DONE]，或虽未见 [DONE] 但已有部分输出）：维持原有的
        //   content_block_stop → message_delta → message_stop 收尾。
        if state.errored {
            // no-op：error 事件已经作为终态发出
        } else if !state.completed && !state.has_output {
            for event in state.emit_error("upstream stream ended without completion") {
                yield Ok(Bytes::from(event));
            }
        } else {
            if state.block_started {
                let block_stop = format_sse("content_block_stop", &json!({
                    "type": "content_block_stop",
                    "index": state.block_index,
                }));
                yield Ok(Bytes::from(block_stop));
            }

            let msg_delta = format_sse("message_delta", &json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": state.output_tokens}
            }));
            yield Ok(Bytes::from(msg_delta));

            yield Ok(Bytes::from(format_sse("message_stop", &json!({"type": "message_stop"}))));
        }
    };

    Box::pin(output)
}

struct StreamState {
    block_index: usize,
    block_started: bool,
    output_tokens: u64,
    current_tool_call: Option<ToolCallState>,
    tool_name_map: ToolNameMap,
    /// 是否已经见过 `[DONE]` 标记。
    completed: bool,
    /// 是否已经产出过任何实质输出（文本 delta 或工具调用），用于判断流
    /// 中途断开时能否保留已产出的部分内容。
    has_output: bool,
    /// 是否已经发出过 Anthropic 的 `error` 事件，发出后即为终态。
    errored: bool,
}

struct ToolCallState {
    id: String,
    name: String,
    arguments_buffer: String,
}

impl StreamState {
    fn new(tool_name_map: ToolNameMap) -> Self {
        Self {
            block_index: 0,
            block_started: false,
            output_tokens: 0,
            current_tool_call: None,
            tool_name_map,
            completed: false,
            has_output: false,
            errored: false,
        }
    }

    /// 根据错误消息内容判断 Anthropic error 的 `type` 字段，只做朴素关键字匹配。
    fn classify_error(message: &str) -> &'static str {
        if message.to_lowercase().contains("overloaded") {
            "overloaded_error"
        } else {
            "api_error"
        }
    }

    /// 收敛出 Anthropic 的 `event: error` 帧：先关掉已打开的 content block（如果有），
    /// 再发 error，避免留下悬空 block。发出后置位 `errored`。
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
        self.current_tool_call = None;

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

    fn process_openai_line(&mut self, line: &str) -> Option<Vec<String>> {
        // 一旦发过 error 事件，流已经终结，后续行一律忽略。
        if self.errored {
            return None;
        }

        let data = line.strip_prefix("data: ")?.trim();

        if data == "[DONE]" {
            self.completed = true;
            return self.finalize_tool_call();
        }

        let parsed: Value = serde_json::from_str(data).ok()?;

        // 部分 OpenAI 兼容供应商（如 OpenRouter）会在流中间插入一个纯 error 对象，
        // 而不是靠 HTTP 状态码报错。这里如果不特判，下面的 `choices` 字段缺失会
        // 直接被 `?` 链吞掉，表现为「什么都没发生」的正常结束——即报告里描述的那类 bug。
        if let Some(error) = parsed.get("error").filter(|e| !e.is_null()) {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("upstream stream failed")
                .to_string();
            return Some(self.emit_error(&message));
        }

        let choice = parsed.get("choices")?.as_array()?.first()?;
        let delta = choice.get("delta")?;

        let mut events = Vec::new();

        // Track usage
        if let Some(usage) = parsed.get("usage") {
            if let Some(tokens) = usage.get("completion_tokens").and_then(|t| t.as_u64()) {
                self.output_tokens = tokens;
            }
        }

        // Handle text content
        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                self.has_output = true;
                // Finalize any pending tool call first
                if let Some(tool_events) = self.finalize_tool_call() {
                    events.extend(tool_events);
                }

                if !self.block_started || self.current_tool_call.is_some() {
                    let block_start = format_sse(
                        "content_block_start",
                        &json!({
                            "type": "content_block_start",
                            "index": self.block_index,
                            "content_block": {"type": "text", "text": ""}
                        }),
                    );
                    events.push(block_start);
                    self.block_started = true;
                }

                let block_delta = format_sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": self.block_index,
                        "delta": {"type": "text_delta", "text": content}
                    }),
                );
                events.push(block_delta);
            }
        }

        // Handle tool calls
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tool_calls {
                let empty_func = json!({});
                let func = tc.get("function").unwrap_or(&empty_func);

                // New tool call starts
                if let Some(id) = tc.get("id").and_then(|id| id.as_str()) {
                    self.has_output = true;
                    // Finalize previous blocks
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
                    if let Some(prev_events) = self.finalize_tool_call() {
                        events.extend(prev_events);
                    }

                    let truncated_name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    // 还原被截断的工具名
                    let name = self
                        .tool_name_map
                        .get(truncated_name)
                        .cloned()
                        .unwrap_or_else(|| truncated_name.to_string());

                    self.current_tool_call = Some(ToolCallState {
                        id: id.to_string(),
                        name: name.clone(),
                        arguments_buffer: String::new(),
                    });

                    events.push(format_sse(
                        "content_block_start",
                        &json!({
                            "type": "content_block_start",
                            "index": self.block_index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": {}
                            }
                        }),
                    ));
                    self.block_started = true;
                }

                // Accumulate arguments
                if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                    if let Some(ref mut tool_state) = self.current_tool_call {
                        tool_state.arguments_buffer.push_str(args);
                        events.push(format_sse(
                            "content_block_delta",
                            &json!({
                                "type": "content_block_delta",
                                "index": self.block_index,
                                "delta": {
                                    "type": "input_json_delta",
                                    "partial_json": args
                                }
                            }),
                        ));
                    }
                }
            }
        }

        // Handle finish_reason
        if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            if finish == "tool_calls" {
                if let Some(tool_events) = self.finalize_tool_call() {
                    events.extend(tool_events);
                }
            }
        }

        if events.is_empty() {
            None
        } else {
            Some(events)
        }
    }

    fn finalize_tool_call(&mut self) -> Option<Vec<String>> {
        let _tool_state = self.current_tool_call.take()?;
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

        Some(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_process_text_delta() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!(
            "data: {}",
            json!({
                "choices": [{"delta": {"content": "Hello"}}]
            })
        );
        let events = state.process_openai_line(&line).unwrap();
        // Should emit content_block_start + content_block_delta
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("content_block_start"));
        assert!(events[1].contains("text_delta"));
        assert!(events[1].contains("Hello"));
        assert!(state.block_started);
    }

    #[test]
    fn test_subsequent_text_delta_no_block_start() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        state.block_started = true; // simulate already started
        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "world"}}]})
        );
        let events = state.process_openai_line(&line).unwrap();
        // Only content_block_delta, no start
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("text_delta"));
    }

    #[test]
    fn test_empty_content_ignored() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!("data: {}", json!({"choices": [{"delta": {"content": ""}}]}));
        assert!(state.process_openai_line(&line).is_none());
    }

    #[test]
    fn test_done_marker() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let result = state.process_openai_line("data: [DONE]");
        // No tool call pending, so None
        assert!(result.is_none());
    }

    #[test]
    fn test_invalid_json_returns_none() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        assert!(state.process_openai_line("data: {invalid}").is_none());
    }

    #[test]
    fn test_no_data_prefix_returns_none() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        assert!(state.process_openai_line("not a data line").is_none());
    }

    #[test]
    fn test_tool_call_start() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!(
            "data: {}",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "id": "call_1",
                            "function": {"name": "search", "arguments": "{\"q\":"}
                        }]
                    }
                }]
            })
        );
        let events = state.process_openai_line(&line).unwrap();
        // Should have content_block_start (tool_use) + content_block_delta (input_json_delta)
        assert!(events.iter().any(|e| e.contains("tool_use")));
        assert!(events.iter().any(|e| e.contains("input_json_delta")));
        assert!(state.current_tool_call.is_some());
    }

    #[test]
    fn test_tool_call_argument_accumulation() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        state.current_tool_call = Some(ToolCallState {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments_buffer: "{\"q\":".to_string(),
        });
        state.block_started = true;

        let line = format!(
            "data: {}",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{"function": {"arguments": "\"rust\"}"}}]
                    }
                }]
            })
        );
        let events = state.process_openai_line(&line).unwrap();
        assert!(events.iter().any(|e| e.contains("input_json_delta")));
        assert_eq!(
            state.current_tool_call.as_ref().unwrap().arguments_buffer,
            "{\"q\":\"rust\"}"
        );
    }

    #[test]
    fn test_finish_reason_tool_calls_finalizes() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        state.current_tool_call = Some(ToolCallState {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments_buffer: "{}".to_string(),
        });
        state.block_started = true;

        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]})
        );
        let events = state.process_openai_line(&line).unwrap();
        assert!(events.iter().any(|e| e.contains("content_block_stop")));
        assert!(state.current_tool_call.is_none());
    }

    #[test]
    fn test_usage_tracking() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!(
            "data: {}",
            json!({
                "choices": [{"delta": {"content": "hi"}}],
                "usage": {"completion_tokens": 42}
            })
        );
        state.process_openai_line(&line);
        assert_eq!(state.output_tokens, 42);
    }

    #[test]
    fn test_finalize_tool_call_no_pending() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        assert!(state.finalize_tool_call().is_none());
    }

    #[test]
    fn test_block_index_increments() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        assert_eq!(state.block_index, 0);

        // Start a text block
        let line1 = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "hi"}}]})
        );
        state.process_openai_line(&line1);
        assert_eq!(state.block_index, 0); // still 0 during first block

        // Start a tool call (should close text block and increment)
        let line2 = format!(
            "data: {}",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{"id": "c1", "function": {"name": "f"}}]
                    }
                }]
            })
        );
        state.process_openai_line(&line2);
        assert_eq!(state.block_index, 1); // incremented after closing text block
    }

    #[test]
    fn test_error_object_mid_stream_emits_error_event() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!(
            "data: {}",
            json!({"error": {"message": "Our servers are currently overloaded. Please try again later.", "code": "server_is_overloaded"}})
        );
        let events = state.process_openai_line(&line).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("event: error"));
        assert!(events[0].contains("Our servers are currently overloaded"));
        assert!(events[0].contains("overloaded_error"));
        assert!(state.errored);
    }

    #[test]
    fn test_error_object_generic_message_is_api_error() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!("data: {}", json!({"error": {"message": "bad gateway"}}));
        let events = state.process_openai_line(&line).unwrap();
        assert!(events[0].contains("\"type\":\"api_error\""));
    }

    #[test]
    fn test_error_field_null_is_not_treated_as_error() {
        // 部分供应商在成功响应里也会带一个恒为 null 的 "error" 字段，不能误判。
        let mut state = StreamState::new(std::collections::HashMap::new());
        let line = format!(
            "data: {}",
            json!({"error": null, "choices": [{"delta": {"content": "hi"}}]})
        );
        let events = state.process_openai_line(&line).unwrap();
        assert!(!state.errored);
        assert!(events.iter().any(|e| e.contains("text_delta")));
    }

    #[test]
    fn test_error_after_open_block_closes_block_before_error() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        state.process_openai_line(&format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "Hello"}}]})
        ));
        assert!(state.block_started);

        let line = format!("data: {}", json!({"error": {"message": "boom"}}));
        let events = state.process_openai_line(&line).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("content_block_stop"));
        assert!(events[1].contains("event: error"));
        assert!(!state.block_started);
    }

    #[test]
    fn test_errored_state_ignores_further_lines() {
        let mut state = StreamState::new(std::collections::HashMap::new());
        state.process_openai_line(&format!("data: {}", json!({"error": {"message": "boom"}})));
        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "late"}}]})
        );
        assert!(state.process_openai_line(&line).is_none());
    }

    // ---- 驱动完整 async 流的端到端测试 ----
    // 传入的每个字符串是一行 SSE data payload（不含 "data: " 前缀），
    // "[DONE]" 会被原样发送以模拟结束标记。

    async fn drive(events: &[&str]) -> Vec<String> {
        let chunks: Vec<Result<Bytes, reqwest::Error>> = events
            .iter()
            .map(|e| Ok(Bytes::from(format!("data: {e}\n\n"))))
            .collect();
        let input = futures::stream::iter(chunks);
        let output = translate_sse_stream(input, std::collections::HashMap::new());
        output
            .map(|r| String::from_utf8(r.unwrap().to_vec()).unwrap())
            .collect::<Vec<_>>()
            .await
    }

    #[tokio::test]
    async fn test_stream_error_object_emits_error_and_no_message_stop() {
        let frames = drive(&[
            r#"{"error":{"message":"Our servers are currently overloaded. Please try again later.","code":"server_is_overloaded"}}"#,
        ])
        .await;
        let combined = frames.join("");
        assert!(combined.contains("event: error"));
        assert!(combined.contains("overloaded_error"));
        assert!(!combined.contains("message_stop"));
        assert!(!combined.contains("message_delta"));
    }

    #[tokio::test]
    async fn test_stream_ends_without_done_or_output_emits_error() {
        // 上游既没发 [DONE] 也没有任何输出内容就断了连接
        let frames = drive(&[]).await;
        let combined = frames.join("");
        assert!(combined.contains("event: error"));
        assert!(!combined.contains("message_stop"));
    }

    #[tokio::test]
    async fn test_stream_ends_without_done_but_with_output_ends_normally() {
        let frames = drive(&[r#"{"choices":[{"delta":{"content":"partial"}}]}"#]).await;
        let combined = frames.join("");
        assert!(!combined.contains("event: error"));
        assert!(combined.contains("message_stop"));
    }

    #[tokio::test]
    async fn test_stream_normal_completion_unaffected() {
        let frames = drive(&[
            r#"{"choices":[{"delta":{"content":"Hi"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])
        .await;
        let combined = frames.join("");
        assert!(!combined.contains("event: error"));
        assert!(combined.contains("content_block_stop"));
        assert!(combined.contains("message_delta"));
        assert!(combined.contains("message_stop"));
    }
}
