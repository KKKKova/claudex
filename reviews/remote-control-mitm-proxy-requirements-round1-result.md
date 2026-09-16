---
type: requirements-review-result
slug: remote-control-mitm-proxy
round: 1
verdict: REVISE
blocking: 4
status: in-review
reviewed_at: 2026-09-16
reviewed_commit: 1428d3d
reviewed_file: docs/specs/remote-control-mitm-proxy/requirements.md
reviewed_sha256: acf4f2e50d6efc26b72340d9a2a8dcc38a8a0b1c9b2ab207be1a41a523b08192
---

# レビュー結果: requirements remote-control-mitm-proxy round1

**REVISE** — blocking 4件（ほかに suggestion 5件、nit 2件）。
次のアクション: blocking 4件だけを直して round2 を出す。suggestion 以下は round2 で同時に扱ってよいが、verdict には影響しない。
最重要の根拠: 終端したリクエストを「どのプロファイルへ」「どのパスのとき」流すかが要件に無く、Success Criteria の「推論が 100% プロファイルのプロバイダへ到達する」を満たす設計が一意に定まらない。

指摘総数が 7 を超えるため重大度でグループ化した。**まず blocking のみ対応**すればよい。

観点別の所在: 完全性=B1・B2・B4・S4、一貫性=B4・S5、実現可能性=B3・S1・S2・S3、過剰設計=指摘なし。

## blocking（4件）

### `B1 [重大] 終端後にどのプロファイルで推論するかの決定規則が無い — FR-006 / FR-008`

<details>
<summary>詳細</summary>

FR-006 は「プロファイルのプロバイダへ翻訳して転送する」と書くが、forward proxy がプロファイルを特定する手段を要件のどこも定めていない。

現行はプロファイルを `ANTHROPIC_BASE_URL=http://api.anthropic.com/proxy/<profile>` のパスで運んでおり（`src/process/launch.rs:291`、ルートは `src/proxy/mod.rs:110` の `/proxy/{profile}/v1/messages`）、担体はこれ1つである。FR-008 はその `ANTHROPIC_BASE_URL` を「渡さない」と明示するため、終端後のリクエストは `POST /v1/messages` としてプロファイル情報を持たずに到着する。

FR-001 が開くのは単一の forward proxy ポートであり、claudex は複数プロファイルの同時起動を前提とする製品である。プロファイル A と B のセッションが同じポートを共有したとき、どちらのプロバイダへ流すのかが決まらない。

要件で決めるべきは「同時起動時にどう振り分けられることを期待するか」である。担体の実装手段（ヘッダ・ポート分離・セッション登録のいずれか）は設計の領分だが、期待挙動が無いまま設計に渡すと設計が発明することになる。

対応: FR-006 に「どのセッションのリクエストかを識別できる」旨の要件を追加し、複数プロファイル同時起動時の受入基準を1つ足す。
</details>

### `B2 [重大] 「推論エンドポイント」の定義が無く、FR-006 と FR-007 の分岐が決まらない — FR-006 / FR-007 / Success Criteria`

<details>
<summary>詳細</summary>

FR-006 と FR-007 は「推論エンドポイントか否か」で転送先を分けるが、その判定規則が要件に無い。`/v1/messages` 以外に `/v1/messages/count_tokens`、`/v1/complete`、beta 系のパスが存在し、どちら側へ倒すかで結果が変わる。

Success Criteria は「セッションの推論リクエストが 100% プロファイルのプロバイダへ到達する（`api.anthropic.com` へ漏れない）」と断定する。判定規則が未定義のままでは、この基準は検証できない。とくに `count_tokens` を FR-007 側（本物へ中継）に倒すと、プロンプト本文がクライアントの claude.ai トークン付きで Anthropic へ渡り、Success Criteria と矛盾する。

対応: 推論とみなすパスの集合を要件に列挙し、列挙外の未知パスを既定でどちらへ倒すかを決める（この既定はセキュリティ判断であり、後段「人間が最終判断すべき箇所」に再掲した）。
</details>

### `B3 [重大] forward proxy の listen アドレスが未定義。既存の proxy_host を継ぐと LAN に開いた MITM proxy になる — FR-001 / 非機能要件（セキュリティ）`

<details>
<summary>詳細</summary>

FR-001 は「別のポートを開く」とだけ書き、どのアドレスに bind するかを定めない。既存の proxy は `proxy_host` 設定に従い、`0.0.0.0` を正規の設定値として受理する（`src/config/mod.rs:990` に `proxy_host: "0.0.0.0"` のテストがある）。

forward proxy がこの値を継ぐと、同一 LAN の第三者が `api.anthropic.com` 宛てのリクエストを投げるだけでプロファイルのプロバイダ鍵で推論を実行できる。CA を信頼させられた端末があれば傍受の踏み台にもなる。非機能要件の「終端対象を `api.anthropic.com` に固定する」は終端先ホストの話であり、待ち受け範囲を縛らない。

対応: forward proxy の bind をループバック固定とする要件を足すか、`proxy_host` を継ぐなら非ループバック時の扱い（拒否か警告か）を決める。
</details>

### `B4 [中] FR-008 の環境変数集合から ANTHROPIC_MODEL が落ち、プロファイルのモデル指定が効かなくなる — FR-008`

<details>
<summary>詳細</summary>

FR-008 は渡す変数4つと渡さない変数4つを列挙するが、`ANTHROPIC_MODEL` がどちらにも無い。現行の Remote Control 経路は `ANTHROPIC_MODEL` を渡しており（`src/process/launch.rs:315`）、`claudex run <profile> -m <model>` と `default_model` はこの変数で効いている。

