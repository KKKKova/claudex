---
type: implement-review-result
slug: remote-control-mitm-proxy
round: 2
commit: a4e42541a367f94e3d78c87e243922eb582cc057
verdict: APPROVE
blocking: 0
status: approved
reviewed_at: 2026-09-18
---

# implement remote-control-mitm-proxy round2 — APPROVE（blocking 0件）

対象: `4dc0b62..a4e4254`（`c31bed2` required-1 対応 / `a4e4254` required-2 対応）
次のアクション: この実装で先へ進んでよい。suggestion 7件・nit 3件は round1 結果の指示どおり未対応のまま残っており、対応するかは依頼側の裁量である。

## round1 required の対応確認

| 指摘 | 対応 | 判定 |
|---|---|---|
| CA 証明書削除失敗の握り潰し | `remove_if_present` が `ENOENT` とそれ以外の `io::Error` を区別し、`cleanup()` が消し残しパスを `Vec<PathBuf>` で返す形へ変更。`stop_proxy()` は空でないときに「消えた」と言わず、パスを挙げて手動削除を促す警告へ切り替えた（`c31bed2`、`src/proxy/forward/handoff.rs:52-84`、`src/process/daemon.rs:145-160`） | 解消 |
| `relay_to` のヘッダ剥離に対する自動テストが皆無 | `test_passthrough_does_not_leak_proxy_authorization` と `test_direct_anthropic_relay_replaces_client_credentials` を追加。上流役の受信ヘッダを直接検査し、`proxy-authorization`／`proxy-connection` の非流出、`host` の付け替え、無関係ヘッダの温存、`authorization` の完全不在、`x-api-key` のプロファイル鍵への厳密置換までを検証する（`a4e4254`） | 解消 |

両コミットを実物で確認した。`cargo test --bin claudex proxy::forward`（35件）はいずれも pass。剥離条件を無効化すると新規2件が red になることは依頼側の申告どおりで、テスト内容（ヘッダの有無とホスト検査の粒度）からも矛盾はない。

`cleanup()` のシグネチャ変更に伴う呼び出し元（`proxy_status()` の stale PID 分岐、`start_proxy()` の正常終了経路）は戻り値を意図的に無視しているが、`cleanup()` 自身が `tracing::warn!` でパスと理由を出すため、二重報告を避けた設計として妥当である。新規に握り潰しが生じていないことを確認した。

## 継続する残項目（round1から変更なし）

suggestion 7件・nit 3件は本round で未対応。round1結果ファイルに詳細がある。「人間が最終判断すべき箇所」3点（FR-010/FR-006の受入基準と実装前提のズレ、`forward.json` の権限、中継先ホスト未検証の設計判断）も引き続き未裁定である。
