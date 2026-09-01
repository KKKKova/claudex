---
type: plan-review-result
slug: multi-account-inference-routing
round: 1
verdict: REQUEST_CHANGES
blocking: 3
reviewed: docs/specs/multi-account-inference-routing/plan.md @ working-tree (3f604e0)
date: 2026-08-29
---

# レビュー結果: plan multi-account-inference-routing round1

> **REQUEST_CHANGES** — blocking 3件 / suggestion 4件 / nit 2件。
> 判断: 中心の設計判断（launch.rs の `else if` 排他を解かず、目標2プロファイルを「DirectAnthropic + `api_key` にBトークン + `remote_control = true`」の形に寄せる）は妥当であり、根拠も実コードと一致する。差し戻しは設計そのものではなく、plan が依拠している前提のうち2つが事実と食い違っている点と、ゲート T001 の出口分岐が1本足りない点による。
> 根拠: (1) plan がスコープ外の根拠に使っている `api_key_keyring` は、実行時に読み出す経路が存在しない。(2) AC-4 のガードが「黙って無効になる組み合わせ」を前提にしているが、その組み合わせは現在正常に動く。(3) AC-1 が記録対象にしている system 先頭制約が「有り」だった場合の分岐が T001 に無く、T002 の実装量が変わる。

## 中心の設計判断の検証

審査対象として名指しされた「launch.rs の分岐変更を縮小する」判断は妥当である。実コードで裏を取った。

- `src/proxy/handler.rs:115` の遅延リフレッシュは `profile.auth_type == AuthType::OAuth` でのみ発火する。目標2プロファイルは `auth_type` 既定（`ApiKey`）なので、この分岐に入らない。`oauth_provider = "claude"` を名乗らせるとAの資格情報が載る、という plan の危惧は経路として実在する。
- `try_forward`（`src/proxy/handler.rs:349`）は受け取った `_headers` を捨てて `state.http_client.post(&url)` から組み直す（同 411-425行）。Claude Code が `CLAUDE_CODE_OAUTH_TOKEN`（Aのトークン）で付ける `Authorization` は上流に漏れない。plan の前提3は正しい。
- `apply_remote_control_env`（`src/process/launch.rs:170`）は `~/.claude` のセッションを `CLAUDE_CODE_OAUTH_TOKEN` に渡し、`ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` を明示的に落とす（同 212-214行）。前提2も正しい。
- 参照行番号（`direct.rs:29`、`launch.rs:40/43/170`、`handler.rs:118/349/385`）はすべて実物と一致した。

不足がコード1点（`apply_auth` の `x-api-key` 固定）に絞られる、という診断にも同意する。

## blocking

### [required-1] `api_key_keyring` は実行時に読まれない — スコープ外節の根拠と T003 Step 2 が成立しない

<details>
<summary>詳細</summary>

plan の「スコープ外」節は *「Bトークンの keyring 保存への移行（平文トークンの扱いは既存の `api_key_keyring` で足り、新設しない）」* と書き、T003 Step 2 は設定例に `api_key_keyring` を併記させる。しかし `api_key_keyring` を読んで `profile.api_key` に入れる処理はコードベースに存在しない。

参照箇所は4つだけで、いずれも読み出しではない。

- `src/config/mod.rs:100` — フィールド定義、`src/config/mod.rs:239` — 既定値 `None`
- `src/config/profile.rs:211-264` — 対話 `profile add` での **書き込み**。keyring に入れた場合は `api_key` を空文字にして保存する
- `src/config/cmd.rs:328` — `validate` の警告条件（どちらも空なら warn）
- `src/tui/mod.rs:135` — 表示用の bool

keyring からの読み出しは `src/oauth/source.rs:87 load_keyring` だけで、これは OAuth トークン用（entry 名も `keyring_entry_name(profile_name)` の別系統）であり `api_key_keyring` の entry 名は見ていない。

本 plan と組み合わせたときの症状が悪い。T002 の判定はトークン接頭辞なので、`api_key` 空 + `api_key_keyring` のみのプロファイルは `apply_auth` の空判定（`direct.rs:31`）で抜け、`Authorization` も `x-api-key` も付かないまま上流に出る。返るのは 401 で、これは AC-5 が「壊したBトークン」で期待している症状とまったく同じである。実機検証（T004 手順1）で両者を取り違える。

