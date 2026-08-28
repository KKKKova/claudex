---
type: implement-review
slug: windows-support
round: 2
commit: fd20d50
targets:
  - src/oauth/providers.rs
  - src/oauth/source.rs
acceptance: docs/specs/windows-support/plan.md の「差分spec」節（受入基準 AC-1〜AC-5）
---

前回指摘への対応一覧（round1: reviews/windows-support-implement-round1-result.md）:

- required-1（GitLab / GitHub device-code の keyring 失敗握り潰し）→ 修正: `login_gitlab` と `login_github` の device-code 経路を `store_keyring(...)?` に戻し失敗を伝播。ファイルが真の保存先である provider（ChatGPT / Claude / Google / Kimi）の best-effort は維持。`store_keyring_best_effort` の doc コメントに GitLab / GitHub device-code が対象外である旨を明記。
- required-2（per-profile `auth.json` が 0644 で新規作成される）→ 修正: `write_codex_credentials_atomic_at` で tmp 書き込み後・rename 前に `#[cfg(unix)]` で 0600 を設定。mode 検証テスト `test_write_codex_credentials_atomic_at_sets_mode_0600` を追加。
- suggestion 6件 / nit 3件 → 今回は未対応（round1 の段階提示に従い任意扱い）。

対応コミット: `fd20d50`（fix/windows-support、round1 対象 `df39c48` の直上）。
