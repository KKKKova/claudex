---
type: implement-review
slug: remote-control-mitm-proxy
round: 2
commit: a4e42541a367f94e3d78c87e243922eb582cc057
targets:
  - src/proxy/forward/handoff.rs
  - src/proxy/forward/integration_tests.rs
  - src/process/daemon.rs
  - src/proxy/mod.rs
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md
---

## 前回指摘への対応一覧

round1 の結果: `reviews/remote-control-mitm-proxy-implement-round1-result.md` @ `e4c170c`（REVISE、blocking 2件）

| 指摘 | 対応 | コミット |
|---|---|---|
| [required] proxy 停止時に CA 証明書の削除失敗を握り潰し、「消えた」と誤って断言する（`handoff.rs:52-64` / `daemon.rs:145-148,158-174`） | 修正 | `c31bed2` |
| [required] `relay_to` のヘッダ剥離に対する自動テストが皆無（`route.rs:270-285`） | 修正 | `a4e4254` |

suggestion 7件と nit 3件は本 round では未対応である。round1 結果の「まず required 2件のみ対応し、suggestion 以下は次round以降でよい」に従った。

### required 1 の対応内容

`handoff::cleanup()` のシグネチャを `pub fn cleanup() -> Vec<std::path::PathBuf>` に変え、消し残したファイルのパスを返すようにした。

- 新規の private `remove_if_present(path) -> std::io::Result<()>` が `ErrorKind::NotFound` を `Ok` に畳み、それ以外の `Err` はそのまま返す。`ENOENT` と権限拒否・ファイルロックが同じ枝に落ちる構造を解消した。
- 削除に失敗したパスは `tracing::warn!`（従来は `debug!`）にパスと理由を残したうえで戻り値に含める。
- `ca_pem_path()` / `handoff_path()` 自体が `Err` の場合は実パスが定まらないため戻り値に含めず、`tracing::warn!` のみ残す。
- `stop_proxy()` は戻り値が空のときだけ従来の `notice: the private CA ... is gone with the proxy` を出す。空でないときは `warning: could not remove the private CA for api.anthropic.com at <paths>. It is STILL on disk and can impersonate api.anthropic.com — delete it manually.` を出す。
- `proxy_status()` の stale PID 分岐と `start_proxy()` の正常終了経路は戻り値を捨てる。`cleanup()` 側がパスと理由を warn に出すため、呼び出し元で重ねていない。

テスト（`handoff.rs`）: `test_remove_if_present_deletes_existing`、`test_remove_if_present_ok_when_absent`。

### required 2 の対応内容

`src/proxy/forward/integration_tests.rs` に2件追加した。**生産コードは変更していない**（`git diff 4dc0b62..a4e4254 -- src/proxy/forward/route.rs` が空）。

`test_passthrough_does_not_leak_proxy_authorization`: 合言葉付きで CONNECT し、TLS 終端後に `/api/claude_code/settings` へ GET する。トンネル内のリクエストに `Proxy-Authorization`・`Proxy-Connection`・番兵 `x-sentinel-client: client-only` を載せる。上流役の受信ヘッダについて、`proxy-authorization` と `proxy-connection` が無いこと、`host` がクライアント送信値と異なること、番兵が残っていることを検査する。

`test_direct_anthropic_relay_replaces_client_credentials`: 同じく合言葉付きで `/v1/messages/count_tokens` へ POST し、クライアント側に `Authorization: Bearer client-supplied-token-should-not-leak` と `x-api-key: client-supplied-key-should-not-leak` を載せる。上流役の受信ヘッダについて、`proxy-authorization` が無いこと、`authorization` ヘッダ自体が無いこと、`x-api-key` が `sk-ant-api-test-alpha`（プロファイル自身の鍵）に厳密一致することを検査する。

ヘルパ `spawn_forward` の `alpha` プロファイルに `api_key = "sk-ant-api-test-alpha"` を設定した。`src/proxy/adapter/direct.rs` の `apply_auth` は `sk-ant-oat` 以外の鍵を `x-api-key` 経路へ回し `authorization` を付けないため、この鍵形で経路を確定させている。

剥離条件を無効化するとどちらのテストも失敗することを、実装側の一時改変で確認した（改変は復元済み、`git diff` が空）。

## 検証結果（コミット `a4e4254` 時点）

| コマンド | 結果 |
|---|---|
| `cargo test --bin claudex` | 446 pass / 3 fail |
| `cargo test --bin claudex forward::integration_tests` | 12 pass |
| `cargo test --bin claudex proxy::forward::handoff` | 3 pass |
| `cargo clippy --all-targets -- -D warnings` | warning ゼロ |
| `cargo fmt --check` | 差分なし |

失敗3件は `terminal::osc8::tests` の既知の事前障害（round1 依頼で既報）。

## round1 から継続する情報

`docs/` は `.gitignore` により追跡対象外である。`git show <SHA>:docs/...` では開けないので作業ツリーのファイルを直接読むこと。参照文書の一覧・設計の改訂（rev1 / rev2）・未裁定の論点2件（FR-010、FR-006 第1）・実行環境の注記は `reviews/remote-control-mitm-proxy-implement-round1.md` に記載しており、いずれも round1 から変更はない。
