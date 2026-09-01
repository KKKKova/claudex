---
type: implement-review-result
slug: multi-account-inference-routing
round: 1
verdict: REQUEST_CHANGES
blocking: 3
reviewed: f944c64..376a7db
date: 2026-08-29
---

# レビュー結果: implement multi-account-inference-routing round1

> **REQUEST_CHANGES** — blocking 3件 / suggestion 4件 / nit 4件。
> 判断: 目標2の本筋（`apply_auth` の OAuth 分岐、`merge_anthropic_beta`、launch 警告）は設計どおりで、実機 e2e も plan が定めた順序で成立している。止めたのは本筋の外にある3点で、いずれも目標2と無関係の既存プロファイルに影響が及ぶ。
> 根拠: `anthropic-beta` の引き継ぎが `DirectAnthropic` 全体に掛かっており、api.anthropic.com 以外の DirectAnthropic 上流（minimax・vertex-ai）にもクライアントの beta フラグが飛ぶ。加えて `config.example.toml` のモデル ID 一括更新が Vertex の ID 形式を壊しており、この変更自体が plan のスコープ宣言にも Deviation Log にも無い。

ローカルで `cargo clippy --all-targets`（warning 無し）と `cargo test`（421 passed）を再実行し、緑を確認した。AC-6 のローカル分は成立している。

## 受入基準の確認

| AC | 状態 | 根拠 |
|---|---|---|
| AC-1 | 確定 | 実機 A-5 で HTTP 200。トークン軸・system 制約軸とも plan に記録あり |
| AC-2 | 確定 | `direct.rs` の単体テスト4件 + 実機ログ。ただし `custom_headers` に beta がある場合の分岐は未テスト（suggestion-1） |
| AC-3 | 確定 | 実機 B-5〜B-7 |
| AC-4 | 確定 | `launch.rs` のテスト4件 + macOS 実行時確認 |
| AC-5 | 確定 | 実機 B-4。壊したトークンで上流 401、フォールバック無し |
| AC-6 | 部分 | ローカル緑を本レビューでも再現。CI `windows-check` は PR 待ち |

## Deviation Log の妥当性

D-1（ゲート先行実装）・D-2（可視性1行）・D-3（`auth` ラベルを provider_type で分岐）・D-4（worktree 不使用）・D-5（手順書の置き場）は、いずれも記録の粒度と判断の理由が対応しており妥当。とくに D-3 は、ログが他アダプタ経由のリクエストに `x-api-key` と嘘をつく問題を先回りで潰しており、AC-5 の証跡としての価値を守っている。

D-6 は内容としては正しい発見だが、手続きが閉じていない（required-3）。また Deviation Log に載っていない変更が2つある（required-2、nit-4）。

## Blocking

<details>
<summary><b>[required-1]</b> クライアントの <code>anthropic-beta</code> 引き継ぎが api.anthropic.com 以外の DirectAnthropic 上流にも掛かる</summary>

`src/proxy/handler.rs:407` のガードは `provider_type == DirectAnthropic` のみで、上流ホストを見ていない。

```rust
let beta_override = if profile.provider_type == crate::config::ProviderType::DirectAnthropic
    && !has_custom_beta
```

しかし `DirectAnthropic` は「Anthropic のプロトコルを話す上流」であって「api.anthropic.com」ではない。`config.example.toml` だけでも該当プロファイルは4つあり、うち2つは Anthropic 本体ではない。

| プロファイル | base_url | この変更の影響 |
|---|---|---|
| `anthropic`（72行） | api.anthropic.com | 意図どおり（むしろ従来 400 になりえた本体リクエストが通る） |
| `minimax`（82行） | MiniMax の Anthropic 互換 | `context-management-2025-06-27` 等が新たに飛ぶ |
| `claude-max-second-account`（173行） | api.anthropic.com | 意図どおり |
| `vertex-ai`（259行） | Google Vertex AI | 同上 |

D-6 の本文は「api.anthropic.com が beta フラグを要求する」ことだけを根拠にしているのに、実装は DirectAnthropic 全体へ広げている。コード中のコメント「DirectAnthropic 以外の上流には意味がない」も、DirectAnthropic と api.anthropic.com を同一視した書き方になっている。

