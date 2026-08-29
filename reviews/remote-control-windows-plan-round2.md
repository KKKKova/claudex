---
type: plan-review
slug: remote-control-windows
round: 2
commit: working-tree（docs/ が gitignore のため plan は非追跡）
targets:
  - docs/specs/remote-control-windows/plan.md
acceptance: plan.md の「差分spec」節（受入基準 AC-1〜AC-5）
---

前回指摘への対応一覧（round1: reviews/remote-control-windows-plan-round1-result.md）:

- suggestion-1（T001 step 4 の挿入位置が両義的）→ 修正: 「L124-125 直後に spawn、L129-132 直後に abort」の2箇所指定に書き直し、`app` のムーブより前に spawn を置く旨を明記。
- suggestion-2（初回 instance の遅延生成で名前衝突が無限リトライになる）→ 修正: 初回 instance は `spawn_pipe_listener` 内で eager に `create` し、失敗は unix 版と同じく1回 warn + `None` の fail-fast に変更。accept() のリトライは2本目以降の一時的失敗のみと明記。
- suggestion-3（スコープ宣言と hyper-util 条件付き追加の矛盾）→ 修正: T001 step 5 の代替実装を削除し「コンパイル不能なら ESCALATE」に変更。
- suggestion-4（テストの Windows コンパイルは AC-2 で検証されない）→ 修正: T002 step 4 の目的を「mac/Linux 側の記述を素直にする」に書き直し、完了条件から Windows テストコンパイル証明の含意を除去。
- suggestion-5（is_proxy_running 常時 false による検証ノイズ）→ 対応方針変更: 手順書注記ではなく実装で解消する。人間判断3をユーザーに諮った結果「他 OS と挙動を合わせたい」との決定により、新タスク T003（`is_proxy_running` / `stop_proxy` の Windows 実装、`windows-sys` を Windows ターゲット限定依存として追加）をスコープに追加。受入基準 AC-5 を追加。
- suggestion-6（RC が Latest release になる）→ 修正: T004 step 2 に `gh release edit --prerelease`（ユーザー実行）を追加。

人間判断の結果（round1「人間が最終判断すべき箇所」）:
1. パイプのセキュリティを Windows 既定 DACL に委ねる判断 → ユーザー承認（2026-08-29）。スコープ外節に記録。
2. 未実行コードの RC 公開と別担当検証 → ユーザー承認。引き継ぎは手順書ファイルではなく fork issue 投稿に変更（T004）。
3. is_proxy_running の扱い → 実装する（上記 T003 追加）。

レビュー範囲の依頼: round1 指摘への対応確認に加え、**新規追加の T003 と AC-5 は round1 で未審査のため新規観点の指摘対象**として扱ってほしい。