どちらかを選んで plan を直す必要がある。

- (a) T003 Step 2 から `api_key_keyring` の併記を落とし、スコープ外節の根拠を「平文 `api_key` で運用する（keyring 読み出しは未実装のため別件）」に書き換える
- (b) `api_key_keyring` の読み出しをスコープに入れる（設定ロード時に entry を引いて `api_key` を埋める、20行程度）。ただし本 plan の主題から外れるので (a) を推す

</details>

### [required-2] AC-4 のガードは、いま動いている構成を起動不能にする

<details>
<summary>詳細</summary>

T003 Step 1 は `is_claude_subscription && profile.remote_control` で `bail!` する。plan はこれを *「黙って無効になる組み合わせを明示エラーにする」* と説明しているが、この組み合わせは無効になっていない。

`is_claude_subscription` の枝（`src/process/launch.rs:43-48`）は `ANTHROPIC_BASE_URL` を設定せず、Claude Code は自身の OAuth で api.anthropic.com に直接出る。この状態で claude.ai の Remote Control は素で成立する（CLAUDE.md の「Claude subscription 特殊処理：跳过代理」がそのまま Remote Control の要件を満たしている）。つまり `remote_control = true` はこのプロファイルでは冗長なだけで、機能は失われていない。

したがって bail は「効いていないフラグを教える」ではなく「Remote Control が使えていた構成の `claudex run` を落とす」変更になる。`remote_control = true` を保険のつもりで書いていたユーザーは、アップデートで起動できなくなる。

推奨は `tracing::warn!` への格下げ（案内文はそのまま使える）で、AC-4 の Then も「案内付き警告を出して起動は続行する」に変える。bail を維持するなら、後方非互換であることを plan に明記して人間の承認対象に上げるべきである。どちらにせよ現状の「黙って無効になる」という記述は事実と違うので直す必要がある。

</details>

### [required-3] T001 の判定分岐に「system 先頭制約あり」が無い — ゲートの出口が実装量を変える

<details>
<summary>詳細</summary>

AC-1 の Then は受理条件として *「トークン種別・必要ヘッダ・system 先頭制約」* の3つを記録対象に挙げている。ところが T001 Step 3 の判定分岐はトークン種別だけで切られており、(a) 受理 / (a) 不可・(b) 可 / 両方不可 の3本しかない。

system 先頭に固定文字列（`You are Claude Code, Anthropic's official CLI for Claude.` 相当）が要求されると分かった場合、T002 の「ヘッダを足すだけ」では届かない。DirectAnthropic は透過（`direct.rs:41 passthrough() -> true`、`translate_request` は clone のみ）なので、Claude Code が送る body がそのまま上流に出る。メインループの system は条件を満たすとしても、補助リクエスト（タイトル生成の haiku 呼び、auto mode の classifier など）が別の system を積む経路があり、そこだけ 400/401 で落ちる。切り分けは難しく、症状は「ときどき失敗する」になる。

T001 Step 3 に4本目を足すこと。「system 先頭制約あり → `direct.rs` に system 先頭ブロックの注入（既に同じ文字列で始まる場合は二重に積まない）が必要。設計に影響するため Deviation ルールに従い plan を更新して再承認」。あわせて T001 Step 1 の試行マトリクスに、system 先頭が条件を満たさない補助リクエスト形（`system` 無し・`max_tokens` 最小）を1本明示的に含めると、制約の有無が確実に切り分かる。

</details>

## suggestion

### [suggestion-1] `anthropic-beta` が二重に付きうる

<details>
<summary>詳細</summary>

`try_forward` は `apply_auth` → `apply_extra_headers` の後、`profile.custom_headers` を無条件に append する（`src/proxy/handler.rs:421-425`）。`custom_headers` に `anthropic-beta` を持つ DirectAnthropic プロファイルでは、T002 が足す beta と合わせてヘッダが2行出る。reqwest は append なので上書きされない。

