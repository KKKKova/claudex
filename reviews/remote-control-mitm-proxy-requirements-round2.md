---
type: requirements-review
slug: remote-control-mitm-proxy
round: 2
commit: working-tree（docs/ が gitignore のため requirements.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\requirements.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md の「受入基準（Given-When-Then）」節（FR-001〜FR-019）
---

前回指摘への対応一覧:

- B1 → FR-015 を追加し、同時起動時の振り分けを要件化した。US4 と受入基準 2 件を追加。担体（ヘッダ・ポート分離・セッション登録）は設計の領分として要件に書いていない
- B2 → FR-006 に推論とみなすパスを列挙した（`/v1/messages`、`/v1/messages/count_tokens`、`/v1/complete` および beta 相当）。列挙外の未知パスは FR-016 で本物へ中継し警告ログに記録する。未知パスの既定方向は人間が決定（本物へ素通し＋警告ログ）。`count_tokens` は推論側に倒し、本物へ送らないことを受入基準に追加
- B3 → FR-001 をループバック固定に改訂し、`proxy_host = "0.0.0.0"` 設定時の受入基準を追加。非機能要件（セキュリティ）にも待ち受け範囲の記述を追加
- B4 → FR-008 の渡す側に `ANTHROPIC_MODEL` を追加し、`default_model` 解決の受入基準を追加
- S1 → Edge Cases の孫プロセスの項を `api.anthropic.com` へ出る場合に書き換え、Node 系と非 Node 系それぞれの帰結を記述。FR-015 の受入基準にも該当ケースを追加
- S2 → proxy 再起動で CA が入れ替わる件を Edge Cases に追加
- S3 → US3 の独立テスト方法と FR-002 の受入基準を「実行時ディレクトリに秘密鍵ファイルが存在しない」へ範囲を狭めた
- S4 → FR-017（値域と既定値）と FR-018（TLS 終端の告知）を追加
- S5 → 非機能要件（互換性）から「旧ソケット方式の設定値は受理しない」の行を削除
- N1 → FR-019（ポート専有時の起動失敗）を追加。ポート番号の決め方そのものは設計の領分として要件に書いていない
- N2 → FR-014 の「Claude subscription プロファイルを含む場合」を「起動対象のプロファイルが Claude subscription のとき」に改めた

背景節に PoC で観測した 9 種のパスの表を追加し、FR-006・FR-007・FR-016 の振り分けがこの表を土台にすることを明示した。

人間判断の 3 点について: 未知パスの既定は「本物へ素通し＋警告ログ」で決定済み。方式そのものの採否と旧ソケット方式の即時削除は、round1 以前に依頼者が決定済みである。
