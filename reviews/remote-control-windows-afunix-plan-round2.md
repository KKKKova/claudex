---
type: plan-review
slug: remote-control-windows-afunix
round: 2
commit: working-tree（docs/ が gitignore のため plan は非追跡。作業ブランチ feature/remote-control-windows、HEAD は本依頼コミット）
targets:
  - docs/specs/remote-control-windows-afunix/plan.md
acceptance: plan.md の「差分spec」節（中量経路。受入基準 AC-1〜AC-6）
---

前回指摘への対応一覧（round1 結果: reviews/remote-control-windows-afunix-plan-round1-result.md）:

- required-1 → 対応。実在判定を `symlink_metadata().is_ok()` に統一（T003 Step 1(b) と「セキュリティ判断」節に根拠を記載）。stale 削除は存在判定に依存せず常に `remove_file` を呼び `ErrorKind::NotFound` のみ許容（T002 Step 4）。plan で閉じる側を採用した。
- required-2 → 対応。AF_UNIX 側の起動ログを `unix socket relay ready`（`proxy listening on` を含まない文言）に変更（T002 Step 4）。AC-5(2) を「`proxy listening on` の行が1回だけ出る」に確定。
- suggestion-1 → 対応。中継本体をプラットフォーム共通の `relay_pump`（Read/Write ジェネリック + shutdown クロージャ）に分離（T002 Step 3）。mac で走るテスト2件（透過性・終端伝播）を T003 Step 4 に追加。
- suggestion-2 → 対応。中継先を 127.0.0.1 固定にし、非ループバック `proxy_host` では中継を立てず warn（launch 側がソケット不在で明示エラーになる fail-closed）。「中心の設計選択」末尾と T002 Step 5 を更新。
- suggestion-3 → 対応。windows-check の check / clippy 両方に `--all-targets` を付与（T001 Step 5）。
- nit（grep パターン）→ 対応。T001 完了条件に `PipeConnection` を追加。
- nit（oauth USER フォールバック）→ スコープ外として plan の「スコープ外」節に別件扱いを明記。
