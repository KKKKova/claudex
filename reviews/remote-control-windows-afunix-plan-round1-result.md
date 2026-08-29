---
type: plan-review-result
slug: remote-control-windows-afunix
round: 1
verdict: REVISE
blocking: 2
reviewed: docs/specs/remote-control-windows-afunix/plan.md @ working-tree (94b48ef)
date: 2026-08-29
---

# レビュー結果: plan remote-control-windows-afunix round1

> **REVISE** — blocking 2件 / suggestion 3件 / nit 2件。
> 判断: 設計の中心（同期 AF_UNIX を TCP バイト中継で既存サーバに接ぐ）と、審査を求められたセキュリティ判断の置き換えは、いずれも妥当である。差し戻しは、Windows の AF_UNIX ソケットファイルに対する `Path::exists()` / `remove_file` の挙動を未検証のまま起動経路と launch ガードの両方の前提に置いている点と、AC-5(2) の合否シグナルが正常系で必ず「不合格」に読める点の2件による。
> 根拠: どちらも実機検証（issue #6 経由・第三者・往復1サイクルが高価）でしか露見せず、露見しても「AF_UNIX 方式そのものの失敗」と誤読されうる。plan 段階で潰せる。

## 依頼で明示的に審査を求められた点

### セキュリティ判断の置き換え（「セキュリティ判断」節）

**妥当と判定する。** 旧 required-1 の本質は「名前付きパイプの名前空間がマシン全域で誰でも新規作成でき、先取りされた口へ我々がトークンを渡してしまう」ことであった。AF_UNIX ではソケットファイルの生成が親ディレクトリの書込権限に従属する。`runtime_dir()`（`dirs::cache_dir()` → `%LOCALAPPDATA%`）配下および `<home>\.claudex` は、いずれも既定 ACL で他の標準ユーザーが書き込めない。先取りの前提が消えるため、同型の脅威は成立しない。unix 版が同じ信頼基盤に乗っているという対比も正しい。

`pipe_served_by_proxy()` 相当のサーバ側 PID 照合を持たない選択も、この前提の下では過剰ではない。またソケットへ「接続する側」の制限が無い点は、既存の 127.0.0.1 TCP 無認証面と同水準であり、こちらは方向が逆（他人が我々に繋ぐだけでトークンは流れない）なので、旧 plan の承認記録が誤っていた論点とは別物である。引き写しになっていない。

### 旧実装の削除範囲

`src/proxy/mod.rs` の `NamedPipeListener` / `PipeConnection` / `spawn_pipe_listener`、`daemon.rs` の `pipe_served_by_proxy()` とパイプ名版 `socket_path()`、`launch.rs` の `#[cfg(windows)]` ブロックが削除対象として過不足なく挙がっている。残す判断（`is_proxy_running` / `stop_proxy` の Win32 実装、env 注入の共通化）も、トランスポート非依存という理由づけどおりである。

### 外部依存の実在確認

`uds_windows` 1.2.1 を取得して照合した。plan が挙げる API はすべて実在する（`UnixListener::bind` / `accept` / `incoming` / `UnixStream::try_clone` / `shutdown`、`io::Read` と `io::Write` は `UnixStream` と `&UnixStream` の双方に実装）。`std::io::copy(&mut &unix, &mut &tcp)` は成立する。`sockaddr_un` 変換はパス長が `sun_path` 長以上なら `InvalidInput` を返すので、`MAX_SOCKET_PATH_LEN = 100` の安全域と二重に守られる。依存する windows-sys は `>=0.60, <0.62` で本体の 0.59 と別バージョンになるが、FFI 束であり共存に問題はない。

## 指摘（重大度降順）

### [required-1] Windows の AF_UNIX ソケットファイルに対する `Path::exists()` / `remove_file` の挙動を未検証のまま、stale 削除と launch ガードの両方の前提にしている — plan.md「T002 Steps 3」「T003 Steps 1」

<details>
<summary>詳細</summary>

