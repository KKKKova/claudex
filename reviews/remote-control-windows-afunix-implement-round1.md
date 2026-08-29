---
type: implement-review
slug: remote-control-windows-afunix
round: 1
commit: d21de9e
base: 42f74ee
targets:
  - src/proxy/mod.rs
  - src/process/daemon.rs
  - src/process/launch.rs
  - Cargo.toml
  - Cargo.lock
  - .github/workflows/ci.yml
  - src/config/mod.rs
  - src/oauth/manager.rs
  - src/oauth/mod.rs
  - src/oauth/providers.rs
  - src/proxy/fallback.rs
  - tests/proxy_integration.rs
acceptance: docs/specs/remote-control-windows-afunix/plan.md（受入基準 AC-1〜AC-6、Deviation Log D-1 / D-2 を含む）
---

レビュー範囲は `git diff 42f74ee..d21de9e`（plan の T001〜T004 と Deviation D-1 / D-2 に対応する差分）。
ベース 42f74ee 以前の差分（先行する windows-support 系の作業）は本レビューの対象外。

実装タスクと対応コミット:

| タスク | コミット |
|---|---|
| T001 名前付きパイプ実装の削除と依存整理・CI clippy 追加 | c5a2542 |
| T002 AF_UNIX リスナー + TCP 中継と Windows 版 socket_path | e09bb42 |
| T003 launch の Windows ガード差し替えとテスト整備 | 7f3a7bf |
| T003b 既存 clippy 債務12件の機械的修正（Deviation D-1） | 963a339 |
| T003c Windows ビルド固有の cfg 取りこぼし4件の修正（Deviation D-2） | 7bb3c3e |
| T004 version bump | d21de9e |

実行時観察の結果（AC-3）:

- mac で release ビルドの proxy を起動し、`curl --unix-socket` で `/health`（200）・`/v1/models`（200）・`/proxy/codex-sub/v1/messages`（200、上流の推論応答本文あり）を確認した
- proxy ログに `incoming request` が1行記録された
- `claudex run codex-sub` 自体は claude.ai トークンの期限切れガードで手前で停止したため未観察。原因は `~/.claude/.credentials.json` が期限切れのまま残り、より新しいキーチェーンの値より優先されること。本差分と独立した既存の事象

AC-4 / AC-5 / AC-6（Windows 実機）は未観察。fork issue #6 で検証者へ引き継ぐ予定。
