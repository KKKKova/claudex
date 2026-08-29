---
type: implement-review-result
slug: remote-control-windows-afunix
round: 2
verdict: APPROVE
blocking: 0
reviewed: e145eee（差分 d21de9e..e145eee）
date: 2026-08-29
---

# レビュー結果: implement remote-control-windows-afunix round2

> **APPROVE** — blocking 0件 / suggestion 1件 / nit 1件。
> 判断: round1 の blocking 1件は構造的に解消している。中継先が `listener.local_addr()` 由来になり、`config.proxy_host` の文字列は中継先の決定に一切関与しなくなった。「proxy が実際に掴んだアドレス以外へは中継しない」が型で保証されるため、乗っ取りの成立条件そのものが消えている。
> 根拠: suggestion 1件は suggestion-2 の修正が新たに持ち込んだ回復不能パスで、確度は低く回避策もある。AC-2 は e145eee を含む SHA で CI 緑を確認した。

## round1 指摘への対応確認

| round1 指摘 | 対応 | 判定 |
|---|---|---|
| required-1 中継先ループバックポートの乗っ取り | `relay_target(bound)` を新設し、unspecified のときだけ同一ファミリのループバックへ写像。それ以外は bind 済みアドレスをそのまま使う。許可リストと無効化分岐は削除 | 解消 |
| suggestion-1 accept ループのバックオフ | `Err` 分岐に 500ms の `sleep`。パイプ版と同じ間隔で、理由もコメントに残っている | 解消 |
| suggestion-2 別ポートでの二重起動 | stale 削除の前に `is_proxy_running()?` で先発の生存を確認し、生きていれば bail。`write_pid` より前なので自 PID の誤検知はない | 対応。副作用は下記 suggestion |
| suggestion-3 Err 経路と起動判定のテスト | `test_relay_pump_shutdown_fires_when_copy_fails` と `relay_target` のテスト2件。ミューテーション確認も実施済み | 解消 |
| suggestion-4 中継バイト数の破棄 | 両方向を join し、seq と対応づけて方向別バイト数を `debug!` に残す。パニックは 0 バイト扱いで warn | 解消 |
| nit `try_clone` の重複 | `cloned_or_warn()` に集約 | 解消 |
| nit 透過性テストのタイムアウト | 両クライアントに `set_read_timeout(5s)` | 解消 |
| nit unix `/tmp` フォールバック | 別件として未対応 | 妥当（本差分は unix 経路を変更していない） |

## 修正の照合結果

| 確認項目 | 結果 |
|---|---|
| required-1 の成立条件が消えたか | 消えた。`proxy_host = "localhost"` が `::1` に解決した場合、`relay_target` は `[::1]:<port>` をそのまま返す。proxy 自身が掴んでいるアドレスなので、他ユーザーが先に bind する余地がない |
| `::` の扱い | 正しい。`IPV6_V6ONLY` を理由に除外していた分岐が不要になり、`::` bind でも `[::1]` へ中継されて成立する。round1 で指摘した内部矛盾も消えている |
| 非ループバック `proxy_host` の挙動変化 | 中継を立てるようになったが、接続先は proxy 自身の bind アドレスであり新しい露出は生まれない |
| `relay_target` の網羅 | V4 / V6 の unspecified と具体アドレスの3系統をテストが覆っている |
| Err 経路テストの実効性 | 有効。`shutdown_to` が発火しないと `tx` の drop により `recv_timeout` が Disconnected で落ちる。依頼側のミューテーション確認とも整合する |
| AC-5(2) の合否シグナル | 影響なし。`unix socket relay ready` に `%target` が増えただけで、`proxy listening on` は含まない |
| AC-2 | 満たしている。CI が回った 08a83e1 と e145eee の差分は依頼ファイル1件のみで、`Check` / `Windows Check` ともに SUCCESS |

## 指摘（重大度降順）

### [suggestion] PID ファイルが無関係の生存プロセスを指すと、`proxy start` が回復手段のないまま bail し続ける — src/proxy/mod.rs:277-283

<details>
<summary>詳細</summary>

suggestion-2 の対応で `spawn_afunix_relay` が `is_proxy_running()` を前提条件にしたため、Windows では PID ファイルの指すプロセスが生きている限り proxy を起動できなくなった。想定どおりの二重起動は塞げるが、PID が再利用されている場合の逃げ道が無い。

- proxy が異常終了して PID ファイルが残り、再起動などで同じ PID が別プロセスに割り当てられると、`is_proxy_running()` は true を返す（他ユーザー所有なら `ERROR_ACCESS_DENIED` も生存扱い、`src/process/daemon.rs:104-108`）。
- 修正前は `write_pid` が黙って上書きするので自然に回復していた。修正後は `proxy start` が bail する。
- bail が案内する `claudex proxy stop` は、その PID に対して `TerminateProcess` を撃つ。相手が無関係な自分のプロセスなら巻き添えで落ち、他ユーザー所有なら `ACCESS_DENIED` で bail して PID ファイルが残る。どちらの分岐でも詰まる。

現状の逃げ道は PID ファイルの手動削除だけで、その所在はメッセージに出ていない。bail 文言に PID と PID ファイルのパスを含めるだけでも復旧できる。より踏み込むなら「PID 生存 かつ ソケットファイル実在」の AND を条件にすると、TerminateProcess 後にソケットが残る経路（AC-6）と区別しきれないため、PID の明示のほうが素直である。

確度は低い（PID 再利用が必要）。ただし塞ぎ方が「起動を止める」方向なので、踏んだときの影響は Remote Control ではなく proxy 全体に及ぶ。

</details>

### [nit] 接続ごとのスレッドが2本から3本になった — src/proxy/mod.rs:380-390

<details>
<summary>詳細</summary>

`join_relay` の導入で、接続ごとの親スレッドが両方向の完了まで生存するようになった。ログのために必要な変更であり、ローカル単一ユーザーの用途では問題にならない。keep-alive の接続では親スレッドがクライアント切断まで待つ点だけ、挙動として認識しておくとよい。

</details>

## 人間が最終判断すべき箇所

1. **suggestion の扱い。** bail 文言に PID とパスを足すだけの1行修正であり、実機検証の前に入れておくと、AC-5 / AC-6 の検証中に踏んだときの往復を減らせる。round3 を起こしてまで直す性質ではない。
2. **AC-4 / AC-5 / AC-6 が依然として未観測であること。** 本修正は Windows 実機で一度も実行されていないと依頼に明記がある。この APPROVE はコードの正しさに対するもので、Windows 上で動くことの確認ではない。
3. **unix 版 `socket_path()` の `/tmp` フォールバックに残る同種の漏出経路。** round1 で nit として挙げた既存債務であり、本差分の責任範囲外として別件化する判断は妥当である。ただし内容は claude.ai トークンの漏出経路なので、issue 化まで持っていくかは人間の判断が要る。
