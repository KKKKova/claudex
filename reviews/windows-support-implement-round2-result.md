---
type: implement-review-result
slug: windows-support
round: 2
commit: fd20d50
verdict: APPROVE
blocking: 0
status: approved
date: 2026-08-28
---

# レビュー結果: implement windows-support round2

**APPROVE — blocking 0件**。
round1 の required 2件はいずれも指摘の核心に直接対応しており、修正によって新たに入った欠陥はない。
検証: `cargo clippy -- -D warnings` 緑、`cargo test oauth::source` 16 passed（追加された 0600 検証テストの実行を `--list` で確認済み）。

round2 は修正確認のみのため、新規観点の指摘は出さない。round1 の suggestion 6件 / nit 3件は当初から任意扱いであり、未対応は verdict に影響しない。

## required-1: keyring 必須 provider の失敗伝播 — 対応済み

<details>
<summary>確認内容</summary>

`login_gitlab`（`src/oauth/providers.rs:470`）と `login_github` の device-code 経路（`:458`）が `store_keyring(profile_name, &token)?` に戻された。いずれも `println!` より前に `?` が置かれているため、keyring 保存が失敗すればエラーが伝播し、成功メッセージは出力されない。round1 で指摘した「保存されていないのに成功と表示する」経路は消えた。

`login_github` のうち `providers.rs:369`（Copilot config / `GITHUB_TOKEN` 経由の早期 return 分岐）が `store_keyring_best_effort` のまま残されている点は正しい判断である。この分岐は `load_credential_chain(&OAuthProvider::Github)` が成功した場合にのみ到達する。すなわち keyring 以外の情報源（環境変数または Copilot CLI の外部ファイル）が現に存在することが確認済みの経路であり、round1 の指摘対象ではない。

`store_keyring_best_effort` の doc コメントに、GitLab と GitHub device-code が対象外である理由が明記された。round1 で「doc コメントは ChatGPT / Claude / Github を挙げているが GitLab には言及がない」と指摘した点も解消している。best-effort の呼び出しは ChatGPT / Claude / Google / Kimi に残るが、いずれも外部 CLI ファイルまたは per-profile ファイルが源真相であり、round1 で確認したとおり実害はない。Qwen が `store_keyring(...)?` のままである点も変更されていない。
</details>

## required-2: per-profile `auth.json` の 0600 化 — 対応済み

<details>
<summary>確認内容</summary>

`write_codex_credentials_atomic_at`（`src/oauth/source.rs:695-704`）で、tmp ファイルへの書き込み後・`rename` 前に `#[cfg(unix)]` ガード付きで `set_permissions(0o600)` が挿入された。

順序が正しい。権限設定を rename より前に置いたことで、最終パスにファイルが現れた時点では既に 0600 である。他ユーザーが読める窓が生じない。rename は tmp の inode を移動するため、権限は最終ファイルに引き継がれる。

`test_write_codex_credentials_atomic_at_sets_mode_0600` が追加され、`tempfile` で実際に書き込んで `mode & 0o777 == 0o600` を検証している。テストが実在し実行されることを `cargo test oauth::source -- --list` で確認した。偽装可能な検証ではない。

Windows 側の ACL は未対応である。round1 で「Windows は `#[cfg(unix)]` で除外するか、当面 Unix のみ対応とする」と提示した範囲に収まるため blocking にはしない。ただし本 PR の主題が Windows 対応であることを踏まえ、下記の人間判断枠に上げる。
</details>

## 人間が最終判断すべき箇所

1. **Windows における平文 `auth.json` の保護が空白のまま残る**。0600 化は `#[cfg(unix)]` ガード内にあり、Windows では権限設定が一切行われない。Windows Credential Manager の文字数上限を回避するために keyring を冗長キャッシュへ降格させた結果、Windows こそが「平文ファイルが唯一の保管先」になるプラットフォームである。ACL 設定を入れるか、Windows のユーザープロファイル配下は既定で他ユーザーから保護されるという前提に依拠するかは、脅威モデルに関わる判断である。

2. **Windows で `cargo test` が一度も走らない構成が round1 から変わっていない**。`ci.yml` の `windows-check` は `cargo check` のみである。今回追加した 0600 検証テストも `#[cfg(unix)]` なので、Windows では存在しないのと同じである。AC-2 は満たしているため plan 違反ではないが、Windows 側の検証は AC-5 の実機引き継ぎに全面的に依存している状態が続く。

3. **round1 の suggestion 6件の扱い**。特に `codex_auth_path` の絶対パス検証欠落（相対パス指定で実行時 CWD にトークンが書かれ、`.gitignore` されずコミットされうる）は、今回 0600 化を入れた同じ関数群の周辺にあり、まとめて直せば安価である。次のリリースに含めるか、別 spec に切り出すかを決めておくとよい。
