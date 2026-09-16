---
type: design-review-result
slug: remote-control-mitm-proxy
round: 1
verdict: REVISE
blocking: 2
status: in-review
reviewed_at: 2026-09-16
reviewed_commit: f9b6827
reviewed_file: docs/specs/remote-control-mitm-proxy/design.md
reviewed_sha256: 3a304a9109c27b0121151bb5eb7de4763517c426ad583ef58b4a912b32d97ef8
---

# レビュー結果: design remote-control-mitm-proxy round1

**REVISE** — blocking 2件（ほかに suggestion 3件、nit 2件）。
次のアクション: 利用者名を終端後の要求へどう引き継ぐかを明記し、`remote_control_mode` の型を「未設定」と明示 `off` が区別できる形に直す。
最重要の根拠: `Proxy-Authorization` は CONNECT に付くヘッダであり、TLS 終端後の内側の要求には現れない。引き継ぎ規則が無いまま実装すると、推論が全て本物の `api.anthropic.com` へ流れる。

FR-001〜FR-019 の 19 件はすべてトレーサビリティ表に載っており、design 側に要件外の ID も無い（grep で照合済み）。Error Handling と Testing Strategy の節も揃っている。指摘は設計の中身2点に集中する。

観点別の所在: 完全性=B1、一貫性=B2・S1、実現可能性=S2・S3、過剰設計=指摘なし。

## blocking（2件）

### `B1 [重大] CONNECT で受けた利用者名を終端後の要求へ引き継ぐ規則が無い — 振り分け規則 / 構成要素（identity.rs）`

<details>
<summary>詳細</summary>

`Proxy-Authorization` はプロキシとの1ホップに対して付くヘッダである。Claude Code は CONNECT 要求にこれを載せるが、その後に張られる TLS の内側を流れる `POST /v1/messages` には載らない。したがって「利用者名の解決」は、CONNECT 時に1回行い、その接続で終端した以降の全要求へ引き継ぐ操作になる。

設計は `identity.rs` の責務を「`Proxy-Authorization` の解析、利用者名からプロファイルへの解決、合言葉の照合」とだけ書き、引き継ぎには触れていない。振り分け規則は「利用者名が解決できないリクエストは、パスによらず本物へ中継する」と定めるため、実装が内側の要求にヘッダを探す形になると、推論が例外なくクライアントの claude.ai 認証ヘッダ付きで本物の `api.anthropic.com` へ流れる。Success Criteria の「FR-006 に列挙したパスのリクエストが 100% プロファイルのプロバイダへ到達する」が満たされず、しかも応答は正常に返るため気づきにくい。

絶対 URI 形式の要求（ブリッジが使う）では要求ごとにヘッダが付くため、解決の入口が CONNECT 終端側と別になる。両者の違いも合わせて書きたい。

対応: 「CONNECT で解決した利用者名を接続の属性として保持し、その接続で終端した全要求に適用する。絶対 URI 形式では要求ごとのヘッダから解決する」旨を振り分け規則か構成要素に1文足す。
</details>

### `B2 [重大] RemoteControlMode の型が未設定と明示 off を区別できず、データモデル節が自己矛盾する — データモデル / FR-009 / FR-017`

<details>
<summary>詳細</summary>

データモデル節は `RemoteControlMode` を `#[serde(default)]` 付きの enum `{ Off, Proxy }` と定める。この型では `remote_control_mode` を書かない config と `remote_control_mode = "off"` と書いた config がどちらも `Off` になり、両者を区別できない。

同じ節が続けて2つの規則を書く。「`remote_control_mode` が `Off` かつ `remote_control` が `true` のとき、警告の上で `Proxy` として扱う」と、「両方が設定されている場合は `remote_control_mode` を優先する」である。`remote_control_mode = "off"` と `remote_control = true` を併記した config に対し、前者は proxy 方式、後者は Gateway モードという逆の結果を指す。

要件側は FR-009 が「旧キーだけが設定されているとき proxy 方式」、FR-017 の受入基準が「`remote_control_mode` も旧キーも書いていない config は Gateway モード」と定めており、両立には「未設定」の表現が要る。

対応: `Option<RemoteControlMode>` にして未設定を `None` で表し、`Some(Off)` を明示的な無効として扱う。その上でデータモデル節の2文を1つの優先規則に書き直す。
</details>

## suggestion（3件、verdict に影響しない）

