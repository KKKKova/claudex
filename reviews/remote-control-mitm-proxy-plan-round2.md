---
type: plan-review
slug: remote-control-mitm-proxy
round: 2
commit: working-tree（docs/ が gitignore のため plan.md は非追跡。参照は作業ツリーの実ファイル）
targets:
  - C:\Users\ASMTPC-076\workspace\claudex\docs\specs\remote-control-mitm-proxy\plan.md
acceptance: docs/specs/remote-control-mitm-proxy/requirements.md（FR-013 は改訂系列 rev1 で改訂中。reviews/remote-control-mitm-proxy-requirements-rev1-round1.md）
---

前回結果: `reviews/remote-control-mitm-proxy-plan-round1-result.md`（round1、REVISE、blocking 3件）

## 前回指摘への対応一覧

| 指摘 | 対応 |
|---|---|
| B1 | 人間裁定を実施し、FR-013 を「環境変数の指す先が claudex 自身の forward proxy のときだけ無視し、他は尊重する」に改訂した（requirements / design を rev1 系列で同時改訂）。plan では T003 に純粋関数 `proxy_url_points_at_port` と `env_proxy_points_at_self` を足し、`ForwardState::new(forward_port)` で分岐させた。T006 Steps 5 に `ProxyState::http_client` を同じ規則で構築する変更を足した。単体テスト5本と検証計画の FR-013 行（2項目）を追加。冒頭「設計の解釈」節に裁定内容を記録 |
| B2 | T005 Steps 4 の `CountTokens` と `LegacyComplete` を、`x-api-key` 固定から `DirectAnthropicAdapter::apply_auth(builder, profile)` を通す形に変更した。`api_key_keyring` が `resolve_api_keys` で解決済みであることも明記 |
| B3 | T008 の完了条件を2本に分けた。`uds_windows\|socket_path` は0件、`ANTHROPIC_UNIX_SOCKET` は `grep -v env_remove` を通して0件とし、`src/process/launch.rs` の `env_remove` が FR-008 の要求で残る理由を併記した |
| S1 | T009 に `test_two_profiles_do_not_cross_route` を追加（10本目）。`spawn_forward` の引数を `alpha_base` / `beta_base` の2つに変更。検証計画の FR-015 第1・第2 行にこのテストを追加 |
| S2 | T008 Steps 5 に proxy 停止時の告知を追加。`proxy_status` 側には出さない理由も明記 |
| N1 | T006 Steps 5 で `forward_state` と `forward_secret` を変数に控える形にし、`unwrap()` を削除 |
| N2 | T009 Steps 1 に `ProxyState` の必須9フィールドを列挙し、`http_client` と `token_manager` の組み方を明記 |
| N3 | 検証計画の FR-002 行と e2e 手順 Step 2 を「秘密鍵ファイルが1件も無い」を見る形に変更し、`proxy.pid` の存在を注記 |

## 人間判断項目への回答（前回結果の末尾3点）

1. B1 の直し方 → 上表のとおり裁定済み
2. `count_tokens` の自動テスト範囲 → 既存単体テスト `oauth_token_uses_bearer_and_no_beta_header`（`src/proxy/adapter/direct.rs:87`、実在を確認済み）に委ね、実鍵を要する確認は e2e 手順 Step 9 に追加した
3. macOS / Linux 未検証の但し書き → T010 Steps 4 で README の Remote Control 節末尾に置くことにした
