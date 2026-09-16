---
type: design-review
slug: remote-control-mitm-proxy
round: 2
commit: working-tree（docs/ が gitignore のため design.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\design.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md（status: approved、FR-001〜FR-019 と受入基準）
---

前回指摘への対応一覧:

- B1 → 「利用者名の解決」節を新設した。CONNECT で1回だけ解析して TLS 接続の属性として保持し、その接続で終端した全要求へ適用する規則と、絶対 URI 形式は要求単位で解析する規則を表で分けた。構成要素の identity.rs の責務にも「解決結果の接続単位での保持」を追記
- B2 → `ProfileConfig::remote_control_mode` を `Option<RemoteControlMode>` に変更し、未設定を `None`、明示 `off` を `Some(Off)` として区別した。旧実装の 2 文は、4 通りを列挙した優先規則の表 1 つに置き換えた
- S1 → 振り分け規則にトークン数とレガシー補完の転送先の注記を追加し、Alternatives Considered の該当文を「クライアントの claude.ai 認証ヘッダを付けたまま本物へ中継する経路は使わない」へ書き換えた
- S2 → テスト戦略に「前提の証跡」の行を追加し、`examples/mitm_poc.rs` を userinfo 付き `HTTPS_PROXY` で駆動する形へ更新して実装の最初に置く旨を明記した。リスク節にも内部実装への依存が 2 つになる旨を追記
- S3 → 「利用者名の解決」節に解決結果 4 通りの表を置き、資格情報が付いているのに解決できない推論要求だけを `502` で止める規則にした。資格情報が付かない要求は FR-015 の第 3 受入基準どおり中継のままとした。Error Handling の該当文も同じ切り分けへ書き換えた
- N1 → API契約の絶対 URI 形式の例を `POST http://api.anthropic.com/v1/environments/bridge` に差し替えた
- N2 → 資格情報と証明書のライフサイクルに、異常終了で 2 ファイルが残った場合は次回起動が無条件に上書きする旨を追記した

レビュアーが挙げた「人間が最終判断すべき箇所」3 点は、Step 6 の人間承認で提示する。うち 1 点目（環境変数に載る資格情報）と 3 点目（内部実装への依存が 2 つ）はリスク節に明記した。
