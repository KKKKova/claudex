---
type: implement-review
slug: remote-control-windows
round: 1
commit: 1152f3a
targets:
  - src/proxy/mod.rs
  - src/process/daemon.rs
  - src/process/launch.rs
  - Cargo.toml
acceptance: docs/specs/remote-control-windows/plan.md の「受入基準」節（AC-1〜AC-5）。docs/ は .gitignore 対象のためワーキングツリー上のパスを参照する。
---

## レビュー範囲

`main..1152f3a` の4コミット。

| SHA | タスク | 内容 |
|---|---|---|
| 0cbf8a8 | T001 | `proxy/mod.rs` に `NamedPipeListener`（`axum::serve::Listener` 実装）と `spawn_pipe_listener()`。`daemon.rs` に `#[cfg(windows)] socket_path()` |
| 2d4ec93 | T003 | `daemon.rs` の `is_proxy_running()` Windows 実装、`stop_proxy()` の Windows 分岐、`pipe_exists()` 新設、`windows-sys` 依存追加 |
| 205f3ca | T002 | `launch.rs` の非対応 bail 削除、`apply_remote_control_env` 等の全プラットフォーム共通化、ソケット実在確認の分岐 |
| 1152f3a | T004 | version を `0.2.8-rc.1` へ |

## 検証の実施状況

- AC-1（mac リグレッション）: `cargo check` / `cargo clippy -- -D warnings` / `cargo test`（400 passed）/ `cargo fmt --check` 実行済み
- AC-2（Windows コンパイル）: PR #5 の CI `Windows Check` ジョブ（`cargo check --target x86_64-pc-windows-msvc`）conclusion = success
- AC-3（mac e2e）: `0.2.8-rc.1` バイナリで proxy 起動 → `claudex run codex-sub -p "..."` → 応答取得。proxy ログに `incoming request profile=codex-sub` と `upstream response status=200 OK`
- AC-4 / AC-5（Windows 実機）: 未実施。T004 で検証担当へ引き継ぐ範囲

## plan からの逸脱

1. **T003 / 軽微**: `stop_proxy()` の unix 分岐をブロック形へ変え、従来は無条件だった `println!("Sent SIGTERM to proxy (PID {pid})")` を unix ブロック内へ移した。plan T003 Step 2 が Windows 側に別文言を要求するため。
2. **T004 / 軽微**: version bump を「マージ後の fork/main」ではなく feature ブランチ上で行い、PR に同乗させた。plan T004 Step 2 の記述はマージ後の main を指していたが、plan スコープ節と T004 Files 欄は `Cargo.toml` の bump を本ブランチの変更として挙げている。
