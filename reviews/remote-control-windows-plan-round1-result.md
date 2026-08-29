---
type: plan-review-result
slug: remote-control-windows
round: 1
verdict: APPROVE
blocking: 0
reviewed: docs/specs/remote-control-windows/plan.md @ working-tree
date: 2026-08-29
---

# レビュー結果: plan remote-control-windows round1

> **APPROVE** — blocking 0件 / suggestion 6件。
> 判断: この計画で実装に進んでよい。suggestion は着手前の文言修正で足りる。
> 根拠: 4観点で正しさ・明示要件との齟齬に当たる欠陥がない。plan が前提として挙げた技術的事実（axum 0.8.8 の `Listener` トレイト、tokio の named_pipe、fork CI の `windows-check`、release.yml のタグ条件）は独立に照合して全て成立を確認した。

## 検証した前提（機械的照合）

| plan の主張 | 照合結果 |
|---|---|
| axum 0.8 の `serve::Listener` は外部実装可能 | 成立。`axum-0.8.8/src/serve/listener.rs:9`。ただし関連型の境界は `Io: AsyncRead + AsyncWrite + Unpin + Send + 'static` / **`Addr: Send`** のみで、plan の「`Addr: Clone + Debug + Send`」は過大。`String` は満たすので実装には影響しない |
| `accept()` は Result を返さない | 成立。`fn accept(&mut self) -> impl Future<Output = (Self::Io, Self::Addr)>`。トレイトの doc が「エラーはログして自前でリトライせよ」と明記しており、plan の retry 設計はトレイトの想定どおり |
| tokio の named_pipe が追加依存なしで使える | 成立。`Cargo.toml:17` が `features = ["full"]` |
| fork CI に Windows check がある | 成立。`.github/workflows/ci.yml` の `windows-check` が `cargo check --target x86_64-pc-windows-msvc` を実行 |
| release.yml はタグ `v*` で全アーティファクトを出す | 成立。`v0.2.8-rc.1` は `v*` に一致し、`publish-release` が draft を解除する |
| 参照行番号（launch.rs L160/196/204/229/257/369、proxy/mod.rs L124-132/L145-177、daemon.rs L30） | 全て fc13316 の実際の位置と一致 |

## 指摘（重大度降順）

### [suggestion] T001 step 4 の挿入位置指示が両義的で、literal に読むとコンパイルエラーになる — plan.md「T001 Steps 4」

<details>

`run()` の unix 側は L124-125（spawn）と L129-132（abort）の2ブロックが `let result = axum::serve(listener, app).await;`（L127）を挟む形になっている。step 4 の「L124-132 の `#[cfg(unix)]` ブロックの直後に、同じ形の `#[cfg(windows)]` ブロック（`spawn_pipe_listener` 呼び出しと abort）を追加」は、(a) 2ブロックを unix と同じく serve を挟んで置く、(b) spawn と abort をまとめて L132 の後に置く、の2通りに読める。(b) を選ぶと `app` は L127 で `axum::serve` にムーブ済みのため use-after-move となり、AC-2 の `windows-check` で落ちる。

**反証の結果**: 「実装者は不整合に気づく」「気づかなくても CI が捕まえる」は成立する。破壊的な silent break にはならないため blocking にはしない。ただし CI 1往復を無駄にするので、「L124-125 の直後に spawn、L129-132 の直後に abort」と2箇所に分けて書くほうがよい。

</details>

### [suggestion] pipe instance を accept() 内で遅延生成すると、名前衝突の検知が起動時に出ず 500ms 無限リトライになる — plan.md「T001 Steps 2-3」

<details>

step 2 は最初の instance を `accept()` の内側で `first_pipe_instance(true)` 付きで作る設計になっている。この場合、既に他プロセスが同名パイプを握っているという致命的な状況が、`spawn_pipe_listener` の戻り値（`None`）ではなく spawn 済みタスク内の warn ログとして現れ、500ms 間隔で永久に繰り返される。unix 版は `UnixListener::bind` 失敗を1回 warn して `None` を返す fail-fast なので、非対称になる。

修正案: 最初の instance を `spawn_pipe_listener` 内で `first_pipe_instance(true)` を付けて eager に作り、`pending` に入れてから `NamedPipeListener` を組む。作成失敗はそこで `None` を返せば unix と同じ形になり、`accept()` 側のリトライは2本目以降の一時的失敗だけを扱えばよくなる。

</details>

### [suggestion] スコープ宣言「依存クレート追加はしない」と T001 step 5 の「hyper-util を明示追加してよい」が矛盾する — plan.md「スコープ」/「T001 Steps 5」

<details>

