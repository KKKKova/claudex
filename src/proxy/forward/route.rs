//! forward proxy が終端した（または絶対 URI で届いた）1リクエストの行き先を決める
//!
//! `classify` がパスから種別を判定し、`dispatch` がその種別と `Identity` の組み合わせで
//! 応答を作る。推論（`/v1/messages`）は既存の翻訳ハンドラ（`handler::handle_messages`）へ、
//! Remote Control の制御系パスは本物の `api.anthropic.com`（または絶対 URI の宛先）へ中継し、
//! `count_tokens` はローカル概算で返す。

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::json;

use crate::config::{ProfileConfig, ProviderType};
use crate::proxy::adapter::direct::DirectAnthropicAdapter;
use crate::proxy::adapter::ProviderAdapter;
use crate::proxy::forward::identity::Identity;
use crate::proxy::forward::ForwardState;
use crate::proxy::handler::handle_messages;
use crate::proxy::ProxyState;

/// 制御系プレフィックスの前方一致対象（claude.ai / Claude Code の内部 API）
const PASSTHROUGH_PREFIXES: &[&str] = &[
    "/v1/environments/",
    "/api/claude_code/",
    "/api/oauth/",
    "/api/event_logging/",
    "/api/eval/",
    "/api/v2/logs",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    Inference,
    CountTokens,
    LegacyComplete,
    Passthrough,
    Unknown,
}

/// query を除いた path 部分で分類する
///
/// 完全一致を前方一致より先に見るので、`/v1/messages/count_tokens` が
/// `Inference` に落ちることはない。
pub fn classify(path: &str) -> RouteClass {
    match path {
        "/v1/messages" => RouteClass::Inference,
        "/v1/messages/count_tokens" => RouteClass::CountTokens,
        "/v1/complete" => RouteClass::LegacyComplete,
        _ if PASSTHROUGH_PREFIXES.iter().any(|p| path.starts_with(p)) => RouteClass::Passthrough,
        _ => RouteClass::Unknown,
    }
}

/// リクエスト本文全体の UTF-8 バイト数を 4 で切り上げ除算した概算
pub fn estimate_input_tokens(body: &[u8]) -> u64 {
    (body.len() as u64).div_ceil(4)
}

/// `identity` が `Resolved` かつそのプロファイルが `DirectAnthropic` のときだけ profile を返す
async fn resolved_direct_anthropic_profile(
    state: &ProxyState,
    identity: &Identity,
) -> Option<ProfileConfig> {
    let Identity::Resolved(name) = identity else {
        return None;
    };
    let config = state.config.read().await;
    let profile = config.find_profile(name)?.clone();
    if profile.provider_type != ProviderType::DirectAnthropic {
        return None;
    }
    Some(profile)
}

/// `state.forward` を取り出す。無ければ warn を残して 502 応答を返す
///
/// `Response` は大きい（clippy::result_large_err）ため `Box` で包む
fn require_forward(state: &ProxyState) -> Result<&std::sync::Arc<ForwardState>, Box<Response>> {
    state.forward.as_ref().ok_or_else(|| {
        tracing::warn!("forward dispatch: forward state unavailable for relay");
        Box::new(
            (
                StatusCode::BAD_GATEWAY,
                "claudex: forward proxy is not initialized",
            )
                .into_response(),
        )
    })
}

/// 終端した（または絶対 URI で届いた）1リクエストの行き先を決めて応答を作る。
/// `authority` は絶対 URI 形式のときの宛先。終端側からは None を渡す
pub async fn dispatch(
    state: std::sync::Arc<ProxyState>,
    identity: Identity,
    req: hyper::Request<hyper::body::Incoming>,
    authority: Option<String>,
) -> Response {
    let (parts, incoming) = req.into_parts();
    let method = parts.method;
    let uri = parts.uri;
    let headers = parts.headers;
    let path = uri.path().to_string();
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| path.clone());

    let body = match incoming.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::warn!(error = %e, "forward dispatch: failed to read request body");
            return (
                StatusCode::BAD_GATEWAY,
                format!("claudex: failed to read request body: {e}"),
            )
                .into_response();
        }
    };

    match classify(&path) {
        RouteClass::Inference => match identity {
            Identity::Resolved(name) => {
                handle_messages(State(state.clone()), Path(name), headers, body).await
            }
            Identity::Absent => {
                relay_upstream(
                    &state,
                    authority.as_deref(),
                    &method,
                    &path_and_query,
                    &headers,
                    body,
                )
                .await
            }
            Identity::Unverified | Identity::Unresolved(_) => {
                tracing::warn!(
                    identity = ?identity,
                    path = %path,
                    "forward dispatch: cannot resolve profile for inference"
                );
                (
                    StatusCode::BAD_GATEWAY,
                    "claudex: cannot resolve the profile for this request",
                )
                    .into_response()
            }
        },
        RouteClass::CountTokens => {
            match resolved_direct_anthropic_profile(&state, &identity).await {
                Some(profile) => {
                    relay_to(
                        &state,
                        &profile.base_url,
                        &method,
                        &path_and_query,
                        &headers,
                        body,
                        Some(&profile),
                    )
                    .await
                }
                None => (
                    StatusCode::OK,
                    Json(json!({ "input_tokens": estimate_input_tokens(&body) })),
                )
                    .into_response(),
            }
        }
        RouteClass::LegacyComplete => {
            match resolved_direct_anthropic_profile(&state, &identity).await {
                Some(profile) => {
                    relay_to(
                        &state,
                        &profile.base_url,
                        &method,
                        &path_and_query,
                        &headers,
                        body,
                        Some(&profile),
                    )
                    .await
                }
                None => (
                    StatusCode::NOT_FOUND,
                    "claudex: /v1/complete is not supported for this profile",
                )
                    .into_response(),
            }
        }
        RouteClass::Passthrough => {
            relay_upstream(
                &state,
                authority.as_deref(),
                &method,
                &path_and_query,
                &headers,
                body,
            )
            .await
        }
        RouteClass::Unknown => {
            tracing::warn!(method = %method, path = %path, "unknown forward path");
            relay_upstream(
                &state,
                authority.as_deref(),
                &method,
                &path_and_query,
                &headers,
                body,
            )
            .await
        }
    }
}

