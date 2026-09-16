---
type: requirements-review-result
slug: remote-control-mitm-proxy
round: 3
verdict: APPROVE
blocking: 0
status: approved
reviewed_at: 2026-09-16
reviewed_commit: 63447b7
reviewed_file: docs/specs/remote-control-mitm-proxy/requirements.md
reviewed_sha256: 6bb2746ee0d0012c41b4d45032a1c4ddf346422775993739a6441c936abd8a69
---

# レビュー結果: requirements remote-control-mitm-proxy round3

**APPROVE** — blocking 0件（nit 1件、verdict に影響しない）。
次のアクション: この要件で基本設計へ進んでよい。requirements.md の `status` を `approved` に、`approved` 欄に日付を入れるのは依頼者側の作業である。
最重要の根拠: round2 の blocking（FR-009 と FR-017 の矛盾）は FR-017 の Given を「`remote_control_mode` も旧キー `remote_control` も書いていない config」に狭めたことで解消し、両者の入力集合が重ならなくなった。

round3 は修正差分に限定して確認した。差分は FR-015・FR-017・FR-018・FR-002 の受入基準・Edge Cases の 5 箇所であり、いずれも指摘に対応する範囲に収まっている。

## round2 指摘の対応確認

| 指摘 | 対応 | 判定 |
|---|---|---|
| B1 FR-009 と FR-017 の矛盾 | FR-017 の 2 つ目の Given を旧キー未設定に限定 | 解消 |
| S1 子孫プロセスの帰結 | FR-015 本文を「セッションのプロセスまたはその子孫」に改訂、受入基準1件追加、Edge Cases を請求先の記述へ書き換え | 解消 |
| S2 count_tokens / complete | Edge Cases に応答の作り方を設計で決める旨を追加 | 解消 |
| N1 FR-002 の受入基準 | 該当の 1 件を削除 | 解消 |

S1 の対応で FR-015 の担体は「子孫まで解決できる方式」に絞られた。セッションごとのポート分離でも接続元プロセスの木をたどる方式でも満たせるため、要件として実現可能である。

## nit（1件）

<details>
<summary>nit 1件</summary>

- `N1 [軽] FR-018 に足した旧キー経路の告知に対応する受入基準が無い — FR-018。本文は「旧キー `remote_control` による起動でも同様とする」と定めたが、受入基準は `remote_control_mode = "proxy"` の場合だけを見る。FR-009 の受入基準に告知の確認を1つ足せば埋まる`
</details>

## 人間が最終判断すべき箇所

1. **FR-015 の担体の選び方**（設計フェーズの最初の分岐）。子孫プロセスまで同じプロファイルへ寄せると決めたため、設計はセッションごとのポート分離か、接続元プロセスの木をたどる方式かを選ぶ。後者は Windows でのプロセス木の解決が必要になり、実装コストが要件からは見えない。
2. **`/v1/messages/count_tokens` と `/v1/complete` への応答の作り方**。ローカル概算・失敗応答・DirectAnthropic のみ転送のどれを選ぶかで、Claude Code 側の文脈管理の挙動が変わる。設計で決めると明記されたが、選択そのものは利用者の体験に触れる判断である。
3. **旧キー利用者を設定変更なしで TLS 終端へ移す方針**（決定済み、記録として残す）。`remote_control = true` のままの利用者は、claudex を更新した時点から私設 CA による終端の対象になる。担保は FR-018 の起動時告知だけであり、その文面が意図を伝えられるかは人間が読んで確かめてほしい。
