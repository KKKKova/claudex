---
type: implement-review-result
slug: windows-support
round: 1
commit: df39c48
verdict: REVISE
blocking: 2
status: in-review
date: 2026-08-28
---

# レビュー結果: implement windows-support round1

**REVISE — blocking 2件**（required 2 / suggestion 6 / nit 3）。
判定根拠: keyring を必須ストアから外す設計変更が、keyring 以外に保存先を持たない provider にも一律適用され、保存されていないのに成功と表示する経路を作った。加えて、本 diff が新規に生成するようになった per-profile の `auth.json` が他ユーザー読み取り可能な権限で作られる。

指摘は11件あるため段階提示とする。**まず required 2件のみ対応すればよい**。suggestion / nit は round2 の対象外であり、対応は任意である。

## required（blocking 2件）

- [required] keyring が唯一の保存先である GitLab / GitHub device-code でも書き込み失敗を握り潰し、保存されていないまま「stored」と表示する — `src/oauth/providers.rs:470`, `src/oauth/providers.rs:458`

<details>
<summary>詳細</summary>

`store_keyring(...)?` → `store_keyring_best_effort(...)` の一括置換（13箇所）は、大半の provider では妥当である。Claude は `~/.claude/.credentials.json`、ChatGPT は `auth.json`、Google は `~/.gemini/oauth_creds.json`、Kimi は `~/.kimi/auth.json` が真の保存先であり、実行時の取得経路（`load_credential_chain`）はいずれも keyring を経由しない。Qwen が `store_keyring(...)?` のまま残されている点も、Qwen が chain 非対応（`source.rs:587` で `bail!`）であることと整合する。

問題は GitLab と GitHub device-code である。

`load_credential_chain` の GitLab 分岐（`src/oauth/source.rs:570-586`）は `GITLAB_TOKEN` / `GL_TOKEN` の環境変数のみを見て、なければ `bail!` する。外部 CLI ファイルは存在しない。つまり `login_gitlab` にとって keyring は唯一の永続保存先である。それにもかかわらず `providers.rs:470` で best-effort 化され、直後の `providers.rs:471` が無条件に `println!("GitLab token stored for profile '{profile_name}'.")` を出して `Ok(())` を返す。

`login_github` の device-code 経路（`providers.rs:458`）も同様である。GitHub Copilot CLI が未導入で `GITHUB_TOKEN` も未設定の環境では、device-code で取得したトークンの保存先は keyring しかない。

**壊れる経路**: keyring backend が利用できない環境（Linux コンテナで D-Bus / secret-service なし、Windows で Credential Manager にアクセスできない等）で `claudex auth login gitlab` を実行する。トークンはどこにも永続化されないが、コマンドは終了コード 0 と成功メッセージを返す。失敗は `tracing::warn!` にしか出ず、`claudex run` 経由では stderr layer が無効化されるためログファイルにしか残らない。ユーザーは認証済みと信じ、環境変数が消えた次のセッションで理由の分からない失敗に遭遇する。

これは diff による回帰である。変更前は `?` でエラーが伝播し、ユーザーは保存できなかったことを知れた。

`store_keyring_best_effort` の doc コメントは正当化の根拠として ChatGPT / Claude / Github を挙げているが、GitLab には言及がない。GitHub についても「外部 CLI ファイルが源真相」という前提は Copilot CLI 導入済みの場合にしか成り立たない。

出典: silent-failure-hunter（GitLab の環境変数依存を指摘）、security-triage（同旨）。correctness-reviewer は「keyring が唯一の保存先となる provider はない」と結論したが、Judge が `source.rs:570-586` と `providers.rs:463-484` を再読して反例を確認した。correctness 側の結論を採らない。
</details>

- [required] per-profile の `auth.json` が 0644 で新規作成され、同一ホストの他ユーザーが平文の access_token / refresh_token を読める — `src/oauth/source.rs:686-693`

<details>
<summary>詳細</summary>

