# Backlog

「後でやる」と判断したものを積んでおく場所。着手したら該当項目を消す。

## 未着手

### 断路器が profile 単位で巻き添えを起こす

`src/proxy/fallback.rs`。閾値 3 回・復旧 30 秒で profile ごと遮断する。特定モデルだけが失敗している状況（例: Codex の `gpt-5.6-sol` が容量不足で断続的に 503）で、同じ profile の健全なモデルまで 30 秒止まる。

案は 2 つ。断路器をモデル単位に分けるか、`server_is_overloaded` のようなモデル固有の失敗を計上対象から外すか。前者はフェイルオーバの意味づけが変わるため設計判断が要る。

2026-07-30 に実測で確認。luna と mini が sol の巻き添えで `circuit breaker open` になった。

### 503 のリトライ

`Our servers are currently overloaded`（HTTP 503 / `server_is_overloaded`）は本来リトライすべきエラー。現状は 1 回で諦める。指数バックオフ付きのリトライを入れる余地がある。ただしストリーミング開始後は再送できないため、開始前に限る。

### classifier を別 profile に振り分ける switch

auto mode の classifier リクエストだけを別プロバイダへ送る。指紋は system 先頭の `You are a security monitor for autonomous AI coding agents.`。

用途はローカルモデル（Ollama 等）に判定させてコードを外部に出さないこと、あるいは判定だけ別モデルに寄せること。

**優先度は低い。** classifier は sonnet スロットを引くと実測で確認済みなので、判定モデルの選択は `[profiles.models].sonnet` で既にできる。別プロバイダに振りたくなったときだけ必要。

### `response.incomplete` の扱いが経路で食い違う

- ストリーミング（`translate/responses_stream.rs`）: エラーにせず `max_tokens` 扱い
- 非ストリーミング（`translate/responses.rs` の `aggregate_streamed_response`）: `response.failed` と同列に失敗扱い

意図的な差ではない。どちらかに揃える。`max_tokens` 到達は正常な打ち切りなので、非ストリーミング側をストリーミング側に合わせるのが筋と思われる。

### 既存の clippy 警告 12 件

2026-07-30 時点。いずれも今回の作業とは無関係な既存分。

| 内容 | 箇所 |
|---|---|
| `field assignment outside of initializer` 8 件 | `src/config/mod.rs` |
| `digits grouped inconsistently by underscores` 5 件 | `src/oauth/manager.rs`, `src/oauth/mod.rs` |
| `unnecessary use of is_none()` 2 件 | `src/oauth/providers.rs` |
| `this assertion is always true` 1 件 | `tests/proxy_integration.rs` |

### `config.example.toml` の `claude-max` の base_url

`https://api.claude.ai` になっているが Messages API のホストではない。この profile は proxy をバイパスするため実害は出ていないが、値としては誤り。

### config.example.toml にモデルスロットの説明が無い

`[profiles.models]` の haiku / sonnet / opus / fable が何に効くか、特に auto mode の classifier が sonnet スロットを引くことが書かれていない。

## 判断済み・やらないもの

### classifier を Anthropic 本家へ振り分ける（旧 mode 4）

**実装しない。** 2026-07-30 決定。

サブスクの OAuth トークンで Anthropic の Messages API を叩くと 401 になる。通すには `anthropic-beta` の claude-code フラグと `You are Claude Code, Anthropic's official CLI for Claude.` の system 注入で公式 CLI になりすます必要がある。

Claude Code 自身が第三者エンドポイント向けにはこの名乗りを外している（実測で `You are a Claude agent, built on Anthropic's Claude Agent SDK.` / `cc_entrypoint=sdk-cli` を確認）。偶然の非対応ではなく意図的な線引きであり、それを迂回する実装は入れない。

Console の API キー経由なら正規の経路だが、意図しない従量課金を招くため見送り。

## 環境メモ

### バイナリ差し替えは rm を挟む

macOS では実行中のバイナリを `cp` で上書きすると署名が無効と判断され、以降その binary が SIGKILL される。

```bash
cargo build --release
claudex proxy stop
rm -f ~/.local/bin/claudex
cp target/release/claudex ~/.local/bin/claudex
```

### auto mode の検証は人間が実行する

Claude Code のセッション内から `claude --permission-mode auto` を起動すると、auto mode の classifier に「自律エージェントループの起動」としてブロックされる。検証は別ターミナルか `!` から行う。

検証プローブに `echo` は使えない。Claude Code の組み込み read-only コマンド集合に入っており classifier を通らない。ファイル書き込みを含むコマンドを使う。
