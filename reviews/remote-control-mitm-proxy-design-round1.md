---
type: design-review
slug: remote-control-mitm-proxy
round: 1
commit: working-tree（docs/ が gitignore のため design.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\design.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md（status: approved、FR-001〜FR-019 と受入基準）
---

観点は上流4観点（完全性・一貫性・実現可能性・過剰設計）。

参考となる既存コード:

- src/proxy/mod.rs（既存 axum サーバ、削除対象の AF_UNIX 中継）
- src/proxy/handler.rs（設計が再利用する翻訳ハンドラ）
- src/process/launch.rs（削除対象の Remote Control 分岐）
- src/process/daemon.rs（PID ファイルと実行時ディレクトリ）
- examples/mitm_poc.rs（forward proxy の PoC）
