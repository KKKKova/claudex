---
type: implement-review-result
slug: remote-control-windows-afunix
round: 1
verdict: REVISE
blocking: 1
reviewed: d21de9e（差分 42f74ee..d21de9e）
date: 2026-08-29
---

# レビュー結果: implement remote-control-windows-afunix round1

> **REVISE** — blocking 1件 / suggestion 4件 / nit 3件。
> 判断: plan の設計はおおむね忠実に実装されており、plan-review で潰した論点（リパースポイント対応、ログ文言の分離、`relay_pump` のテスト可能化）はすべて差分に入っている。差し戻しは、旧 required-1 と同型の fail-open が「パイプ名前空間」から「中継先のループバック未 bind ポート」へ場所を変えて再発している1件による。
> 根拠: `proxy_host = "localhost"` のとき proxy は解決順先頭の1アドレス（Windows では `[::1]`）しか bind しないが、中継は `127.0.0.1` へハードコードで繋ぐ。誰も掴んでいない `127.0.0.1:<port>` は同一マシンの別標準ユーザーが権限なしで bind でき、claude.ai トークンと全プロンプトがそこへ流れる。

## 確認できた点

| 確認項目 | 結果 |
|---|---|
| AC-2（Windows CI） | 満たしている。fork PR #5 の `Check` / `Windows Check` がいずれも SUCCESS。CI が回った a4baca4 と本レビュー対象 d21de9e の差分は依頼ファイル1件のみ |
| plan-review required-1（`exists()` 依存） | 解消。`spawn_afunix_relay` は存在判定を挟まず `remove_file` し `NotFound` のみ許容、launch 側は `symlink_metadata()` 判定（`src/proxy/mod.rs:253`、`src/process/launch.rs:190`） |
| plan-review required-2（AC-5(2) の文言衝突） | 解消。AF_UNIX 側は `unix socket relay ready`。`proxy listening on unix socket` は `#[cfg(unix)]` で Windows には出ない |
| plan-review suggestion-1（中継のテスト可能化） | 対応。`relay_pump` は `#[cfg(any(windows, test))]` で共通化され、mac で走るテスト2件が入っている |
| plan-review suggestion-3（`--all-targets`） | 対応。check / clippy 双方に付与済み |
| `windows-sys` feature の削減 | 妥当。`git grep` で残る利用は `Win32_Foundation` / `Win32_System_Threading` のみ |
| shutdown 伝播の方向 | 正しい。unix EOF → tcp の書き込み閉、tcp EOF → unix の書き込み閉 |
| D-1 / D-2 の12+4件 | いずれもテストコードと cfg の機械的修正で、アサーション対象・実行時挙動は不変 |
| トークンのログ漏出 | 無し。`incoming request` は `describe_credential` で `Authorization` を `(present)` に潰す。中継はバイト列をログに出さない |

## 指摘（重大度降順）

### [required-1] `proxy_host = "localhost"` のとき、中継先 `127.0.0.1:<port>` を同一マシンの別ローカルユーザーが乗っ取れる（旧 required-1 と同型の fail-open） — src/proxy/mod.rs:127 / src/proxy/mod.rs:295

<details>
<summary>詳細</summary>

**成立条件と経路**

1. `start_proxy` は `tokio::net::TcpListener::bind("localhost:<port>")` で bind する（`src/proxy/mod.rs:118`）。tokio / std の `bind` は解決されたアドレスを先頭から試し、**最初に成功した1つだけ**を掴む。Windows の `getaddrinfo` は RFC 6724 の宛先選択により `localhost` に対して `::1` を先に返すため、実際に listen されるのは `[::1]:<port>` のみで、`127.0.0.1:<port>` は誰も掴んでいない。
2. それでも中継の起動判定（`src/proxy/mod.rs:127`）は `"localhost"` を許可リストに含めるため `spawn_afunix_relay` が走り、`relay_afunix_connection` は無条件に `TcpStream::connect(("127.0.0.1", port))` する（`src/proxy/mod.rs:295`）。
3. 同一マシンの別ローカル標準ユーザーは、特権も被害者プロファイルへの書込権限もなしに `127.0.0.1:<port>` を bind できる。空いているポートなので競合も ACL チェックも起きない。
4. `claudex run` の2段ガード（`src/process/launch.rs:187-197`）は PID ファイルのプロセス生存とソケットファイルの `symlink_metadata` しか見ない。**中継先 TCP エンドポイントの素性は誰も検証していない。** ガードを通過して `CLAUDE_CODE_OAUTH_TOKEN` が Claude Code に渡る。
5. Claude Code が AF_UNIX へ流す HTTP は `relay_pump` がバイト列のまま攻撃者の listener へ複製する。`Authorization: Bearer <claude.ai OAuth token>` と全プロンプト本文が渡り、応答も攻撃者が自由に注入できる。