目標2プロファイルの雛形には `custom_headers` を書かない想定なので既定では踏まないが、既存プロファイルに OAuth トークンを置いた場合に当たる。T002 で方針を1行決めておくと安い（`custom_headers` に `anthropic-beta` があれば `apply_auth` 側は付けない、あるいはカンマ結合でマージする）。

</details>

### [suggestion-2] 接頭辞判定の取りこぼしがサイレントに 401 になる

<details>
<summary>詳細</summary>

`is_anthropic_oauth_token` は `starts_with("sk-ant-oat")` の一致だけを見る。一致しない値は黙って `x-api-key` 経路に落ちる。接頭辞が将来変わった場合や、リフレッシュトークン（`sk-ant-ort` 系）を取り違えて貼った場合の症状は上流 401 で、required-1 で書いた「壊れたトークン」「keyring のみ」とも区別がつかない。

T002 のログに `auth = oauth-bearer | x-api-key` が入るので追跡自体はできる。そこに一段足して、`sk-ant-` で始まるが `api` でも `oat` でもない値のときに warn を1回出すようにしておくと、実機検証での切り分けが速い。

</details>

### [suggestion-3] T002 Step 2 のログのコード片はそのままでは通らない見込み

<details>
<summary>詳細</summary>

plan は `auth = %if is_anthropic_oauth_token(&profile.api_key) { "oauth-bearer" } else { "x-api-key" }` と書いている。`tracing` のフィールド値に `if` 式を直接置く形は素直に parse されない。直前で `let auth = if ... { "oauth-bearer" } else { "x-api-key" };` に束縛して `auth = %auth` とするのが確実である。

実装時に即座に露見する範囲だが、plan にコード片として載っている以上、委譲プロンプトにそのまま入る。1行の修正で済む。

</details>

### [suggestion-4] T001 の結果が非追跡ファイルにしか残らない

<details>
<summary>詳細</summary>

T004 は結果を fork issue に残す設計なのに、T001 は `docs/specs/multi-account-inference-routing/spike-oauth-acceptance.md`（`docs/` は gitignore、plan 自身が「新規、非追跡」と明記）だけを記録先にしている。T002 が使う beta ヘッダ値も、後続の設計分岐も、すべてここから出る。ブランチを捨てるか作業ツリーを掃除すると確定値が消える。

T001 の完了条件に「判定結果（採用トークン種別・確定ヘッダ構成・system 制約の有無）を fork issue #7 にコメントとして転記する」を1行足すことを勧める。issue #6/#7 の運用と揃う。

</details>

## nit

1. **spike ファイルへの記録範囲を限定する。** T001 Step 2 は *「HTTP ステータスとエラー本文」* を記録させる。エラー本文にリクエストの一部が echo される可能性があるので、「トークン本体は記録しない（先頭4文字と末尾4文字まで）」を1行明記しておくとよい。記録先は非追跡とはいえローカルの平文ファイルである。
2. **設定例の隣接に注意する。** `config.example.toml` の Claude Max 雛形（157-165行）は `base_url = "https://api.claude.ai"` で、目標2の雛形は `https://api.anthropic.com` である。すぐ下に並べると取り違えやすい。雛形のコメントで「claude-max とは base_url も auth_type も別物」と1行示すと事故が減る。

## 人間が最終判断すべき箇所

1. **required-2 を warn に倒すか bail のままにするか。** 「効かないフラグを黙って許すべきでない」という立場を取るなら bail のままでも筋は通るが、その場合は後方非互換の変更である。既存ユーザーの規模を知っているのは人間だけなので、ここは判断を預ける。
2. **required-1 の (a)/(b) の選択。** (b)（`api_key_keyring` の読み出し実装）は本 plan の主題外だが、平文トークンを config に置く運用を避けたいなら今が入れどきでもある。plan の規模（約120行）が1.2倍程度になる。
3. **T001 が両方不可（ESCALATE）だった場合の落とし先。** plan は「目標2は不成立」で終わるとしているが、その時点で目標1（`extra_env` に setup-token、Remote Control を諦める）へ戻す運用判断が要る。ゲートを引く前に、不成立時に何を選ぶかを決めておくと、実機検証をユーザーに依頼する回数が減る。