Windows の AF_UNIX ソケットファイルは、通常のファイルではなく `IO_REPARSE_TAG_AF_UNIX` のリパースポイントとして作られる。Rust の `Path::exists()` は `fs::metadata`、すなわちリパースポイントを追跡する `CreateFileW` 経由の stat であり、ハンドラを持たないリパースタグに対しては開けずに失敗しうる。この経路が失敗すると `exists()` は「存在しない」を返す。plan はこの挙動を確認した形跡がなく、二箇所で前提にしている。

- T002 step 3:「既存ファイルは `std::fs::remove_file` で削除する（削除失敗は `?` で致命）」。存在判定で条件付けるなら、実在する stale ファイルを取りこぼして `bind` が `EADDRINUSE` で失敗する（**AC-6 が落ちる**）。条件を付けずに常に呼ぶなら、初回起動時の `NotFound` が `?` で致命になり proxy が起動しない。plan の文面はどちらとも読め、どちらの読みでも欠陥になる。
- T003 step 1(b): `!socket.exists()` を launch ガードにしている。ここで偽陰性が出ると、proxy が正常稼働していても `claudex run` が "proxy socket not found" で拒否する。**AC-4（本 plan の中心）が、AF_UNIX の往復に一度も到達しないまま FAIL する。**

いずれも mac では再現せず、CI のコンパイル検証にも掛からない。露見するのは第三者による実機検証の往復1サイクルを消費したあとで、しかも症状（run が socket not found、または proxy が起動しない）は「AF_UNIX 方式が使えない」という誤った結論にも読める。これは本 plan が「mac 緑 ≠ Windows 動作」として明示的に避けようとしている失敗形そのものである。

対処は plan 段階で閉じられる。(a) stale 削除は `remove_file` の `ErrorKind::NotFound` のみ許容して他は致命とし、存在判定に依存しない。(b) 実在確認は `symlink_metadata().is_ok()` を使う（`FILE_FLAG_OPEN_REPARSE_POINT` で開くためリパースポイントでも成功する）。あるいは、どちらの挙動なのかを実機検証の採取物に含めて先に確定させる（`dir` での実在確認だけでは Rust 側の判定と一致するとは限らない）。plan にどちらを採るか書き、根拠のない `exists()` 依存を残さないこと。

</details>

### [required-2] AC-5(2) の合否シグナルが、正常系で必ず「不合格」に読める — plan.md「受入基準 AC-5」

<details>
<summary>詳細</summary>

AC-5(2) は「proxy ログに `proxy listening on` が2度出ない」と書かれている。一方 T002 step 3 は、AF_UNIX 側の起動ログを `tracing::info!(path = …, "proxy listening on unix socket")` と定めている。既存の unix 経路（`src/proxy/mod.rs:167`）と同じ文言である。したがって Windows の正常な単一起動でも、ログには

```
proxy listening on 127.0.0.1:13456
proxy listening on unix socket path=...
```

の2行が出る。`proxy listening on` を数える限り、健全な状態が AC-5(2) の不合格として報告される。

plan の T002 step 3 には「AC-5(2) のログ判定は TCP 側の `proxy listening on {bind_addr}` を数えるため、この行は `unix socket` を含む別文言にする」という注記があり、衝突自体は認識されている。しかし `unix socket` を含めても `proxy listening on` の部分文字列一致は避けられず、AC-5 本文が更新されていない。

AC-5 は issue #6 経由で第三者が実行し、判定を文面どおりに行う。曖昧さのコストは往復1サイクルである。AC-5(2) 側に数える文字列を確定して書くこと（例: `proxy listening on 127.0.0.1:` の出現が1回）。あるいは AF_UNIX 側のログ文言を `proxy listening on` を含まない形に変える。

</details>

### [suggestion-1] 中継本体を Windows 専用にせず、mac の `cargo test` で覆える形にできる — plan.md「T002 Steps 3」

<details>
<summary>詳細</summary>

