---
type: requirements-review-result
slug: remote-control-mitm-proxy
rev: 1
round: 1
verdict: REVISE
blocking: 1
status: in-review
reviewed_at: 2026-09-17
reviewed_commit: a468a5f
reviewed_file: docs/specs/remote-control-mitm-proxy/requirements.md
reviewed_sha256: 343a188d254f11488b8a8056d305305a1e12c49609cbeafae5026cc94ad429aa
---

# レビュー結果: requirements remote-control-mitm-proxy rev1-round1

**REVISE** — blocking 1件（ほかに suggestion 2件）。
次のアクション: FR-013 が名指しする環境変数を `HTTPS_PROXY` 1つから、claudex 自身が子プロセスへ撒く変数を含む集合へ広げる。
最重要の根拠: claudex は子プロセスへ `HTTPS_PROXY` と小文字の `https_proxy` の両方を設定する。`HTTPS_PROXY` だけを無視の対象にすると、セッション内から proxy を起動した場合に小文字側で自己ループが成立する。

レビュー範囲は改訂の3箇所（FR-013 の要件文、受入基準2項目、Edge Cases 1項目）と、承認済み記述との整合に限った。FR-013 以外の FR・Success Criteria・非機能要件・Non-Goals に差分が無いことは確認した。

改訂そのものの方向は承認済みの記述と矛盾しない。「自分自身を指すときだけ無視する」という人間裁定は、Edge Cases の追加項と受入基準2項目に素直に落ちている。

## blocking（1件）

### `B1 [重大] 無視の対象が HTTPS_PROXY だけで、claudex 自身が撒く小文字版から自己ループが成立する — FR-013 / 受入基準 FR-013`

<details>
<summary>詳細</summary>

改訂前の FR-013 は「環境変数 `HTTPS_PROXY` を参照しない」であり、実装は全プロキシ設定を一括で無効化するため、変数名の網羅性は問題にならなかった。改訂後は「自身の forward proxy を指す場合にこれを無視し、それ以外は尊重する」となり、**どの変数を見るか**が結果を分ける。

承認済み design の「子プロセスに渡す環境変数」は、`HTTPS_PROXY` と `https_proxy` の両方に同じ値（`http://<プロファイル名>:<合言葉>@127.0.0.1:<port>`）を設定すると定めている。理由も「大文字と小文字のどちらを読むかがクライアントによって異なるため」である。したがって `claudex run` したセッションの中から `claudex proxy start` を打つと、proxy プロセスは両方を継承する。

このとき上流クライアントが小文字側を読むと、推論要求が自分自身の forward proxy へ入る。継承した値には userinfo が入っているため利用者名の解決にも成功し、要求は `handle_messages` へ渡り、その上流接続がまた自分自身へ入る。応答が返らないまま再帰する。

受入基準も同じ穴を持つ。第1項が `HTTPS_PROXY` しか Given に置いていないため、小文字版や `ALL_PROXY` を設定した状態は検証されない。

対応: FR-013 の要件文と受入基準で、対象を「上流クライアントが読むプロキシ関連の環境変数（大文字・小文字の両表記を含む）」と書く。少なくとも claudex 自身が設定する `HTTPS_PROXY` / `https_proxy` の両方が Given に現れる形にしたい。
</details>

## suggestion（2件、verdict に影響しない）

### `S1 [中] 会社プロキシを尊重すると決めた結果、CONNECT 素通し経路の扱いが未定義になった — FR-013 / FR-004`

<details>
<summary>詳細</summary>

改訂は「会社のプロキシなど他のアドレスを指す場合は、社内から外へ出る唯一の経路でありうるため尊重する」と書き、会社プロキシ配下の利用を支援対象に含めた。

FR-004 は `api.anthropic.com` 以外への CONNECT を「素通しで中継する」と定める。素通しは宛先へ直接 TCP を張る動作であり、社外へ直接出られない環境では失敗する。上流への接続だけ会社プロキシを尊重しても、素通しトンネル側は塞がったままになる。会社プロキシ配下の利用者から見ると、推論は通るのに Claude Code の他の通信が全滅する。

要件として素通し経路のチェーンまで求めるのか、会社プロキシ配下は上流接続のみ支援して素通しは Non-Goal に落とすのかを、1行決めておきたい。
</details>

### `S2 [中] 「自身を指す」の判定基準が表記のゆれを吸収できるか読み取れない — 受入基準 FR-013 / Edge Cases`

<details>
<summary>詳細</summary>

Edge Cases は「アドレスで判別して無視する」と書く。forward proxy の実体は `127.0.0.1:<forward_proxy_port>` だが、利用者が書く値は `http://localhost:13457` や `http://[::1]:13457` にもなりうる。この3つを同一と見なすのかどうかで、自己ループを塞げる範囲が変わる。

判定をポート番号の一致だけに寄せるのか、ループバックの別名を解決するのかを受入基準に1文足すと、設計が迷わない。
</details>

## 人間が最終判断すべき箇所

1. **会社プロキシ配下をどこまで支援するか**（S1 に直結）。上流接続だけ通せばよいのか、Claude Code の他の通信（GitHub、npm など）も claudex 経由で通したいのかで、必要な実装量が変わる。後者を選ぶと素通しトンネルを会社プロキシへチェーンする設計が要る。
2. **自己ループを塞ぐ範囲**（B1 の直し方）。claudex が撒く2変数だけを見るのか、`ALL_PROXY` を含むプロキシ関連の環境変数すべてを見るのかは、守りの広さと実装の単純さの交換である。
3. **この改訂が承認済み design と競合していること**。design の `ForwardState::client` は `.no_proxy()` で全プロキシ設定を無効化すると書かれており、改訂後の FR-013 とは両立しない。design rev1 の依頼で整合を確認するが、両文書を同時に承認する順序は人間が決めてほしい。