### `S1 [中] count_tokens の DirectAnthropic 分岐が「本物へ送らない」という自己記述と噛み合わない — 振り分け規則 / Alternatives Considered`

<details>
<summary>詳細</summary>

振り分け規則は `count_tokens` を「プロファイルが `DirectAnthropic` ならそのプロファイルの `base_url` へ」と定める。claudex が README と `src/process/launch.rs:181` の警告文で案内している構成は、まさに `provider_type = "DirectAnthropic"` かつ `base_url = "https://api.anthropic.com"` である。この構成では宛先が本物の `api.anthropic.com` になる。

Alternatives Considered は「いずれの場合も本物の `api.anthropic.com` へは送らない（FR-006）」と書くため、文面が実際の振る舞いと食い違う。意図は「クライアントの claude.ai 認証ヘッダを付けた中継経路へは送らない」であり、プロファイル自身の鍵で Anthropic へ出るのは要件の意図どおりである。誤読を避けるため、この一文を意図のとおりに書き直したい。
</details>

### `S2 [中] 設計の土台である「Claude Code が全 CONNECT に userinfo を付ける」の証跡が残っていない — 冒頭の設計判断`

<details>
<summary>詳細</summary>

利用者名による振り分けは、Claude Code が `HTTPS_PROXY` の userinfo を `Proxy-Authorization` として送ることに全面的に依存する。設計は実測で確認したと書くが、`examples/mitm_poc.rs` は userinfo 無しの PoC であり、確認の再現手順がリポジトリに残っていない。

この前提が崩れると設計全体が組み替えになる。plan の最初のタスクを「userinfo 付き `HTTPS_PROXY` で CONNECT のヘッダを観測する」に置き、PoC を1つ更新して証跡を残す形にしたい。
</details>

### `S3 [中] 利用者名の解決失敗を、セッション由来と外部プロセス由来で区別していない — 振り分け規則 / Error Handling`

<details>
<summary>詳細</summary>

解決に失敗した要求は、パスによらず警告ログを残して本物へ中継する。この既定は「claudex が起動していない Claude Code プロセス」（FR-015 の第3受入基準）に対しては正しい。

一方、claudex が起動したセッションでも、config からプロファイル名が消えた場合や合言葉が食い違う場合に同じ経路へ落ちる。このとき推論は利用者の claude.ai アカウントで実行され、応答も正常に返るため、利用者は誤配に気づかない。Success Criteria の「誤配 0 件」と衝突する経路である。

proxy 再起動の場合は CA の入れ替わりで TLS が先に失敗するため実害は出ない。残るのは稼働中の config 編集という狭い経路なので blocking にはしないが、「合言葉が付いていたが一致しない」場合だけは中継せず `502` で止める、といった切り分けを検討したい。
</details>

## nit（2件）

<details>
<summary>nit 2件</summary>

- `N1 [軽] API契約の絶対 URI 形式の例が成立しない — API契約。例に挙げた POST http://127.0.0.1:8776/mcp は、同じ設計が渡す NO_PROXY=localhost,127.0.0.1,::1 によって proxy を経由しない。ブリッジの POST http://api.anthropic.com/v1/environments/bridge を例にするほうが実態に合う`
- `N2 [軽] 異常終了で残った forward.json と forward-ca.pem の扱いが未記述である — 資格情報と証明書のライフサイクル。claudex run 側は PID の生存も見るため実害は無いが、次回起動で上書きする旨を1文添えると実装が迷わない`
</details>

## 人間が最終判断すべき箇所

1. **プロファイル名と合言葉を `HTTPS_PROXY` の環境変数に置くこと**。この2つはセッションの全子孫プロセスの環境変数に載り、同一利用者の他プロセスからも読める。読めた側は claudex の推論経路を使えるため、ローカル利用者を信頼境界の内側と見なす前提を確認してほしい。
2. **`count_tokens` をローカル概算（4 バイト = 1 トークン）で返す判断**。Claude Code が自動圧縮の発動判断にこの値を使っている場合、圧縮の起きる位置が前後する。精度を捨ててでも動かす方針でよいかは利用体験の判断である。
3. **Claude Code の内部実装への依存が1つ増えること**。既存の「claude.ai ログインの判定」に加えて「userinfo を CONNECT に転送する」が前提に加わる。どちらが変わっても Remote Control は止まる。リスク節の README 明記で足りるかを判断してほしい。
