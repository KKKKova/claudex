---
type: plan-approval
slug: remote-control-mitm-proxy
approved_at: 2026-09-17
approved_by: human
approval: human
plan: docs/specs/remote-control-mitm-proxy/plan.md
review_result: reviews/remote-control-mitm-proxy-plan-round2-result.md
---

# 承認記録: plan remote-control-mitm-proxy

plan-review round2 が APPROVE（blocking 0件）。人間が plan.md を承認し、`status: approved` に更新した。

規模はタスク10個・見積変更 約1700行。編成は7グループで並列は最大2、担当は `opus-xhigh` 1件（`Proxy-Authorization` の解析とプロファイル解決）、`sonnet-xhigh` 3件、`sonnet-high` 1件、`sonnet-medium` 5件。

## 承認の場で下した判断

**素通しトンネルの会社プロキシへのチェーンは対象外とする。** `api.anthropic.com` 以外への CONNECT は宛先へ直接 TCP を張るため、社外へ直接出られないネットワークでは Claude Code の他の通信が失敗する。FR-013 の改訂で上流接続は他のプロキシを尊重するようになったが、それは自己ループを避ける判定の副産物であり、会社プロキシ配下での全面的な動作を約束するものではない。plan の「スコープ外」に明記した。

この論点は requirements rev1 の S1、design rev1 の S2、plan round2 の人間判断項目2 で、3人のレビュアーが同じ箇所を指している。

## 改訂系列 rev1 について

plan round1 の指摘 B1（推論の上流接続が親シェルのプロキシ設定を読み、FR-013 を満たす経路が計画に無い）を受けて人間裁定を行い、FR-013 を「環境変数の指す先が claudex 自身の forward proxy のときだけ無視し、他は尊重する」に改訂した。承認済みの requirements.md と design.md を改訂したため、それぞれ rev1 系列でレビューを回し、両方とも APPROVE を得ている。

requirements rev1 の結果は当初 REVISE で発行され、design rev1 を読んだ後に APPROVE へ訂正されている（結果ファイル内に訂正の記録あり）。降格した指摘は「無視の対象が `HTTPS_PROXY` だけで小文字版から自己ループしうる」というもので、plan 側では `HTTPS_PROXY` / `https_proxy` / `ALL_PROXY` / `all_proxy` の4つを見る形にして塞いだ。

## 承認後に plan へ加えた修正

いずれも round2 の指摘と requirements rev1 の suggestion に対応するもので、レビュー範囲を超える変更ではない。

1. T003 Steps 8: 読むプロキシ環境変数を4つに広げた
2. T009 Steps 1: 統合テストのヘルパで `ForwardState::client` を `.no_proxy()` に組み直す（開発機のプロキシ設定を拾うと Passthrough 系テストが落ちる）
3. 検証計画 FR-013 第1: 大文字・小文字の両表記を検証する形にした
4. スコープ外: 素通しトンネルのチェーンを対象外と明記

次フェーズ: /implement
