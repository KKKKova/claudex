---
type: plan-review-result
slug: remote-control-mitm-proxy
round: 1
verdict: REVISE
blocking: 3
status: in-review
reviewed_at: 2026-09-17
reviewed_commit: d784e26
reviewed_file: docs/specs/remote-control-mitm-proxy/plan.md
reviewed_sha256: 3742169373f979e9e4c041c0c9f1348d0f50906a05ea205591f5a3c6dc018a5d
---

# レビュー結果: plan remote-control-mitm-proxy round1

**REVISE** — blocking 3件（ほかに suggestion 2件、nit 3件）。
次のアクション: FR-013 を満たす経路の追加、`count_tokens` の認証ヘッダの訂正、T008 の完了条件と T007 の矛盾の解消。
最重要の根拠: 推論の上流接続は `ProxyState::http_client`（`src/proxy/mod.rs:66`）を使うが、これは `.no_proxy()` 無しで構築されている。FR-013 に対応するのは T003 の `ForwardState::client` だけであり、plan 自身の FR-013 検証手順が通らない。

FR-001〜FR-019 の 19 件はすべて検証計画に載っている（grep で照合済み）。タスク10個・約1600行という規模は変更範囲に見合っており、過剰な抽象や投機的機能は見当たらない。指摘は3件の具体的な齟齬に集中する。

観点別の所在: 完全性=B1・S1・S2、一貫性=B3、実現可能性=B2、過剰設計=指摘なし。

## blocking（3件）

### `B1 [重大] 推論経路の上流クライアントに no_proxy が無く、FR-013 を満たす経路が計画に無い — T003 / 検証計画 FR-013`

<details>
<summary>詳細</summary>

T003 は `ForwardState::client` を `.no_proxy()` で作り、トレーサビリティ上はこれで FR-013 を満たすことになっている。しかし推論は `handler::handle_messages` へ渡り、その上流接続は `ProxyState::http_client`（`src/proxy/mod.rs:66-68`、`src/proxy/handler.rs:456`）である。このクライアントは `reqwest::Client::builder().timeout(..).build()` のみで構築されており、reqwest の既定どおり環境変数のプロキシ設定を読む。

FR-013 の受入基準は「Given 親シェルに `HTTPS_PROXY` が設定されている / When proxy が上流へ接続する / Then 自分自身へ接続せず、直接出る」である。plan の検証計画も「`$env:HTTPS_PROXY = "http://127.0.0.1:9"` を設定したシェルから `claudex proxy start` → 推論が通る」と書くが、現状の `http_client` では到達不能なポート 9 へ繋ぎに行って失敗する。親シェルの `HTTPS_PROXY` が forward proxy 自身を指していた場合は自己ループになる。

`src/proxy/mod.rs` は plan の変更対象ファイルに入っているため、対応は範囲内である。ただし企業プロキシ配下の利用者への影響があるので、後段「人間が最終判断すべき箇所」にも挙げた。

対応: T003 か T006 のステップに `ProxyState::http_client` を `.no_proxy()` で構築する変更を足し、トレーサビリティの FR-013 行を両クライアントを指す形にする。
</details>

### `B2 [重大] count_tokens と /v1/complete の転送で認証ヘッダの作り方が誤っている — T005 Steps 4`

<details>
<summary>詳細</summary>

T005 は `DirectAnthropic` プロファイルへの転送について「プロファイル自身の鍵を `x-api-key` に載せる」と指示する。claudex の既存実装はここを鍵の形で分けている。`src/proxy/adapter/direct.rs:39` の `apply_auth` は、`is_anthropic_oauth_token` が真なら `authorization: Bearer <token>` を付け、`x-api-key` は付けない。API キー（`sk-ant-api…`）のときだけ `x-api-key` を使う。`anthropic-version` ヘッダもここで付く。

claudex が README と `src/process/launch.rs:181` の警告文で案内している多アカウント構成は、`provider_type = "DirectAnthropic"` に第二アカウントの**トークン**を `api_key` として置く形である。この構成でトークンを `x-api-key` に載せると上流が 401 を返し、Claude Code のトークン数取得が壊れる。`/v1/complete` も同じである。

同じステップに「`try_forward` が使っているのと同じ既存関数を使う」とも書いてあり、指示が二重になっている。後者が正しい。

対応: ステップ本文を「`adapter::direct::DirectAdapter::apply_auth` を通して認証ヘッダを付ける」に直し、`x-api-key` 固定の記述を削る。クライアントの `Authorization` と `x-api-key` を落とす指示はそのままでよい。
</details>

