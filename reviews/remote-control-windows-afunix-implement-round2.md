---
type: implement-review
slug: remote-control-windows-afunix
round: 2
commit: e145eee
base: d21de9e
targets:
  - src/proxy/mod.rs
acceptance: docs/specs/remote-control-windows-afunix/plan.md（受入基準 AC-1〜AC-6、Deviation Log D-1 / D-2 / D-3 を含む）
---

前回指摘への対応一覧（修正はすべて `e145eee` の1コミット。レビュー範囲は `git diff d21de9e..e145eee`）。

| 指摘 | 対応 |
|---|---|
| required-1 中継先ループバックポートの乗っ取り | 対応。`spawn_afunix_relay(bound: SocketAddr)` / `relay_afunix_connection(unix, target: SocketAddr, seq: u64)` へ変更し、`start_proxy` は `spawn_afunix_relay(listener.local_addr()?)?` を無条件に呼ぶ。中継先は新設の純関数 `relay_target(bound) -> SocketAddr` が決め、unspecified のときだけ同一ファミリのループバックへ写像する。`host` 文字列の許可リストと warn 分岐は削除した |
| suggestion-1 accept ループのバックオフ | 対応。`Err` 分岐に 500ms の `thread::sleep` を追加 |
| suggestion-2 別ポートでの二重起動によるソケット/PID 奪取 | 限定対応。`spawn_afunix_relay` 内で stale 削除の前に `is_proxy_running()?` を確認し、生存していれば削除も bind もせず `bail!`。`main.rs` / CLI と unix 経路は未変更 |
| suggestion-3 `relay_pump` の Err 経路と起動判定のテスト | 対応。`test_relay_pump_shutdown_fires_when_copy_fails` を追加。`relay_target` を `#[cfg(any(windows, test))]` にし `test_relay_target_maps_unspecified_to_same_family_loopback` / `test_relay_target_keeps_bound_address_as_is` を追加 |
| suggestion-4 中継バイト数を捨てている | 対応。`JoinHandle` を join し、`debug!(seq, upstream_bytes, downstream_bytes, "unix socket relay connection finished")` を出す。スレッドのパニックは 0 バイト扱いで `warn!` を残す |
| nit `try_clone` の同型エラー処理4回 | 対応。`cloned_or_warn()` ヘルパへ寄せた |
| nit 透過性テストの読み取りタイムアウト欠如 | 対応。`read_exact` の前に両クライアントへ `set_read_timeout(5s)` を設定 |
| nit unix 版 `socket_path()` の `/tmp` フォールバック | 未対応（本差分は unix 経路を変更していない既存債務のため、別件として扱う） |

異議: なし。

ミューテーション確認: `relay_pump` の `shutdown_to()` を `Ok` 分岐だけに移した状態で `test_relay_pump_shutdown_fires_when_copy_fails` が失敗することを確認し、元に戻した。

静的ゲートの結果: mac の `cargo clippy --all-targets -- -D warnings` / `cargo test`（406件）/ `cargo fmt --check`、および `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` がいずれも成功。

修正で新たに生じた挙動の変化:

- 非ループバックの `proxy_host` でも AF_UNIX 中継を立てるようになった（従来は warn を出して TCP のみで起動）。bind が失敗すれば proxy 自体が起動しない
- Windows で PID ファイルが生存プロセスを指す場合、`proxy start` が `bail!` するようになった。stale PID が無関係プロセスに再利用されている場合も起動しない

AC-4 / AC-5 / AC-6（Windows 実機）は未観測のまま。本修正は Windows 実機で一度も実行していない。