`relay_connection` の中身は「2本のストリームを双方向に `io::copy` し、片方向の終端で相手に `shutdown(Write)` を伝播する」だけで、AF_UNIX 固有の要素を含まない。`Read + Write + Send` のジェネリック関数（あるいは `TcpStream` 同士で駆動できる内部関数）に切り出せば、mac の `cargo test` で「上りと下りが透過する」「片側 close が伝播する」を検証できる。

本 plan で唯一実行検証の手段が無いのが Windows 専用コードであり、その中で論理を持つのは中継部だけである。分離すれば、残る未検証部分は `bind` / `accept` / パス解決という薄い層に縮む。T003 のテスト整備の範囲を広げる形で収まる。

</details>

### [suggestion-2] 中継先ホストの読み替えが `0.0.0.0` のみで、`::` やホスト名指定で外れる — plan.md「中心の設計選択」末尾 / 「T002 Steps 4」

<details>
<summary>詳細</summary>

`config.proxy_host` が `::`（IPv6 全アドレス）や `0.0.0.0` 以外の非ループバック表記の場合、`TcpStream::connect((host, port))` はその文字列をそのまま接続先に使う。`::` への connect は失敗し、中継が全接続で落ちる。症状は AC-4 の FAIL であり、原因の切り分けは難しい。

中継先は常にループバックでよい。バインドアドレスに関わらず `127.0.0.1` 固定にするか、ループバック以外の明示指定は起動時に明示エラーにするほうが、設定と挙動の対応が読める。

</details>

### [suggestion-3] `windows-check` に `--all-targets` が無く、T003 で足すテストは Windows でコンパイル検証されない — plan.md「T001 Steps 5」

<details>
<summary>詳細</summary>

現行の `windows-check` は `cargo check --target x86_64-pc-windows-msvc` のみで、`#[cfg(test)]` は Windows 向けにコンパイルされない。clippy を足す step を書くタイミングで `--all-targets` も併せて付けておくと、テストコードが Windows で壊れている状態を CI が拾う。suggestion-1 を採る場合は特に効く。

</details>

### [nit] T001 の完了条件 grep パターンが `PipeConnection` を拾わない

<details>
<summary>詳細</summary>

`git grep -n "named_pipe\|pipe_served\|NamedPipe"` は、削除対象として step 1 に挙がっている `PipeConnection` にマッチしない。`Pipe` 単体を含める、または対象名を列挙する。

</details>

### [nit] `read_claude_ai_session()` の keychain フォールバックが Windows では必ず失敗し、エラー文が誤誘導する

<details>
<summary>詳細</summary>

`src/oauth/source.rs:138` の `read_claude_keychain()` は `std::env::var("USER")` を読む。Windows では未設定（`USERNAME`）なので、`~/.claude/.credentials.json` が読めなかったときのメッセージが「keychain fallback failed: cannot determine keychain account (USER unset)」になる。主経路はファイル読みで、Claude Code は Windows でもこのファイルを書くため実害は小さい。ただし AC-4 が FAIL したときの切り分けを鈍らせる位置にある。本 plan のスコープ外なので、直すなら別件でよい。

</details>

## 人間が最終判断すべき箇所

1. **required-1 を「plan で閉じる」か「実機で確定させる」か。** 前者なら `symlink_metadata` 等への置き換えを plan に書いて実装に進める。後者なら実機検証の採取物に判定を足す設計になり、AC-4 の往復が1サイクル増える可能性を受け入れることになる。
2. **PR #5 続行の決定（ユーザー決定済み）と、rc.1 Release の扱い。** plan の提案（警告追記して残す）は履歴の保存として筋が通るが、公開バイナリを残す判断そのものは人間の領分である。
3. **中継方式が持ち込むスレッドモデルの受容。** 接続ごとにスレッド2本を detach し、graceful 停止を持たない割り切りは、ローカル単一ユーザー用途としては妥当と判定した。将来 Windows で常駐時間が延びる使い方をするなら、この判断は見直し対象になる。
