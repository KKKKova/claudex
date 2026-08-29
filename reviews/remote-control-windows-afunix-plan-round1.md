---
type: plan-review
slug: remote-control-windows-afunix
round: 1
commit: working-tree（docs/ が gitignore のため plan は非追跡。作業ブランチ feature/remote-control-windows = PR #5 続行、HEAD は本依頼コミット）
targets:
  - docs/specs/remote-control-windows-afunix/plan.md
acceptance: plan.md の「差分spec」節（中量経路。受入基準 AC-1〜AC-6）
---

参照資料:

- 破棄の根拠と作り直しの前提: `docs/handoff/2026-08-29-remote-control-af-unix.md`（非追跡・working-tree）、fork issue #6 全コメント
- 旧実装のセキュリティ指摘: `reviews/remote-control-windows-implement-round1-result.md`（required-1。本 plan の「セキュリティ判断」節が置き換えを主張しており、審査対象）
- 削除対象の現行実装: 本ブランチ HEAD の `src/proxy/mod.rs` / `src/process/daemon.rs` / `src/process/launch.rs`
