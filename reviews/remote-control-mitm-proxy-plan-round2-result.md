---
type: plan-review-result
slug: remote-control-mitm-proxy
round: 2
verdict: APPROVE
blocking: 0
status: approved
reviewed_at: 2026-09-17
reviewed_commit: a468a5f
reviewed_file: docs/specs/remote-control-mitm-proxy/plan.md
reviewed_sha256: eb38b4cd70f1211ad202ab29c2150a400f53aa7b091777153faf3acf21b304ac
---

# レビュー結果: plan remote-control-mitm-proxy round2

**APPROVE** — blocking 0件（nit 1件）。
次のアクション: この計画で実装に進んでよい。plan.md の `status` を `approved` にするのは依頼者側の作業である。
最重要の根拠: round1 の blocking 3件はいずれも、既存コードの実物と突き合わせて解消を確認した。とくに B2 が指す `DirectAnthropicAdapter`（`src/proxy/adapter/direct.rs:21`）と `resolve_api_keys`（`src/config/mod.rs:585`）は実在する。

round2 は修正差分に限定して確認した。

## round1 指摘の対応確認

| 指摘 | 対応 | 判定 |
|---|---|---|
| B1 推論経路の上流クライアント | 人間裁定を経て FR-013 を改訂（rev1 系列）。T003 に `proxy_url_points_at_port` と `env_proxy_points_at_self` を新設し、T006 Steps 5 で `ProxyState::http_client`（現行66〜68行）を同じ規則へ揃えた。検証計画に FR-013 の2項目を追加 | 解消 |
| B2 count_tokens の認証ヘッダ | `x-api-key` 固定をやめ、`DirectAnthropicAdapter::apply_auth` を通す形へ。`api_key_keyring` が解決済みである旨も明記 | 解消 |
| B3 T008 の完了条件 | grep を2本に分割し、`ANTHROPIC_UNIX_SOCKET` 側は `grep -v env_remove` を通す形へ。残る理由（FR-008 第1受入基準）も併記 | 解消 |
| S1 2プロファイルの誤配テスト | `test_two_profiles_do_not_cross_route` を追加し、`spawn_forward` を `alpha_base` / `beta_base` の2引数へ。検証計画の FR-015 行にも反映 | 解消 |
| S2 停止時の告知 | T008 Steps 5 に追加。`proxy_status` 側に出さない理由も明記 | 解消 |
| N1 unwrap | `forward_state` と `forward_secret` を変数に控える形へ | 解消 |
| N2 ProxyState の必須フィールド | 9フィールドを列挙し、`http_client` と `token_manager` の組み方を明記 | 解消 |
| N3 FR-002 の検証 | 「秘密鍵ファイルが1件も無い」を見る形へ | 解消 |

B1 の修正は要件・設計の改訂と揃っている。`proxy_url_points_at_port` が `127.0.0.1` に加えて `localhost` と `::1` を受けるため、design rev1 のレビューで suggestion として挙げたループバック別名の穴は、plan の側で先に塞がっている。

## nit（1件）

<details>
<summary>nit 1件</summary>

- `N1 [軽] 統合テストの ForwardState::client が開発機のプロキシ設定を拾う — T009 Steps 1。ヘルパは ProxyState::http_client を .no_proxy() で組むが、ForwardState::new(0) が作る client は env_proxy_points_at_self(0) が偽になるため環境変数を尊重する。開発機のシェルに HTTPS_PROXY が設定されていると、上流役の wiremock（127.0.0.1）への中継がそのプロキシへ向かい、Passthrough 系のテストが落ちうる。ヘルパ内で ForwardState の client を差し替えるか、NO_PROXY の設定を前提として注記しておくと安定する`
</details>

## 人間が最終判断すべき箇所

1. **実装の入口が T001（PoC の証跡化）であること**。この設計は「Claude Code が `HTTPS_PROXY` の userinfo を CONNECT に載せる」という実測に全面依存する。T001 の結果が想定と違えば、以降のタスクは組み替えになる。T001 の出力を人間が一度見てから T002 以降へ進める運用を勧める。
2. **会社プロキシ配下での素通しトンネル**（requirements rev1 の S1、design rev1 の S2 と同じ論点）。上流接続は他のプロキシを尊重するようになったが、`api.anthropic.com` 以外への CONNECT は宛先へ直接 TCP を張る。社外へ直接出られない環境では Claude Code の他の通信が失敗する。この計画では扱っていないため、Non-Goal として明示するかどうかを決めてほしい。
3. **実機検証が Windows のみであること**。e2e 手順は Windows 前提で書かれており、macOS / Linux は未検証のまま出る。README の但し書き（T010 Steps 4）の文面で足りるかを人間が確かめてほしい。
