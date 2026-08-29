---
type: implement-review
slug: remote-control-windows
round: 2
commit: 300792b
targets:
  - src/proxy/mod.rs
  - src/process/daemon.rs
  - src/process/launch.rs
  - Cargo.toml
  - README.md
  - config.example.toml
acceptance: docs/specs/remote-control-windows/plan.md の「受入基準」節（AC-1〜AC-5）。docs/ は .gitignore 対象のためワーキングツリー上のパスを参照する。
---

## 前回指摘への対応

| 指摘 | 対応 | 修正コミット |
|---|---|---|
| [required] パイプ名先取りによる fail-open | 対応 | 300792b |
| [required] `stop_proxy` が `TerminateProcess` の成否を見ない | 対応 | 300792b |
| [suggestion] README / `config.example.toml` の「Unix only」4箇所 | 対応 | 93b7078 |
| [suggestion] `is_proxy_running` が `GetExitCodeProcess` 失敗を停止と同一視 | 未対応 | — |
| [suggestion] `accept` の無限リトライが恒久障害を隠す | 未対応 | — |
| [suggestion] `USERNAME` 未設定時のフォールバック名が全ユーザー共通 | 未対応 | — |
| [nit] `WaitNamedPipeW(name, 0)` のコメント | 該当コード削除により消滅 | 300792b |
| [nit] Deviation Log に T004 の逸脱が未記載 | 対応（plan.md に追記。docs/ は git 管理外） | — |
| [nit] CI の `windows-check` に clippy がない | 未対応 | — |

### required-1 の対応内容

3点の変更で fail-open を塞いだ。

1. `pipe_exists()` を `pipe_served_by_proxy()` へ置き換えた（`daemon.rs`）。`CreateFileW` でクライアント端を開き、`GetNamedPipeServerProcessId` が返すサーバ PID を PID ファイルの値と照合する。不一致・`ERROR_PIPE_BUSY`・`read_pid()` が `None` はいずれも `Ok(false)`、その他の失敗は `Err`。ハンドルは全経路で `CloseHandle` する。`CreateFileW` には `SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS` を指定した。
2. `spawn_pipe_listener()` の戻り値を `Option<JoinHandle>` から `Result<JoinHandle>` へ変え、`start_proxy()` が `?` で bail するようにした（`proxy/mod.rs`）。あわせてパイプ生成を `write_pid()` より前へ移し、失敗時に PID ファイルを残さない。`write_pid()` 自体が失敗した場合はパイプリスナーを `abort()` してから返す。
3. `launch.rs` の Windows ガードを2分岐に分け、それぞれ別の文言で bail する。

`windows-sys` に `Win32_Storage_FileSystem` と `Win32_Security` を追加した（`CreateFileW` の item 属性が `Win32_Security` を要求するため）。

### required-2 の対応内容

`stop_proxy()` の Windows 分岐で、`OpenProcess` が null のときと `TerminateProcess` が 0 を返したときに `GetLastError()` を添えて `bail!` する。`remove_pid()?` の手前で抜けるため PID ファイルは残る。成功時のみ `println!("Terminated proxy (PID {pid})")` を出す。

## 検証の実施状況

- AC-1: `cargo check` / `cargo clippy -- -D warnings` / `cargo test`（400 passed）/ `cargo fmt --check` すべて成功
- AC-2: PR #5 の CI `Windows Check` で再実行中
- AC-3: 修正後の `0.2.8-rc.1` バイナリで proxy 再起動 → `claudex run codex-sub -p "..."` → 応答取得。proxy ログに `incoming request profile=codex-sub` と `upstream response status=200 OK`
- AC-4 / AC-5: 未実施（Windows 実機）

## 実装者が把握している未解決点

- `pipe_served_by_proxy()` の照合と Claude Code が実際に接続する瞬間の間に TOCTOU が残る。proxy がその間に死ねば名前が解放され、攻撃者が取り直せる。塞ぐには接続側（Claude Code）がサーバ PID を検証する必要があり、claudex の制御外である。
- `ERROR_PIPE_BUSY` の fail-closed は正当な利用も弾きうる（`claudex run` 同時実行時など）。再実行すれば通る。
- fail-closed 化により、Windows で `claudex proxy start --port 9000` による2本目の proxy 起動が失敗するようになった。従来は TCP のみで起動し、1本目の PID ファイルを黙って上書きしていた。