**なぜ blocking か**

これは旧 implement-review round1 の required-1（パイプ名の先取り）と同型である。plan の「セキュリティ判断」節が守っているのは `%LOCALAPPDATA%\claudex` 配下のソケットファイルだけで、**中継先のループバックポートはその信頼境界の外側にある**。攻撃者はソケットファイルに一切触らない。plan 承認時にこのレイヤは存在せず、本実装で新しく生まれた露出である。

**反証の確認**

- 既存ガードで止まるか → 止まらない（上記4）。削除された `pipe_served_by_proxy()` に相当する「相手の素性を確かめる」機構は中継先に対して存在しない。
- ディレクトリ ACL で止まるか → 止まらない。守備範囲が違う。
- 既定設定で踏むか → 踏まない。既定は `127.0.0.1`（`src/config/mod.rs:284`、`config.example.toml:9`）で、この場合は proxy 自身が当該アドレスを掴むため他ユーザーは奪えない。`0.0.0.0` も同様。したがって成立は `"localhost"` 設定に限定される。
- コード自身の推論と整合するか → **矛盾している。** `::` は「Windows の `IPV6_V6ONLY` 既定で IPv4 接続を受けない」という理由で許可リストから外されている。`localhost` が `::1` に解決した場合はまさにその除外理由に該当するのに、許可リストに残っている。

**修正の方向**

`spawn_afunix_relay` に `port` ではなく `listener.local_addr()`（`src/proxy/mod.rs:118` で取得済み）を渡し、unspecified（`0.0.0.0` / `::`）のときだけ同一ファミリのループバックへ写像した `SocketAddr` へ connect する。「proxy が実際に bind したアドレス以外へは中継しない」が構造的に保証され、文字列の許可リスト自体が不要になる。暫定でよければ許可リストから `"localhost"` を外すだけでも成立条件は消える。

</details>

### [suggestion-1] AF_UNIX の accept ループにバックオフが無く、持続的な accept 失敗でタイトループになる — src/proxy/mod.rs:270-281

<details>
<summary>詳細</summary>

`uds_windows::Incoming::next` は `Some(self.listener.accept().map(...))` を返すだけで、エラーでも `None` を返さない（クレート 1.2.1 の `src/stdnet/net.rs:599-604` で確認）。したがって accept が持続的に失敗する状態に入ると、`warn!` を吐きながら無制限に回り続ける。CPU を1コア占有し、proxy のログファイルも際限なく膨らむ。

この間も PID ファイルとソケットファイルは残るため、`claudex proxy status` も launch 側の2段ガードも「正常」を返す。利用者からは Remote Control が理由不明に無応答になるだけである。

削除された名前付きパイプ実装は、同じ失敗経路に 500ms の `sleep` を2箇所置いていた（42f74ee 時点の `src/proxy/mod.rs:227,235`）。本差分はその退行にあたる。持続的失敗の確度自体は低いが、修正は数行で済む。

</details>

### [suggestion-2] `--port` を変えた二重起動で、後発プロセスがソケットファイルと PID ファイルを黙って奪う — src/proxy/mod.rs:253 / src/main.rs:114-122

<details>
<summary>詳細</summary>

`claudex proxy start` は非 daemon 経路で `is_proxy_running()` を確認しない。同一ポートなら TCP bind が `AddrInUse` で落ちて fail-closed になるが、`--port` を変えると bind は成功し、`spawn_afunix_relay` が既存ソケットを無条件削除して bind し直し、`write_pid` が PID ファイルを上書きする。先発プロセスは生きたまま、Remote Control の口だけが後発に移る。

名前付きパイプ版は `first_pipe_instance(true)` で同じ場面が fail-closed になっていたので、これも退行にあたる。権限境界は跨がない（同一ユーザー）ので security 指摘にはしない。

あわせて AC-5(2) の検知力についても記録しておく。proxy のログファイルはプロセスごとに別ファイル（`proxy-{ts}-{pid}.log`、`src/proxy/mod.rs:54-58`）なので、1つのログ中で `proxy listening on` を数える方法では二重起動そのものを捕まえられない。AC-5(2) が実際に見ているのは「`claudex run` が既存 proxy を再利用し、二重に起動しないこと」であって、上記の `--port` 経路は範囲外である。

</details>

### [suggestion-3] `relay_pump` のエラー経路と、中継の起動判定にテストが無い — src/proxy/mod.rs:219-229 / src/proxy/mod.rs:127-140

<details>
<summary>詳細</summary>

