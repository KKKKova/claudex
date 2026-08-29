---
type: plan-review-result
slug: multi-account-inference-routing
round: 2
verdict: APPROVE
blocking: 0
reviewed: docs/specs/multi-account-inference-routing/plan.md @ working-tree (cca69f0)
date: 2026-08-29
---

# レビュー結果: plan multi-account-inference-routing round2

> **APPROVE** — blocking 0件 / suggestion 2件 / nit 4件。
> 判断: round1 の blocking 3件はいずれも解消している。`api_key_keyring` への依存は設定例と根拠の両方から消え、AC-4 は起動を止めない形に変わり、T001 のゲートには system 制約の分岐が入った。
> 根拠: 残る suggestion 2件は T001 手順書の書き方と `custom_headers` の大小文字揺れで、どちらも実装フェーズの Deviation Log で吸収できる。suggestion-1 だけは T001 の出口の読み違いを招きうるので、手順書を書く前に1段直しておくと安い。

## round1 指摘への対応確認

| round1 指摘 | 対応 | 判定 |
|---|---|---|
| required-1 `api_key_keyring` が実行時に読まれない | (a) 採用。T003 Step 2 の設定例から併記を削除し、スコープ外節を「書き込みだけ実装・読み出し無し。平文 `api_key` で運用、読み出しは別件」に書き換え | 解消 |
| required-2 AC-4 のガードが後方非互換 | `bail!` → `eprintln!("warning: …")` に格下げ。差分spec の「黙って無効になる」を「冗長なだけで機能は失われていない」に訂正。AC-4 の Then も「警告を出して起動続行」に変更 | 解消 |
| required-3 T001 に system 制約の分岐が無い | Step 3 に4本目を追加（system 注入が要る → plan 更新して再承認）。試行マトリクスの変則2を「system 無し（補助リクエスト形の代表）」に明示 | 解消。ただし分岐の排他性に読み違いの余地（suggestion-1） |
| suggestion-1 `anthropic-beta` の二重付与 | `apply_auth` 内で `custom_headers.contains_key("anthropic-beta")` を見て付けない。テスト項目にも追加 | 対応。大小文字の揺れが残る（nit-1） |
| suggestion-2 接頭辞不一致のサイレント 401 | `sk-ant-` 始まりで `api` でも `oat` でもない値に `tracing::warn!` | 対応。発火頻度が nit-2 |
| suggestion-3 ログ片が parse されない | `let auth = …;` の束縛形に修正 | 解消 |
| suggestion-4 T001 の記録先 | 完了条件に fork issue #7 への転記を追加 | 解消 |
| nit-1 spike のトークン記録範囲 | Step 2 に「先頭4文字と末尾4文字まで。エラー本文に echo されていたら伏せる」を追加 | 解消 |
| nit-2 claude-max 雛形との取り違え | T003 Step 2 の雛形コメントに1行追加 | 解消 |
| 人間判断3 T001 両方不可時 | 該当分岐に「目標1の運用を継続する」を明記 | 解消 |

## 修正の照合結果

実コードに当てて確認した点。

- **`eprintln!("warning: …")` の流儀は実在する。** `check_session_lifetime` の期限警告が `src/process/launch.rs:243` で同じ形（`eprintln!("warning: …")` して続行）を取っている。T003 が「同じ流儀」と書いているのは正しく、launch.rs に `tracing::warn!` の前例は無い。
- **`redundant_remote_control_warning` を純粋関数に切れる。** 判定材料は `auth_type` / `oauth_provider` / `remote_control` の3フィールドだけで、`ProfileConfig` から閉じる。テストがプロセス起動に触れないという T003 の前提は成立する。
- **`custom_headers` は `HashMap<String, String>`**（`src/config/mod.rs:105`）で、`try_forward` は `apply_auth` の後に無条件 append する（`src/proxy/handler.rs:421-425`）。`contains_key` で先回りする設計は経路として正しい。
- **スコープ外節の記述が事実と一致した。** `api_key_keyring` の参照は定義・対話 add の書き込み・`validate` の警告・TUI 表示だけで、読み出しは無い。

## suggestion

### [suggestion-1] T001 Step 3 の4分岐が排他でない — 「(a) 受理」を読んだ時点で止まりうる

<details>
<summary>詳細</summary>

