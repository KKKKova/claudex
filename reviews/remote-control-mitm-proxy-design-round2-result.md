---
type: design-review-result
slug: remote-control-mitm-proxy
round: 2
verdict: APPROVE
blocking: 0
status: approved
reviewed_at: 2026-09-16
reviewed_commit: 07e70f1
reviewed_file: docs/specs/remote-control-mitm-proxy/design.md
reviewed_sha256: f1b34f8298d0f9761da6278750004c48a91d1a73e93238d6ba208f2aceba7627
---

# レビュー結果: design remote-control-mitm-proxy round2

**APPROVE** — blocking 0件、nit 0件。
次のアクション: この設計で /plan に進んでよい。design.md の `status` を `approved` にするのは依頼者側の作業である。
最重要の根拠: round1 の blocking 2件は、新設した「利用者名の解決」節と `Option<RemoteControlMode>` への変更で、いずれも判定が一意に決まる形になった。

round2 は修正差分に限定して確認した。差分は「利用者名の解決」節の新設、データモデルの優先規則の表、振り分け規則の注記、API契約の例、テスト戦略の「前提の証跡」行、リスク節の3箇所である。

## round1 指摘の対応確認

| 指摘 | 対応 | 判定 |
|---|---|---|
| B1 利用者名の引き継ぎ | 「利用者名の解決」節を新設。CONNECT は接続単位で1回解析し以後の全要求へ適用、絶対 URI 形式は要求単位と入口を分けた。integration テストにも2本目以降の要求への適用を追加 | 解消 |
| B2 未設定と明示 off の区別 | `Option<RemoteControlMode>` へ変更し、4通りの優先規則の表に置き換えた | 解消 |
| S1 count_tokens の転送先 | 振り分け規則に注記を追加し、Alternatives の文を「claude.ai 認証ヘッダを付けたまま本物へ中継する経路は使わない」へ | 解消 |
| S2 前提の証跡 | テスト戦略に「前提の証跡」行を追加し、PoC の更新を実装の最初に置いた | 解消 |
| S3 解決失敗の切り分け | 解決結果4通りの表を追加。資格情報が付いているのに解決できない推論要求だけ `502` | 解消 |
| N1 絶対 URI の例 | ブリッジの `POST http://api.anthropic.com/v1/environments/bridge` へ差し替え | 解消 |
| N2 異常終了時の残存ファイル | 次回起動が無条件に上書きする旨を追記 | 解消 |

B2 の優先規則の表を要件と突き合わせた。`None` + `true` が proxy 方式（FR-009）、`None` + `false`/未設定が Gateway モード（FR-017 の第2受入基準）、`Some(Off)` が旧キーより優先、`Some(Proxy)` が proxy 方式であり、4通りが重ならず要件の2条と食い違わない。

S3 の表も要件と整合する。ヘッダなしの推論要求を本物へ中継する行が FR-015 の第3受入基準に対応し、`502` で止めるのは claudex が起動したセッションでのみ起こる2通りに限られる。

観点別の所在: 完全性・一貫性・実現可能性・過剰設計のいずれも指摘なし。FR-001〜FR-019 のトレーサビリティは round1 で照合済みで、差分により FR-009・FR-015・FR-017 の対応先が更新されたことを確認した。

## 人間が最終判断すべき箇所

round1 で挙げた3点は設計のリスク節に記録された。承認の場で人間が確かめるべきは次の形になる。

1. **プロファイル名と合言葉がセッションの全子孫プロセスの環境変数に載ること**（リスク節の第2項）。同一利用者のローカルプロセスを信頼境界の内側と見なす前提を受け入れるかどうか。
2. **Claude Code の内部実装への依存が2つに増えること**（リスク節の第1項）。claude.ai ログインの判定に加え、userinfo を `Proxy-Authorization` として CONNECT に載せる挙動に依存する。どちらが変わっても Remote Control は止まる。
3. **`count_tokens` をローカル概算（4 バイト = 1 トークン）で返すこと**（リスク節の第5項）。自動圧縮の発動位置が前後する可能性を、精度より可用性を採る判断として受け入れるかどうか。