/// `authority` があれば `https://{authority}` へ、無ければ `state.forward` の
/// `upstream_base` へ中継する
async fn relay_upstream(
    state: &ProxyState,
    authority: Option<&str>,
    method: &hyper::Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let forward = match require_forward(state) {
        Ok(forward) => forward,
        Err(resp) => return *resp,
    };

    let base = match authority {
        Some(authority) => format!("https://{authority}"),
        None => forward.upstream_base.clone(),
    };

    relay_to(state, &base, method, path_and_query, headers, body, None).await
}

/// `base` + `path_and_query` へ中継する本体。`examples/mitm_poc.rs` の `relay()` が土台。
///
/// `auth_profile` が `Some` のときはクライアントの `Authorization` / `x-api-key` を落とし、
/// `DirectAnthropicAdapter::apply_auth` で上流向けの認証ヘッダを付け直す。
async fn relay_to(
    state: &ProxyState,
    base: &str,
    method: &hyper::Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Bytes,
    auth_profile: Option<&ProfileConfig>,
) -> Response {
    let forward = match require_forward(state) {
        Ok(forward) => forward,
        Err(resp) => return *resp,
    };

    let url = format!("{base}{path_and_query}");

    let mut out_headers = HeaderMap::new();
    for (name, value) in headers.iter() {
        // host はクライアント側の接続に紐づくので付け替えさせる。
        // Proxy-Authorization / Proxy-Connection は claudex との1ホップのヘッダなので落とす。
        if name.as_str().eq_ignore_ascii_case("host")
            || name.as_str().eq_ignore_ascii_case("proxy-authorization")
            || name.as_str().eq_ignore_ascii_case("proxy-connection")
        {
            continue;
        }
        // auth_profile がある経路は apply_auth で組み直すので、クライアントの認証ヘッダは
        // ここで落とす（多アカウント構成でクライアントの資格情報を上流へ漏らさないため）。
        if auth_profile.is_some()
            && (name.as_str().eq_ignore_ascii_case("authorization")
                || name.as_str().eq_ignore_ascii_case("x-api-key"))
        {
            continue;
        }
        out_headers.append(name, value.clone());
    }

    let mut builder = forward
        .client
        .request(method.clone(), url.as_str())
        .headers(out_headers);
    if let Some(profile) = auth_profile {
        builder = DirectAnthropicAdapter.apply_auth(builder, profile);
    }
    builder = builder.body(body);

    let resp = match builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(url = %url, error = %e, "forward dispatch: upstream error");
            return (
                StatusCode::BAD_GATEWAY,
                format!("claudex: upstream error: {e}"),
            )
                .into_response();
        }
    };

    let status = resp.status();
    let mut response_builder = Response::builder().status(status.as_u16());
    for (name, value) in resp.headers().iter() {
        // 本文は reqwest 側でデコード済みなので、転送エンコーディングは持ち越さない
        if matches!(
            name.as_str(),
            "transfer-encoding" | "content-length" | "connection"
        ) {
            continue;
        }
        response_builder = response_builder.header(name, value);
    }

    match response_builder.body(axum::body::Body::from_stream(resp.bytes_stream())) {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(error = %e, "forward dispatch: failed to build relayed response");
            (
                StatusCode::BAD_GATEWAY,
                "claudex: failed to build relayed response",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_inference() {
        assert_eq!(classify("/v1/messages"), RouteClass::Inference);
    }

    #[test]
    fn test_classify_count_tokens_before_prefix() {
        assert_eq!(
            classify("/v1/messages/count_tokens"),
            RouteClass::CountTokens
        );
    }

    #[test]
    fn test_classify_legacy_complete() {
        assert_eq!(classify("/v1/complete"), RouteClass::LegacyComplete);
    }

    #[test]
    fn test_classify_passthrough_prefixes() {
        for path in [
            "/v1/environments/bridge",
            "/v1/environments/abc/work/poll",
            "/api/claude_code/settings",
            "/api/oauth/validate",
            "/api/event_logging/v2/batch",
            "/api/eval/sdk-1",
            "/api/v2/logs",
        ] {
            assert_eq!(classify(path), RouteClass::Passthrough, "path: {path}");
        }
    }

    #[test]
    fn test_classify_unknown() {
        for path in ["/v1/organizations", "/", "/v1/messages/extra"] {
            assert_eq!(classify(path), RouteClass::Unknown, "path: {path}");
        }
    }

    #[test]
    fn test_estimate_input_tokens() {
        assert_eq!(estimate_input_tokens(b""), 0);
        assert_eq!(estimate_input_tokens(b"a"), 1);
        assert_eq!(estimate_input_tokens(b"abcd"), 1);
        assert_eq!(estimate_input_tokens(b"abcde"), 2);
    }
}
