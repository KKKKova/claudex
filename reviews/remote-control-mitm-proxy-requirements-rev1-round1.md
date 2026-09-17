---
type: requirements-review
slug: remote-control-mitm-proxy
rev: 1
round: 1
commit: working-tree（docs/ が gitignore のため requirements.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\requirements.md
acceptance: 本ファイルの「改訂範囲」の節（承認済み requirements.md の FR-013 に関する差分のみ）
---

## 改訂の発生源

- 起因した指摘: `reviews/remote-control-mitm-proxy-plan-round1-result.md` の B1
- 人間裁定: 2026-09-17。選択肢「シェルのプロキシ設定を一切使わない」と「自分自身を指すときだけ無視する」を提示し、後者が選ばれた

## 改訂範囲

`docs/specs/remote-control-mitm-proxy/requirements.md` の3箇所のみ。

1. 機能要件表の FR-013 の要件文
2. 受入基準 `### FR-013` の項（1項目 → 2項目）
3. Edge Cases への1項目追加（親シェルのプロキシ設定が claudex 自身を指す場合）

改訂前後の差分は `git diff` で取れない（非追跡のため）。改訂前の FR-013 は「システムは常に、上流への接続で環境変数 `HTTPS_PROXY` を参照しない」、受入基準は「Given 親シェルに `HTTPS_PROXY` が設定されている / When proxy が上流へ接続する / Then 自分自身へ接続せず、直接 `api.anthropic.com` へ出る」の1項目であった。

FR-013 以外の FR、Success Criteria、非機能要件、ユーザーストーリー、Non-Goals には触れていない。
