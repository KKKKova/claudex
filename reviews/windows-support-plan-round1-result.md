---
type: plan-review-result
slug: windows-support
round: 1
verdict: REVISE
blocking: 1
reviewed: docs/specs/windows-support/plan.md @ working-tree（2026-08-28、リポジトリ /docs/ local-only 運用のため未コミット版）
---

# plan-review: windows-support round1

**verdict: REVISE**（blocking 1件 / suggestion 0件 / nit 2件）

事実主張は全件を機械検証した。衝突3ファイル（`config.example.toml` / `src/config/mod.rs` / `src/oauth/source.rs`）は一時 worktree での実マージで再現し、いずれも「両側追記」型であることをハンク単位で確認。keyring の `windows-native` feature、PR 差分規模（329 insertions）、main との距離（11コミット）、`#[cfg(unix)]` ゲート（pty / proxy UnixListener / daemon / launch）、version 0.2.5→0.2.6、release.yml の現行 matrix と `tar czf` 固定のパッケージ手順も plan の記述どおり。

## 指摘（重大度降順）

### [required] T002: fix/windows-support の push では fork CI が走らず、「緑まで反復」ループが成立しない

<details>
<summary>ci.yml のトリガーは `push: branches: [main]` と `pull_request: branches: [main]` のみ。T002 手順4「push して fork CI の Windows check を観察」はトリガー条件を満たさない。</summary>

- 現行 `ci.yml:3-7` は main への push / main 宛 PR のみでしか起動しない。T002 はブランチ `fix/windows-support` 上で作業し fork へ push する計画だが、この push では追加した Windows check ジョブは一度も走らない。
- T002 の完了条件「fork CI で Windows check ジョブ成功（AC-2）」への到達経路が plan 記載の手順に存在しない。実装エージェントは「CI が走らない」状態で手順4に立ち往生するか、無断で運用を変える（Deviation）ことになる。
- なお T001 は既存 PR#1（main 宛）への push のため `pull_request` トリガーで走る。問題は T002 のみ。
- **対処案**（いずれか1つを plan に明記すれば解消）: (a) T002 開始時に fix/windows-support → fork main の PR を開き、pull_request トリガーで反復する（手順5「PR 経由でも可」との整合もよい）。 (b) ci.yml に `workflow_dispatch` または対象ブランチの push トリガーを追加する。
</details>

### [nit] T002 手順2: release.yml の Linux 専用ステップは既にガード済み

<details>
<summary>「musl ツール install ステップなど Linux 専用ステップに `if: runner.os == 'Linux'` 相当のガードを付ける」とあるが、現行 release.yml の "Install system dependencies (Linux)" は既に `if: runner.os == 'Linux'` 付き、cross install も `if: matrix.cross` でガード済み。</summary>

実装時に「付けるべきガードが見つからない」混乱を招くだけで実害はない。記述を「既存ガードが Windows を除外することを確認する」に直すのが正確。Package/Upload ステップに Windows 分岐（.exe / .zip）が必要という認識は正しい。
</details>

### [nit] 差分spec の「launch.rs」の実パスは `src/process/launch.rs`

<details>
<summary>CLAUDE.md の構造図（`src/launch.rs`）が旧構成のまま。ゲート自体は `src/process/launch.rs` に `#[cfg(unix)]` 10箇所で確認済みで、主張内容は正しい。</summary>

plan 側はパス表記のみ。ついでに CLAUDE.md の構造図更新を検討してもよい（本レビューの合否には無関係）。
</details>

## 観点別サマリ

- 完全性: 指摘なし（AC-1〜5 すべてに検証手段あり、スコープ外が明記され、Deviation ルールも定義済み）
- 一貫性: nit 2件（上記）。AC ⇄ タスク ⇄ 検証計画の対応は完全
- 実現可能性: required 1件（CI トリガー）。それ以外の手順（`pull/1/head` refspec、windows-latest での msvc check、`claudex-${TAG}-<target>.zip` 命名、0.2.6 bump）は検証済みで成立する
- 過剰設計: 指摘なし（キャッシュを初回から足さない、aarch64-windows を需要待ちにする等、むしろ抑制が効いている）

## 人間が最終判断すべき箇所

1. **未実機検証の Windows バイナリを公開リリースに載せる順序**: release.yml は draft 作成後に自動 publish する。AC-5 のとおり実機検証は公開後に別担当が行う計画であり、「動くか未確認の .exe を配る」期間が生じる。fork の利用者が実質自分たちだけなら問題ないが、これは運用判断。
2. **fork/main への取り込み履歴方針**: T001 は `--merge`（マージコミット）、T002 は「PR 経由でもローカルマージ+push でも可」と幅がある。履歴の一貫性をどこまで求めるかは運用者の好み。
