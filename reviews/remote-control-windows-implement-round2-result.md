---
type: implement-review-result
slug: remote-control-windows
round: 2
commit: 300792b
verdict: APPROVE
blocking: 0
---

# implement remote-control-windows round2 — APPROVE（blocking 0件）

対象: `1152f3a..300792b`（93b7078 ドキュメント / 300792b blocking 2件の修正）
round2 は修正確認である。round1 の指摘への対応と、修正によって新たに入った欠陥の有無だけを見た。

## round1 指摘の処理

| round1 の指摘 | 判定 | 根拠 |
|---|---|---|
| [required] パイプ名先取りによる fail-open | **解消** | 下記 |
| [required] `stop_proxy` が `TerminateProcess` の成否を見ない | **解消** | 下記 |
| [suggestion] README / `config.example.toml` の「Unix only」4箇所 | 解消 | 4箇所すべて消え、残存を grep で確認。「実機未検証」の但し書きも入った |
| [suggestion] `is_proxy_running` の `GetExitCodeProcess` 失敗 | 未対応 | 据え置き。suggestion は差し戻し理由にしない |
| [suggestion] `accept` の無限リトライ | 未対応 | 同上。ただし残存リスクと連動する（末尾3を参照） |
| [suggestion] `USERNAME` フォールバック名 | 未対応 | 同上 |
| [nit] `WaitNamedPipeW` のコメント | 消滅 | 当該コードが `CreateFileW` 経路に置き換わった |
| [nit] Deviation Log の T004 逸脱 | 対応 | plan.md に追記済み |
| [nit] `windows-check` に clippy がない | 未対応 | 同上 |

### required-1 の解消確認

塞ぐべきだった4段の連鎖のうち、2段目と4段目が断たれている。

- 2段目（fail-open）: `spawn_pipe_listener` が `Result` を返すようになり、`start_proxy` が `?` で bail する（`proxy/mod.rs:123-124, 261-289`）。`first_pipe_instance(true)` の失敗はもう warn で流れない。TCP の `bind` と同じ fail-closed に揃った。パイプ生成を `write_pid()` より前へ移し、`write_pid()` 自身が失敗した場合は `abort()` してから返す順序も正しい。stale な PID ファイルは残らない。
- 4段目（作成者を検証しないガード）: `pipe_exists()` が `pipe_served_by_proxy()` に置き換わり、`CreateFileW` でクライアント端を開いて `GetNamedPipeServerProcessId` のサーバ PID を PID ファイルと照合する（`daemon.rs:207-300`）。不一致・`ERROR_PIPE_BUSY`・`read_pid()` が `None` はすべて `Ok(false)`、想定外のエラーコードは `Err`。素性を確かめられない経路がすべて fail-closed 側へ倒れている。`ERROR_ACCESS_DENIED` も `Err` 側に落ちるため、DACL で閉じられた先取りパイプも通らない。
- `SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS` の指定は `ImpersonateNamedPipeClient` への備えとして適切である。
- ハンドルは全経路で `CloseHandle` される。`GetLastError()` を `CloseHandle` より前に読む順序も正しい（`daemon.rs:271-282`、`stop_proxy` 側も同様）。

### required-2 の解消確認

`daemon.rs:152-178`。`OpenProcess` が null のとき、`TerminateProcess` が 0 を返したときのそれぞれで `GetLastError()` を添えて `bail!` する。いずれも `remove_pid()?` の手前で抜けるため PID ファイルは残る。成功時だけ `println!("Terminated proxy (PID {pid})")` を出す。round1 で指摘した「生きていると判定 → 殺せない → 殺したと表示 → PID ファイル削除」の一本道は成立しなくなった。

### 新たに入った欠陥の有無

見つからなかった。確認した内容は次のとおり。

- 追加した Win32 シンボル（`CreateFileW` / `GetNamedPipeServerProcessId` / `GENERIC_READ` / `INVALID_HANDLE_VALUE` / `ERROR_PIPE_BUSY` / `FILE_SHARE_NONE` / `OPEN_EXISTING` / `SECURITY_ANONYMOUS` / `SECURITY_SQOS_PRESENT`）の存在と、`CreateFileW` の各引数の型整合を windows-sys 0.59 のローカルソースで照合した。`Win32_Storage_FileSystem` と `Win32_Security` の feature 追加も、`lpSecurityAttributes` の型が `Win32::Security::SECURITY_ATTRIBUTES` であることから必要かつ十分である。
- `spawn_pipe_listener` 内の `use anyhow::Context;` は、`proxy/mod.rs` が module レベルで `anyhow::Result` しか import していないため重複しない。
- `pipe_served_by_proxy()` は pipe instance を1本消費するが、`accept()` が接続確立後に次の instance を先回り作成するため、後続の接続が待たされることはない。消費された instance は hyper 側で即 EOF となって閉じる。
- `pipe_exists` への参照は `src/` に残っていない。

## 指摘（非blocking）

- **[suggestion] Windows で2本目の proxy（`claudex proxy start --port 9000`）が起動できなくなった。** `first_pipe_instance(true)` の失敗が致命になったためである。従来は TCP のみで起動し、1本目の PID ファイルを黙って上書きしていた。挙動としては fail-closed 化のほうが正しく、退行とは見なさない。ただし利用者から見れば仕様変更なので、意図した変更である旨をどこかに残す価値はある。
- **[suggestion] `ERROR_PIPE_BUSY` の fail-closed は正当な利用も弾きうる。** 全 instance が使用中の瞬間に `claudex run` が重なると偽陰性になる。安全側に倒す判断は妥当だが、エラー文言が「検証できなかった」ではなく「未検証のパイプへは渡さない」と読めるため、再実行で解決することが利用者に伝わりにくい。

## 人間が最終判断すべき箇所

1. **AC-2 が本 SHA でまだ緑になっていない。** 依頼時点で CI は再実行中である。追加シンボルの存在と型整合はレビュー側で照合済みなのでコンパイル失敗の確度は低いが、証明は済んでいない。この APPROVE は AC-2 の成功を前提とする。
2. **`pipe_served_by_proxy()` の照合と Claude Code の実接続の間の TOCTOU は残る。** 接続側の検証は claudex の制御外であり、実装で塞ぐ手段がない。受け入れるかどうかは設計判断である。
3. **残存リスクは round1 の suggestion「`accept` の無限リトライ」と連動している。** `connect()` 失敗の分岐は唯一の instance を drop したまま 500ms スリープするため、その間だけパイプ名が完全に消える。ここで名前を奪われると、以降 claudex は `first_pipe_instance` 無しの `create()` で攻撃者所有パイプの追加 instance として同居し、接続先が確率的に分かれる。`pipe_served_by_proxy()` はこの状態を確定的には検出できない。発生確率は低いが、この suggestion を潰すと残存リスクも同時に小さくなる関係にある。