f944c64 時点ではクライアントのヘッダは一切上流へ渡らなかったため、これは minimax / vertex-ai の利用者にとって挙動変更にあたる。未知の beta フラグを無視する上流なら無害だが、それは検証されていないし、目標2は minimax にも Vertex にも用が無い。

**推奨**: ガードを上流ホストで絞る。`profile.base_url` をパースして host が `api.anthropic.com` のときだけ `beta_override` を組む。これなら api-key の `anthropic` プロファイルも救われる（`add_oauth` はトークン種別で従来どおり分岐）。ホスト判定を純粋関数に切り出せば suggestion-1 のテストと同時に潰せる。

絞らずに広げる判断を採るなら、その根拠（minimax / Vertex での実測、あるいは「未知 beta は無視される」ことの確認）を Deviation Log に足すこと。
</details>

<details>
<summary><b>[required-2]</b> <code>vertex-ai</code> の <code>default_model</code> が Vertex の ID 形式を満たさなくなった</summary>

`config.example.toml:263` が `claude-sonnet-4@20250514` から `claude-sonnet-5` に変わっている。

Vertex AI の publisher model は `<model>@<version>` 形式で、`base_url`（259行付近）も `.../publishers/anthropic/models` で終わり、モデル名がそのままパスに連結される。バージョン修飾子の無い ID はこの経路では解決しない。変更前の `claude-sonnet-4@20250514` が `@` を持っていたのはそのため。

376a7db のコミットメッセージは「第一者 Anthropic モデル ID を現行世代へ」であり、Vertex は第一者ではない。一括置換で巻き込まれたものと読める（同コミットは `anthropic`・`claude-max`・`model_aliases` も更新しており、そちらは妥当）。

さらにこのモデル ID 更新そのものが、plan のスコープ宣言（`config.example.toml` は目標2の雛形追加のため）にも Deviation Log にも無い。

**推奨**: `vertex-ai` の行は Vertex が提供する形式の ID に戻す（世代を上げるなら `@` 付きのバージョンを確認してから）。合わせて、モデル ID の一括更新を Deviation Log に軽微として1項足す。
</details>

<details>
<summary><b>[required-3]</b> D-6 が自ら「要再承認」と宣言しているが、承認の記録が無い</summary>

plan の Deviation ルールは「設計に影響＝plan 更新して再承認」と定めており、D-6 の見出しも `（設計に影響 / 要再承認）` で、本文も「ユーザーの再承認を得てから確定する」と書いている。

しかし plan の frontmatter は `approved: 2026-08-29` のままで、D-6 追加後に再承認された旨の記録がどこにも無い。D-6 のコミット（91d1ad8）は最初の承認より後にある。

実機検証結果の節が rc.4 の結果を含んでいるので、実質的にはユーザーが D-6 込みの成果物を動かしている。だが「動かした」と「設計変更を承認した」は別で、Deviation ルールを自分で立てた以上、記録が無いこと自体が逸脱になる。

**推奨**: D-6 に再承認の日付を1行足すか、plan の frontmatter に改訂を記す。承認が実際には得られていないなら、それを取るのが先。
</details>

## Suggestion

<details>
<summary><b>[suggestion-1]</b> beta の組み立て判断が <code>try_forward</code> に埋め込まれていてテストできない</summary>

`merge_anthropic_beta` 自体はテスト7件で十分に覆われている。覆われていないのは、それを**呼ぶかどうか**を決める `handler.rs:403-419` の判断のほうで、200行超の async fn の中にあるため単体テストが書けない。

plan の T002 Step 3 は「`custom_headers` に `anthropic-beta` を持つ profile では beta を付けないこと（`Anthropic-Beta` と大文字で書いた場合も含む）」をテスト項目として明示していた。D-6 でロジックが `direct.rs` から `handler.rs` へ移った際、テストが一緒に移らず落ちている。D-6 の本文にもその旨は無い。

**推奨**: `fn should_forward_beta(profile: &ProfileConfig) -> bool`（あるいは required-1 のホスト判定と統合した1関数）に切り出してテストする。切り出せば required-1 の修正もそこ1箇所で済む。
</details>

<details>
<summary><b>[suggestion-2]</b> <code>custom_headers</code> に beta を書くと OAuth フラグごと消える</summary>

`has_custom_beta` が真のとき `beta_override` は `None` になり、`apply_auth` からも beta 付与が外れた（`direct.rs` のテストのコメントが明記している）ため、`oauth-2025-04-20` を付ける経路が一つも残らない。

