---
type: plan-review-result
slug: remote-control-windows
round: 2
verdict: REVISE
blocking: 1
reviewed: docs/specs/remote-control-windows/plan.md @ working-tree
date: 2026-08-29
---

# レビュー結果: plan remote-control-windows round2

> **REVISE** — blocking 1件 / suggestion 2件 / nit 3件。
> 判断: round1 の suggestion 6件はすべて妥当に反映されている。差し戻しの理由は、新規追加した T003 が実装する `stop_proxy` の Windows 経路をどの受入基準も実行検証しない点、この1件のみである。
> 根拠: T003 は `TerminateProcess` によるプロセス強制終了という新しい破壊的経路を増やすが、AC-5 は `status` と二重起動しか見ておらず、AC-2 のコンパイル証明以外に確認手段がない。

## round1 指摘への対応確認

| round1 指摘 | 対応 | 判定 |
|---|---|---|
| S1 T001 step 4 の挿入位置が両義的 | 2箇所指定に分割、`app` ムーブより前に spawn する旨を明記 | 解消 |
| S2 初回 instance の遅延生成 | `spawn_pipe_listener` 内で `first_pipe_instance(true)` 付き eager 生成、失敗は1回 warn + `None` | 解消。`accept()` 側の2本目以降は `first_pipe_instance` なしで正しい |
| S3 スコープ宣言と hyper-util の矛盾 | 代替実装を削除し ESCALATE に変更。スコープの Cargo.toml 記述も `windows-sys` 追加に整合 | 解消 |
| S4 テストの Windows コンパイル証明 | step 4 の目的を書き直し、完了条件から含意を除去 | 解消 |
| S5 `is_proxy_running` 常時 false | T003 として実装をスコープ追加 | 対応。ただし T002 側が未追随（下記 suggestion 1） |
| S6 RC が Latest release になる | T004 step 2 に `gh release edit --prerelease` を追加、完了条件にも prerelease 表示を明記 | 解消 |

## 新規スコープ（T003 / AC-5）の照合結果

| 確認項目 | 結果 |
|---|---|
| `windows-sys` 0.59 の追加が依存ツリーを増やさないか | 増やさない。`Cargo.lock:4197` に 0.59.0 が既に存在（推移依存）。feature は加算的なので `Win32_Foundation` / `Win32_System_Threading` の有効化で足りる |
| API と feature の対応 | 正しい。`OpenProcess` / `TerminateProcess` / `PROCESS_QUERY_LIMITED_INFORMATION` / `PROCESS_TERMINATE` が `Win32_System_Threading`、`CloseHandle` / `GetLastError` / `ERROR_ACCESS_DENIED` が `Win32_Foundation` |
| 参照行番号 L71-87 / L89-108 | fc13316 の `is_proxy_running` / `stop_proxy` の実位置と一致 |
| 強制終了で pid ファイルが残らないか | 残らない。`stop_proxy` は分岐の外で `remove_pid()?` を無条件に呼ぶ（daemon.rs L104 相当） |
| cfg の網羅性 | step 3 の `#[cfg(not(any(unix, windows)))]` で閉じている |

## 指摘（重大度降順）

### [required] T003 が実装する `stop_proxy` の Windows 経路を、どの受入基準も実行検証しない — plan.md「AC-5」/「T004 Steps 4」

<details>

T003 step 2 は `OpenProcess(PROCESS_TERMINATE) → TerminateProcess` という新しい破壊的経路を追加する。これに対して AC-5 が見るのは `claudex proxy status` と二重の `claudex run` だけで、`claudex proxy stop` は含まれない。T004 step 4 の issue 本文の項目にも `proxy start` → `run` しかなく stop がない。検証計画表の AC-5 行も「status / 二重起動」と明記している。結果として、この経路の確認手段は AC-2 のコンパイル証明のみになる。

**反証の結果**: 「AC-4 と同じ issue に含めるのだから暗黙に入る」は成立しない（issue 本文の項目列挙に stop がない）。「pid 再利用で無関係プロセスを殺す事故は稀」は成立するが、指摘の中心は事故確率ではなく、実装した機能に対応する受入基準が欠けていること（観点1 完全性）であり、これは確率ではなく plan の記述として確定している。

なお `TerminateProcess` は unix の `SIGTERM` と違い捕捉も無視もできない即時終了である。pid 再利用時の被害範囲は unix 側より大きく、「動かして初めて分かる」種類の実装であるため、コンパイル証明だけで RC に載せて第三者へ渡す状態は避けたい。

**修正案**（いずれも1行で足りる）: AC-5 の Then に「`claudex proxy stop` で実際に停止し、以後 `status` が not running を返す」を加え、T004 step 4 の issue 項目に stop の手順と期待出力を1行足す。

