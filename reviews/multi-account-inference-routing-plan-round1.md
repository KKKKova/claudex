---
type: plan-review
slug: multi-account-inference-routing
round: 1
commit: working-tree（docs/ が gitignore のため plan は非追跡。作業ブランチ feature/multi-account-inference-routing、HEAD は本依頼コミット）
targets:
  - docs/specs/multi-account-inference-routing/plan.md
acceptance: plan.md の「差分spec」節（中量経路。受入基準 AC-1〜AC-6）
---

参照資料:

- 調査の引き継ぎ: `docs/handoff/2026-08-29-multi-account-inference-routing.md`（非追跡・working-tree）、fork issue #7 全文
- 土台の前提: `docs/handoff/2026-08-29-remote-control-af-unix.md`（非追跡）、fork issue #6
- 変更対象の現行コード: `src/proxy/adapter/direct.rs` / `src/proxy/handler.rs` / `src/process/launch.rs` / `config.example.toml`（いずれも main = 本ブランチ分岐点のまま）
- 引き継ぎ資料が挙げた実装対象2点のうち launch.rs の分岐変更を本 plan は縮小している（plan の「差分spec」節に理由を記載）。この判断が審査対象