スコープ節は `Cargo.toml` の変更を「RC 版 version bump のみ。依存クレート追加はしない」と宣言しているが、T001 step 5 は条件付きで `hyper-util` の明示追加を許可している。実装エージェントにとってスコープ宣言は「これ以外は触らない」の拘束であり、同一文書内で拘束と例外が矛盾している状態は避けたい。

加えて step 5 の前提自体が不要である可能性が高い。上表のとおり `Listener` の境界は `Io: AsyncRead + AsyncWrite + Unpin + Send + 'static` と `Addr: Send` だけで、`NamedPipeServer` はこれを満たす。step 5 を削り、万一コンパイルが通らなければ Deviation として ESCALATE する扱いにすれば、スコープ宣言と矛盾しない。

</details>

### [suggestion] T002 step 4 の「テストを Windows コンパイル対象に含める」は AC-2 では検証されない — plan.md「T002 Steps 4」/「完了条件」

<details>

`cargo check --target x86_64-pc-windows-msvc`（`--tests` / `--all-targets` なし）は `#[cfg(test)]` 配下をコンパイルしない。したがって `test_remote_control_env` 等から `#[cfg(unix)]` を外しても、Windows で実際に通るかは CI で証明されない。証明したいなら CI の check に `--all-targets` を足す（release.yml ではなく ci.yml の変更でスコープ追加になる）か、step 4 の狙いを「Windows でのコンパイル証明」ではなく「mac/Linux 側の記述を素直にする」と書き直して完了条件から外すか、どちらかに寄せる。

</details>

### [suggestion] Windows では `claudex run` が毎回プロキシ起動を試み、T002 の生存判定と `proxy status` の答えが食い違う — plan.md「スコープ外」/「T002 Steps 2」

<details>

`is_proxy_running()` は non-unix で常に `false`（daemon.rs L79）なので、Windows の `claudex run` は `main.rs:75` で必ず `start_proxy_background` に入る。既に外部プロキシが動いていれば in-process 側は TCP bind に失敗してエラーログを吐き、`/health` は外部プロキシが答えるため処理自体は続行する。機能は壊れないが、実機検証者のログに毎回 bind 失敗が出る。

一方 T002 step 2 は Windows の生存判定に `read_pid()?.is_some()` を使う。これは pid ファイルの存在だけを見るので、`is_proxy_running()`（常に false）と答えが食い違い、クラッシュ後の stale pid でも通る。AC-4 は第三者が実施するため、この2点は「想定内の挙動」として T003 の手順書に明記しておかないと、検証レポートが不具合報告として返ってくる。

</details>

### [suggestion] `v0.2.8-rc.1` は prerelease 指定がないため fork の Latest release になる — plan.md「スコープ外」/「T003 Steps 2」

<details>

release.yml の `publish-release` は `gh release edit "$TAG" --draft=false` のみで `--prerelease` を付けない。RC タグを push すると `v0.2.8-rc.1` が fork の最新リリースとして表示され、`v0.2.7` を使っている側が誤って RC を取る余地が生まれる。plan は「正式版は実機検証の結果を見て別途判断」と明言しているので、その意図とリリース表示が食い違う。T003 step 2 の後に `gh release edit v0.2.8-rc.1 --prerelease`（ユーザー実行）を1手順足すのが最小の解。

</details>

## 指摘なしの観点

- 完全性: 全タスクに Files / Steps / 完了条件があり、AC-1〜AC-4 が検証計画表で1対1に対応している。指摘なし。
- 過剰設計: タスク3個・約220行は変更規模に見合う。step 5 の保険（上記 suggestion）以外に投機的な抽象・将来対応はない。

## 人間が最終判断すべき箇所

1. **名前付きパイプを Windows 既定のセキュリティ記述子に委ねる判断**。`reject_remote_clients(true)` でリモートは拒否されるが、同一マシンの他ユーザーに対する保護は既定 DACL 任せになる。プロキシは既に `127.0.0.1` の TCP を無認証で開いており露出の水準は同等（退行ではない）だが、「auth.json の ACL 未対応と同じく人間に委ねる」という判断を明示的に受け入れるかは人が決める領域である。
2. **一度も実行されていないコードを RC として公開する是非**。AC-1〜AC-3 は mac とコンパイルのみで、Windows のパイプ経路は AC-4 まで一切実行されない。plan はこれを認めた上で「実装自体を検証手段とする」と設計しており論理は通っているが、第三者・別環境に検証を委ねる前提でよいかは判断が要る。
3. **`is_proxy_running()` の Windows 未対応をスコープ外のまま実機検証に出す判断**（上記 suggestion 5の背景）。直せば検証ノイズが減るが plan の規模が増える。手順書での注記で足りるとするかどうか。
