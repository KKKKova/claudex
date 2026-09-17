---
type: plan-review
slug: remote-control-mitm-proxy
round: 1
commit: working-tree（docs/ が gitignore のため plan.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\plan.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md（status: approved、FR-001〜FR-019 と受入基準）
---

設計: `docs/specs/remote-control-mitm-proxy/design.md`（status: approved）

参考となる既存コード:

- src/proxy/mod.rs（既存 axum サーバ、T008 の削除対象）
- src/proxy/handler.rs（T005 が呼ぶ `handle_messages`）
- src/process/launch.rs（T007 の変更対象）
- src/process/daemon.rs（T003・T008 の変更対象）
- src/config/mod.rs（T002 の変更対象）
- examples/mitm_poc.rs（T001 の変更対象、T003・T005・T006 の土台）
- Cargo.toml（T002 で dev-dependencies から dependencies へ移す5クレート）

plan の「実行編成」節（担当プリセット割当と並列可否）も審査対象に含む。
