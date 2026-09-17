---
type: requirements-review-result
slug: remote-control-mitm-proxy
round: 2
verdict: REVISE
blocking: 1
status: in-review
reviewed_at: 2026-09-16
reviewed_commit: c649535
reviewed_file: docs/specs/remote-control-mitm-proxy/requirements.md
reviewed_sha256: 6023d711e28f673702269e1e86066422ef0c7b50b2d1a82e9769dfa811191827
---

# レビュー結果: requirements remote-control-mitm-proxy round2

**REVISE** — blocking 1件（ほかに suggestion 2件、nit 1件）。
次のアクション: FR-017 の受入基準が FR-009 と矛盾する1点だけを直す。round1 の指摘 11 件はすべて解消を確認した。
最重要の根拠: 旧キー `remote_control = true` だけの config に対し、FR-009 は proxy 方式、FR-017 の受入基準は Gateway モードと、正反対の結果を要求している。

round2 は修正確認である。以下の指摘はいずれも round2 の修正で新たに入った箇所に限る。

## round1 指摘の対応確認

| 指摘 | 対応 | 判定 |
|---|---|---|
| B1 プロファイル特定 | FR-015・US4・受入基準2件・Non-Goal 追加 | 解消 |
| B2 推論エンドポイント定義 | FR-006 に列挙、FR-016 で未知パスの既定を規定、背景に観測表 | 解消 |
| B3 listen アドレス | FR-001 をループバック固定、`proxy_host = "0.0.0.0"` の受入基準追加 | 解消 |
| B4 ANTHROPIC_MODEL | FR-008 の渡す側に追加、`default_model` 解決の受入基準追加 | 解消 |
| S1〜S5、N1〜N2 | Edge Cases 書き換え、FR-017〜FR-019 追加、互換性の空振り行を削除、FR-014 の主語を明確化 | 解消 |

## blocking（1件）

### `B1 [重大] 旧キーのみの config に対し FR-009 と FR-017 の受入基準が正反対の結果を要求する — FR-009 / FR-017`

<details>
<summary>詳細</summary>

FR-017 の2つ目の受入基準は「`remote_control_mode` を書いていない config がある / When `claudex run` する / Then forward proxy を使わず従来の Gateway モードで起動する」と書く。この Given は、FR-009 が扱う「`remote_control = true` だけを設定した config」を包含する。FR-009 は同じ入力に対し「警告を出した上で proxy 方式として扱う」と要求する。

実装はどちらか一方しか満たせない。FR-017 側に倒すと US2（既存設定のまま新方式へ移る）が成立せず、互換性の非機能要件「旧キー `remote_control` を警告付きで受理する」とも食い違う。

round2 の修正意図（値域と既定値の明文化）からは、FR-017 が旧キーの経路を除外し損ねた書き落としと読める。FR-017 の受入基準を「`remote_control_mode` も旧キー `remote_control` も無い config」に限定すれば解消する。
</details>

## suggestion（2件、verdict に影響しない）

### `S1 [中] 孫プロセスの帰結が、セッション特定の自然な実装と両立しない恐れがある — Edge Cases / FR-015`

<details>
<summary>詳細</summary>

Edge Cases は「Node の孫は成立するが、セッションを特定できないため FR-015 により本物へ中継される」と断定する。この断定が成り立つのは、リクエストの発信元をセッションのプロセスそのものまで絞り込める担体を選んだ場合に限る。

担体をセッションごとのポート分離にすると、孫は `HTTPS_PROXY` を継承するため同じポートへ来る。接続元 PID からプロセス木をたどる方式でも、孫はセッションの子孫として解決される。どちらの場合も孫のリクエストはプロファイルのプロバイダへ向かい、Edge Cases の記述と逆になる。

要件として「孫は本物へ」を維持するなら設計はその担体を選ばされる。維持しないなら、この行を「担体の選択に依存する」と弱めるのが正確である。設計フェーズの最初に確認したい。
</details>

### `S2 [中] FR-006 に加えた count_tokens と /v1/complete の期待結果が未定義である — FR-006`

<details>
<summary>詳細</summary>

FR-006 は `/v1/messages/count_tokens` と `/v1/complete` を推論側に倒し、受入基準は「本物の `api.anthropic.com` へは送られない」ことだけを見る。OpenAI 互換プロバイダにはどちらにも対応する endpoint が無いため、翻訳して転送するという要件をそのまま満たせない。何を返すか（ローカル概算、固定の失敗応答、DirectAnthropic のみ転送）が決まっていない。

背景の観測表にこの 2 つは現れておらず、実際に呼ばれる頻度は低い。そのため blocking にはしないが、設計で「到達したらどう応じるか」を決めないと実装が発明することになる。
</details>

## nit（1件）

<details>
<summary>nit 1件</summary>

- `N1 [軽] FR-002 の2つ目の受入基準は要件本文を検証していない — FR-002。「2 回のハンドシェイクで同一のリーフ証明書が提示される」は証明書の安定性を見るだけで、「秘密鍵をメモリ上にのみ保持する」の検証にはならない。1つ目の受入基準（実行時ディレクトリに秘密鍵ファイルが存在しない）で足りている`
</details>

## 人間が最終判断すべき箇所

1. **ラウンド規律の扱い**。上流レビューの最大ラウンドは 2 であり、今回がその最終ラウンドである。blocking は FR-017 の受入基準1行に閉じるため、round3 を1回認めて修正差分だけを確認するか、依頼者の自己修正のまま設計フェーズへ進めるかを決めてほしい。レビュアーとしては前者を推す。差分が1行なら確認は数分で終わり、要件の承認記録が残るからである。
2. **孫プロセスの扱い**（S1 に直結）。MCP サーバやフックが `api.anthropic.com` を直接叩く構成を「プロファイルのプロバイダへ向ける」か「本物へ逃がす」かは、claudex の責任範囲をどこで切るかというビジネス判断である。前者は孫のトークン消費がプロファイルの請求に乗り、後者は孫の通信が利用者の claude.ai アカウントに乗る。
3. **旧キー利用者を自動で TLS 終端へ移すこと**（B1 の背後にある前提）。FR-009 は `remote_control = true` のままの利用者を、設定変更なしに私設 CA による TLS 終端の対象にする。FR-018 の起動時告知で足りるとするか、明示的な設定変更を求めるかは製品判断である。