</details>

### [suggestion] T002 step 2 が `read_pid()?.is_some()` のままで、T003 が用意する実在確認を使っていない — plan.md「T002 Steps 2」

<details>

round2 で `is_proxy_running()` が Windows でも実プロセス確認になるのに、T002 step 2 の Windows 分岐は pid ファイルの存在だけを見る `read_pid()?.is_some()` のまま残っている。同じ問いに2つの答えが並ぶ状態で、弱いほうが Remote Control の起動可否判定に使われる。

さらに、unix 側が `socket.exists()` で得ていた「プロキシは生きているがリスナーが立っていない」の検知が Windows では失われる。`spawn_pipe_listener` が `None` を返した場合（初回 `create` 失敗）、プロセスは TCP で生きているため `read_pid()` も `is_proxy_running()` も true を返し、失敗は Claude Code 内部の分かりにくいエラーとして現れる。これは AC-4 の検証者が最初に踏みやすい経路である。

**修正案**: 最低限 `is_proxy_running()?` に置き換えて判定を一本化する。unix と同じ水準まで揃えるなら、T003 で `windows-sys` を入れるついでに `WaitNamedPipeW(pipe_name, 0)` でパイプ実在を確認する（`Win32_System_Pipes` feature の追加が必要）。

</details>

### [suggestion] AC-5 の「run が二重起動を試みない」に観測可能な合否シグナルがない — plan.md「AC-5」

<details>

二重起動の抑止は `main.rs:75` の分岐であり、外から見える差は `tracing::info!("proxy not running, starting in background...")` が出るかどうかだけである。既定のログレベルと出力先次第では検証者に見えない。加えて、抑止が効かなかった場合でも in-process プロキシは TCP bind に失敗して静かに終わり、`/health` は既存プロキシが答えるため、表面上は成功と区別がつかない。

**修正案**: AC-5 の Then に観測手段を明記する。例えば「proxy ログに `proxy listening on` が2度出ない」「`RUST_LOG=info` で `starting in background` が出ない」のいずれかを合否条件として issue に書く。

</details>

<details>
<summary>nit 3件（verdictに影響しない）</summary>

- **差分spec の `Addr` 境界が未修正** — 「事前確認済みの事実」節は `Addr: Clone + Debug + Send` のままだが、T001 step 2 は正しい `Addr: Send` に直っている。同一文書内で食い違うので、事実節も揃えたい。
- **`OpenProcess` は終了済みプロセスに成功しうる** — 他プロセスがハンドルを保持している間は pid 解決が成功し、終了済みでも「生存」と判定される。`GetExitCodeProcess` の結果が `STILL_ACTIVE` かを併せて見ると確実になる（`OpenProcess` に `PROCESS_QUERY_LIMITED_INFORMATION` があれば呼べる）。plan が許容を明言した pid 再利用とは別の経路なので、許容するなら step 1 に一言添えるとよい。
- **強制終了の非対称を doc コメントに残す** — unix は SIGTERM で `remove_pid()` まで含めた正常終了、Windows は `TerminateProcess` で in-flight リクエストごと即断される。`stop_proxy` の Windows 分岐に「なぜ graceful にできないか」を1行残すと、後から SIGTERM 相当を探す人の手間が減る。

</details>

## 指摘なしの観点

- 一貫性: スコープの変更ファイル一覧と T001〜T004 の Files が過不足なく対応し、round1 で指摘した Cargo.toml の矛盾も解消している。指摘なし。
- 過剰設計: T003 の追加はユーザーの明示判断（「他 OS と挙動を合わせたい」）に基づくもので、投機的拡張ではない。タスク4個・約280行は妥当。

## 人間が最終判断すべき箇所

1. **`TerminateProcess` と pid 再利用の組み合わせを許容するか**。plan は「unix の `kill(pid, 0)` と同等の限界」として許容しているが、誤爆したときの挙動は unix より重い（捕捉・無視ができない即時終了）。許容する判断そのものは妥当だが、明示的に受け入れてほしい。
2. **T003 を RC に同梱するか、AC-4 の後に回すか**。RC を出す目的は Windows 実機の結果を早く得ることであり、T003 は検証体験の改善であって Remote Control の成立条件ではない。同梱すれば RC の変更面が広がり、AC-4 が失敗したときの切り分け対象が増える。
3. **引き継ぎ成果物が fork issue になり、リポジトリに手順が残らなくなった点**。`docs/` は gitignore なので元々追跡外だったが、issue に移すと plan の完了証跡が外部サービス側だけに残る。追跡性の観点で許容するかを決めてほしい。
