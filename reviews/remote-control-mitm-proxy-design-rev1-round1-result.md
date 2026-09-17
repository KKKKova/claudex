---
type: design-review-result
slug: remote-control-mitm-proxy
rev: 1
round: 1
verdict: APPROVE
blocking: 0
status: approved
reviewed_at: 2026-09-17
reviewed_commit: a468a5f
reviewed_file: docs/specs/remote-control-mitm-proxy/design.md
reviewed_sha256: 710e57a58adbfe8506dc9add63779856d2b29f99f20d457188fea84101067105
---

# レビュー結果: design remote-control-mitm-proxy rev1-round1

**APPROVE** — blocking 0件（suggestion 2件）。
次のアクション: この改訂で plan round2 の審査へ進んでよい。suggestion は plan 側で拾っても設計へ戻しても構わない。
最重要の根拠: plan round1 の B1（推論の上流接続が `ProxyState::http_client` を使うのに FR-013 の対象外だった）は、両クライアントを名指しする段落の追加で解消した。

レビュー範囲は改訂の2箇所（`ForwardState` 表の `client` 行と直後の段落、トレーサビリティの FR-013 行）と、改訂された FR-013 との整合に限った。ほかの節に差分が無いことは節構成と本文で確認した。

## 改訂の確認

| 確認点 | 結果 |
|---|---|
| plan round1 B1 の解消 | `ProxyState::http_client`（`src/proxy/mod.rs`）を FR-013 の対象に明記。トレーサビリティ行も両クライアントを指す形へ更新 | 
| 改訂後 FR-013 との整合 | 「自身の forward proxy を指す場合だけ無視し、それ以外は尊重する」と一致 |
| 判別条件の具体性 | `127.0.0.1:<forward_proxy_port>` の host と port の一致、と実装可能な形で書かれている |
| 要件より広い記述 | 設計は `HTTPS_PROXY` と `https_proxy` の両表記を対象にする。要件は `HTTPS_PROXY` のみを書くが、設計が広い側なので矛盾ではない |

最後の行は requirements rev1 のレビューにも影響した。当初そちらで「小文字版から自己ループが成立する」を blocking として出したが、設計が両表記を担保しているため実装上の穴にはならない。requirements rev1-round1 の結果を APPROVE へ訂正済みである（@ 68112a9）。

## suggestion（2件、verdict に影響しない）

### `S1 [中] ループバックの別名表記を自己判定で吸収できない — データモデル（FR-013 の段落）`

<details>
<summary>詳細</summary>

判別は `127.0.0.1:<forward_proxy_port>` との host・port 一致で行うと書かれている。利用者が `HTTPS_PROXY=http://localhost:13457` や `http://[::1]:13457` と書いた場合、指す先は同じ forward proxy だが「自身ではない」と判定され、自己ループが残る。

claudex 自身が撒く値は `127.0.0.1` 形式なので主要経路は塞がる。残るのは利用者が手で書いた場合に限られるため blocking にはしない。判別を「ループバックの別名（`localhost`・`::1`）を含めて一致とみなす」と1文広げれば消える。
</details>

### `S2 [中] reqwest が読む他のプロキシ変数が自己判定の対象外である — データモデル（FR-013 の段落）`

<details>
<summary>詳細</summary>

段落が挙げるのは `HTTPS_PROXY` と `https_proxy` の2つである。reqwest は `ALL_PROXY` / `all_proxy` も読むため、そこに自身の forward proxy を書かれた場合は無視されず自己ループになる。

claudex はこの変数を撒かないので発生は利用者の設定次第であり、実害は小さい。「自身を指すプロキシ設定は種類を問わず無視する」と書くか、対象を3種に広げるかを決めておくと、実装が判断を迫られない。
</details>

## 人間が最終判断すべき箇所

1. **自己判定をどこまで広げるか**（S1・S2 に共通）。守りを広げるほど実装は増え、「利用者が明示したプロキシ設定を claudex が勝手に無視する」範囲も広がる。claudex が撒く形だけを塞ぐ現行案でよいかを決めてほしい。
2. **会社プロキシ配下での素通しトンネルの扱い**。requirements rev1 の S1 と同じ論点である。上流接続は会社プロキシを尊重するようになったが、`api.anthropic.com` 以外への CONNECT は宛先へ直接 TCP を張るため、社外へ直接出られない環境では失敗する。設計として扱うか Non-Goal に落とすかは人間の判断である。