列挙から落ちたまま設計へ進むと、Claude Code が自前の既定モデル名（`claude-*`）を送り、プロバイダ側で解決できない。モデルスロット（`ANTHROPIC_DEFAULT_SONNET_MODEL` 等）は分岐の外で設定されるため部分的に埋まるが、スロット未設定のプロファイルと `-m` による明示指定は救われない。

対応: FR-008 の「渡す」側に `ANTHROPIC_MODEL` を加えるか、モデル名の解決を proxy 側で行う要件を別途立てる。
</details>

## suggestion（5件、verdict に影響しない）

### `S1 [中] 孫プロセスが api.anthropic.com へ出る場合が Edge Cases に無い — Edge Cases`

<details>
<summary>詳細</summary>

Edge Cases は「`HTTPS_PROXY` を継承した孫プロセス（MCP サーバ、フック）が `api.anthropic.com` **以外**へ出る場合」だけを扱う。危ないのは `api.anthropic.com` へ出る場合である。Node 以外の孫（curl、Python）は `NODE_EXTRA_CA_CERTS` を見ないため私設 CA を信頼せず TLS に失敗する。Node の孫は信頼するため、そのリクエストが推論経路へ吸われてプロファイルのプロバイダへ向かう。どちらを期待挙動とするかを1行決めておくと設計が迷わない。
</details>

### `S2 [中] proxy 再起動で CA が入れ替わり、実行中セッションが切れる — FR-002 / FR-012`

<details>
<summary>詳細</summary>

CA は proxy プロセスの寿命でのみ有効（非機能要件）であり、`NODE_EXTRA_CA_CERTS` は子プロセス起動時にしか読まれない。proxy を再起動すると、走っているセッションは旧 CA を信頼したまま新 CA のリーフ証明書を受け取り、TLS ハンドシェイクに失敗する。claude.ai トークンを起動時にしか読まない既存の制約と同種の制約であり、Edge Cases に明記して利用者への提示方法（再起動を促すか）を決めたい。
</details>

### `S3 [中] US3 の受入基準が「秘密鍵が残らない」を検証していない — US3 / FR-002 / FR-012`

<details>
<summary>詳細</summary>

US3 の理由は「CA 秘密鍵の漏洩は通信の傍受を許すから」だが、独立テスト方法は公開証明書ファイルの消滅を見ている。消えても消えなくても秘密鍵の所在は分からない。FR-002 の受入基準「CA 秘密鍵がどのファイルにも存在しない」も、全ファイルを走査する手段が現実には無く、そのままでは検証できない。「実行時ディレクトリに秘密鍵ファイルが無い」まで範囲を狭めれば実行可能になる。
</details>

### `S4 [中] remote_control_mode の値域と既定値、および TLS 終端の告知に対応する FR が無い — FR-008 / FR-009 / 非機能要件`

<details>
<summary>詳細</summary>

新しい設定キー `remote_control_mode` は FR-008 と FR-009 で `"proxy"` の場合だけが語られ、ほかにどの値を取るか、未設定時の既定が何かが定まらない。非機能要件の「Remote Control は既定で無効とし、有効時は TLS を終端する旨を起動時に告知する」も、対応する FR と受入基準を持たない。告知は US3 の「なりすまし鍵を残さない」と同じ関心に属するため、FR として立てたほうが検証できる。
</details>

### `S5 [軽] 互換性の「旧ソケット方式の設定値は受理しない」が指す設定値が存在しない — 非機能要件（互換性）`

<details>
<summary>詳細</summary>

現行 config にソケット方式固有のキーは無く、Remote Control 関連は `remote_control`（`src/config/mod.rs:141`）だけである。その `remote_control` は FR-009 で受理すると決めているため、この行は空振りするか、FR-009 と矛盾して読める。対象のキー名を書くか、行を削るのがよい。
</details>

## nit（2件）

<details>
<summary>nit 2件</summary>

- `N1 [軽] forward proxy のポート番号の決め方（固定値・設定キー・自動選択）が未定義である — FR-001。Edge Cases はポート専有時の失敗だけを扱う`
- `N2 [軽] FR-014 の「Claude subscription プロファイルを含む場合」の「含む」が曖昧である — FR-014。判定は起動するプロファイル単位のはずであり、現行実装（src/process/launch.rs:175）もそうなっている`
</details>

## 人間が最終判断すべき箇所

1. **未知のパスをどちらへ倒すか**（B2 に直結）。Claude Code が将来 `/v1/` 配下に新しいエンドポイントを足したとき、素通しの既定は「推論が Anthropic へ漏れる」方向へ、終端の既定は「非推論の通信がプロバイダへ誤送されて機能が壊れる」方向へ倒れる。どちらの失敗を受け入れるかはビジネス判断であり、機械的には決まらない。
2. **私設 CA で TLS を終端する回避策を製品機能として持つこと自体の是非**。利用者の端末で `api.anthropic.com` を騙る鍵を生成する設計であり、リスク文書にある「Claude Code 側の判定が変われば再び塞がれる」を README への明記だけで足りるとするかも併せて判断が要る。
3. **旧ソケット方式をコードごと削除する判断**（Non-Goals）。新方式が動かない環境（CA を信頼させられない、企業プロキシと併用する等）に当たった利用者へ残す退避先が無くなる。段階的廃止と即時削除のどちらを取るかは運用判断である。