Step 3 の判定は4つの箇条書きが並列に置かれているが、1本目（`(a)` が受理される → 本 plan のまま進む）と3本目（system 先頭制約あり → plan 更新して再承認）は同時に真になりうる。setup-token が Claude Code の system 文字列付きで受理され、かつ system 無しの変則2 が落ちる、というのが最もありそうな結果であり、そのとき plan の指示は「そのまま進む」と「再承認」で割れる。

round1 の required-3 が防ごうとしたのはまさにこの取りこぼしなので、書き方で戻らないようにしておきたい。判定を2段に分けるのが素直である。

1. **トークン種別の軸**（`(a)` 可 / `(a)` 不可・`(b)` 可 / 両方不可）で採用トークンを決める
2. **system 制約の軸**（変則2 が通る / 落ちる）を独立に判定し、落ちるなら1の結果によらず plan 更新して再承認

Step 3 の見出しを「判定は次の2軸を独立に行う」に変え、既存の4箇条をこの2群に振り分ける修正で足りる。

</details>

### [suggestion-2] `contains_key("anthropic-beta")` は大小文字を見ない

<details>
<summary>詳細</summary>

`custom_headers` は `HashMap<String, String>` で、キーは TOML に書かれた文字列がそのまま入る（正規化する処理は無い）。`Anthropic-Beta` や `ANTHROPIC-BETA` と書いた profile では `contains_key("anthropic-beta")` が偽になり、`apply_auth` が beta を足す。その後 `try_forward` が `custom_headers` を append するとき、reqwest は `HeaderName` に落とす際に小文字化するため、同じ名前のヘッダが2行出る。suggestion-1（round1）で塞ごうとした状況がそのまま再現する。

TOML のヘッダ名を小文字で書く慣習は強制されていないので、比較側を寛容にするのが確実である。

```rust
let has_beta = profile
    .custom_headers
    .keys()
    .any(|k| k.eq_ignore_ascii_case("anthropic-beta"));
```

T002 Step 1 のコード片1行と、Step 3 のテスト項目（`Anthropic-Beta` 表記のケース）に反映するだけで済む。

</details>

## nit

1. **接頭辞不一致の warn が毎リクエスト出る。** `apply_auth` は転送のたびに呼ばれる（`src/proxy/handler.rs:421`）ので、設定を直すまで警告がログを埋める。切り分け用途としては1回出れば足りるので、`std::sync::Once` で1度だけにするか、profile ロード時（`config` の `validate` 側）に寄せる手もある。意図的に毎回出す判断ならそのままでよい。
2. **`OAUTH_BETA` の宣言場所が決まっていない。** T002 の Interfaces には `is_anthropic_oauth_token` しか挙がっておらず、`OAUTH_BETA` はコード片の中にだけ出てくる。`direct.rs` のファイル先頭に `const OAUTH_BETA: &str = "…";` を置く、と1行足しておくと委譲プロンプトが自己完結する。
3. **スコープの記述が古い。** 53行目の `src/process/launch.rs`（ガード追加のみ）は required-2 の格下げ後の内容と食い違う。「案内警告の追加のみ」に直す。
4. **T001 の変則番号が2軸で衝突している。** ヘッダ行にも body 行にも「変則1 / 変則2」があり、手順書を書く時点で取り違えやすい。実行するのは著者ではなくユーザーなので、H1/H2・B1/B2 のように軸ごとに接頭辞を分けるか、Step 1 の成果物を最初から番号付きの試行表（12行）にしておくと、結果の記録と照合が楽になる。

## 人間が最終判断すべき箇所

1. **suggestion-1 を T001 の手順書起草前に取り込むか。** T001 はゲートであり、その出口の読み方が割れると後続の再承認が飛ぶ。取り込むなら Step 3 の見出し1行と箇条書きの並べ替えで済む。
2. **AC-4 を実機で確認する範囲。** 警告に格下げしたことで、AC-4 の失敗は「起動しない」ではなく「警告が出ない」になった。単体テストで `Some`/`None` は押さえられるが、`eprintln!` が実際にユーザーの目に入るか（PTY モード経由でも流れるか）は実機でしか分からない。T004 の手順に1行足すか、AC-4 は単体テストで足りると割り切るかの判断が要る。
3. **T001 の12試行がBのクォータに与える影響。** `max_tokens` を最小にする設計にはなっているが、リクエスト数そのものはユーザーのアカウントに乗る。全マトリクスを回すか、`(a)` + 変則2 の4試行から始めて必要なら広げるかは、Bのアカウントの余裕を知っている人間の判断である。