`write_codex_credentials_atomic_at` は `std::fs::create_dir_all`（0755）→ `std::fs::write`（umask 022 下で 0644）→ `rename` で書き込む。`PermissionsExt` による権限設定はリポジトリ全体に存在せず、Windows 側の ACL 設定も皆無である。

既定パス `~/.codex/auth.json` の 0644 化は diff 前から同一のコードであり（`git show 2de4638:src/oauth/source.rs` で確認済み）、本 diff が導入した欠陥ではない。required とする根拠は次の2点である。

第一に、`codex_auth_path` の導入により claudex 自身が `~/.codex/auth-work.json` 等を**新規作成**するようになった。これは diff で生まれた経路である。Codex CLI が 0600 で作ったファイルを更新するのではなく、claudex が最初から 0644 で作る。

第二に、required 1件目と同じ設計変更により、keyring が失敗する環境ではこの平文ファイルが唯一の保管先に昇格した。diff 前は keyring 保存が必須（失敗すればエラー）だったため、平文ファイルは冗長系だった。

**壊れる経路**: ホームディレクトリが 0755 のホスト（macOS 既定、Debian 系 Linux 既定）で `claudex auth login chatgpt --codex-auth-path ~/.codex/auth-work.json` を実行する。同一ホストの別 UID のユーザーまたはプロセスが `cat ~/.codex/auth-work.json` で access_token と refresh_token を取得できる。

修正は `write_codex_credentials_atomic_at` の1箇所で済む（tmp ファイル書き込み後、rename 前に `set_permissions(0o600)`。Windows は `#[cfg(unix)]` で除外するか、当面 Unix のみ対応とする）。

出典: security-triage（実測で final mode = 100644、dir mode = 40755 を確認）。
</details>

## suggestion（対応任意・6件）

<details>
<summary>詳細（6件）</summary>

