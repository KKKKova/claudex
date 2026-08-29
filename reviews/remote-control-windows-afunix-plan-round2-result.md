---
type: plan-review-result
slug: remote-control-windows-afunix
round: 2
verdict: APPROVE
blocking: 0
reviewed: docs/specs/remote-control-windows-afunix/plan.md @ working-tree (1c77f2b)
date: 2026-08-29
---

# レビュー結果: plan remote-control-windows-afunix round2

> **APPROVE** — blocking 0件 / suggestion 3件。
> 判断: round1 の blocking 2件はいずれも解消している。実在判定は `symlink_metadata` に統一され、stale 削除は存在判定から切り離された。AC-5(2) は数える文字列が確定し、AF_UNIX 側のログ文言が衝突しない形に変わっている。
> 根拠: suggestion 3件はすべて round1 の修正で新たに入ったもので、いずれも T002 / T003 の完了条件（mac の clippy と test）か非既定設定でしか現れず、実装フェーズの Deviation Log で吸収できる範囲である。

## round1 指摘への対応確認

| round1 指摘 | 対応 | 判定 |
|---|---|---|
| required-1 `exists()` 依存 | 実在判定を `symlink_metadata().is_ok()` に変更（T003 Step 1(b)）。stale 削除は存在判定なしで常に `remove_file` し `NotFound` のみ許容（T002 Step 4）。理由を「セキュリティ判断」節と doc コメント指示に明記 | 解消 |
| required-2 AC-5(2) の合否シグナル | AF_UNIX 側を `unix socket relay ready` に変更し、AC-5(2) を「`proxy listening on` の行が1回だけ」に確定 | 解消 |
| suggestion-1 中継本体のテスト可能化 | `relay_pump` をプラットフォーム共通に分離し、T003 Step 4 に mac で走るテスト2件を追加 | 対応。下記 suggestion-1 / 3 は本対応が生んだ副作用 |
| suggestion-2 中継先ホストの読み替え | 中継先を `127.0.0.1` 固定にし、非ループバック `proxy_host` では中継を立てず warn | 対応。許容リストの取りこぼしは下記 suggestion-2 |
| suggestion-3 `--all-targets` | check / clippy 双方に付与（T001 Step 5） | 解消 |
| nit grep パターン | 完了条件に `PipeConnection` を追加 | 解消 |
| nit oauth の `USER` フォールバック | スコープ外として別件扱いを明記 | 解消（判断として妥当） |

## 修正の照合結果

| 確認項目 | 結果 |
|---|---|
| AC-5(2) の文字列が Windows で一意か | 一意。`proxy listening on unix socket`（`src/proxy/mod.rs:167`）は `#[cfg(unix)]` で Windows ではコンパイルされないため、残るのは TCP 側の1行のみ |
| stale 削除の両失敗形が閉じたか | 閉じた。初回起動の `NotFound` は許容、実在する stale の取りこぼしは存在判定を使わないことで消える |
| `symlink_metadata` が偽陰性を消すか | 消す。`FILE_FLAG_OPEN_REPARSE_POINT` 経由で開くため `IO_REPARSE_TAG_AF_UNIX` でも成功する。親ディレクトリ不在時は `Err` で fail-closed 側に倒れる |
| `relay_pump` のシグネチャが Windows 経路で成立するか | 成立する。`try_clone()` が所有ハンドルを返すため `impl Read + Send + 'static` / `impl Write + Send + 'static` を満たす |
| セキュリティ判断の記述が round1 判定から変質していないか | していない。実在判定の手段が変わっただけで、信頼基盤（親ディレクトリの既定 ACL）の主張は同一 |

## 指摘（重大度降順）

### [suggestion-1] `relay_pump` を cfg なしで定義すると unix ビルドで未使用となり、AC-1 の `clippy -- -D warnings` が落ちる — plan.md「T002 Steps 3」

<details>
<summary>詳細</summary>

T002 step 3 は `relay_pump` を「cfg なしで定義し、mac の `cargo test` で検証可能にする」と指定している。呼び出し元は `#[cfg(windows)] spawn_afunix_relay` と `#[cfg(test)]` のテストだけなので、mac の通常ビルド（`cargo clippy` はテストを含まない）では未使用の private 関数になり `dead_code` が出る。T002 / T003 の完了条件と AC-1 が要求する `-D warnings` はこれで落ちる。

`#[cfg(any(windows, test))]` を付けるか、`#[cfg_attr(not(windows), allow(dead_code))]` とする方針を plan に書いておくと、実装者が完了条件で足止めされない。実害は数分だが、指定が明示的である以上そのまま従うと必ず踏む。

</details>

### [suggestion-2] ループバック許容リストが `::1` を落としており、`::` は中継先 `127.0.0.1` と噛み合わない — plan.md「T002 Steps 5」

<details>
<summary>詳細</summary>

`matches!(host.as_str(), "0.0.0.0" | "::" | "127.0.0.1" | "localhost")` には IPv6 ループバックの `::1` が入っていない。この設定では中継が立たず、warn だけ出て Remote Control が使えない。

逆に `::` は許容されているが、中継先は `127.0.0.1` 固定である。Windows の `IPV6_V6ONLY` 既定は有効で、Rust std / tokio はこれを明示的に落とさないため、`::` で bind した listener は IPv4 の接続を受けない。接続ごとに `TcpStream::connect` が失敗して warn を吐き続け、症状は「応答が返らない」になる。切り分けは AC-4 の FAIL と見分けにくい。

既定値は `127.0.0.1`（`src/config/mod.rs:284`）なので、どちらも非既定設定でのみ現れる。中継先をバインドアドレスの family に合わせる（`::` / `::1` なら `::1` へ繋ぐ）か、許容リストを `0.0.0.0` / `127.0.0.1` / `localhost` に絞って `::` 系は warn 側に倒すのが素直である。

</details>

### [suggestion-3] 終端伝播テストが `shutdown_to` の発火を観測せずに通りうる — plan.md「T003 Steps 4(b)」

<details>
<summary>詳細</summary>

`relay_pump` は `to` を所有したままスレッドに移すため、コピー終了時に `to` が drop される。テストで `to` に単独のソケットを渡すと、`shutdown_to` が空でも drop だけで反対側は EOF を観測する。つまり (b) は `shutdown_to` を検証しないまま緑になりうる。

実運用の Windows 経路では `to` は `try_clone()` した一方のハンドルで、drop しても複製が残るため EOF にならない。そこで `shutdown_to` が効いている。テストも同じ構図（`try_clone` した相方を保持したまま）にするか、`shutdown_to` の中でフラグやチャネルを立てて発火自体を assert する形にすると、検証したい性質と一致する。

</details>

## 人間が最終判断すべき箇所

1. **suggestion 3件を plan に取り込んでから実装に入るか、Deviation Log で吸収するか。** いずれも実装フェーズで即座に露見する範囲であり、承認を保留する重さではない。取り込むなら T002 / T003 の該当ステップの1行修正で済む。
2. **`v0.2.8-rc.1` Release の扱い（未処理判断1）。** 動かない公開バイナリを注記付きで残す提案は履歴保存として筋が通るが、公開物を残す判断そのものは人間の領分である。
3. **AC-4 が第三者の実機検証に依存する構造。** plan 側でできる事前潰しは round1 / round2 で出し切った。以降に残るリスクは Windows 実機でしか観測できない領域（`uds_windows` の bind / accept の実挙動、Bun 側の connect）であり、FAIL 時の採取物の設計が唯一の緩和策になる。T004 Step 4 の採取物リストで足りるかを、検証者に渡す前に人間が一度確認する価値がある。
