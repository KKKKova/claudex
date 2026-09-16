---
type: requirements-review
slug: remote-control-mitm-proxy
round: 3
commit: working-tree（docs/ が gitignore のため requirements.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\requirements.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md の「受入基準（Given-When-Then）」節（FR-001〜FR-019）
---

依頼者が round3 を 1 回認めた。レビュー範囲は下記の修正差分に限る。

前回指摘への対応一覧:

- B1 → FR-017 の 2 つ目の受入基準の Given を「`remote_control_mode` も旧キー `remote_control` も書いていない config」に限定した。FR-009 との重なりを解消
- S1 → 孫プロセスの扱いを人間が決定（プロファイルのプロバイダへ向ける）。FR-015 の本文を「そのリクエストを発したセッションのプロセスまたはその子孫の起動プロファイルへ」に改め、受入基準に子孫プロセスの 1 件を追加。Edge Cases の該当項も、子孫のトークン消費がプロファイルの請求に乗る旨へ書き換えた
- S2 → `/v1/messages/count_tokens` と `/v1/complete` が届いた場合の応答の作り方を設計で決める旨を Edge Cases に追加した。本物へ送らないことは FR-006 で確定のまま
- N1 → FR-002 の 2 つ目の受入基準（2 回のハンドシェイクで同一のリーフ証明書）を削除した

あわせて、人間判断により旧キー利用者を設定変更なしで proxy 方式へ移す方針を維持し、FR-018 の告知が旧キー経路でも出ることを本文に明示した。