- [suggestion] `codex_auth_path` に絶対パス検証がなく、相対パス指定で実行時 CWD にトークンが書き出される — `src/oauth/source.rs:246-258`

  `expand_user_path` は `~/` と `~\` の展開のみ行い、それ以外は `PathBuf::from(trimmed)` で無検証に通す。`codex_auth_path = "auth-work.json"` のような相対パス指定だと、リフレッシュ時に `claudex run` の CWD（利用者のプロジェクトディレクトリ）へトークン入り JSON が生成されうる。`.gitignore` は `config.toml` を除外するが `auth*.json` は除外しないため、`git add .` でリポジトリにコミットされうる。`expand_user_path` の後段で `is_absolute()` を検査して非絶対パスを早期エラーにするのが素直である。required 2件目と同じ関数群の修正なのでまとめて対処できる。

  併せて、tmp ファイル名 `auth.json` → `auth.tmp` が完全に予測可能で `O_EXCL` なしのため、`codex_auth_path` が他ユーザー書き込み可能なディレクトリを指す場合にシンボリックリンク攻撃が成立する。同じ入力検証で塞げる。

- [suggestion] `status()` の chain フォールバックで `chain_err` が debug ログに落ち、診断情報が失われる — `src/oauth/providers.rs:631-648`

  keyring と credential chain の双方が失敗した場合、呼び出し元は一律 `("no token", "-")` を表示する。新設の `or_else` は `chain_err`（ファイル欠如・JSON 不正・権限エラー等）を `tracing::debug!` に埋めて `keyring_err` だけを返すため、既定ログレベルでは何も出ない。ユーザーは `claudex auth status` を見ても、keyring 未対応なのか `auth.json` が壊れているのかを判別できない。コメントは「Windows で keyring 不可な場合の救済」しか説明しておらず、chain 側の失敗理由を捨てる判断は明記されていない。

- [suggestion] `login()` が実際のログイン成否より先に `codex_auth_path` を config へ永続化する — `src/oauth/providers.rs:166-176`

  `--codex-auth-path` 指定時、`login_chatgpt` が失敗しても config への書き込みは既に完了している。エラー自体は `?` で伝播するので silent ではないが、再実行時に意図しないパスが設定済みの状態から始まる。ログイン成功後に永続化する順序が素直である。

- [suggestion] `auth logout` が本 diff で claudex 所有となった per-profile `auth.json` を削除しない — `src/oauth/providers.rs:689-695`

  logout は keyring エントリのみを削除する。`~/.codex/auth.json` を消さないのは Codex CLI の所有物だからという説明が成り立つが、`~/.codex/auth-work.json` は claudex が自ら生成したファイルであり、削除責任が新たに発生している。さらに本 diff が追加した status の chain フォールバックにより、logout 後も `claudex auth status` が valid を返し続ける。

- [suggestion] テスト欠落: `write_codex_credentials_atomic_at`、`~\` 展開、`codex_path` の伝播 — `src/oauth/source.rs:664-697`, `src/oauth/source.rs:250`, `src/oauth/manager.rs:131-160`

  追加された5件のテストは `codex_auth_path` の解決と account_id 復元をカバーするが、書き込み側は未検証である。特に「既存ファイルの `auth_mode` / `id_token` を保持する」挙動は Codex CLI との共存の根幹であり、`tempfile` で素直にテストできる（read 側は既に同じ手法でテスト済み）。`expand_user_path` の `strip_prefix("~\\")` 分岐は Windows 対応の核だが、`cfg` ガードのない純粋な文字列処理なので mac 上でもテスト可能であるにもかかわらず未検証である。`codex_path` の伝播については Judge が全経路を手作業で追跡し、既定パスへ落ちる経路がないことを確認した（correctness-reviewer の検証と一致）。

- [suggestion] `expand_user_path` が既存実装と重複し、`codex_auth_path(profile...)` の解決手順が3箇所で反復する — `src/oauth/source.rs:246-258`, `src/oauth/manager.rs:134,155`, `src/oauth/providers.rs:726,811,831`

  tilde 展開は `src/sets/source.rs:36-40` に既に存在する（`~\` 対応の有無で仕様が微妙に異なる）。また「profile から `codex_auth_path` を取り出して `source::codex_auth_path()` で解決する」2ステップが3箇所に独立して書かれている。`profile_codex_auth_path(&ProfileConfig) -> Result<PathBuf>` に括れば伝播漏れの余地も減る。ただし `docs/constitution.md` が存在せず配置規約が未定義のため、規約違反ではなく任意提案として扱う。

  なお `_at` / `_with_codex` / `_to` サフィックスによる関数の二重化そのものは妥当である。いずれも既定値で本体に委譲する1〜3行の薄いラッパーであり、Rust にデフォルト引数がない制約下の標準的な書き方である。引数なし版には codex_path を必要としない実呼び出し元が残っている。
</details>

## nit（verdict に影響しない・3件）

<details>
<summary>詳細（3件）</summary>

- [nit] `src/oauth/token.rs` と `src/oauth/handler.rs` が未使用のまま旧関数（per-profile 非対応）を使い続けている — `src/oauth/token.rs:11-45`, `src/oauth/handler.rs:23-209`

  いずれも自ファイル内のテスト以外から呼ばれていない。両者は `read_codex_credentials()` / `load_credential_chain()` / `refresh_chatgpt_token()` の引数なし版を使っており、将来これらが使われると `codex_auth_path` による分離をこの経路だけ回避する。本 diff が作った重複ではないが、per-profile 分離の導入によって「旧経路が残っている」ことの意味が変わった。削除するか `_at` / `_with_codex` 系へ揃えるかを決めておくとよい。

- [nit] `test_custom_codex_path_differs_from_default` が偽装可能で、直前の tilde テストと重複している — `src/oauth/source.rs:888-894`

  `assert_ne!` で「ファイル名が違う」ことしか見ておらず、`~` 展開が壊れて生文字列がそのまま `PathBuf` になっても通る。同ファイルの `test_codex_auth_path_tilde_is_expanded` が期待値一致で正しく検証しているため、網羅の穴ではなく冗長である。コメントの「隔離多账号的核心保证」という主張に対しては証明力がない。

- [nit] CI ワークフローに `permissions:` がなく、サードパーティ action が SHA pin されていない — `.github/workflows/ci.yml:1-49`

  新規 `windows-check` を含め `permissions:` ブロックがない。`dtolnay/rust-toolchain@stable` はブランチ ref、`actions/checkout@v4` と `Swatinem/rust-cache@v2` は可変タグである。`pull_request_target` は使われておらず fork PR 経由の即時悪用経路はないため優先度は低い。本 diff で新規に生まれた問題ではなく、既存ジョブと同じ書き方に揃えた結果である。
</details>

## 観点別の実施記録

| 観点 | 結果 |
|---|---|
| correctness | 指摘なし（≥80 の候補ゼロ。ただし「keyring が唯一の保存先の provider はない」という結論は Judge が反証し不採用） |
| silent-failure | 4件検出 → required 1 / suggestion 2 / 既存挙動として不採用 1 |
| simplicity | nit 3件のみ（`_at` 系の二重化は妥当と判定） |
| test-coverage | 8件検出 → suggestion 1（統合）/ nit 1 / 残りは criticality < 8 で不採用 |
| reuse-abstraction | 2件検出 → suggestion 1（統合）。規約未定義のため任意提案扱い |
| security | 6件検出 → required 1 / suggestion 2 / nit 1 / 不採用 2 |
| design | UI 接触なし（変更ファイルに対象拡張子・`design/` 配下なし）。観点省略 |

反証により不採用とした指摘: `write_codex_credentials_atomic_at` の JSON パース失敗を無ログで握り潰す点（`source.rs:670`）は、silent-failure-hunter が HIGH で挙げたが、diff 前と完全に同一のコードである（引数が `&cred_path` から `cred_path` に変わっただけ）。本 diff が導入した欠陥ではないため round1 の blocking にはしない。ただし per-profile 対応で呼び出し面が広がった点は記録しておく。同様に、既定 `~/.codex/auth.json` の 0644 化そのものも diff 前からの挙動である（required 2件目は claudex が新規作成する per-profile ファイルに限って採る）。

## 人間が最終判断すべき箇所

1. **keyring を「冗長キャッシュ」に降格させる設計判断そのもの**（`src/oauth/source.rs:72-80` の8行）。この diff は OS キーチェーンを必須ストアから外し、平文ファイルを源真相に据えた。Windows Credential Manager の ~2560 文字上限という具体的な制約への対処としては合理的だが、結果として全 provider のトークンが平文ファイル依存になる。この降格を全 provider に一律適用するのか、ChatGPT に限定して他は `?` 伝播に戻すのかは、脅威モデルに関わる判断であり機械的に決められない。required 1件目の修正方針もここに従属する。

2. **Windows でテストが一度も実行されない構成のまま v0.2.6 を公開した点**。`ci.yml` の `windows-check` は `cargo check --target x86_64-pc-windows-msvc` のみで、`cargo test` を走らせない。plan の AC-2 はこれを満たしているため plan 違反ではない。ただし `test_codex_auth_path_custom_absolute_preserved` の `cfg!(windows)` 分岐は CI でも実機でも一度も通らず、「Windows のパス処理はテスト済み」という誤った安心感を生む。AC-5 の実機検証を別担当に委ねる現在の分担で足りるか、`windows-check` に `cargo test` を足すかは、リリース済みという事実を踏まえた運用判断である。

3. **`docs/specs/windows-support/plan.md` が git 管理外である点**（Deviation Log 1件目に記録あり）。受入基準がリポジトリに残らないため、後続のレビューや第三者による検証が working tree の存在に依存する。今回は working tree から読めたが、恒久的な扱いを決めておく必要がある。
