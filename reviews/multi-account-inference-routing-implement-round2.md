---
type: implement-review
slug: multi-account-inference-routing
round: 2
commit: 03aaf7c
targets:
  - src/proxy/adapter/direct.rs
  - src/proxy/adapter/mod.rs
  - src/proxy/handler.rs
  - src/process/launch.rs
  - config.example.toml
acceptance: docs/specs/multi-account-inference-routing/plan.md の「受入基準」表（AC-1〜AC-6）
---

## 前回指摘への対応

| 指摘ID | 対応 |
|---|---|
| required-1 | `should_forward_beta(profile) -> bool` を新設。`provider_type == DirectAnthropic` かつ `base_url` の host が `api.anthropic.com`（大小文字無視）のときだけ true。パース失敗時は false。テスト7件（api.anthropic.com / MiniMax / Vertex AI / api.claude.ai / OpenAICompatible+host一致 / パース不能 / 大文字ホスト） |
| required-2 | **異議 + ユーザー裁定**。下記「異議」節 |
| required-3 | ユーザーの再承認を 2026-08-30 に取得。plan frontmatter に `revised: 2026-08-30` を追加し、D-6 の見出しと本文に承認の記録と承認範囲を明記 |
| suggestion-1 | `should_forward_beta` と `build_beta_override` の2関数に切り出してテスト。plan T002 Step 3 が求めていた `custom_headers` の `anthropic-beta`（`Anthropic-Beta` 表記を含む）のテストを回収 |
| suggestion-2 | `build_beta_override` で「クライアントヘッダ → `custom_headers` の `anthropic-beta` → 必要なら oauth」の順に合流させる。`custom_headers` ループ側は `forward_beta` が真のとき `anthropic-beta` をスキップ。テスト4件 |
| suggestion-3 | `OAUTH_BETA` の doc を実機 A-5 での確定に書き換え。plan D-1 末尾も更新 |
| suggestion-4 | `api_key.is_empty()` のとき `auth = "none"` |
| nit-1 | Deviation Log を D-1〜D-7 の昇順に並べ替え |
| nit-2 | `config.example.toml` のプレースホルダを `YOUR_SECOND_ACCOUNT_SETUP_TOKEN` に統一 |
| nit-3 | `merge_anthropic_beta` の doc を入出力の説明に限定し、方針の記述を `should_forward_beta` と呼び出し側へ移設 |
| nit-4 | D-7 に版上げ（rc.3 → rc.4）を記録 |

## 異議: required-2（`vertex-ai` の `default_model`）

指摘は「Vertex の publisher model は `<model>@<version>` 形式なので、バージョン修飾子の無い ID はこの経路で解決しない」としている。この技術的前提に異議を出す。

Anthropic の API リファレンスは Vertex のモデル ID について次のように定めている。

> Vertex model IDs take **no prefix** - current-generation models (Opus 4.8/4.7/4.6, **Sonnet 5**, Sonnet 4.6) use the bare first-party ID (e.g. `"claude-opus-5"`); dated-snapshot models use an `@` version separator (e.g. `claude-opus-4-5@20251101`, **not** `claude-opus-4-5-20251101`).

`@` が要るのは日付スナップショットのモデルであり、現行世代は裸の ID を使う。`claude-sonnet-5` は現行世代として名指しで挙げられている。したがって `claude-sonnet-5` は Vertex でも解決する。

なお Vertex 実機での確認は行っていない。ユーザーは Vertex を使わないため（裁定 2026-08-30）、設定例として `claude-sonnet-5` のまま残す。

指摘のうち「この変更が plan のスコープ宣言にも Deviation Log にも無い」という手続きの部分は受け入れ、D-7 として記録した。

## 差分

`376a7db..03aaf7c`。plan は `docs/specs/multi-account-inference-routing/plan.md`（gitignore 対象・非追跡のため作業ツリーで参照）。
