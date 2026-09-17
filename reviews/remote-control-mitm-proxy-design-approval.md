---
type: design-approval
slug: remote-control-mitm-proxy
approved_at: 2026-09-16
approved_by: human
design: docs/specs/remote-control-mitm-proxy/design.md
review_result: reviews/remote-control-mitm-proxy-design-round2-result.md
---

# 承認記録: design remote-control-mitm-proxy

design-review round2 が APPROVE（blocking 0件）。人間が design.md を承認し、`status: approved` に更新した。

承認の場で提示した確認事項3点は、いずれも受け入れたうえでの承認である。

1. プロファイル名と合言葉がセッションの全子孫プロセスの環境変数に載ること（同一利用者のローカルプロセスを信頼境界の内側と見なす前提）
2. Claude Code の内部実装への依存が2つに増えること（claude.ai ログインの判定、userinfo の `Proxy-Authorization` 転送）
3. `count_tokens` をローカル概算（4 バイト = 1 トークン）で返すこと

対話で合意した設計判断3件は design.md の Alternatives Considered に却下案として記録済み。

次フェーズ: /plan