### `B3 [重大] T008 の完了条件が T007 の実装と矛盾し、満たすと FR-008 が壊れる — T008 完了条件 / T007 Steps 4-8`

<details>
<summary>詳細</summary>

T008 の完了条件は `grep -rn "ANTHROPIC_UNIX_SOCKET\|uds_windows\|socket_path" src/ Cargo.toml` が0件であることを求める。一方 T007 は `apply_forward_proxy_env` の中で `cmd.env_remove("ANTHROPIC_UNIX_SOCKET")` を残す（FR-008 が「渡さない」と定める4変数の1つであり、親シェルに残っている場合に落とす必要がある）。この文字列は `src/process/launch.rs` に必ず残るため、完了条件は達成できない。

実装者が完了条件に合わせて `env_remove` を削ると、FR-008 の第1受入基準（子プロセスの環境に `ANTHROPIC_UNIX_SOCKET` が存在しない）が親シェル由来の値で破れる。

対応: grep の対象から `src/process/launch.rs` の `env_remove` 行を除く形にする（例: `grep -rn ... src/ Cargo.toml | grep -v env_remove` とするか、検査対象を `uds_windows` と `socket_path` に限り、`ANTHROPIC_UNIX_SOCKET` は「`env_remove` 以外の出現が0件」と書く）。
</details>

## suggestion（2件、verdict に影響しない）

### `S1 [中] 統合テストに FR-015 第1受入基準（2プロファイルの誤配なし）に相当するものが無い — T009`

<details>
<summary>詳細</summary>

T009 のヘルパは `alpha` と `beta` の2プロファイルを持つ config を組むが、テスト9本のうち2つの利用者名を同時に使うものは無く、誤配の検証は実機手順（検証計画の FR-015 第1・第2）だけに委ねられている。

利用者名による振り分けはこの設計の最大の判断であり、その中核が自動テストで守られない。別々の userinfo で2本の TLS 接続を張り、それぞれの `base_url` に向けた `MockServer` が自分の分だけを受けることを見るテストを1本足せば、手順はすでにあるヘルパの組み合わせで済む。
</details>

### `S2 [中] design が定めた「proxy 停止時の告知」がどのタスクにも現れない — T008 / design§資格情報と証明書のライフサイクル`

<details>
<summary>詳細</summary>

design は「proxy 停止時と `claudex run` の開始時に」CA が入れ替わる旨を告知すると書く。plan では `claudex run` 側（T007 Steps 4-6）だけが実装され、停止側は T008 の `cleanup()` 追加のみで告知が無い。

FR-018 は起動時の告知しか要求していないため blocking にはしない。ただし停止側の告知は「走っているセッションを道連れにした」と利用者が気づく唯一の手がかりであり、T008 に1行足す価値がある。
</details>

## nit（3件）

<details>
<summary>nit 3件</summary>

- `N1 [軽] T006 Steps 5 の state.forward.as_ref().unwrap() は CLAUDE.md の「生産コードで unwrap しない」に反する — T006。直前に構築した Arc を変数に控えて clone すれば unwrap は要らない`
- `N2 [軽] T009 のヘルパが組む ProxyState の列挙に http_client と token_manager が抜けている — T009 Steps 1。現行の ProxyState（src/proxy/mod.rs:41）はこの2つも必須フィールドである`
- `N3 [軽] 検証計画 FR-002 の「実行時ディレクトリに forward-ca.pem と forward.json のみ」は PID ファイルを勘定していない — 検証計画。「秘密鍵ファイルが無いこと」を見る基準に直すほうが実態に合う`
</details>

## 人間が最終判断すべき箇所

1. **`ProxyState::http_client` を `.no_proxy()` にするかどうか**（B1 の直し方）。FR-013 は上流接続で `HTTPS_PROXY` を参照しないと一律に定めるが、企業プロキシの内側から claudex を使う利用者は、プロバイダへ出るために親の `HTTPS_PROXY` を必要とする。両立しないため、自己ループ防止を優先して一律に無効化するか、forward proxy 自身を指す場合だけ無効化するかを決めてほしい。
2. **`count_tokens` の正しさをどこまで自動テストで守るか**（B2 に関連）。`DirectAnthropic` かつ OAuth トークンという構成の検証には実鍵が要るため、統合テストでは `apply_auth` の単体テスト（既存の `oauth_token_uses_bearer_and_no_beta_header`）に相乗りする形が現実的である。実機確認の項目に足すかどうかは運用判断である。
3. **実機検証を Windows のみで出す判断**。要件の Non-Goals どおりだが、macOS / Linux 利用者には「実装は共通だが未検証」という状態で届く。README のどこにその但し書きを置くかは人間が決めてほしい。