追加された2件のテストはどちらも正常 EOF 経路しか通らない。(a) は `shutdown_to` が no-op、(b) は `from` が最初から空で `copy` が `Err` にならない。`Err` 分岐から `shutdown_to()` の呼び出しを外しても両方緑のままである。この分岐が壊れると、クライアントが異常切断したときに反対方向のスレッドと TCP 接続が閉じられず滞留する。

中継の起動判定 `matches!(host.as_str(), …)` は OS 依存の副作用を持たない純ロジックだが `#[cfg(windows)]` の中に埋まっており、mac のテストでは到達しない。Windows CI は `check` / `clippy` のみでテストを実行しないため、この判定はどこからも機械的に検証されない。required-1 の修正でこの判定に手を入れるなら、`relay_pump` と同じく切り出してテストで覆う価値がある。

</details>

### [suggestion-4] 中継バイト数を返しているのに呼び出し側で捨てており、実機 FAIL 時の切り分け材料が残らない — src/proxy/mod.rs:333-339

<details>
<summary>詳細</summary>

`relay_pump` は転送バイト数を返すが、`relay_afunix_connection` は `JoinHandle` ごと破棄している。方向ごとの転送量が残らないため、AC-4 が FAIL したときに「接続は来たが上りが0バイト」なのか「上りは流れたが下りが返らない」なのかをログから切り分けられない。`unix socket connection accepted` の seq と対応づけて、終了時に方向・バイト数を `debug!` で1行残すだけで、実機検証の往復コストが下がる。

</details>

### [nit] `try_clone` のエラー処理が同型で4回繰り返されている — src/proxy/mod.rs:303-330

<details>
<summary>詳細</summary>

`match x.try_clone() { Ok(v) => v, Err(e) => { warn!; return; } }` がストリーム種別だけ変えて4回並ぶ。小さなヘルパかマクロに寄せると読みやすくなる。挙動は変わらないため任意である。

</details>

### [nit] 透過性テストに読み取りタイムアウトが無く、リグレッション時にハングで詰まる — src/proxy/mod.rs:359-379

<details>
<summary>詳細</summary>

`test_relay_pump_is_transparent_both_directions` は `read_exact` の前に `set_read_timeout` を設定していない。中継方向が入れ替わる種のリグレッションが入ると、アサーション失敗ではなく無限ブロックになる。終端伝播テスト側はタイムアウトを設定しているので、対称にしておくとよい。

</details>

### [nit] plan の「unix 版と同じ信頼基盤」という論拠は、unix 側の `/tmp` フォールバックでは成立していない（既存債務・別件） — src/process/daemon.rs:36-43

<details>
<summary>詳細</summary>

unix 版 `socket_path()` は 100 バイト超過時に `std::env::temp_dir()` 配下へ落ちる。`/tmp` は全ユーザー書込可（sticky）なので、別ユーザーが先に `claudex-<uid>-proxy.sock` を作っておくと、`spawn_unix_listener` の `remove_file` が EPERM になり warn + `None` で TCP のみ起動を続け（`src/proxy/mod.rs:183-188`）、launch 側の `socket.exists()` は攻撃者のソケットを見て通してしまう。

本差分は unix 経路を一切変更していないので blocking にはしない。ただし plan の「セキュリティ判断」節が Windows 版を承認する論拠として引いている「unix 版がユーザー所有 runtime ディレクトリに置く信頼基盤と同じ」という前提は、この経路で部分的に偽である。Windows 版のフォールバック `<home>\.claudex\p.sock` はむしろ `/tmp` より安全なので、判断の結論そのものは覆らない。別 issue として起票する材料である。

</details>

## 人間が最終判断すべき箇所

1. **required-1 の修正範囲。** `listener.local_addr()` 由来に組み替えると許可リストごと不要になり構造的に閉じるが、`start_proxy` と `spawn_afunix_relay` のインタフェースに手が入る。`"localhost"` を許可リストから外すだけの暫定対処でも成立条件は消える。どちらを採るかは、rc.2 の実機検証をいつ回すかとの兼ね合いになる。
2. **unix 版 `/tmp` フォールバックの扱い。** 既存債務であり本差分の責任範囲外だが、内容は claude.ai トークンの漏出経路である。別 issue として切るか、この機会に一緒に直すかは人間の判断が要る。
3. **AC-4 / AC-5 / AC-6 が未観測であること。** AC-1 / AC-2 は CI で確認でき、AC-3 は `curl --unix-socket` による観察が依頼に記録されているが、`claudex run` 経由の経路は claude.ai トークン期限切れで未到達である。Windows 実機の3項目とあわせ、承認後もこの差分は「動くと確認された」状態ではない。