plan 段階の「custom_headers を尊重して二重付与しない」は、`apply_auth` が OAuth beta だけを付けていた前提の判断だった。D-6 でクライアント由来のフラグも同じヘッダに合流したので、いま同じ規則を適用すると、`custom_headers` に beta を1つ書いた目標2プロファイルは OAuth フラグとクライアントの機能フラグを両方失い、401 と 400 を同時に踏む。設定を足したのに壊れる方向なので、気づきにくい。

**推奨**: `custom_headers` の値を `merge_anthropic_beta` の入力に合流させ、`custom_headers` ループ側では beta をスキップする。「尊重」を上書きではなく和にする。
</details>

<details>
<summary><b>[suggestion-3]</b> <code>OAUTH_BETA</code> の doc コメントが実機確定後も「T001 未実施」のまま</summary>

`src/proxy/adapter/direct.rs:11-14`:

```
/// T001（実トークンでの受理条件確認）が未実施のため暫定値。定数をここに閉じ、
/// 確定次第この1箇所を差し替える。
```

plan の実機検証結果は AC-1 トークン軸を「確定」とし、`oauth-2025-04-20` で api.anthropic.com が受理することを A-5 で観測している。値は暫定ではなくなった。plan 側の D-1 末尾（「暫定値であり…確定値で差し替える」）も同じく古い。

コメントが「未検証」と言い続けると、次に触る人がこの定数を疑って余計な検証を挟む。確定した旨と根拠（A-5）に書き換えるのが安い。
</details>

<details>
<summary><b>[suggestion-4]</b> <code>api_key</code> が空でも <code>auth = "x-api-key"</code> と記録される</summary>

`handler.rs:388-397` の分岐はキーの中身だけを見るため、`api_key` が空（＝`apply_auth` が認証ヘッダを一切付けない）ときも `x-api-key` と出る。

AC-5 はこのログを「推論がどちらのアカウントへ向いたか」の証跡として使う設計なので、認証ヘッダが無い状態を「x-api-key で送った」と記録するのは D-3 が避けようとしたのと同じ種類の嘘になる。`api_key.is_empty()` のとき `"none"` を出す1分岐で済む。
</details>

## Nit

- **nit-1**: Deviation Log の並びが D-1〜D-4 → D-6 → D-5 になっている。追記順なのは読めば分かるが、番号で参照する文書なので昇順に並べ替えるか、D-6 が後から挿入された旨を添えると探しやすい。
- **nit-2**: `config.example.toml:176` のプレースホルダ `"<second account's setup-token>"` が、同ファイルの他の雛形（`YOUR_ANTHROPIC_KEY` 等）と書式が違う。そのまま貼って動かす人はいないので実害は無いが、揃えたほうが grep しやすい。
- **nit-3**: `merge_anthropic_beta` の doc コメントが「認証以外のヘッダは転送しない」という**全体方針**を述べているが、この関数自体はヘッダ転送の可否を決めていない（決めているのは呼び出し側）。方針は呼び出し側の `beta_override` に置き、関数側は入出力の説明に絞るほうが、後から「ここを直せば全ヘッダ転送できる」と誤読されにくい。
- **nit-4**: `Cargo.toml` / `Cargo.lock` の版上げ（0.2.8-rc.3 → rc.4）が Deviation Log に無い。実機検証のためのタグ付けに要るのは D-5 の記述から読み取れるが、スコープ宣言外のファイルである点は D-2 と同じ扱いにしておくと一貫する。

## 人間が最終判断すべき箇所

1. **required-1 の絞り方**。ホストで絞る（推奨）か、DirectAnthropic 全体に広げたまま minimax / Vertex での無害性を確認して記録するか。前者はコード数行、後者は実機かドキュメント調査が要る。目標2の受入には前者で足りる。
2. **required-3 の再承認**。D-6 の設計変更を承認済みとして記録するのか、いまから承認を取るのか。実機で rc.4 を回した事実をもって承認とみなす運用も選べるが、その場合は Deviation ルールの「再承認」の定義を plan 側に書いておかないと次回また同じ穴が開く。
3. **required-2 の Vertex モデル ID**。`claude-sonnet-4@20250514` に戻すだけでよいか、Vertex 側の現行世代 ID を調べて上げるか。設定例なので前者で止めても実害は無い。
</content>
</invoke>
