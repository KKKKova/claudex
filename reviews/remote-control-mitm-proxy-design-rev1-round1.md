---
type: design-review
slug: remote-control-mitm-proxy
rev: 1
round: 1
commit: working-tree（docs/ が gitignore のため design.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\design.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md の FR-013（改訂系列 rev1 で同時改訂。reviews/remote-control-mitm-proxy-requirements-rev1-round1.md）
---

## 改訂の発生源

- 起因した指摘: `reviews/remote-control-mitm-proxy-plan-round1-result.md` の B1
- 人間裁定: 2026-09-17。上流接続でのプロキシ設定の扱いを「自分自身を指すときだけ無視する」に決定

## 改訂範囲

`docs/specs/remote-control-mitm-proxy/design.md` の2箇所のみ。

1. 「データモデル」節の `ForwardState` の表の `client` 行、およびその直後に追加した段落（上流へ出るクライアントが2つあり、FR-013 が両方にかかること）
2. 「トレーサビリティ」表の FR-013 行

改訂前の `client` 行は「`.no_proxy()` で構築する（FR-013）」、トレーサビリティの FR-013 行は「データモデル（`ForwardState::client` を `.no_proxy()` で構築）」であった。

改訂の要点は、`src/proxy/mod.rs` の既存 `ProxyState::http_client` も FR-013 の対象に含めた点である。推論の上流接続はこのクライアントを使うため、`ForwardState::client` だけでは FR-013 を満たせない。

アーキテクチャ、振り分け規則、利用者名の解決、資格情報と証明書のライフサイクル、子プロセスに渡す環境変数、API契約、Alternatives Considered には触れていない。
